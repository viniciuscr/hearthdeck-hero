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

pub fn read_movies(app: &AppPaths) -> MoviesFile {
    let path = movies_path(app);
    json_store::read(&path).unwrap_or_else(|error| {
        log::warn!("{}: {error:#}", path.display());
        MoviesFile::default()
    })
}

pub fn write_movies(app: &AppPaths, movies: &MoviesFile) -> Result<()> {
    let path = movies_path(app);
    json_store::write(&path, movies).with_context(|| format!("writing {}", path.display()))
}

pub fn progress_for<'a>(movies: &'a MoviesFile, id: &str) -> Option<&'a MovieProgress> {
    movies.progress.iter().find(|progress| progress.id == id)
}

pub fn upsert_progress(movies: &mut MoviesFile, progress: MovieProgress) {
    match movies.progress.iter_mut().find(|entry| entry.id == progress.id) {
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
                id: "tt1".into(),
                position_ms: 1000,
                duration_ms: 2000,
                updated_at: Utc::now(),
                finished: false,
                video_id: None,
            },
        );
        write_movies(&app, &movies).unwrap();
        let read = read_movies(&app);
        assert_eq!(read.progress.len(), 1);
        assert_eq!(progress_for(&read, "tt1").unwrap().position_ms, 1000);
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
}
