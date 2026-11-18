//! Local Movies state. Non-secret records go through `json_store`; the session
//! key is a private file written with an explicit `0600` mode.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use heronite_data::json_store;
use heronite_data::paths::AppPaths;
use heronite_data::temps;

use heronite_watch_core::auth::AuthKey;
use heronite_watch_core::provider::{Credentials, Secret};
use heronite_watch_core::types::MovieProgress;

pub const MOVIES_FILE: &str = "movies.json";
pub const AUTH_FILE: &str = "stremio-auth.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MoviesFile {
    #[serde(default)]
    pub progress: Vec<MovieProgress>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn movies_path(app: &AppPaths) -> PathBuf {
    app.config.join(MOVIES_FILE)
}

/// Where a provider keeps its credentials. Stremio's file keeps its old name.
pub fn auth_path_for(app: &AppPaths, provider: &str) -> PathBuf {
    app.config.join(format!("{provider}-auth.json"))
}

pub fn auth_path(app: &AppPaths) -> PathBuf {
    auth_path_for(app, "stremio")
}

/// The owner of records written before the provider was part of the file.
const LEGACY_PROVIDER: &str = "stremio";

pub fn read_movies(app: &AppPaths) -> MoviesFile {
    let path = movies_path(app);
    let mut movies = json_store::read(&path).unwrap_or_else(|error| {
        log::warn!("{}: {error:#}", path.display());
        MoviesFile::default()
    });
    // A record from before providers were recorded is Stremio's; the next
    // write makes it explicit.
    for progress in &mut movies.progress {
        if progress.provider.is_empty() {
            progress.provider = LEGACY_PROVIDER.into();
        }
    }
    movies
}

pub fn write_movies(app: &AppPaths, movies: &MoviesFile) -> Result<()> {
    let path = movies_path(app);
    json_store::write(&path, movies).with_context(|| format!("writing {}", path.display()))
}

pub fn progress_for<'a>(movies: &'a MoviesFile, provider: &str, id: &str) -> Option<&'a MovieProgress> {
    movies.progress.iter().find(|progress| progress.provider == provider && progress.id == id)
}

pub fn upsert_progress(movies: &mut MoviesFile, progress: MovieProgress) {
    let same = |entry: &MovieProgress| entry.provider == progress.provider && entry.id == progress.id;
    match movies.progress.iter_mut().find(|entry| same(entry)) {
        Some(entry) => *entry = progress,
        None => movies.progress.push(progress),
    }
}

/// The credentials port over the app's per-provider files.
pub struct StoreCredentials<'a> {
    app: &'a AppPaths,
}

impl<'a> StoreCredentials<'a> {
    pub fn new(app: &'a AppPaths) -> Self {
        Self { app }
    }
}

impl Credentials for StoreCredentials<'_> {
    fn get(&self, provider: &str) -> Result<Option<Secret>> {
        let path = auth_path_for(self.app, provider);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        let value: Value = serde_json::from_slice(&bytes).with_context(|| format!("{} is not JSON", path.display()))?;
        Ok(Some(Secret::new(value)))
    }

    /// `write_atomic` creates the file with `0600` before writing, so it is
    /// never world-readable, not even briefly.
    fn set(&self, provider: &str, secret: &Secret) -> Result<()> {
        let path = auth_path_for(self.app, provider);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let bytes = serde_json::to_vec(secret.value()).context("encoding the credentials")?;
        temps::write_atomic(&path, &bytes, Some(0o600)).with_context(|| format!("writing {}", path.display()))
    }

    fn clear(&self, provider: &str) -> Result<()> {
        match fs::remove_file(auth_path_for(self.app, provider)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("removing the credentials"),
        }
    }
}

/// The Stremio session key, as the account link flow returns it.
pub fn write_auth(app: &AppPaths, key: &AuthKey) -> Result<()> {
    StoreCredentials::new(app).set("stremio", &Secret::new(serde_json::json!({"authKey": key})))
}

pub fn read_auth(app: &AppPaths) -> Result<Option<AuthKey>> {
    let Some(secret) = StoreCredentials::new(app).get("stremio")? else {
        return Ok(None);
    };
    let value = secret.value();
    let key = value.get("authKey").and_then(Value::as_str).or_else(|| value.as_str());
    match key {
        Some(key) if !key.is_empty() => Ok(Some(AuthKey::new(key))),
        _ => bail!("{} has no authKey", auth_path(app).display()),
    }
}

pub fn clear_auth(app: &AppPaths) -> Result<()> {
    StoreCredentials::new(app).clear("stremio")
}

/// The TMDB API key (v3), written where the TMDB catalog source reads it and
/// nowhere else. The credentials file is 0600, like the account's.
pub fn write_tmdb_key(app: &AppPaths, api_key: &str) -> Result<()> {
    let store = StoreCredentials::new(app);
    // Keep what the credential already holds: the session a person approved and
    // the v4 read token live in the same object, and replacing it would sign them
    // out (and drop the token) every time the key file is adopted.
    let mut value = store.get("tmdb")?.map(|secret| secret.value().clone()).unwrap_or_else(|| serde_json::json!({}));
    if !value.is_object() {
        value = serde_json::json!({});
    }
    value["apiKey"] = serde_json::json!(api_key.trim());
    store.set("tmdb", &Secret::new(value))
}

/// The stored TMDB key, when there is one. A key that is only whitespace is no
/// key at all.
pub fn read_tmdb_key(app: &AppPaths) -> Result<Option<String>> {
    let Some(secret) = StoreCredentials::new(app).get("tmdb")? else {
        return Ok(None);
    };
    let value = secret.value();
    let key = value.get("apiKey").and_then(Value::as_str).or_else(|| value.as_str()).unwrap_or_default().trim().to_owned();
    Ok((!key.is_empty()).then_some(key))
}

pub fn clear_tmdb_key(app: &AppPaths) -> Result<()> {
    StoreCredentials::new(app).clear("tmdb")
}

/// The Simkl client id (the application's credential for that API), written
/// where the Simkl sign-in reads it. Kept beside any session the credential
/// already holds, so re-keying the app does not sign the person out.
pub fn write_simkl_client_id(app: &AppPaths, client_id: &str) -> Result<()> {
    let store = StoreCredentials::new(app);
    let mut value = store.get("simkl")?.map(|secret| secret.value().clone()).unwrap_or_else(|| serde_json::json!({}));
    if !value.is_object() {
        value = serde_json::json!({});
    }
    value["clientId"] = serde_json::json!(client_id.trim());
    store.set("simkl", &Secret::new(value))
}

/// The stored Simkl client id, when there is one. Whitespace is not an id.
pub fn read_simkl_client_id(app: &AppPaths) -> Result<Option<String>> {
    let Some(secret) = StoreCredentials::new(app).get("simkl")? else {
        return Ok(None);
    };
    let value = secret.value();
    let client_id = value.get("clientId").and_then(Value::as_str).or_else(|| value.as_str()).unwrap_or_default().trim().to_owned();
    Ok((!client_id.is_empty()).then_some(client_id))
}

/// Forgets the Simkl client id and the session with it: without the app's
/// credential nothing of that provider works here.
pub fn clear_simkl_client_id(app: &AppPaths) -> Result<()> {
    StoreCredentials::new(app).clear("simkl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::path::Path;

    fn paths(root: &Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    #[test]
    fn progress_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        let mut movies = MoviesFile::default();
        upsert_progress(
            &mut movies,
            MovieProgress {
                provider: "stremio".into(),
                id: "tt1".into(),
                position_ms: 1000,
                duration_ms: 2000,
                updated_at: Utc::now(),
                finished: false,
                video_id: None,
                imdb: None,
                episodes: Default::default(),
            },
        );
        write_movies(&app, &movies).unwrap();
        let read = read_movies(&app);
        assert_eq!(read.progress.len(), 1);
        assert_eq!(progress_for(&read, "stremio", "tt1").unwrap().position_ms, 1000);
    }

    #[test]
    fn auth_round_trips_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        write_auth(&app, &AuthKey::new("abc")).unwrap();
        assert_eq!(read_auth(&app).unwrap(), Some(AuthKey::new("abc")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(auth_path(&app)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn clearing_missing_auth_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        clear_auth(&paths(dir.path())).unwrap();
        assert!(read_auth(&paths(dir.path())).unwrap().is_none());
    }

    #[test]
    fn a_record_without_a_provider_is_stremios() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        fs::create_dir_all(&app.config).unwrap();
        fs::write(
            movies_path(&app),
            r#"{"progress":[{"id":"tt1","position_ms":1000,"duration_ms":2000,"updated_at":"2026-01-01T00:00:00Z","finished":false}]}"#,
        )
        .unwrap();

        let movies = read_movies(&app);
        assert!(progress_for(&movies, "stremio", "tt1").is_some());
        assert!(progress_for(&movies, "youtube", "tt1").is_none());
    }

    #[test]
    fn progress_is_per_provider() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        let mut movies = MoviesFile::default();
        let mut stremio = progress_with(1000);
        stremio.provider = "stremio".into();
        let mut youtube = progress_with(2000);
        youtube.provider = "youtube".into();
        upsert_progress(&mut movies, stremio);
        upsert_progress(&mut movies, youtube);
        write_movies(&app, &movies).unwrap();

        let read = read_movies(&app);
        assert_eq!(read.progress.len(), 2);
        assert_eq!(progress_for(&read, "stremio", "tt1").unwrap().position_ms, 1000);
        assert_eq!(progress_for(&read, "youtube", "tt1").unwrap().position_ms, 2000);
    }

    fn progress_with(position_ms: u64) -> MovieProgress {
        MovieProgress {
            provider: String::new(),
            id: "tt1".into(),
            position_ms,
            duration_ms: 2000,
            updated_at: Utc::now(),
            finished: false,
            video_id: None,
            imdb: None,
            episodes: Default::default(),
        }
    }
}

#[cfg(test)]
mod tmdb_key_tests {
    use super::*;
    use std::path::Path;

    fn paths(root: &Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    #[test]
    fn the_key_round_trips_and_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        assert_eq!(read_tmdb_key(&app).unwrap(), None, "no key before one is written");
        write_tmdb_key(&app, "  abc123  ").unwrap();
        assert_eq!(read_tmdb_key(&app).unwrap().as_deref(), Some("abc123"), "typed spaces are not part of it");
        clear_tmdb_key(&app).unwrap();
        assert_eq!(read_tmdb_key(&app).unwrap(), None);
    }

    #[test]
    fn a_blank_key_is_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        write_tmdb_key(&app, "   ").unwrap();
        assert_eq!(read_tmdb_key(&app).unwrap(), None);
    }
}

#[cfg(test)]
mod simkl_client_id_tests {
    use super::*;
    use std::path::Path;

    fn paths(root: &Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    #[test]
    fn the_client_id_round_trips_and_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        assert_eq!(read_simkl_client_id(&app).unwrap(), None, "no client id before one is written");
        write_simkl_client_id(&app, "  abc123  ").unwrap();
        assert_eq!(read_simkl_client_id(&app).unwrap().as_deref(), Some("abc123"), "typed spaces are not part of it");
        clear_simkl_client_id(&app).unwrap();
        assert_eq!(read_simkl_client_id(&app).unwrap(), None);
    }

    #[test]
    fn a_blank_client_id_is_no_client_id() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        write_simkl_client_id(&app, "   ").unwrap();
        assert_eq!(read_simkl_client_id(&app).unwrap(), None);
    }

    #[test]
    fn a_new_client_id_keeps_the_session_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let app = paths(dir.path());
        StoreCredentials::new(&app)
            .set("simkl", &Secret::new(serde_json::json!({"clientId": "old", "accessToken": "simkl_at_x"})))
            .unwrap();
        write_simkl_client_id(&app, "new").unwrap();
        let secret = StoreCredentials::new(&app).get("simkl").unwrap().unwrap();
        assert_eq!(read_simkl_client_id(&app).unwrap().as_deref(), Some("new"));
        assert_eq!(secret.value().get("accessToken").and_then(Value::as_str), Some("simkl_at_x"), "re-keying does not sign out");
    }
}
