//! The Watch space's Collection entry point: the providers' rails become the
//! synthetic `Library` the existing screens read.

use std::collections::HashSet;

use anyhow::Result;
use heronite_data::paths::AppPaths;
use heronite_data::{DataPaths, Library};
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::{Ctx, Rail};
use heronite_watch_core::types::MovieItem;
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
    Ok(assemble(rails))
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
    use heronite_watch_core::provider::CatalogDescriptor;

    fn movie(id: &str, top: bool) -> MovieItem {
        MovieItem { id: id.into(), kind: "movie".into(), name: id.into(), top, ..MovieItem::default() }
    }

    fn rail(id: &'static str, items: Vec<MovieItem>) -> Rail {
        Rail { catalog: CatalogDescriptor { id, kind: "movie" }, items }
    }

    #[test]
    fn the_first_copy_of_a_title_wins() {
        let movies =
            assemble(vec![rail("continue-watching", vec![movie("tt1", false)]), rail("top", vec![movie("tt1", true), movie("tt2", true)])]);
        let ids: Vec<(&str, bool)> = movies.iter().map(|movie| (movie.id.as_str(), movie.top)).collect();
        assert_eq!(ids, [("tt1", false), ("tt2", true)]);
    }
}
