//! The Watch space's Collection entry point: the providers' rails become the
//! synthetic `Library` the existing screens read.

use std::collections::HashSet;

use anyhow::Result;
use heronite_data::paths::AppPaths;
use heronite_data::{DataPaths, Library};
use heronite_watch_core::catalog::{CatalogCtx, CatalogRequest, WatchCatalogSource};
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::{Ctx, Rail, RailDescriptor};
use heronite_watch_core::types::MovieItem;
use heronite_watch_state::catalog_cache::CatalogIds;
use heronite_watch_state::collection;
use heronite_watch_state::store::StoreCredentials;

use crate::registry;

/// Where the proxy keeps its synthetic library.
pub use collection::ROOT;

/// The movies Collection, refreshed from the providers when possible. A
/// provider failure keeps the last good library instead of emptying the screen.
pub fn library(app: &AppPaths, include_hidden: bool) -> Result<Library> {
    let root = app.cache.join(ROOT);
    let http = UreqTransport::new();
    if let Err(error) = collection::refresh(&root, || items(&http, app)) {
        log::warn!("movies provider: {error:#}; keeping the last library");
    }
    Library::load_with(&DataPaths::at(root), include_hidden)
}

/// Every provider's rails, in registry order. A provider that fails aborts the
/// refresh, so the last good library stays on screen.
fn items(http: &dyn Transport, app: &AppPaths) -> Result<Vec<MovieItem>> {
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
    Ok(assemble(rails))
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
    fn the_first_copy_of_a_title_wins() {
        let movies =
            assemble(vec![rail("continue-watching", vec![movie("tt1", false)]), rail("top", vec![movie("tt1", true), movie("tt2", true)])]);
        let ids: Vec<(&str, bool)> = movies.iter().map(|movie| (movie.id.as_str(), movie.top)).collect();
        assert_eq!(ids, [("tt1", false), ("tt2", true)]);
    }
}
