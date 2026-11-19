//! The person's own shelf on disk: `<config>/watch-personal.json`.
//!
//! **User data**, not cache: a library refresh never touches it, a cache wipe never
//! loses it, and it is hand-editable. Writes go through `json_store`, which writes a
//! temporary file and renames it, so a reader never sees half a file.
//!
//! An unreadable file is the default value — the person's shelf is not a reason to
//! refuse to start — and it is said **once per path** in the log, because the file is
//! read on every collection of rails.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};

use heronite_data::json_store;
use heronite_data::paths::AppPaths;
use heronite_watch_core::personal::PersonalLibrary;

/// The file, under the config directory.
pub const FILE: &str = "watch-personal.json";

pub fn path(app: &AppPaths) -> PathBuf {
    app.config.join(FILE)
}

/// The shelf as stored; a missing or unreadable file is the empty one.
pub fn read(app: &AppPaths) -> PersonalLibrary {
    let path = path(app);
    match json_store::read(&path) {
        Ok(library) => library,
        Err(error) => {
            // A file that is simply not there yet is not worth a line.
            if path.is_file() {
                warn_once(&path, &error);
            }
            PersonalLibrary::default()
        }
    }
}

pub fn write(app: &AppPaths, library: &PersonalLibrary) -> Result<()> {
    let path = path(app);
    keep_unreadable(&path);
    json_store::write(&path, library).with_context(|| format!("writing {}", path.display()))
}

/// Keeps a file that is there but does not parse beside itself (`….json.bak`)
/// before the first write replaces it: the shelf is hand-editable, and one
/// mistyped brace must not be the end of what the person typed.
fn keep_unreadable(path: &Path) {
    if !path.is_file() || json_store::read::<PersonalLibrary>(path).is_ok() {
        return;
    }
    let backup = path.with_extension("json.bak");
    if backup.exists() {
        return;
    }
    match std::fs::copy(path, &backup) {
        Ok(_) => log::warn!("{}: kept as {} before it was replaced", path.display(), backup.display()),
        Err(error) => log::warn!("{}: {error}; it could not be kept aside", path.display()),
    }
}

/// Says how a file could not be read once, not on every frame that reads it.
fn warn_once(path: &Path, error: &anyhow::Error) {
    static SAID: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    let said = SAID.get_or_init(|| Mutex::new(HashSet::new()));
    let first = said.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(path.to_path_buf());
    if first {
        log::warn!("{}: {error:#}; starting from an empty shelf", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::personal::ShelfTitle;
    use serde_json::Value;

    fn app(dir: &Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    fn item(kind: &str, id: &str) -> ShelfTitle {
        ShelfTitle { provider: "stremio".into(), kind: kind.into(), id: id.into(), name: id.into(), ..ShelfTitle::default() }
    }

    #[test]
    fn a_missing_file_is_an_empty_shelf() {
        let dir = tempfile::tempdir().unwrap();
        let library = read(&app(dir.path()));
        assert!(library.watchlist.is_empty() && library.favourites.is_empty() && library.lists.is_empty());
    }

    #[test]
    fn the_shelf_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let mut library = PersonalLibrary::default();
        library.add_watchlist(&item("movie", "tt1"));
        library.add_favourite(&item("series", "tt2"));
        library.lists.push(heronite_watch_core::personal::PersonalList {
            id: "sci-fi".into(),
            name: "Sci-fi to watch".into(),
            items: vec![item("movie", "tt3")],
            ..Default::default()
        });
        write(&app, &library).unwrap();

        let read_back = read(&app);
        assert_eq!(read_back, library);
        assert!(read_back.watchlisted(&item("movie", "tt1")));
        assert!(read_back.favourited(&item("series", "tt2")));
        assert_eq!(read_back.list("sci-fi").map(|list| list.items.len()), Some(1));
    }

    #[test]
    fn a_field_a_newer_build_wrote_survives_a_save_here() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        std::fs::create_dir_all(&app.config).unwrap();
        let written = serde_json::json!({
            "watchlist": [],
            "favourites": [],
            "lists": [],
            "fromTheFuture": { "collections": ["tmdb:top-rated"] }
        });
        std::fs::write(path(&app), written.to_string()).unwrap();

        let mut library = read(&app);
        library.add_watchlist(&item("movie", "tt1"));
        write(&app, &library).unwrap();

        let stored: Value = json_store::read(&path(&app)).unwrap();
        assert_eq!(stored["fromTheFuture"]["collections"][0], Value::from("tmdb:top-rated"), "{stored}");
        assert_eq!(stored["watchlist"][0]["id"], Value::from("tt1"));
    }

    #[test]
    fn a_corrupt_file_is_an_empty_shelf_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        std::fs::create_dir_all(&app.config).unwrap();
        std::fs::write(path(&app), b"not json").unwrap();
        let library = read(&app);
        assert!(library.watchlist.is_empty(), "the app still starts");
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_before_the_first_write_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        std::fs::create_dir_all(&app.config).unwrap();
        std::fs::write(path(&app), b"{\"watchlist\": [ oops").unwrap();

        let mut library = PersonalLibrary::default();
        library.add_watchlist(&item("movie", "tt1"));
        write(&app, &library).unwrap();

        let backup = path(&app).with_extension("json.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{\"watchlist\": [ oops", "what the person typed is not lost");
        assert_eq!(read(&app).watchlist.len(), 1, "and the shelf is written");

        // A second write keeps the first backup rather than overwriting it.
        write(&app, &library).unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{\"watchlist\": [ oops");
    }
}
