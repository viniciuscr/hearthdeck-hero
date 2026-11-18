//! The catalog sources Watch lets you browse, and the registry that lists them.
//!
//! A catalog source says what there is to browse (see
//! [`WatchCatalogSource`]): it plays nothing, has no account and is never asked
//! for a stream. Settings → Watch → Sources (Catalogs) turns them on and off by id, which
//! is why a source's id is all the settings file stores.
//!
//! [`WatchCatalogSource`]: heronite_watch_core::catalog::WatchCatalogSource

use heronite_watch_core::catalog::WatchCatalogSource;

pub mod cinemeta;

use heronite_watch_core::collections::WatchCollectionSource;

static TMDB_COLLECTIONS: heronite_watch_tmdb::TmdbCollections = heronite_watch_tmdb::TmdbCollections;
static COLLECTION_SOURCES: [&dyn WatchCollectionSource; 1] = [&TMDB_COLLECTIONS];

/// Every collection source this build has, in the order Settings lists them.
pub fn collection_sources() -> &'static [&'static dyn WatchCollectionSource] {
    &COLLECTION_SOURCES
}

static CINEMETA: cinemeta::Cinemeta = cinemeta::Cinemeta::builtin();
static TMDB: heronite_watch_tmdb::Tmdb = heronite_watch_tmdb::Tmdb::new();
static SOURCES: [&dyn WatchCatalogSource; 2] = [&CINEMETA, &TMDB];

/// Every catalog source this build has, in the order they are listed.
pub fn sources() -> &'static [&'static dyn WatchCatalogSource] {
    &SOURCES
}

/// The sources named by `enabled`, in the order given.
///
/// An id this build no longer has is skipped: a settings file from another
/// version is not a fault.
pub fn enabled(enabled: &[String]) -> Vec<&'static dyn WatchCatalogSource> {
    enabled.iter().filter_map(|id| sources().iter().copied().find(|source| source.descriptor().id == id.as_str())).collect()
}

/// The source with `id`, for one page request.
pub fn find(id: &str) -> Option<&'static dyn WatchCatalogSource> {
    sources().iter().copied().find(|source| source.descriptor().id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sources_are_cinemeta_then_tmdb_and_are_found_by_id() {
        let ids: Vec<&str> = sources().iter().map(|source| source.descriptor().id).collect();
        assert_eq!(ids, ["cinemeta", "tmdb"]);
        assert_eq!(find("cinemeta").map(|source| source.descriptor().name), Some("Cinemeta"));
        assert_eq!(find("tmdb").map(|source| source.descriptor().name), Some("TMDB"));
        assert!(find("nope").is_none());
    }

    #[test]
    fn only_the_enabled_ids_are_listed_and_unknown_ones_are_skipped() {
        let enabled = crate::enabled(&["tmdb".to_owned(), "gone".to_owned(), "cinemeta".to_owned()]);
        assert_eq!(
            enabled.iter().map(|source| source.descriptor().id).collect::<Vec<_>>(),
            ["tmdb", "cinemeta"],
            "the order given, unknown ids skipped"
        );
        assert!(crate::enabled(&[]).is_empty(), "nothing on means nothing to browse by");
    }
}
