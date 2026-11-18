//! Signing a person in to TMDB — the token flow, never a password.
//!
//! An API **key** belongs to an application: it is what the catalogs and the
//! metadata use, and it is asked for first (Settings → Watch → Sources). A
//! **session** belongs to a person: TMDB hands out a request token, the person
//! approves it on its own site, and the token becomes a session id that names
//! their account (watchlist, favourites, rated lists). Nothing here replaces the
//! key — every call below is authenticated with it.
//!
//! The approval happens in the person's browser, so their password, a 2FA code
//! and any bot check stay on TMDB's page, where they belong. This crate never
//! sees any of them: the session id is the only secret that comes back, and it
//! is kept beside the key in the credential store.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use heronite_watch_core::http::{http_status, Transport};
use heronite_watch_core::provider::Secret;

use super::API;

/// Where a request token is approved (the token follows).
pub const APPROVE: &str = "https://www.themoviedb.org/authenticate";

/// The person's TMDB account, once they approved a token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// The session id every account call takes.
    pub id: String,
    /// Whose account it is, when TMDB said: what "Signed in as …" shows.
    pub username: String,
    /// TMDB's numeric id for the account, which its account endpoints take in
    /// the path (`/account/{id}/movie/recommendations`). `0` when it was not
    /// known — a session kept by an older build — and then it is asked for.
    pub account_id: u64,
}

/// The account a session names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: u64,
    pub username: String,
}

/// What asking for a session came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The token is real but nobody approved it yet: the person has the page
    /// open, or closed it. Asking again is safe.
    Pending,
    Approved(Session),
}

/// The page the person approves `token` on.
pub fn approve_url(token: &str) -> String {
    format!("{APPROVE}/{}", token.trim())
}

/// Asks TMDB for a request token, which the person then approves in a browser.
/// The key travels as `api_key`, like every other call: the transport carries
/// no headers.
pub fn start(http: &dyn Transport, key: &str) -> Result<String> {
    let mut url = Url::parse(&format!("{API}/authentication/token/new"))?;
    url.query_pairs_mut().extend_pairs([("api_key", key)]);
    let body = match http.get(&url) {
        Ok(body) => body,
        // Nothing but the key is in this request, so a refusal is the key.
        // Kept as a context, not a fresh error: `key_rejected` reads the status.
        Err(error) if http_status(&error) == Some(401) => {
            return Err(error).context(
                "TMDB refused this app's key (HTTP 401): use the v3 API key from themoviedb.org → Settings → API (32 characters); a v4 read token is not accepted.",
            )
        }
        Err(error) => return Err(error).context("TMDB sign-in"),
    };
    let answer: TokenAnswer = serde_json::from_slice(&body).context("TMDB sign-in")?;
    let token = answer.request_token.trim().to_owned();
    if token.is_empty() {
        anyhow::bail!("TMDB answered no request token");
    }
    Ok(token)
}

/// Turns an approved request token into a session and asks whose it is.
///
/// A refusal here is not a bad key: the key was accepted a moment ago for
/// [`start`], so TMDB is saying the token has not been approved yet.
pub fn finish(http: &dyn Transport, key: &str, token: &str) -> Result<Step> {
    let mut url = Url::parse(&format!("{API}/authentication/session/new"))?;
    url.query_pairs_mut().extend_pairs([("api_key", key), ("request_token", token.trim())]);
    let body = match http.get(&url) {
        Ok(body) => body,
        Err(error) if http_status(&error) == Some(401) => return Ok(Step::Pending),
        Err(error) => return Err(error).context("TMDB sign-in"),
    };
    let answer: SessionAnswer = serde_json::from_slice(&body).context("TMDB sign-in")?;
    let id = answer.session_id.trim().to_owned();
    if id.is_empty() {
        anyhow::bail!("TMDB answered no session");
    }
    // Signing in worked; the name and the account id are not worth failing it
    // for (a rail that needs the id asks for it later, once).
    let account = account(http, key, &id).unwrap_or_else(|error| {
        log::debug!("TMDB account: {error:#}");
        Account { id: 0, username: String::new() }
    });
    Ok(Step::Approved(Session { id, username: account.username, account_id: account.id }))
}

/// The account a session names: its id (what the account endpoints take in the
/// path) and the name to show (`""` when TMDB does not say).
pub fn account(http: &dyn Transport, key: &str, session: &str) -> Result<Account> {
    let mut url = Url::parse(&format!("{API}/account"))?;
    url.query_pairs_mut().extend_pairs([("api_key", key), ("session_id", session.trim())]);
    let body = http.get(&url).context("TMDB account")?;
    let answer: AccountAnswer = serde_json::from_slice(&body).context("TMDB account")?;
    Ok(Account { id: answer.id.unwrap_or(0), username: answer.username.or(answer.name).unwrap_or_default().trim().to_owned() })
}

/// Whether `error` is TMDB refusing the API key itself (a key that was reset,
/// or a paste that lost a character). The message names the host only.
pub fn key_rejected(error: &anyhow::Error) -> bool {
    http_status(error) == Some(401)
}

/// The `tmdb` credential with a session in it: the key stays, so signing out
/// never means typing it again.
pub fn secret_with_session(api_key: &str, session: &Session) -> Secret {
    Secret::new(serde_json::json!({
        "apiKey": api_key,
        "sessionId": session.id,
        "username": session.username,
        "accountId": session.account_id,
    }))
}

/// The v4 read access token in the `tmdb` credential, when one was set. v4 is
/// the only place TMDB serves the account's recommendations, and it takes this
/// token as a `Bearer` header rather than the v3 key in the query.
pub fn access_token_of(secret: &Secret) -> Option<String> {
    let token = secret.value().get("accessToken").and_then(|token| token.as_str())?.trim().to_owned();
    (!token.is_empty()).then_some(token)
}

/// The account's **object id**: the 24-character id v4 puts in the path. It is
/// the token's `sub` claim, so it is read from the token rather than asked for —
/// `/4/account` itself is 404 for this app, and the numeric v3 id is a different
/// value (`/4/account/{numeric}` is a 404 too).
pub fn account_object_id(read_token: &str) -> Option<String> {
    let payload = read_token.split('.').nth(1)?;
    let bytes = base64url(payload)?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let sub = claims.get("sub")?.as_str()?.trim().to_owned();
    (!sub.is_empty()).then_some(sub)
}

/// base64url without padding, which is what a JWT segment is. Written out rather
/// than pulling a dependency in for twenty lines.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => continue,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// The same credential with a v4 read token kept beside the key and the session.
pub fn secret_with_access_token(secret: &Secret, token: &str) -> Secret {
    let mut value = secret.value().clone();
    if let Some(map) = value.as_object_mut() {
        map.insert("accessToken".to_owned(), serde_json::Value::String(token.trim().to_owned()));
    }
    Secret::new(value)
}

/// The session in the `tmdb` credential, when one was signed in. A credential
/// with the key alone, or one whose session id is empty, has none.
pub fn session_of(secret: &Secret) -> Option<Session> {
    let value = secret.value();
    let id = value.get("sessionId").and_then(|id| id.as_str())?.trim().to_owned();
    if id.is_empty() {
        return None;
    }
    let username = value.get("username").and_then(|name| name.as_str()).unwrap_or_default().trim().to_owned();
    let account_id = value.get("accountId").and_then(|id| id.as_u64()).unwrap_or(0);
    Some(Session { id, username, account_id })
}

#[derive(Deserialize)]
struct TokenAnswer {
    #[serde(default)]
    request_token: String,
}

#[derive(Deserialize)]
struct SessionAnswer {
    #[serde(default)]
    session_id: String,
}

#[derive(Deserialize)]
struct AccountAnswer {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    fn base64url_of(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let mut buffer = 0u32;
            for (index, byte) in chunk.iter().enumerate() {
                buffer |= (*byte as u32) << (16 - 8 * index);
            }
            for index in 0..chunk.len() + 1 {
                out.push(ALPHABET[((buffer >> (18 - 6 * index)) & 0x3f) as usize] as char);
            }
        }
        out
    }

    #[test]
    fn the_tokens_sub_claim_is_the_account_object_id() {
        // A made-up id: the claim is all this reads, and nobody's account is in
        // the repository.
        let sub = "0123456789abcdef01234567";
        let token = format!("header.{}.signature", base64url_of(format!(r#"{{"sub":"{sub}"}}"#).as_bytes()));
        assert_eq!(account_object_id(&token).as_deref(), Some(sub));
        assert_eq!(account_object_id("not-a-jwt"), None);
        assert_eq!(account_object_id("header.e30.sig"), None, "no sub claim");
    }

    use super::*;
    use heronite_watch_core::http::HttpStatus;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Answers by path; a path mapped to `None` is a 401.
    struct Fake {
        answers: HashMap<&'static str, Option<&'static str>>,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            let mut answers = HashMap::new();
            answers.insert("/3/authentication/token/new", Some(r#"{"success":true,"request_token":"tok123"}"#));
            answers.insert("/3/authentication/session/new", Some(r#"{"success":true,"session_id":"sess456"}"#));
            answers.insert("/3/account", Some(r#"{"id":42,"username":"cinephile"}"#));
            Self { answers, seen: Mutex::new(Vec::new()) }
        }

        fn without(mut self, path: &'static str) -> Self {
            self.answers.insert(path, None);
            self
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            match self.answers.get(url.path()) {
                Some(Some(body)) => Ok(body.as_bytes().to_vec()),
                Some(None) => Err(anyhow::Error::new(HttpStatus::new(url, 401))),
                None => Err(anyhow::anyhow!("the fake has no answer for {}", url.path())),
            }
        }

        fn post_json(&self, _: &Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            anyhow::bail!("signing in needs no POST")
        }
    }

    #[test]
    fn a_token_is_asked_for_with_the_key_and_approved_in_the_browser() {
        let fake = Fake::new();
        let token = start(&fake, "key1").unwrap();
        assert_eq!(token, "tok123");
        assert_eq!(approve_url(&token), "https://www.themoviedb.org/authenticate/tok123");
        let seen = fake.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].contains("api_key=key1"), "the key travels as a query, like every call: {}", seen[0]);
    }

    #[test]
    fn an_unapproved_token_is_pending_not_a_failure() {
        let fake = Fake::new().without("/3/authentication/session/new");
        assert_eq!(finish(&fake, "key1", "tok123").unwrap(), Step::Pending);
    }

    #[test]
    fn an_approved_token_becomes_a_named_session() {
        let fake = Fake::new();
        let step = finish(&fake, "key1", "tok123").unwrap();
        assert_eq!(step, Step::Approved(Session { id: "sess456".into(), username: "cinephile".into(), account_id: 42 }));
    }

    #[test]
    fn a_session_without_a_name_is_still_a_session() {
        let mut fake = Fake::new();
        fake.answers.insert("/3/account", None);
        let step = finish(&fake, "key1", "tok123").unwrap();
        assert_eq!(step, Step::Approved(Session { id: "sess456".into(), username: String::new(), account_id: 0 }));
    }

    #[test]
    fn a_refused_key_is_recognised() {
        let fake = Fake::new().without("/3/authentication/token/new");
        let error = start(&fake, "wrong").unwrap_err();
        assert!(key_rejected(&error), "a 401 on the key's own call is the key: {error:#}");
        assert!(!error.to_string().contains("wrong"), "the key never reaches the message");
    }

    #[test]
    fn the_credential_keeps_the_key_and_the_session() {
        let session = Session { id: "sess456".into(), username: "cinephile".into(), account_id: 42 };
        let secret = secret_with_session("key1", &session);
        assert_eq!(session_of(&secret), Some(session));
        // A key alone has no session, and neither has an empty one.
        assert_eq!(session_of(&super::super::secret("key1")), None);
        let empty = Secret::new(serde_json::json!({"apiKey": "key1", "sessionId": "  "}));
        assert_eq!(session_of(&empty), None);
        // The key is still read by the catalog path from the same credential.
        let key = secret.decode::<super::super::Key>().expect("the catalog's key shape still parses");
        assert_eq!(key.api_key, "key1");
    }
}
