//! Settings → Watch, as a file of their own (`watch-settings.json`): what is
//! neither theme, account nor home rails.
//!
//! Not secret, so it goes through `json_store` like the rest of the config;
//! unknown fields are kept, so a newer build's settings survive a write here.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use heronite_data::json_store;
use heronite_data::paths::AppPaths;

pub const SETTINGS_FILE: &str = "watch-settings.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Stop the Stremio service once a session ends, when this app started it
    /// (one the person or their desktop session started is never stopped).
    #[serde(default = "stop_service_by_default")]
    pub stop_service_when_done: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn stop_service_by_default() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { stop_service_when_done: true, extra: Map::new() }
    }
}

pub fn path(app: &AppPaths) -> PathBuf {
    app.config.join(SETTINGS_FILE)
}

/// The settings as stored; a file that cannot be read is the default, said so
/// once in the log.
pub fn read(app: &AppPaths) -> Settings {
    let path = path(app);
    json_store::read(&path).unwrap_or_else(|error| {
        log::warn!("{}: {error:#}", path.display());
        Settings::default()
    })
}

pub fn write(app: &AppPaths, settings: &Settings) -> Result<()> {
    let path = path(app);
    json_store::write(&path, settings).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    #[test]
    fn stopping_the_service_is_on_by_default() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&app(dir.path())).stop_service_when_done, "no file: the default stops what we started");
        assert!(Settings::default().stop_service_when_done);
    }

    #[test]
    fn the_choice_round_trips_and_keeps_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let mut settings = Settings { stop_service_when_done: false, ..Settings::default() };
        settings.extra.insert("fromTheFuture".into(), Value::from(7));
        write(&app, &settings).unwrap();

        let read_back = read(&app);
        assert!(!read_back.stop_service_when_done);
        assert_eq!(read_back.extra.get("fromTheFuture"), Some(&Value::from(7)));

        let stored: Value = json_store::read(&path(&app)).unwrap();
        assert_eq!(stored["stopServiceWhenDone"], Value::Bool(false), "{stored}");
    }

    #[test]
    fn a_corrupt_file_falls_back_to_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        std::fs::create_dir_all(&app.config).unwrap();
        std::fs::write(path(&app), b"not json").unwrap();
        assert!(read(&app).stop_service_when_done);
    }
}
