//! Browsing a catalog a page at a time, into the Watch library.
//!
//! The Movies and Series tabs are not a filter over the rails: they ask the
//! catalog source for the next page of a catalog (`top`, a genre, a search) and
//! it into the same projection the rest of the screens read, so cards, details,
//! streams and progress keep working unchanged. Everything here is blocking:
//! the app calls it from a background task.

use anyhow::{anyhow, Result};
use heronite_data::paths::AppPaths;

use heronite_watch_core::catalog::CatalogCtx;
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_state::collection;
use heronite_watch_state::store::StoreCredentials;

pub use heronite_watch_core::catalog::{
    CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor, WatchCatalogSource,
};

/// The catalog sources this build has, and which of them the person turned on.
pub fn sources() -> &'static [&'static dyn WatchCatalogSource] {
    heronite_watch_catalogs::sources()
}

/// The sources the settings turn on, in the order they are listed.
pub fn enabled(ids: &[String]) -> Vec<&'static dyn WatchCatalogSource> {
    heronite_watch_catalogs::enabled(ids)
}

/// The catalogs a source offers the tabs: its own manifest decides, so the
/// screens render what it declares instead of assuming Cinemeta's.
pub fn catalogs(app: &AppPaths, provider: &str) -> Result<Vec<CatalogDescriptor>> {
    let http = UreqTransport::new();
    catalogs_with(&http, app, provider)
}

/// [`catalogs`] with an injected transport, for tests.
pub fn catalogs_with(http: &dyn Transport, app: &AppPaths, source: &str) -> Result<Vec<CatalogDescriptor>> {
    let credentials = StoreCredentials::new(app);
    let ids = heronite_watch_state::catalog_cache::CatalogIds::at(app);
    let ctx = CatalogCtx { http, credentials: &credentials, cache: Some(&ids) };
    source_of(source)?.catalogs(&ctx)
}

/// The catalog source with `id`, or a fault naming it.
fn source_of(id: &str) -> Result<&'static dyn WatchCatalogSource> {
    heronite_watch_catalogs::find(id).ok_or_else(|| anyhow!("Unknown catalog source: {id}"))
}

/// The key a page's items carry: the kind, the source, the catalog, the order
/// and the filters that produced them. The kind leads because the projection
/// replaces a whole kind's pages by prefix; the source follows because two
/// sources may name a catalog alike.
pub fn feed_key(source: &str, request: &CatalogRequest) -> String {
    let mut key = format!("{}:{source}:{}:{:?}", request.kind, request.catalog.trim(), request.sort);
    for (id, value) in &request.filters {
        key.push_str(&format!(";{id}={value}"));
    }
    key
}

/// One page of a catalog, from the provider that owns it.
pub fn page(app: &AppPaths, source: &str, request: &CatalogRequest) -> Result<CatalogPage> {
    let http = UreqTransport::new();
    page_with(&http, app, source, request)
}

/// [`page`] with an injected transport, for tests.
pub fn page_with(http: &dyn Transport, app: &AppPaths, source: &str, request: &CatalogRequest) -> Result<CatalogPage> {
    let credentials = StoreCredentials::new(app);
    let ids = heronite_watch_state::catalog_cache::CatalogIds::at(app);
    let ctx = CatalogCtx { http, credentials: &credentials, cache: Some(&ids) };
    let mut page = source_of(source)?.catalog(&ctx, request)?;
    let key = feed_key(source, request);
    for item in &mut page.items {
        item.catalog = Some(key.clone());
    }
    Ok(page)
}

/// Fetches the next page of `request` and merges it into the Watch library.
/// Returns the page (its ids are what a search shows) and how many items were
/// new; an empty page means the catalog has no more.
pub fn more_with(http: &dyn Transport, app: &AppPaths, source: &str, request: &CatalogRequest) -> Result<(CatalogPage, usize)> {
    let page = page_with(http, app, source, request)?;
    let root = app.cache.join(collection::ROOT);
    let added = collection::append(&root, &page.items)?;
    Ok((page, added))
}

/// The first page of a feed, replacing whatever the previous catalog, order or
/// filter had put in the library for that kind. Returns the page and how many
/// items are new.
pub fn replace_with(http: &dyn Transport, app: &AppPaths, source: &str, request: &CatalogRequest) -> Result<(CatalogPage, usize)> {
    let page = page_with(http, app, source, request)?;
    let root = app.cache.join(collection::ROOT);
    let added = collection::replace(&root, &request.kind, &page.items)?;
    Ok((page, added))
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
            // A different catalog answers with a different title, so a replaced
            // feed is visible in the projection.
            let body: &[u8] = if url.path().contains("imdbRating") {
                br#"{"metas":[{"id":"tt9999999","type":"movie","name":"Best Rated"}]}"#
            } else {
                br#"{"metas":[{"id":"tt0133093","type":"movie","name":"The Matrix"},{"id":"tt0234215","type":"movie","name":"Reloaded"}]}"#
            };
            Ok(body.to_vec())
        }
        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            Ok(b"{}".to_vec())
        }
    }

    /// Answers with no items: the catalog's end.
    struct Empty;

    impl Transport for Empty {
        fn get(&self, _: &Url) -> Result<Vec<u8>> {
            Ok(br#"{"metas":[]}"#.to_vec())
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
        let request = CatalogRequest {
            kind: "movie".into(),
            catalog: "top".into(),
            skip: 100,
            search: None,
            sort: CatalogSort::Popular,
            filters: vec![("genre".into(), "Sci-Fi".into())],
            account: false,
        };
        let page = page_with(&fake, &app(dir.path()), "cinemeta", &request).unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.more, "a full page says another may follow");
        assert!(
            page.items.iter().all(|item| item.catalog.is_some()) && page.items.iter().all(|item| item.provider == "stremio"),
            "a page's items carry their feed and name who plays them"
        );
        let seen = fake.seen.lock().unwrap().clone();
        assert_eq!(seen, ["https://v3-cinemeta.strem.io/catalog/movie/top/skip=100&genre=Sci-Fi.json"]);
    }

    #[test]
    fn the_order_and_the_filters_reach_the_provider_as_path_extras() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake { seen: Mutex::new(Vec::new()) };
        let mut request = CatalogRequest::first("movie");
        request.sort = CatalogSort::Rating;
        request.filters = vec![("genre".into(), "Sci-Fi".into())];
        page_with(&fake, &app(dir.path()), "cinemeta", &request).unwrap();
        let seen = fake.seen.lock().unwrap().clone();
        assert_eq!(seen, ["https://v3-cinemeta.strem.io/catalog/movie/imdbRating/genre=Sci-Fi.json"], "the order picks the catalog");
    }

    #[test]
    fn an_empty_page_says_there_is_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let page = page_with(&Empty, &app(dir.path()), "cinemeta", &CatalogRequest::first("movie")).unwrap();
        assert!(page.items.is_empty() && !page.more, "an empty page ends the feed");
    }

    #[test]
    fn the_facade_lists_the_sources_and_only_the_ones_the_settings_name() {
        let ids: Vec<&str> = sources().iter().map(|source| source.descriptor().id).collect();
        assert_eq!(ids, ["cinemeta", "tmdb"], "the catalogue Watch ships with, then TMDB");

        let on = enabled(&["tmdb".to_owned(), "gone".to_owned(), "cinemeta".to_owned()]);
        assert_eq!(on.iter().map(|source| source.descriptor().id).collect::<Vec<_>>(), ["tmdb", "cinemeta"]);
        assert!(enabled(&[]).is_empty(), "nothing on means nothing to browse by");
    }

    #[test]
    fn an_unknown_source_is_a_fault_that_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let error =
            page_with(&Fake { seen: Mutex::new(Vec::new()) }, &app(dir.path()), "nope", &CatalogRequest::first("movie")).unwrap_err();
        assert!(format!("{error:#}").contains("Unknown catalog source: nope"), "{error:#}");
    }

    #[test]
    fn a_feed_key_names_the_catalog_the_order_and_the_filters() {
        let base = CatalogRequest::first("movie");
        let mut by_rating = base.clone();
        by_rating.sort = CatalogSort::Rating;
        let mut filtered = base.clone();
        filtered.filters = vec![("genre".into(), "Sci-Fi".into())];
        let mut later = base.clone();
        later.skip = 150;

        assert_eq!(feed_key("cinemeta", &base), feed_key("cinemeta", &base), "the same request is the same feed");
        assert_ne!(feed_key("cinemeta", &base), feed_key("cinemeta", &by_rating));
        assert_ne!(feed_key("cinemeta", &base), feed_key("cinemeta", &filtered));
        assert_ne!(feed_key("cinemeta", &base), feed_key("tmdb", &base), "another source is another feed");
        assert_eq!(feed_key("cinemeta", &base), feed_key("cinemeta", &later), "the page is not part of the feed");
    }

    #[test]
    fn the_first_page_of_another_order_replaces_the_previous_feed() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let fake = Fake { seen: Mutex::new(Vec::new()) };
        let root = app.cache.join(collection::ROOT);

        let (page, _) = more_with(&fake, &app, "cinemeta", &CatalogRequest::first("movie")).unwrap();
        assert_eq!(page.items.len(), 2);
        // A rails item (no feed) has to survive the replacement.
        let rails = heronite_watch_core::types::MovieItem {
            id: "tt0000001".into(),
            kind: "movie".into(),
            name: "Rail Film".into(),
            top: true,
            ..Default::default()
        };
        collection::append(&root, &[rails]).unwrap();

        let mut rated = CatalogRequest::first("movie");
        rated.sort = CatalogSort::Rating;
        let (page, added) = replace_with(&fake, &app, "cinemeta", &rated).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(added, 1);

        let library = Library::load_with(&DataPaths::at(root), true).unwrap();
        let titles: Vec<&str> = library.games.iter().map(|game| game.title.as_str()).collect();
        assert!(titles.contains(&"Rail Film"), "the rails stay: {titles:?}");
        assert!(titles.contains(&"Best Rated"), "the new feed's page is there: {titles:?}");
        assert!(!titles.contains(&"The Matrix"), "the previous feed's page is gone: {titles:?}");
    }

    #[test]
    fn a_page_is_merged_into_the_library_and_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let fake = Fake { seen: Mutex::new(Vec::new()) };
        let request = CatalogRequest::first("movie");
        let (page, added) = more_with(&fake, &app, "cinemeta", &request).unwrap();
        assert_eq!(page.items.len(), 2, "the page is what the tab shows");
        assert_eq!(added, 2, "and it is merged into the library");
        let library = Library::load_with(&DataPaths::at(app.cache.join(collection::ROOT)), true).unwrap();
        assert_eq!(library.games.len(), 2);
        assert!(library.games.iter().any(|game| game.title == "The Matrix"));

        // The same page again adds nothing.
        let (page, added) = more_with(&fake, &app, "cinemeta", &request).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(added, 0);
    }
}
