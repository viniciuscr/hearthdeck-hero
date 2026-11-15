//! The small cache a catalogue keeps between runs: the IMDb id TMDB holds for
//! its own ids. They never change and each costs a request, so a page of twenty
//! must not pay twenty requests on every visit.
//!
//! Regenerable and safe to delete, like the rest of the cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};

use heronite_data::{json_store, paths::AppPaths};
use heronite_watch_core::catalog::CatalogCache;

/// Where kept ids live under the app cache.
pub const ROOT: &str = "watch-catalog";

/// The cache on disk: one file per `(source, kind, id)`.
pub struct CatalogIds {
    root: PathBuf,
}

impl CatalogIds {
    pub fn new(cache: &Path) -> Self {
        Self { root: cache.join(ROOT).join("ids") }
    }

    pub fn at(app: &AppPaths) -> Self {
        Self::new(&app.cache)
    }

    fn path(&self, source: &str, kind: &str, id: u64) -> PathBuf {
        self.root.join(format!("{}-{}-{id}.json", part(source), part(kind)))
    }
}

impl CatalogCache for CatalogIds {
    fn imdb_id(&self, source: &str, kind: &str, id: u64) -> Option<Option<String>> {
        let path = self.path(source, kind, id);
        // A file that is not there means "never asked"; a file that is, even
        // with a null id, means "asked, and it has none".
        if !path.is_file() {
            return None;
        }
        let value: Value = json_store::read(&path).ok()?;
        Some(value.get("imdb").and_then(Value::as_str).map(str::to_owned))
    }

    fn keep_imdb_id(&self, source: &str, kind: &str, id: u64, imdb: Option<&str>) {
        let path = self.path(source, kind, id);
        if let Err(error) = json_store::write(&path, &json!({ "imdb": imdb })) {
            log::warn!("watch catalog cache: {error:#}");
        }
    }
}

/// A cache in memory, for a test or a throwaway run.
#[derive(Default)]
pub struct MemoryIds {
    ids: Mutex<HashMap<(String, String, u64), Option<String>>>,
}

impl CatalogCache for MemoryIds {
    fn imdb_id(&self, source: &str, kind: &str, id: u64) -> Option<Option<String>> {
        self.ids.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&(source.to_owned(), kind.to_owned(), id)).cloned()
    }

    fn keep_imdb_id(&self, source: &str, kind: &str, id: u64, imdb: Option<&str>) {
        self.ids
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((source.to_owned(), kind.to_owned(), id), imdb.map(str::to_owned));
    }
}

/// A path segment from a name: anything that is not a letter, a digit, `-` or
/// `_` becomes `-`, so an id can never escape the directory.
fn part(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(dir: &Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    #[test]
    fn an_id_is_kept_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let ids = CatalogIds::at(&app(dir.path()));
        assert_eq!(ids.imdb_id("tmdb", "movie", 603), None, "never asked before");

        ids.keep_imdb_id("tmdb", "movie", 603, Some("tt0133093"));
        assert_eq!(ids.imdb_id("tmdb", "movie", 603), Some(Some("tt0133093".to_owned())));

        // A title TMDB holds no IMDb id for is remembered as such, so it is not
        // asked again on every page.
        ids.keep_imdb_id("tmdb", "movie", 604, None);
        assert_eq!(ids.imdb_id("tmdb", "movie", 604), Some(None), "asked before, and it has none");
        assert_eq!(ids.imdb_id("tmdb", "tv", 604), None, "the kind is part of the key");
    }

    #[test]
    fn the_file_lives_under_the_watch_catalog_root() {
        let dir = tempfile::tempdir().unwrap();
        let ids = CatalogIds::at(&app(dir.path()));
        ids.keep_imdb_id("tmdb", "movie", 603, Some("tt0133093"));
        let path = dir.path().join("cache").join(ROOT).join("ids").join("tmdb-movie-603.json");
        assert!(path.is_file(), "{}", path.display());
    }

    #[test]
    fn an_odd_name_cannot_escape_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let ids = CatalogIds::at(&app(dir.path()));
        ids.keep_imdb_id("../../etc", "mo/vie", 1, Some("tt1"));
        assert_eq!(ids.imdb_id("../../etc", "mo/vie", 1), Some(Some("tt1".to_owned())));
        let listed: Vec<_> = std::fs::read_dir(dir.path().join("cache").join(ROOT).join("ids")).unwrap().flatten().collect();
        assert_eq!(listed.len(), 1, "one file, inside the directory");
    }

    #[test]
    fn the_memory_cache_answers_the_same_way() {
        let ids = MemoryIds::default();
        assert_eq!(ids.imdb_id("tmdb", "movie", 603), None);
        ids.keep_imdb_id("tmdb", "movie", 603, Some("tt0133093"));
        assert_eq!(ids.imdb_id("tmdb", "movie", 603), Some(Some("tt0133093".to_owned())));
        ids.keep_imdb_id("tmdb", "movie", 603, None);
        assert_eq!(ids.imdb_id("tmdb", "movie", 603), Some(None));
    }
}
