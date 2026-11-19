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
    /// they are listed (Settings → Watch → Sources → Catalogs). A source this build does
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
    /// Leave greyed placeholders out of the lists entirely: a person who does not
    /// want titles they cannot play in the way of the ones they can.
    #[serde(default)]
    pub hide_placeholders: bool,
    /// The rails the Watch space's **first page** draws, in order
    /// (Settings → Watch → Home). Two kinds of id: `continue:<provider>` is that
    /// provider's Continue watching, and anything else is a collection a source
    /// declared (`tmdb:top-rated:movie`, `tmdb:recommended:movie`). One this
    /// build does not have is skipped, and the list is hand-editable.
    ///
    /// `collections` is what the field was called while the rails lived on the
    /// Shelf: a file written then still reads.
    #[serde(default, alias = "collections")]
    pub home_rails: Vec<String>,
    /// The **media providers** whose streams the Watch space may use
    /// (Settings → Watch → Sources → Streams). A provider left out keeps its
    /// account and its rails; only asking it for a stream is refused, so a
    /// person can keep Stremio's history without its torrents.
    #[serde(default = "streams_by_default")]
    pub stream_providers: Vec<String>,
    /// The **trackers** the person turned on (`simkl`): services the watch
    /// history is sent to, after the local write and never instead of it. A
    /// tracker this build does not have, or one nobody is signed in to, is
    /// skipped — the file is hand-editable and survives a version change.
    #[serde(default = "trackers_by_default")]
    pub trackers: Vec<String>,
    /// The providers whose history is **brought in** (`simkl`): the reverse of
    /// `trackers`. Separate on purpose — turning import on must never queue a
    /// backfill of what was watched here. Empty until the person chooses, like
    /// everything else.
    #[serde(default = "import_by_default")]
    pub import_providers: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn stop_service_by_default() -> bool {
    true
}

/// **Nothing is on until the person turns it on.** A catalogue is a choice about
/// what the tabs browse, so a fresh machine browses none; the Sources page is
/// where they are switched, in any combination — some, all or none.
fn catalogs_by_default() -> Vec<String> {
    Vec::new()
}

/// Same for what describes a title: no source is asked until one is chosen, and
/// then in the order the person put them.
fn metadata_by_default() -> Vec<String> {
    Vec::new()
}

/// Same for trackers: switching one on is also its first sync, so it is a
/// decision, not a default.
fn trackers_by_default() -> Vec<String> {
    Vec::new()
}

/// Nothing is pulled until the person turns it on.
fn import_by_default() -> Vec<String> {
    Vec::new()
}

/// The exception, and the space's own: the media provider is what makes Watch a
/// Watch space (its account, its rails and its streams). It is on until the
/// person turns it off in Sources → Streams.
fn streams_by_default() -> Vec<String> {
    vec!["stremio".to_owned()]
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stop_service_when_done: true,
            hide_placeholders: false,
            catalog_sources: catalogs_by_default(),
            metadata_sources: metadata_by_default(),
            home_rails: Vec::new(),
            trackers: trackers_by_default(),
            import_providers: import_by_default(),
            stream_providers: streams_by_default(),
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
    fn hiding_placeholders_is_off_until_asked() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        assert!(!read(&app).hide_placeholders, "a fresh install shows what a provider knows");

        let mut settings = read(&app);
        settings.hide_placeholders = true;
        write(&app, &settings).unwrap();
        assert!(read(&app).hide_placeholders, "and keeps the choice");
    }

    #[test]
    fn stopping_the_service_is_on_by_default() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&app(dir.path())).stop_service_when_done, "no file: the default stops what we started");
        assert!(Settings::default().stop_service_when_done);
    }

    #[test]
    fn nothing_is_on_until_the_person_turns_it_on() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = read(&app(dir.path()));
        assert!(fresh.catalog_sources.is_empty(), "no catalogue is browsed by default");
        assert!(fresh.metadata_sources.is_empty(), "no source describes a title by default");
        assert!(fresh.trackers.is_empty(), "no tracker is sent to by default");
        assert_eq!(fresh.stream_providers, ["stremio"], "the media provider is the space itself, and stays on");
        assert!(Settings::default().catalog_sources.is_empty());
    }

    #[test]
    fn no_collection_rail_is_on_to_begin_with() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            read(&app(dir.path())).home_rails.is_empty(),
            "the Shelf starts with Continue watching, the watchlist and the favourites only"
        );
        assert!(Settings::default().home_rails.is_empty());
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
