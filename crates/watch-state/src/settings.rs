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
    /// The catalog sources the Movies and Series tabs may browse, in the order
    /// they are listed (Settings → Watch → Catalogs). A source this build does
    /// not have is skipped by the registry, so a file from another version is
    /// not a fault.
    #[serde(default = "catalogs_by_default")]
    pub catalog_sources: Vec<String>,
    /// The metadata sources the details page may ask, in priority order
    /// (Settings → Watch → Metadata providers): the first to fill a field keeps
    /// it, the next fills what is still empty. TMDB leads and Cinemeta is the
    /// fallback — a source this build does not have is skipped, so the order is
    /// already right when it lands.
    #[serde(default = "metadata_by_default")]
    pub metadata_sources: Vec<String>,
    /// The collection rails the person turned on for the Shelf tab, in rail order
    /// (Settings → Watch → Collections). Ids are the sources' own
    /// (`tmdb:top-rated:movie`); one this build does not have is skipped.
    #[serde(default)]
    pub collections: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn stop_service_by_default() -> bool {
    true
}

/// Cinemeta is the catalogue Watch ships with, so it is on to begin with.
fn catalogs_by_default() -> Vec<String> {
    vec!["cinemeta".to_owned()]
}

/// TMDB first, Cinemeta as the fallback: the richer source decides the card, and
/// nothing is built around Cinemeta being the one that answers.
fn metadata_by_default() -> Vec<String> {
    vec!["tmdb".to_owned(), "cinemeta".to_owned()]
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stop_service_when_done: true,
            catalog_sources: catalogs_by_default(),
            metadata_sources: metadata_by_default(),
            collections: Vec::new(),
            extra: Map::new(),
        }
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
    fn cinemeta_is_the_catalog_source_to_begin_with() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&app(dir.path())).catalog_sources, ["cinemeta"], "no file: the catalogue we ship with");
        assert_eq!(Settings::default().catalog_sources, ["cinemeta"]);
    }

    #[test]
    fn tmdb_leads_the_metadata_sources_and_cinemeta_follows() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&app(dir.path())).metadata_sources, ["tmdb", "cinemeta"], "no file: TMDB first, Cinemeta as the fallback");
        assert_eq!(Settings::default().metadata_sources, ["tmdb", "cinemeta"]);
    }

    #[test]
    fn no_collection_rail_is_on_to_begin_with() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read(&app(dir.path())).collections.is_empty(),
            "the Shelf starts with Continue watching, the watchlist and the favourites only"
        );
        assert!(Settings::default().collections.is_empty());
    }

    #[test]
    fn the_chosen_catalog_sources_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let settings = Settings { catalog_sources: vec!["tmdb".to_owned(), "cinemeta".to_owned()], ..Settings::default() };
        write(&app, &settings).unwrap();
        assert_eq!(read(&app).catalog_sources, ["tmdb", "cinemeta"], "the order is the person's");
        let stored: Value = json_store::read(&path(&app)).unwrap();
        assert_eq!(stored["catalogSources"], Value::from(vec!["tmdb", "cinemeta"]), "{stored}");
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
