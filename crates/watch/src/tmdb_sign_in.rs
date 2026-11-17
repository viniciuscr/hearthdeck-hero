//! Signing in to TMDB, as the app asks for it: the key comes from the
//! credential store, the approval happens in the person's browser, and the
//! session goes back beside the key.
//!
//! Nothing consumes the session yet — TMDB's account lists (watchlist,
//! favourites, rated) are the next slice — so a failure to sign in degrades to
//! "not signed in": the catalogs and the metadata work with the key alone.

use anyhow::{Context, Result};
use heronite_data::paths::AppPaths;
use heronite_watch_core::http::UreqTransport;
use heronite_watch_core::provider::Credentials;
use heronite_watch_state::store::StoreCredentials;
use heronite_watch_tmdb::auth;

pub use heronite_watch_tmdb::auth::{approve_url, Session, Step};

/// Asks TMDB for a request token; the person approves it at [`approve_url`].
pub fn start(app: &AppPaths) -> Result<String> {
    let key = key(app)?;
    auth::start(&UreqTransport::new(), &key)
}

/// Turns an approved token into a session and keeps it beside the key.
/// [`Step::Pending`] means the person has not approved it yet.
pub fn finish(app: &AppPaths, token: &str) -> Result<Step> {
    let key = key(app)?;
    let step = auth::finish(&UreqTransport::new(), &key, token)?;
    if let Step::Approved(session) = &step {
        StoreCredentials::new(app).set("tmdb", &auth::secret_with_session(&key, session)).context("keeping the TMDB session")?;
    }
    Ok(step)
}

/// The session the person signed in with, when they did.
pub fn session(app: &AppPaths) -> Option<Session> {
    let secret = StoreCredentials::new(app).get("tmdb").ok().flatten()?;
    auth::session_of(&secret)
}

/// Forgets the session and keeps the key, so using the catalogs needs nothing
/// typed again.
pub fn sign_out(app: &AppPaths) -> Result<()> {
    let key = key(app)?;
    StoreCredentials::new(app).set("tmdb", &heronite_watch_tmdb::secret(&key)).context("forgetting the TMDB session")
}

/// Whether `error` is TMDB refusing the key itself (reset, or a paste that lost
/// a character) rather than a network fault.
pub use heronite_watch_tmdb::auth::key_rejected;

/// The key to sign in with: the build's own when it carries one, else a key an
/// older build stored. The person never supplies one — TMDB's application key is
/// the app's, and the login is the only thing they do.
fn key(app: &AppPaths) -> Result<String> {
    if let Some(key) = heronite_watch_tmdb::built_in_key() {
        return Ok(key.to_owned());
    }
    crate::store::read_tmdb_key(app)?.context("This build carries no TMDB API key: compile with HEARTHDECK_TMDB_API_KEY.")
}

/// Whether TMDB can be used at all in this build: a key to send.
pub fn configured(app: &AppPaths) -> bool {
    key(app).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::paths::DataPaths;
    use heronite_watch_core::provider::Secret;

    fn app_paths(dir: &std::path::Path) -> AppPaths {
        AppPaths::for_data(&DataPaths::at(dir.to_path_buf()))
    }

    #[test]
    fn a_session_is_read_back_and_signing_out_keeps_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        assert_eq!(session(&app), None, "nothing signed in before there is a key");
        crate::store::write_tmdb_key(&app, "key1").unwrap();
        assert_eq!(session(&app), None, "a key alone is not a session");

        let kept = auth::secret_with_session("key1", &Session { id: "sess456".into(), username: "cinephile".into(), account_id: 42 });
        StoreCredentials::new(&app).set("tmdb", &kept).unwrap();
        assert_eq!(session(&app).map(|s| s.username), Some("cinephile".to_owned()));

        sign_out(&app).unwrap();
        assert_eq!(session(&app), None);
        assert_eq!(crate::store::read_tmdb_key(&app).unwrap().as_deref(), Some("key1"), "signing out is not losing the key");
    }

    #[test]
    fn signing_in_without_a_key_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        let error = start(&app).unwrap_err();
        assert!(format!("{error:#}").contains("API key"), "{error:#}");
        // And the empty credential shape is not mistaken for a session.
        assert_eq!(auth::session_of(&Secret::new(serde_json::json!({ "apiKey": "key1" }))), None);
    }
}
