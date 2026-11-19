//! Signing in to Simkl, as the app asks for it: the app's client id comes from
//! the credential store, the person approves a code on their phone, and the
//! tokens go back beside the id.
//!
//! This is TMDB's shape with a TV in it: the credential that authenticates the
//! *application* is asked for once (the bootstrap writes a file it is adopted
//! from), and the person's part is a code shown as a QR — no keyboard, no
//! password. Nothing consumes the tokens yet; the calls they unlock are the next
//! slice, so a failure here degrades to "not signed in".

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use heronite_data::paths::AppPaths;
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::Credentials;
use heronite_watch_simkl::auth;
use heronite_watch_simkl::{Tokens, PROVIDER};
use heronite_watch_state::store::StoreCredentials;

pub use heronite_watch_simkl::auth::{DeviceCode, Session, Step, SLOW_DOWN};

/// Asks Simkl for a code to show; the person approves it at
/// [`DeviceCode::verification_url`], and [`poll`] notices on its own.
pub fn start(app: &AppPaths) -> Result<DeviceCode> {
    let client_id = client_id(app)?;
    auth::start(&UreqTransport::new(), &client_id)
}

/// One poll for the approval. [`Step::Pending`] means the phone has not done it
/// yet; [`Step::Approved`] keeps the tokens beside the client id.
pub fn poll(app: &AppPaths, code: &DeviceCode) -> Result<Step> {
    let client_id = client_id(app)?;
    let step = auth::poll(&UreqTransport::new(), &client_id, code)?;
    if let Step::Approved(tokens) = &step {
        StoreCredentials::new(app).set(PROVIDER, &auth::secret_with_tokens(&client_id, tokens)).context("keeping the Simkl session")?;
    }
    Ok(step)
}

/// The Simkl account signed in on this machine, when one is.
pub fn session(app: &AppPaths) -> Option<Session> {
    let secret = StoreCredentials::new(app).get(PROVIDER).ok().flatten()?;
    auth::session_of(&secret)
}

/// The tokens a tracker delivers with, when the person is signed in: the access
/// token and the refresh token that replaces it. `None` after signing out.
pub fn tokens(app: &AppPaths) -> Option<heronite_watch_simkl::Tokens> {
    let secret = StoreCredentials::new(app).get(PROVIDER).ok().flatten()?;
    auth::tokens_of(&secret)
}

/// Refreshed this long before expiry, so a delivery never meets an expired token.
const REFRESH_BEFORE: Duration = Duration::hours(24);

/// The tokens a tracker delivers with, refreshed when they are close to expiring.
/// Simkl's tokens last seven days; without this a machine stops delivering after
/// one week with a 401 and nobody knows why.
pub fn tokens_fresh(app: &AppPaths) -> Result<Option<Tokens>> {
    tokens_fresh_with(&UreqTransport::new(), app)
}

/// [`tokens_fresh`] with an injected transport, for tests.
pub fn tokens_fresh_with(http: &dyn Transport, app: &AppPaths) -> Result<Option<Tokens>> {
    let store = StoreCredentials::new(app);
    let Some(secret) = store.get(PROVIDER)? else {
        return Ok(None);
    };
    let Some(tokens) = auth::tokens_of(&secret) else {
        return Ok(None);
    };
    if tokens.expires_at > Utc::now() + REFRESH_BEFORE {
        return Ok(Some(tokens));
    }
    let client_id = client_id(app)?;
    let fresh = auth::refresh(http, &client_id, &tokens.refresh_token)?;
    store.set(PROVIDER, &auth::secret_with_tokens(&client_id, &fresh)).context("keeping the refreshed Simkl session")?;
    Ok(Some(fresh))
}

/// Forgets the session and keeps the client id, so signing in again needs
/// nothing typed.
pub fn sign_out(app: &AppPaths) -> Result<()> {
    let client_id = client_id(app)?;
    StoreCredentials::new(app).set(PROVIDER, &auth::secret_with_client_id(&client_id)).context("forgetting the Simkl session")
}

/// Whether Simkl can be used at all in this build: a client id to send.
pub fn configured(app: &AppPaths) -> bool {
    client_id(app).is_ok()
}

/// Whether `error` is Simkl refusing the client id itself (unknown, or an AUTH
/// V1 app) rather than a network fault.
pub use heronite_watch_simkl::auth::key_rejected;

/// A build made without the client id compiled in reads one from
/// [`CLIENT_ID_FILE`] in the config folder: it is adopted into the credential
/// store once, and everything else reads it from there.
pub fn adopt_client_id_file(app: &AppPaths) -> bool {
    let Some(client_id) = heronite_watch_simkl::client_id_file(&app.config) else {
        return false;
    };
    // The file is an explicit act by whoever set the machine up, so it wins over
    // whatever the store already held.
    if crate::store::read_simkl_client_id(app).ok().flatten().as_deref() == Some(client_id.as_str()) {
        return false;
    }
    match crate::store::write_simkl_client_id(app, &client_id) {
        Ok(()) => true,
        Err(error) => {
            log::warn!("reading the Simkl client id file: {error:#}");
            false
        }
    }
}

/// The client id to sign in with: the store's, else the build's own.
pub(crate) fn client_id(app: &AppPaths) -> Result<String> {
    if let Some(client_id) = crate::store::read_simkl_client_id(app)? {
        if !client_id.trim().is_empty() {
            return Ok(client_id.trim().to_owned());
        }
    }
    if let Some(client_id) = heronite_watch_simkl::built_in_client_id() {
        return Ok(client_id.to_owned());
    }
    anyhow::bail!("Simkl needs this app's client id: the bootstrap sets it, or set HEARTHDECK_SIMKL_CLIENT_ID when building.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::paths::DataPaths;
    use heronite_watch_simkl::CLIENT_ID_FILE;

    fn app_paths(dir: &std::path::Path) -> AppPaths {
        AppPaths::for_data(&DataPaths::at(dir.to_path_buf()))
    }

    #[test]
    fn the_client_id_file_is_adopted_once() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        std::fs::create_dir_all(&app.config).unwrap();
        std::fs::write(app.config.join(CLIENT_ID_FILE), "file-client\nsecond line ignored\n").unwrap();
        assert!(adopt_client_id_file(&app), "the file is taken");
        assert_eq!(crate::store::read_simkl_client_id(&app).unwrap().as_deref(), Some("file-client"));
        assert!(!adopt_client_id_file(&app), "the store has it now, nothing to adopt");
        assert!(configured(&app), "the build can talk to Simkl");
    }

    #[test]
    fn the_client_id_file_replaces_an_id_the_store_already_had() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        std::fs::create_dir_all(&app.config).unwrap();
        crate::store::write_simkl_client_id(&app, "old-client").unwrap();
        std::fs::write(app.config.join(CLIENT_ID_FILE), "\"new-client\"\n").unwrap();
        assert!(adopt_client_id_file(&app), "the file is the more explicit intent");
        assert_eq!(crate::store::read_simkl_client_id(&app).unwrap().as_deref(), Some("new-client"));
    }

    #[test]
    fn a_session_is_read_back_and_signing_out_keeps_the_client_id() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        assert_eq!(session(&app), None, "nothing signed in before there is a client id");
        crate::store::write_simkl_client_id(&app, "client1").unwrap();
        assert_eq!(session(&app), None, "a client id alone is not a session");

        let tokens = heronite_watch_simkl::Tokens {
            access_token: "simkl_at_x".into(),
            refresh_token: "simkl_rt_y".into(),
            scope: "media:read".into(),
            expires_at: chrono::Utc::now(),
        };
        StoreCredentials::new(&app).set(PROVIDER, &auth::secret_with_tokens("client1", &tokens)).unwrap();
        assert_eq!(session(&app).map(|session| session.scope), Some("media:read".to_owned()));

        sign_out(&app).unwrap();
        assert_eq!(session(&app), None);
        assert_eq!(crate::store::read_simkl_client_id(&app).unwrap().as_deref(), Some("client1"), "signing out is not losing the app's id");
    }

    #[test]
    fn signing_in_without_a_client_id_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        let error = start(&app).unwrap_err();
        assert!(format!("{error:#}").contains("this app's client id"), "{error:#}");
    }

    struct Refreshing {
        answer: &'static str,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl heronite_watch_core::http::Transport for Refreshing {
        fn get(&self, _: &url::Url) -> Result<Vec<u8>> {
            unreachable!("a refresh is a POST")
        }
        fn post_json(&self, _: &url::Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            unreachable!("a refresh answers with a status")
        }
        fn post_json_answer(&self, _: &url::Url, _: &serde_json::Value) -> Result<(u16, Vec<u8>)> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok((200, self.answer.as_bytes().to_vec()))
        }
    }

    fn session_until(at: chrono::DateTime<Utc>) -> Tokens {
        Tokens { access_token: "at".into(), refresh_token: "rt".into(), scope: "media:read".into(), expires_at: at }
    }

    #[test]
    fn a_fresh_session_is_used_as_it_is_and_an_expiring_one_is_refreshed() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        heronite_watch_state::store::write_simkl_client_id(&app, "client-1").unwrap();
        let store = StoreCredentials::new(&app);

        store.set(PROVIDER, &auth::secret_with_tokens("client-1", &session_until(Utc::now() + Duration::days(5)))).unwrap();
        let quiet = Refreshing { answer: "", calls: Default::default() };
        assert_eq!(tokens_fresh_with(&quiet, &app).unwrap().unwrap().access_token, "at");
        assert_eq!(quiet.calls.load(std::sync::atomic::Ordering::Relaxed), 0, "a fresh token costs no request");

        store.set(PROVIDER, &auth::secret_with_tokens("client-1", &session_until(Utc::now() + Duration::hours(1)))).unwrap();
        let fake = Refreshing {
            answer: r#"{"access_token":"at2","refresh_token":"rt2","expires_in":604800,"scope":"media:read"}"#,
            calls: Default::default(),
        };
        assert_eq!(tokens_fresh_with(&fake, &app).unwrap().unwrap().access_token, "at2");
        assert_eq!(tokens(&app).unwrap().access_token, "at2", "the store keeps the new pair");
        assert_eq!(fake.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}
