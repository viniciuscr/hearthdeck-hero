//! Browsing a provider's catalog a page at a time, into the Watch library.
//!
//! The Movies and Series tabs are not a filter over the rails: they ask the
//! provider for the next page of a catalog (`top`, a genre, a search) and merge
//! it into the same projection the rest of the screens read, so cards, details,
//! streams and progress keep working unchanged. Everything here is blocking:
//! the app calls it from a background task.

use anyhow::{anyhow, Result};
use heronite_data::paths::AppPaths;

use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::Ctx;
use heronite_watch_core::types::MovieItem;
use heronite_watch_state::collection;
use heronite_watch_state::store::StoreCredentials;

use crate::registry;

pub use heronite_watch_core::provider::CatalogRequest;

/// How many items a page holds (Cinemeta answers 50; the provider decides).
pub const PAGE: u32 = 50;

/// One page of a catalog, from the provider that owns it.
pub fn page(app: &AppPaths, provider: &str, request: &CatalogRequest) -> Result<Vec<MovieItem>> {
    let http = UreqTransport::new();
    page_with(&http, app, provider, request)
}

/// [`page`] with an injected transport, for tests.
pub fn page_with(http: &dyn Transport, app: &AppPaths, provider: &str, request: &CatalogRequest) -> Result<Vec<MovieItem>> {
    let credentials = StoreCredentials::new(app);
    let ctx = Ctx { http, credentials: &credentials };
    registry::find(provider).ok_or_else(|| anyhow!("Unknown Watch provider: {provider}"))?.catalog(&ctx, request)
}

/// Fetches the next page of `request` and merges it into the Watch library.
/// Returns the page (its ids are what a search shows) and how many items were
/// new; an empty page means the catalog has no more.
pub fn more_with(http: &dyn Transport, app: &AppPaths, provider: &str, request: &CatalogRequest) -> Result<(Vec<MovieItem>, usize)> {
    let items = page_with(http, app, provider, request)?;
    let root = app.cache.join(collection::ROOT);
    let added = collection::append(&root, &items)?;
    Ok((items, added))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use heronite_data::{DataPaths, Library};
    use serde_json::Value;
    use url::Url;

    /// Answers every catalog request with `metas`.
    struct Fake {
        seen: Mutex<Vec<String>>,
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            Ok(br#"{"metas":[{"id":"tt0133093","type":"movie","name":"The Matrix"},{"id":"tt0234215","type":"movie","name":"Reloaded"}]}"#
                .to_vec())
        }
        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            Ok(b"{}".to_vec())
        }
    }

    fn app(root: &std::path::Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    #[test]
    fn a_page_asks_the_path_extras_and_marks_its_items() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake { seen: Mutex::new(Vec::new()) };
        let request = CatalogRequest { kind: "movie".into(), catalog: "top".into(), skip: 100, search: None, genre: Some("Sci-Fi".into()) };
        let items = page_with(&fake, &app(dir.path()), "stremio", &request).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.catalog && item.provider == "stremio"), "a page's items are catalog items");
        let seen = fake.seen.lock().unwrap().clone();
        assert_eq!(seen, ["https://v3-cinemeta.strem.io/catalog/movie/top/skip=100&genre=Sci-Fi.json"]);
    }

    #[test]
    fn a_page_is_merged_into_the_library_and_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let fake = Fake { seen: Mutex::new(Vec::new()) };
        let request = CatalogRequest::first("movie");
        let (items, added) = more_with(&fake, &app, "stremio", &request).unwrap();
        assert_eq!(items.len(), 2, "the page is what a search shows");
        assert_eq!(added, 2, "and it is merged into the library");
        let library = Library::load_with(&DataPaths::at(app.cache.join(collection::ROOT)), true).unwrap();
        assert_eq!(library.games.len(), 2);
        assert!(library.games.iter().any(|game| game.title == "The Matrix"));

        // The same page again adds nothing.
        let (items, added) = more_with(&fake, &app, "stremio", &request).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(added, 0);
    }
}
