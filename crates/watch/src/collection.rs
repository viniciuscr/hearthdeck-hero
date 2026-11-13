//! The Watch space's Collection entry point: the linked account's films become
//! the synthetic `Library` the existing screens read.

use anyhow::Result;
use heronite_data::paths::AppPaths;
use heronite_data::{DataPaths, Library};
use heronite_watch_core::http::UreqTransport;
use heronite_watch_core::types::MovieItem;
use heronite_watch_state::{collection, store};
use heronite_watch_stremio::{catalog, endpoints::Endpoints};

/// Where the proxy keeps its synthetic library.
pub use collection::ROOT;

/// The movies Collection, refreshed from Stremio when possible. A provider
/// failure keeps the last good library instead of emptying the screen.
pub fn library(app: &AppPaths, include_hidden: bool) -> Result<Library> {
    let root = app.cache.join(ROOT);
    if let Err(error) = collection::refresh(&root, || fetch(app)) {
        log::warn!("movies provider: {error:#}; keeping the last library");
    }
    Library::load_with(&DataPaths::at(root), include_hidden)
}

fn fetch(app: &AppPaths) -> Result<Vec<MovieItem>> {
    let http = UreqTransport::new();
    let key = store::read_auth(app)?;
    catalog::fetch(&http, &Endpoints::default(), key)
}
