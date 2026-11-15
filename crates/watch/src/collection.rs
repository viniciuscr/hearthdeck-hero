//! The Watch space's Collection entry point: the providers' rails become the
//! synthetic `Library` the existing screens read.

use std::collections::HashSet;

use anyhow::Result;
use heronite_data::paths::AppPaths;
use heronite_data::{DataPaths, Library};
use heronite_watch_core::catalog::{CatalogCtx, CatalogRequest, WatchCatalogSource};
use heronite_watch_core::collections::{mark, CollectionDescriptor, MARK_PREFIX};
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::{Ctx, Rail, RailDescriptor};
use heronite_watch_core::types::MovieItem;
use heronite_watch_state::catalog_cache::CatalogIds;
use heronite_watch_state::collection::{self, Fetched};
use heronite_watch_state::store::StoreCredentials;

use crate::registry;

/// Where the proxy keeps its synthetic library.
pub use collection::ROOT;

/// The movies Collection, refreshed from the providers when possible. A
/// provider failure keeps the last good library instead of emptying the screen.
pub fn library(app: &AppPaths, include_hidden: bool) -> Result<Library> {
    let root = app.cache.join(ROOT);
    let http = UreqTransport::new();
    // The collection rails this build has and the person turned on. The refresh
    // keeps a rail's titles while its source cannot answer, and drops the ones a
    // rail no longer holds — or a rail that is now off — with the rail.
    let rails = crate::collections::enabled(app);
    let ids: Vec<String> = rails.iter().map(|(_, rail)| rail.id.clone()).collect();
    if let Err(error) = collection::refresh(&root, &ids, || items(&http, app, &rails)) {
        log::warn!("movies provider: {error:#}; keeping the last library");
    }
    Library::load_with(&DataPaths::at(root), include_hidden)
}

/// Every provider's rails, in registry order. A provider that fails aborts the
/// refresh, so the last good library stays on screen.
fn items(http: &dyn Transport, app: &AppPaths, collections: &[(String, CollectionDescriptor)]) -> Result<Fetched> {
    let credentials = StoreCredentials::new(app);
    let ctx = Ctx { http, credentials: &credentials };
    let mut rails = Vec::new();
    for provider in registry::providers() {
        rails.extend(provider.browse(&ctx)?);
    }
    // Browsing rails come from the catalogue sources the person turned on, not
    // from a media provider: a source that is off contributes nothing, to the
    // home or to the tabs.
    let ids = CatalogIds::at(app);
    let catalog_ctx = CatalogCtx { http, credentials: &credentials, cache: Some(&ids) };
    let enabled = heronite_watch_state::settings::read(app).catalog_sources;
    rails.extend(catalog_rails(&heronite_watch_catalogs::enabled(&enabled), &catalog_ctx));
    // The person's own shelf travels with the library: a title they kept opens
    // like any other, even on a refresh where no rail carried it.
    let shelf = crate::personal::read(app);
    let mut kept = shelf.watchlist;
    kept.extend(shelf.favourites);
    if !kept.is_empty() {
        rails.push(Rail { catalog: RailDescriptor { id: "personal", kind: "" }, items: kept });
    }
    let mut movies = assemble(rails);
    // The collection rails the person turned on, marked on the titles they
    // brought: the Shelf tab reads the mark back, so drawing a rail asks the
    // provider nothing. What they did not bring this time leaves with them.
    let (rails, answered) = collection_rails(http, app, collections);
    mark_collections(&mut movies, rails);
    Ok(Fetched { movies, answered })
}

/// One rail per enabled collection, in the person's order, and the ids of the
/// rails that answered — an empty page is an answer, a source that could not be
/// asked (no key, an outage) is not.
///
/// A rule is a catalogue page asked of the source that declared it, so one rail
/// costs what a tab page costs; a source that cannot answer costs that rail and
/// nothing else.
fn collection_rails(
    http: &dyn Transport,
    app: &AppPaths,
    collections: &[(String, CollectionDescriptor)],
) -> (Vec<(String, Vec<MovieItem>)>, Vec<String>) {
    let mut rails = Vec::new();
    let mut answered = Vec::new();
    for (source, descriptor) in collections {
        match crate::collections::rail_with(http, app, source, descriptor) {
            Ok(items) => {
                answered.push(descriptor.id.clone());
                if !items.is_empty() {
                    rails.push((descriptor.id.clone(), items));
                }
            }
            // The reason goes to the log only: a transport URL can carry a
            // debrid key.
            Err(error) => log::warn!("watch collection {}: {error:#}", descriptor.id),
        }
    }
    (rails, answered)
}

/// Marks the rails' titles in the library, adding the ones no other rail holds.
/// The mark is the rail's (`collection:<id>`, see [`mark`]), which is what the
/// Shelf tab reads back.
///
/// A collection is a membership, not a second copy: a title the rails already
/// carry keeps their fields. A rail the person turned on outranks a catalogue
/// page's own feed key (that page can be replaced; the rail cannot), and the
/// first rail of the settings order names the title — this slice keeps one mark
/// per title.
fn mark_collections(movies: &mut Vec<MovieItem>, rails: Vec<(String, Vec<MovieItem>)>) {
    for (id, items) in rails {
        let rail_mark = mark(&id);
        for item in items {
            match movies.iter_mut().find(|movie| movie.id == item.id && movie.kind == item.kind) {
                Some(movie) => {
                    if movie.catalog.as_deref().is_some_and(|feed| feed.starts_with(MARK_PREFIX)) {
                        continue;
                    }
                    movie.catalog = Some(rail_mark.clone());
                }
                None => movies.push(MovieItem { catalog: Some(rail_mark.clone()), ..item }),
            }
        }
    }
}

/// One rail per enabled source and kind, from that source's first catalogue of
/// the kind, every item marked as a Top item so the home shelves can pick them.
fn catalog_rails(sources: &[&dyn WatchCatalogSource], ctx: &CatalogCtx<'_>) -> Vec<Rail> {
    let mut rails = Vec::new();
    for source in sources {
        let catalogs = match source.catalogs(ctx) {
            Ok(catalogs) => catalogs,
            // One source failing (no key, an outage) must not empty the home.
            Err(error) => {
                log::warn!("watch catalogs {}: {error:#}", source.descriptor().id);
                continue;
            }
        };
        for kind in ["movie", "series"] {
            let Some(descriptor) = catalogs.iter().find(|catalog| catalog.kind == kind) else { continue };
            let request = CatalogRequest { catalog: descriptor.id.clone(), ..CatalogRequest::first(kind) };
            match source.catalog(ctx, &request) {
                Ok(page) if !page.items.is_empty() => {
                    let items = page.items.into_iter().map(|mut movie| {
                        movie.top = true;
                        movie
                    });
                    rails.push(Rail { catalog: RailDescriptor { id: "top", kind }, items: items.collect() });
                }
                Ok(_) => {}
                Err(error) => log::warn!("watch catalogs {}: {error:#}", source.descriptor().id),
            }
        }
    }
    rails
}

/// Continue watching first, then each catalog. The first copy of a title wins,
/// so a title in two rails keeps the placement and the fields of the first.
fn assemble(rails: Vec<Rail>) -> Vec<MovieItem> {
    let mut seen = HashSet::new();
    let mut movies = Vec::new();
    for rail in rails {
        for movie in rail.items {
            if seen.insert(format!("{}:{}", movie.kind, movie.id)) {
                movies.push(movie);
            }
        }
    }
    movies
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::catalog::{CatalogDescriptor, CatalogPage, CatalogSourceDescriptor};

    fn movie(id: &str, top: bool) -> MovieItem {
        MovieItem { id: id.into(), kind: "movie".into(), name: id.into(), top, ..MovieItem::default() }
    }

    struct NoNet;

    impl Transport for NoNet {
        fn get(&self, _: &url::Url) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn post_json(&self, _: &url::Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    struct NoCredentials;

    impl heronite_watch_core::provider::Credentials for NoCredentials {
        fn get(&self, _: &str) -> Result<Option<heronite_watch_core::provider::Secret>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &heronite_watch_core::provider::Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    /// A catalogue source of the test's own, offering one film.
    struct Stub;

    const STUB: CatalogSourceDescriptor = CatalogSourceDescriptor { id: "stub", name: "Stub" };

    impl WatchCatalogSource for Stub {
        fn descriptor(&self) -> &CatalogSourceDescriptor {
            &STUB
        }
        fn catalogs(&self, _: &CatalogCtx<'_>) -> Result<Vec<CatalogDescriptor>> {
            Ok(vec![CatalogDescriptor {
                id: "shelf".into(),
                name: "Shelf".into(),
                kind: "movie".into(),
                sorts: Vec::new(),
                filters: Vec::new(),
                search: false,
                paging: true,
            }])
        }
        fn catalog(&self, _: &CatalogCtx<'_>, _: &CatalogRequest) -> Result<CatalogPage> {
            Ok(CatalogPage { items: vec![movie("tt1", false)], more: false, consumed: 1 })
        }
    }

    #[test]
    fn an_enabled_source_supplies_the_browsing_rail() {
        let http = NoNet;
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };
        let rails = catalog_rails(&[&Stub], &ctx);
        assert_eq!(rails.len(), 1, "one rail, for the kind it offers");
        assert_eq!((rails[0].catalog.id, rails[0].catalog.kind), ("top", "movie"));
        assert!(rails[0].items.iter().all(|item| item.top), "a browsing rail's items are Top items");
    }

    #[test]
    fn a_source_that_is_off_contributes_no_rail() {
        let http = NoNet;
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };
        assert!(catalog_rails(&[], &ctx).is_empty(), "nothing enabled, nothing to browse");
    }

    fn rail(id: &'static str, items: Vec<MovieItem>) -> Rail {
        Rail { catalog: RailDescriptor { id, kind: "movie" }, items }
    }

    #[test]
    fn a_personal_title_rides_along_after_the_rails() {
        let movies = assemble(vec![rail("top", vec![movie("tt1", true)]), rail("personal", vec![movie("tt2", false)])]);
        let ids: Vec<&str> = movies.iter().map(|movie| movie.id.as_str()).collect();
        assert_eq!(ids, ["tt1", "tt2"], "a kept title the rails did not carry is still in the library");
    }

    #[test]
    fn a_collection_rail_marks_its_titles_and_adds_the_missing_ones() {
        let mut movies = vec![movie("tt1", true)];
        let mut held = movie("tt1", true);
        held.catalog = Some("movie:tmdb:top:Popular".into());
        let mut page = movie("tt2", false);
        page.catalog = Some("movie:tmdb:top:Popular".into());
        mark_collections(&mut movies, vec![("tmdb:top-rated:movie".into(), vec![held, page])]);

        let marks: Vec<(&str, Option<&str>)> = movies.iter().map(|movie| (movie.id.as_str(), movie.catalog.as_deref())).collect();
        assert_eq!(
            marks,
            [("tt1", Some("collection:tmdb:top-rated:movie")), ("tt2", Some("collection:tmdb:top-rated:movie"))],
            "the rail that was already there gains the mark; the one only the rule brought joins the library"
        );
        assert!(movies[0].top, "a marked title keeps the fields the rails gave it");
    }

    #[test]
    fn the_first_collection_of_the_settings_order_names_a_title() {
        let mut movies = Vec::new();
        let rail = |id: &str, movie: MovieItem| (id.to_owned(), vec![movie]);
        mark_collections(
            &mut movies,
            vec![
                rail("tmdb:popular:movie", movie("tt1", false)),
                rail("tmdb:top-rated:movie", MovieItem { kind: "series".into(), ..movie("tt1", false) }),
            ],
        );
        let marks: Vec<(&str, &str, Option<&str>)> =
            movies.iter().map(|movie| (movie.id.as_str(), movie.kind.as_str(), movie.catalog.as_deref())).collect();
        assert_eq!(
            marks,
            [("tt1", "movie", Some("collection:tmdb:popular:movie")), ("tt1", "series", Some("collection:tmdb:top-rated:movie")),],
            "a title in two rails keeps the first, and the same id in another kind is another title"
        );
    }

    #[test]
    fn a_collection_whose_source_cannot_answer_costs_that_rail_only() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        // Turned on, but the credential store holds no TMDB key: the rule cannot
        // be resolved, and that is a log line rather than a broken library.
        let settings =
            heronite_watch_state::settings::Settings { collections: vec!["tmdb:top-rated:movie".to_owned()], ..Default::default() };
        heronite_watch_state::settings::write(&app, &settings).unwrap();
        let (rails, answered) = collection_rails(&NoNet, &app, &crate::collections::enabled(&app));
        assert!(rails.is_empty() && answered.is_empty(), "a keyless source is skipped, not fatal");
    }

    #[test]
    fn the_first_copy_of_a_title_wins() {
        let movies =
            assemble(vec![rail("continue-watching", vec![movie("tt1", false)]), rail("top", vec![movie("tt1", true), movie("tt2", true)])]);
        let ids: Vec<(&str, bool)> = movies.iter().map(|movie| (movie.id.as_str(), movie.top)).collect();
        assert_eq!(ids, [("tt1", false), ("tt2", true)]);
    }
}
