//! The Shelf tab's provider rails: what the sources declare, and one rail's page.
//!
//! A collection is a saved query a source answers with its own catalogue, so this
//! is the catalogue facade with a different question — no new network path, no new
//! cache. The person turns rails on in Settings; the tab draws the ones that are on.

use anyhow::Result;
use heronite_data::paths::AppPaths;
use heronite_watch_core::catalog::CatalogCtx;
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::types::MovieItem;
use heronite_watch_state::catalog_cache::CatalogIds;
use heronite_watch_state::store::StoreCredentials;

pub use heronite_watch_core::collections::{CollectionDescriptor, CollectionSourceDescriptor, WatchCollectionSource};

/// Every rail the registered sources declare, with the source that answers it.
pub fn descriptors(app: &AppPaths) -> Vec<(String, CollectionDescriptor)> {
    let http = UreqTransport::new();
    descriptors_with(&http, app)
}

/// [`descriptors`] with an injected transport, for tests.
pub fn descriptors_with(http: &dyn Transport, app: &AppPaths) -> Vec<(String, CollectionDescriptor)> {
    let credentials = StoreCredentials::new(app);
    let ids = CatalogIds::at(app);
    let ctx = CatalogCtx { http, credentials: &credentials, cache: Some(&ids) };
    let mut rails = Vec::new();
    for source in heronite_watch_catalogs::collection_sources() {
        match source.collections(&ctx) {
            Ok(declared) => rails.extend(declared.into_iter().map(|rail| (source.descriptor().id.to_owned(), rail))),
            // A source that cannot list its rails (a missing key) is said in the
            // log; the others still declare.
            Err(error) => log::warn!("watch collections {}: {error:#}", source.descriptor().id),
        }
    }
    rails
}

/// The rails the settings turned on, in the person's order. An id this build does
/// not have is skipped, the way every other settings list treats one.
pub fn enabled(app: &AppPaths) -> Vec<(String, CollectionDescriptor)> {
    let wanted = heronite_watch_state::settings::read(app).collections;
    let declared = descriptors(app);
    wanted.iter().filter_map(|id| declared.iter().find(|(_, rail)| rail.id == *id).cloned()).collect()
}

/// One rail's titles: its rule, asked of the source that declared it.
pub fn rail(app: &AppPaths, source: &str, descriptor: &CollectionDescriptor) -> Result<Vec<MovieItem>> {
    let http = UreqTransport::new();
    rail_with(&http, app, source, descriptor)
}

/// [`rail`] with an injected transport, for tests.
pub fn rail_with(http: &dyn Transport, app: &AppPaths, source: &str, descriptor: &CollectionDescriptor) -> Result<Vec<MovieItem>> {
    Ok(crate::catalog::page_with(http, app, source, &descriptor.rule)?.items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;
    use url::Url;

    struct Fake {
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Self { seen: Mutex::new(Vec::new()) }
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            // A catalogue page, and the id TMDB holds for the title on it: without
            // the second, the source drops the result (it could not be played).
            if url.path().contains("external_ids") {
                return Ok(br#"{"id":603,"imdb_id":"tt0133093"}"#.to_vec());
            }
            Ok(br#"{"metas":[{"id":"603","type":"movie","name":"The Matrix"}]}"#.to_vec())
        }
        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            Ok(b"{}".to_vec())
        }
    }

    fn app(root: &std::path::Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    #[test]
    fn tmdb_declares_its_rails_without_a_key_and_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let rails = descriptors_with(&fake, &app(dir.path()));
        let ids: Vec<&str> = rails.iter().map(|(_, rail)| rail.id.as_str()).collect();
        assert_eq!(ids, ["tmdb:top-rated:movie", "tmdb:popular:movie", "tmdb:top-rated:series", "tmdb:popular:series"]);
        assert_eq!(rails[0].0, "tmdb", "the source that answers it");
        assert!(fake.seen.lock().unwrap().is_empty(), "declaring costs nothing");
    }

    #[test]
    fn only_the_enabled_rails_are_listed_in_the_persons_order() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let settings = crate::settings::Settings {
            collections: vec!["tmdb:popular:series".to_owned(), "gone".to_owned(), "tmdb:top-rated:movie".to_owned()],
            ..Default::default()
        };
        crate::settings::write(&app, &settings).unwrap();
        let rails = enabled(&app);
        let ids: Vec<&str> = rails.iter().map(|(_, rail)| rail.id.as_str()).collect();
        assert_eq!(ids, ["tmdb:popular:series", "tmdb:top-rated:movie"], "the order is the person's, unknown ids skipped");
    }
}
