//! Browsing a catalog a page at a time, into the Watch library.
//!
//! The Movies and Series tabs are not a filter over the rails: they ask the
//! catalog source for the next page of a catalog (`top`, a genre, a search) and
//! it into the same projection the rest of the screens read, so cards, details,
//! streams and progress keep working unchanged. Everything here is blocking:
//! the app calls it from a background task.

use std::time::Duration;

use anyhow::{anyhow, Result};
use heronite_data::paths::AppPaths;

use heronite_watch_core::catalog::CatalogCtx;
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_state::collection;
use heronite_watch_state::store::StoreCredentials;

pub use heronite_watch_core::catalog::{
    CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor, WatchCatalogSource,
};

/// How much of a catalog answer the transport keeps. A page can be one whole CDN
/// file — Simkl's airing calendar is ~2.5 MB of JSON for a month of shows — where
/// the default megabyte is for manifests and detail records.
const CATALOG_BYTES: u64 = 8 << 20;
/// A CDN file of that size deserves longer than a manifest does.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

/// The transport a catalog is read with: the app's own, with room for a whole
/// list file.
fn catalog_transport() -> UreqTransport {
    UreqTransport::with_limits(CATALOG_TIMEOUT, CATALOG_BYTES)
}

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
    let http = catalog_transport();
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
    let http = catalog_transport();
    page_with(&http, app, source, request)
}

/// [`page`] with an injected transport, for tests.
pub fn page_with(http: &dyn Transport, app: &AppPaths, source: &str, request: &CatalogRequest) -> Result<CatalogPage> {
    let credentials = StoreCredentials::new(app);
    let ids = heronite_watch_state::catalog_cache::CatalogIds::at(app);
    let ctx = CatalogCtx { http, credentials: &credentials, cache: Some(&ids) };
    let mut page = source_of(source)?.catalog(&ctx, request)?;
    let key = feed_key(source, request);
    for title in &mut page.items {
        title.rails = vec![heronite_watch_core::title::RailMark::feed(source, &key)];
    }
    Ok(page)
}

/// Where a kept title says it came from: Simkl's two lists on a film's page.
pub const RELATED_SOURCE: &str = "simkl";
/// The key a title opened from those lists wears: not a catalogue's, not the
/// page's scratch feed — a title the person chose, which nothing replaces.
pub const OPENED_CATALOG: &str = "opened";
/// The key those titles wear. It is deliberately **not** a catalogue's
/// (`{kind}:…`): a catalogue page's replacement removes the feeds of its own
/// kind, and these are not one of them.
pub const RELATED_CATALOG: &str = "related";

/// The mark's prefix: one feed per page, so a page's rails replace the last
/// page's rather than piling up (`related:{imdb}`).
pub fn related_prefix() -> String {
    format!("{RELATED_CATALOG}:")
}

/// Keeps a page's rail titles in the Watch library, so each has a key the
/// console's own tiles can draw and open — and **replaces** the set the previous
/// page kept, so the library holds one page's recommendations at a time.
///
/// The mark is not decoration: the library's refresh keeps a title because *a
/// rail* named it (`heronite_watch_state::collection::refresh` drops a title no
/// mark names, since "no mark" means "what the sources answered just now"), and a
/// catalogue page's replacement removes the feed keys of its own kind. A title
/// kept here therefore wears `related:{page}` under Simkl's name — which is also
/// what its card says it came from — and survives both, while the next page's
/// keep takes it away.
pub fn keep_related(app: &AppPaths, page: &str, titles: Vec<heronite_watch_core::title::Title>) -> Result<usize> {
    use heronite_watch_core::title::RailMark;

    let catalog = format!("{RELATED_CATALOG}:{page}");
    let marked: Vec<_> = titles.into_iter().map(|title| title.listed(RailMark::feed(RELATED_SOURCE, &catalog))).collect();
    let root = app.cache.join(collection::ROOT);
    // The page being looked at is never taken away, even when it is itself one of
    // the last page's recommendations: that is the title on screen.
    let added = collection::replace_scoped(&root, &related_prefix(), Some(page), &marked)?;
    // Where they went, and what the write cost: the one line a bug report about
    // "the rails are empty" needs.
    log::info!("watch rails: {added} of {} titles kept for {page} under {catalog} (projection {})", marked.len(), root.display());
    Ok(added)
}

/// The ids the projection holds, **whatever the screens show**: a title the person
/// hid is still kept, so a page's rails must not count it as missing — they would
/// ask for it again on every render and never be satisfied.
pub fn kept_ids(app: &AppPaths) -> std::collections::HashSet<String> {
    heronite_watch_state::collection::titles(&app.cache.join(heronite_watch_state::collection::ROOT))
        .into_iter()
        .map(|title| title.key.id.as_str().to_owned())
        .collect()
}

/// A title the person opened from a page's rails: it stops being scratch. Its
/// mark becomes `simkl/opened`, which no replacement owns — not the page's feed
/// (the next page takes that away) and not a catalogue's (a tab's replacement
/// removes its own kind). Opening a recommendation is how it joins the library,
/// so nothing the person actually opened disappears under them.
pub fn adopt_related(app: &AppPaths, title: heronite_watch_core::title::Title) -> Result<bool> {
    use heronite_watch_core::title::RailMark;

    let root = app.cache.join(collection::ROOT);
    let adopted = collection::adopt(&root, &title, RailMark::feed(RELATED_SOURCE, OPENED_CATALOG))?;
    log::info!(
        "watch rails: {} ({}) adopted as an opened title ({})",
        title.name,
        title.key.id.as_str(),
        if adopted { "written" } else { "already" }
    );
    Ok(adopted)
}

/// [`more_with`] with the transport a catalog needs: a page may be a whole CDN
/// file, which the app's default transport would cut short. The screens use
/// this; only a test injects its own.
pub fn more(app: &AppPaths, source: &str, request: &CatalogRequest) -> Result<(CatalogPage, usize)> {
    more_with(&catalog_transport(), app, source, request)
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

/// [`replace_with`] with the transport a catalog needs (see [`more`]).
pub fn replace(app: &AppPaths, source: &str, request: &CatalogRequest) -> Result<(CatalogPage, usize)> {
    replace_with(&catalog_transport(), app, source, request)
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
        };
        let page = page_with(&fake, &app(dir.path()), "cinemeta", &request).unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.more, "a full page says another may follow");
        let stremio = heronite_watch_core::title::ProviderId::declared("stremio");
        assert!(
            page.items.iter().all(|title| !title.rails.is_empty()) && page.items.iter().all(|title| title.played_by(&stremio)),
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
        assert_eq!(ids, ["cinemeta", "tmdb", "simkl"], "the catalogue Watch ships with, then TMDB, then Simkl");

        let on = enabled(&["tmdb".to_owned(), "gone".to_owned(), "cinemeta".to_owned()]);
        assert_eq!(on.iter().map(|source| source.descriptor().id).collect::<Vec<_>>(), ["tmdb", "cinemeta"]);
        assert!(enabled(&[]).is_empty(), "nothing on means nothing to browse by");
    }

    #[test]
    fn keeping_a_pages_rails_again_leaves_the_same_titles() {
        use heronite_watch_core::metadata::Related;

        let related = |id: &str| Related { id: id.into(), kind: "movie".into(), name: id.into(), ..Related::default() };
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let titles = |app: &AppPaths| heronite_watch_state::collection::titles(&app.cache.join(collection::ROOT));
        let rails = || vec![crate::metadata::related_title(&related("tt400")), crate::metadata::related_title(&related("tt401"))];

        assert_eq!(keep_related(&app, "tt4", rails()).unwrap(), 2);
        // The app keeps the **whole** set every time, because a keep replaces the
        // page's scoped feed: handing it only the titles it thought missing deleted
        // the rest, and the next reload asked for them again — a loop, not a
        // refresh. This is that rule, held (the count is what was written, not
        // what was missing: the feed is replaced, then written again).
        keep_related(&app, "tt4", rails()).unwrap();
        let ids: Vec<String> = titles(&app).iter().map(|title| title.key.id.as_str().to_owned()).collect();
        assert_eq!(ids, ["tt400", "tt401"], "the page's rails survive being kept again");
    }

    #[test]
    fn a_pages_rails_are_kept_under_the_page_and_replace_the_last_pages() {
        use heronite_watch_core::metadata::Related;

        let related = |id: &str| Related { id: id.into(), kind: "movie".into(), name: id.into(), ..Related::default() };
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let root = app.cache.join(collection::ROOT);
        let titles = |app: &AppPaths| heronite_watch_state::collection::titles(&app.cache.join(collection::ROOT));

        let kept = keep_related(
            &app,
            "tt1",
            vec![crate::metadata::related_title(&related("tt200")), crate::metadata::related_title(&related("tt201"))],
        )
        .unwrap();
        assert_eq!(kept, 2);
        let marks: Vec<String> = titles(&app)
            .iter()
            .map(|title| title.rails.first().map(heronite_watch_core::title::RailMark::as_wire).unwrap_or_default())
            .collect();
        assert_eq!(marks, ["related:tt1", "related:tt1"], "one feed per page, under Simkl's name");

        // Another page's keep takes them away: the library holds one page at a time.
        keep_related(&app, "tt2", vec![crate::metadata::related_title(&related("tt300"))]).unwrap();
        let ids: Vec<String> = titles(&app).iter().map(|title| title.key.id.as_str().to_owned()).collect();
        assert_eq!(ids, ["tt300"], "the previous page's rails are replaced, not piled up");
        assert!(root.exists(), "the projection is what was written");

        // The page's tiles draw by library key, so the kept titles must come back
        // as games from the same load the screens do — this is the line that was
        // missing when the rails could only ever say "Loading titles…".
        let loaded = heronite_data::Library::load_with(&heronite_data::DataPaths::at(root.clone()), false).expect("the projection loads");
        let kept = loaded
            .games
            .iter()
            .filter_map(|game| game.game.extra.as_ref()?.get("watchId")?.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        assert_eq!(kept, ["tt300"], "the kept title is a game the page can draw and open: {kept:?}");
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
        let provider = heronite_watch_core::title::ProviderId::declared("stremio");
        let rail_film = heronite_watch_core::title::Title::new(
            heronite_watch_core::title::TitleKey::of_source("movie", "tt0000001", &provider),
            "Rail Film",
        )
        .plays(provider);
        collection::append(&root, &[rail_film]).unwrap();

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
