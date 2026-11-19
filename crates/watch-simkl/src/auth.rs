//! Simkl's device ("PIN") flow — how a TV signs in without a keyboard.
//!
//! The application holds a **client id** (asked for once, see [`crate`]); the
//! person's part is a short code, shown on screen as a QR and typed on a phone:
//!
//! 1. [`start`] asks Simkl for a device code and the page that approves it. The
//!    page pre-fills the code (`verification_uri_complete`), which is what the QR
//!    carries.
//! 2. [`poll`] asks again every interval until the person approves. Simkl says
//!    "not yet" with `authorization_pending`, which is a normal step, not an
//!    error — and it says it forever when the person declined, so the caller's
//!    own deadline (`expires_in`) is what ends the loop.
//!
//! The device code is the polling credential: it is never shown and never
//! logged. The tokens that come back are, like TMDB's session, kept in the
//! credential store and never in a message.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use url::Url;

use heronite_watch_core::http::{http_status, HttpStatus, Transport};
use heronite_watch_core::provider::Secret;

/// Simkl's API root.
pub const API: &str = "https://api.simkl.com";
/// The page a code is approved on, when Simkl does not send a complete URL.
pub const VERIFY: &str = "https://simkl.com/pin";
/// What the app asks to do: read the library, and write back what was watched
/// (simkl silently downgrades an unrecognised scope to read-only, so the answer's
/// own scope is what [`Session`] reports).
pub const SCOPE: &str = "media:read media:write";
/// The seconds a `slow_down` adds to the interval.
pub const SLOW_DOWN: u64 = 5;
/// The token lifetime Simkl documents, for an answer that omits `expires_in`.
const ACCESS_TOKEN_SECONDS: u64 = 7 * 24 * 60 * 60;

const DEVICE: &str = "/oauth2/device";
const TOKEN: &str = "/oauth2/token";
/// The grant the polling endpoint takes.
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The grant that trades a refresh token for a new pair.
const REFRESH_GRANT: &str = "refresh_token";

/// What Simkl handed out for one sign-in attempt: the credential to poll with,
/// the code to show, and how often/how long.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceCode {
    /// The polling credential. Never shown, never logged.
    pub device_code: String,
    /// `XXXX-YYYY`, shown to the person (hyphen included).
    pub user_code: String,
    /// The page that pre-fills the code: what the QR carries.
    pub verification_url: String,
    /// How long to wait between polls, in seconds.
    pub interval_secs: u64,
    /// How long the code lives, in seconds: the caller's deadline.
    pub expires_in: u64,
}

/// The polling credential is a secret: it is never printed, not even by a log
/// line that formats the whole value.
impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("device_code", &"<redacted>")
            .field("user_code", &self.user_code)
            .field("verification_url", &self.verification_url)
            .field("interval_secs", &self.interval_secs)
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// What one poll came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// The person has not approved it yet (or declined: Simkl never says).
    Pending,
    /// Polled too fast: wait [`SLOW_DOWN`] more seconds and ask again.
    SlowDown,
    /// Approved, with the tokens every later call takes.
    Approved(Tokens),
}

/// The person's Simkl account, once a code was approved.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    /// What a later slice exchanges for a fresh access token: the access token
    /// itself only lives seven days.
    pub refresh_token: String,
    pub scope: String,
    /// When the access token stops working, so a caller knows to refresh before
    /// it is refused.
    pub expires_at: DateTime<Utc>,
}

/// The tokens are secrets (Simkl revokes a leaked one): `Debug` names what the
/// credential holds and never its values, like [`Secret`]'s.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// The person's Simkl account as the screens read it: no secret in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub scope: String,
    pub expires_at: DateTime<Utc>,
}

/// Asks Simkl for a device code. Nothing here is the person's yet: the approval
/// happens at [`DeviceCode::verification_url`].
///
/// A refusal is about the app's client id (nothing else is in the request), and
/// it is kept as a context so [`key_rejected`] can read the status.
pub fn start(http: &dyn Transport, client_id: &str) -> Result<DeviceCode> {
    let url = Url::parse(&format!("{API}{DEVICE}"))?;
    let body = serde_json::json!({ "client_id": client_id.trim(), "scope": SCOPE });
    let answer = match http.post_json(&url, &body) {
        Ok(answer) => answer,
        Err(error) if http_status(&error) == Some(401) => return Err(error).context(
            "Simkl refused this app's client id (HTTP 401): it must be an AUTH V2 client from api.simkl.org; an AUTH V1 id cannot use the device flow.",
        ),
        Err(error) => return Err(error).context("Simkl sign-in"),
    };
    let answer: DeviceAnswer = serde_json::from_slice(&answer).context("Simkl sign-in")?;
    let device_code = answer.device_code.trim().to_owned();
    let user_code = answer.user_code.trim().to_owned();
    if device_code.is_empty() || user_code.is_empty() {
        bail!("Simkl answered no device code");
    }
    Ok(DeviceCode {
        verification_url: verification_url(&answer),
        device_code,
        user_code,
        // Simkl's own defaults: five seconds, fifteen minutes.
        interval_secs: answer.interval.filter(|seconds| *seconds > 0).unwrap_or(5),
        expires_in: answer.expires_in.filter(|seconds| *seconds > 0).unwrap_or(900),
    })
}

/// Replaces an expired access token. Simkl's tokens last seven days; the refresh
/// token is what keeps a machine signed in without asking the person again.
pub fn refresh(http: &dyn Transport, client_id: &str, refresh_token: &str) -> Result<Tokens> {
    let url = Url::parse(&format!("{API}{TOKEN}"))?;
    let body = serde_json::json!({
        "grant_type": REFRESH_GRANT,
        "client_id": client_id.trim(),
        "refresh_token": refresh_token.trim(),
    });
    let (status, answer) = http.post_json_answer(&url, &body).context("Simkl refresh")?;
    match status {
        200 => tokens_from(&answer),
        400 => match error_code(&answer).as_deref() {
            Some(code) => Err(anyhow!("Simkl refused the refresh ({code}): sign in again")),
            None => Err(anyhow!("Simkl refused the refresh: sign in again")),
        },
        401 | 403 => Err(anyhow::Error::from(HttpStatus::new(&url, status))).context("Simkl refused this session: sign in again"),
        status => Err(anyhow!("Simkl answered HTTP {status} to the refresh")),
    }
}

/// The tokens a token answer carries; one parser for the sign-in and the refresh.
fn tokens_from(answer: &[u8]) -> Result<Tokens> {
    let answer: TokenAnswer = serde_json::from_slice(answer).context("Simkl tokens")?;
    let access_token = answer.access_token.trim().to_owned();
    if access_token.is_empty() {
        bail!("Simkl answered no access token");
    }
    let seconds = answer.expires_in.filter(|seconds| *seconds > 0).unwrap_or(ACCESS_TOKEN_SECONDS);
    Ok(Tokens {
        access_token,
        refresh_token: answer.refresh_token.trim().to_owned(),
        scope: answer.scope.trim().to_owned(),
        expires_at: Utc::now() + Duration::seconds(seconds as i64),
    })
}

/// One poll for the approval. [`Step::Pending`] is the normal answer until the
/// person is done, and also what a declined code looks like: Simkl records no
/// refusal, so the caller's deadline is the only way out.
pub fn poll(http: &dyn Transport, client_id: &str, code: &DeviceCode) -> Result<Step> {
    let url = Url::parse(&format!("{API}{TOKEN}"))?;
    let body = serde_json::json!({
        "grant_type": DEVICE_GRANT,
        "client_id": client_id.trim(),
        "device_code": code.device_code,
    });
    let (status, answer) = http.post_json_answer(&url, &body)?;
    match status {
        200 => Ok(Step::Approved(tokens_from(&answer)?)),
        400 => match error_code(&answer).as_deref() {
            Some("slow_down") => Ok(Step::SlowDown),
            Some("expired_token") => Err(anyhow!("Simkl expired the code before it was approved: start the sign-in again")),
            // The normal answer until the person is done.
            Some("authorization_pending") | None => Ok(Step::Pending),
            // Polling again would only repeat it: name the code, never the body.
            Some(other) => Err(anyhow!("Simkl refused the poll ({other}): start the sign-in again")),
        },
        401 => Err(anyhow::Error::from(HttpStatus::new(&url, 401))).context(
            "Simkl refused this app's client id (HTTP 401): it must be an AUTH V2 client from api.simkl.org; an AUTH V1 id cannot use the device flow.",
        ),
        status => Err(anyhow::Error::from(HttpStatus::new(&url, status))).context("Simkl sign-in"),
    }
}

/// Whether `error` is Simkl refusing the client id itself (unknown, or an AUTH
/// V1 app) rather than a network fault. The message names the host only.
pub fn key_rejected(error: &anyhow::Error) -> bool {
    http_status(error) == Some(401)
}

/// The `simkl` credential with the client id and the tokens in it.
pub fn secret_with_tokens(client_id: &str, tokens: &Tokens) -> Secret {
    Secret::new(serde_json::json!({
        "clientId": client_id.trim(),
        "accessToken": tokens.access_token,
        "refreshToken": tokens.refresh_token,
        "scope": tokens.scope,
        "expiresAt": tokens.expires_at.to_rfc3339(),
    }))
}

/// The `simkl` credential with the client id and no session: what signing out
/// keeps.
pub fn secret_with_client_id(client_id: &str) -> Secret {
    Secret::new(serde_json::json!({ "clientId": client_id.trim() }))
}

/// The client id in the `simkl` credential, when one was stored.
pub fn client_id_of(secret: &Secret) -> Option<String> {
    text(secret.value(), "clientId")
}

/// The tokens in the `simkl` credential, when the person signed in. A credential
/// with the client id alone has none.
pub fn tokens_of(secret: &Secret) -> Option<Tokens> {
    let value = secret.value();
    let access_token = text(value, "accessToken")?;
    Some(Tokens {
        access_token,
        refresh_token: text(value, "refreshToken").unwrap_or_default(),
        scope: text(value, "scope").unwrap_or_default(),
        // An expiry this build cannot read is treated as "already past": a
        // caller refreshes, which is the safe direction.
        expires_at: text(value, "expiresAt")
            .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
            .map_or_else(Utc::now, |at| at.with_timezone(&Utc)),
    })
}

/// The session in the `simkl` credential, when one was signed in.
pub fn session_of(secret: &Secret) -> Option<Session> {
    let tokens = tokens_of(secret)?;
    Some(Session { scope: tokens.scope, expires_at: tokens.expires_at })
}

fn text(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// The page that approves `answer`'s code, pre-filled when Simkl sent a complete
/// URL and assembled from `verification_uri` + `user_code` when it did not.
fn verification_url(answer: &DeviceAnswer) -> String {
    if let Some(complete) = answer.verification_uri_complete.as_deref().map(str::trim).filter(|url| !url.is_empty()) {
        return complete.to_owned();
    }
    let base = answer.verification_uri.as_deref().map(str::trim).filter(|url| !url.is_empty()).unwrap_or(VERIFY);
    let Ok(mut url) = Url::parse(base) else {
        return base.to_owned();
    };
    if !url.query_pairs().any(|(key, _)| key == "user_code") {
        url.query_pairs_mut().append_pair("user_code", answer.user_code.trim());
    }
    url.to_string()
}

/// The OAuth error code in a token answer (`authorization_pending`, `slow_down`,
/// `expired_token`), when the body carries one.
fn error_code(body: &[u8]) -> Option<String> {
    let error: OAuthError = serde_json::from_slice(body).ok()?;
    let code = error.error.trim().to_owned();
    (!code.is_empty()).then_some(code)
}

#[derive(Deserialize)]
struct DeviceAnswer {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    user_code: String,
    #[serde(default)]
    verification_uri: Option<String>,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct TokenAnswer {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct OAuthError {
    #[serde(default)]
    error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const DEVICE_BODY: &str = r#"{
        "device_code": "dev-secret",
        "user_code": "BDWP-HQPK",
        "verification_uri": "https://simkl.com/pin",
        "verification_uri_complete": "https://simkl.com/pin?user_code=BDWP-HQPK",
        "expires_in": 900,
        "interval": 5
    }"#;
    const TOKEN_BODY: &str = r#"{
        "access_token": "simkl_at_x",
        "token_type": "Bearer",
        "expires_in": 604800,
        "refresh_token": "simkl_rt_y",
        "scope": "media:read media:write"
    }"#;

    /// Answers every POST with one status and body; records the requests.
    struct Fake {
        status: u16,
        body: String,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(status: u16, body: &str) -> Self {
            Self { status, body: body.to_owned(), seen: Mutex::new(Vec::new()) }
        }
    }

    impl Transport for Fake {
        fn get(&self, _: &Url) -> Result<Vec<u8>> {
            unreachable!("signing in is polling, not fetching")
        }
        fn post_json(&self, url: &Url, body: &serde_json::Value) -> Result<Vec<u8>> {
            let (status, bytes) = self.post_json_answer(url, body)?;
            if !(200..300).contains(&status) {
                return Err(HttpStatus::new(url, status).into());
            }
            Ok(bytes)
        }
        fn post_json_answer(&self, url: &Url, body: &serde_json::Value) -> Result<(u16, Vec<u8>)> {
            self.seen.lock().unwrap().push(format!("{} {body}", url.as_str()));
            Ok((self.status, self.body.as_bytes().to_vec()))
        }
    }

    fn code() -> DeviceCode {
        DeviceCode {
            device_code: "dev-secret".into(),
            user_code: "BDWP-HQPK".into(),
            verification_url: "https://simkl.com/pin?user_code=BDWP-HQPK".into(),
            interval_secs: 5,
            expires_in: 900,
        }
    }

    #[test]
    fn a_device_code_carries_the_page_that_pre_fills_it() {
        let fake = Fake::new(200, DEVICE_BODY);
        let code = start(&fake, "client-1").unwrap();
        assert_eq!(code.user_code, "BDWP-HQPK");
        assert_eq!(code.verification_url, "https://simkl.com/pin?user_code=BDWP-HQPK");
        assert_eq!((code.interval_secs, code.expires_in), (5, 900));
        let seen = fake.seen.lock().unwrap().clone();
        assert!(seen[0].starts_with("https://api.simkl.com/oauth2/device "), "{}", seen[0]);
        assert!(seen[0].contains("client-1"), "the client id travels in the body: {}", seen[0]);
        assert!(seen[0].contains("media:read media:write"), "the scope is asked for: {}", seen[0]);
    }

    #[test]
    fn a_url_without_the_complete_form_is_assembled_from_the_code() {
        let answer = DeviceAnswer {
            device_code: "dev".into(),
            user_code: "BDWP-HQPK".into(),
            verification_uri: Some("https://simkl.com/pin".into()),
            verification_uri_complete: None,
            interval: None,
            expires_in: None,
        };
        assert_eq!(verification_url(&answer), "https://simkl.com/pin?user_code=BDWP-HQPK");
        let bare = DeviceAnswer { verification_uri: None, verification_uri_complete: None, ..answer };
        assert_eq!(verification_url(&bare), "https://simkl.com/pin?user_code=BDWP-HQPK");
    }

    #[test]
    fn an_approved_code_becomes_tokens() {
        let fake = Fake::new(200, TOKEN_BODY);
        let step = poll(&fake, "client-1", &code()).unwrap();
        let Step::Approved(tokens) = step else { panic!("approved: {step:?}") };
        assert_eq!(tokens.access_token, "simkl_at_x");
        assert_eq!(tokens.refresh_token, "simkl_rt_y");
        assert_eq!(tokens.scope, "media:read media:write");
        assert!(tokens.expires_at > Utc::now(), "seven days from now");
        let seen = fake.seen.lock().unwrap().clone();
        assert!(seen[0].contains("urn:ietf:params:oauth:grant-type:device_code"), "{}", seen[0]);
        assert!(seen[0].contains("dev-secret"), "the device code travels in the body: {}", seen[0]);
    }

    #[test]
    fn a_pending_code_is_a_step_not_a_failure() {
        let fake = Fake::new(400, r#"{"error":"authorization_pending","error_description":"User has not authorized the pin yet."}"#);
        assert_eq!(poll(&fake, "client-1", &code()).unwrap(), Step::Pending);
    }

    #[test]
    fn an_unknown_refusal_ends_the_poll_instead_of_waiting_forever() {
        let fake = Fake::new(400, r#"{"error":"invalid_request","error_description":"missing device_code"}"#);
        let error = poll(&fake, "client-1", &code()).unwrap_err();
        assert!(format!("{error:#}").contains("invalid_request"), "{error:#}");
        assert!(format!("{error:#}").contains("start the sign-in again"), "{error:#}");
    }

    #[test]
    fn polling_too_fast_asks_for_a_longer_wait() {
        let fake = Fake::new(400, r#"{"error":"slow_down"}"#);
        assert_eq!(poll(&fake, "client-1", &code()).unwrap(), Step::SlowDown);
    }

    #[test]
    fn an_expired_code_says_to_start_again() {
        let fake = Fake::new(400, r#"{"error":"expired_token"}"#);
        let error = poll(&fake, "client-1", &code()).unwrap_err();
        assert!(format!("{error:#}").contains("start the sign-in again"), "{error:#}");
    }

    #[test]
    fn a_refused_client_id_is_recognised_and_never_echoed() {
        let refused = r#"{"error":"invalid_client","error_description":"This client_id is not enabled for OAuth 2.0"}"#;
        let error = start(&Fake::new(401, refused), "client-1").unwrap_err();
        assert!(key_rejected(&error), "a 401 on the client id's own call is the client id: {error:#}");
        assert!(!format!("{error:#}").contains("client-1"), "the id never reaches the message");

        let error = poll(&Fake::new(401, refused), "client-1", &code()).unwrap_err();
        assert!(key_rejected(&error), "a 401 while polling is still the client id: {error:#}");
        assert!(!format!("{error:#}").contains("dev-secret"), "the device code never reaches the message");
    }

    #[test]
    fn the_credential_keeps_the_client_id_and_the_tokens() {
        let tokens = Tokens {
            access_token: "simkl_at_x".into(),
            refresh_token: "simkl_rt_y".into(),
            scope: "media:read".into(),
            expires_at: Utc::now(),
        };
        let secret = secret_with_tokens("client-1", &tokens);
        assert_eq!(client_id_of(&secret).as_deref(), Some("client-1"));
        assert_eq!(tokens_of(&secret), Some(tokens.clone()));
        assert_eq!(session_of(&secret).map(|session| session.scope), Some("media:read".to_owned()));
        // A client id alone is not a session.
        let alone = Secret::new(serde_json::json!({"clientId": "client-1"}));
        assert_eq!(tokens_of(&alone), None);
        assert_eq!(session_of(&alone), None);
        assert!(!format!("{secret:?}").contains("simkl_at_x"), "a credential never prints its secrets");
    }

    #[test]
    fn a_credential_never_prints_its_secrets() {
        let code = code();
        assert!(!format!("{code:?}").contains("dev-secret"), "the device code is the polling credential");
        assert!(format!("{code:?}").contains("BDWP-HQPK"), "the code shown to the person is not a secret");
        let tokens = Tokens {
            access_token: "simkl_at_x".into(),
            refresh_token: "simkl_rt_y".into(),
            scope: "media:read".into(),
            expires_at: Utc::now(),
        };
        let printed = format!("{tokens:?}");
        assert!(!printed.contains("simkl_at_x") && !printed.contains("simkl_rt_y"), "{printed}");
    }

    #[test]
    fn a_missing_expiry_reads_as_already_past() {
        let secret = Secret::new(serde_json::json!({"clientId": "c", "accessToken": "simkl_at_x"}));
        let tokens = tokens_of(&secret).expect("the access token is what makes it a session");
        assert!(tokens.expires_at <= Utc::now(), "a caller refreshes rather than being refused");
        assert!(tokens.refresh_token.is_empty() && tokens.scope.is_empty());
    }

    #[test]
    fn a_refresh_replaces_the_tokens() {
        let fake = Fake::new(200, TOKEN_BODY);
        let tokens = refresh(&fake, "client-1", "simkl_rt_y").expect("refreshed");
        assert_eq!(tokens.access_token, "simkl_at_x");
        assert!(tokens.expires_at > Utc::now(), "a week from now");
    }

    #[test]
    fn a_refused_refresh_says_to_sign_in_again() {
        let fake = Fake::new(401, r#"{"error":"invalid_grant"}"#);
        let error = refresh(&fake, "client-1", "simkl_rt_y").unwrap_err();
        assert!(format!("{error:#}").contains("sign in again"), "{error:#}");
    }
}
