//! Browsing: the catalogs the Movies and Series tabs page through.
//!
//! A **catalog source** says what there is to browse. It declares the catalogs
//! it offers — each with the orders and filters it can honour, whether it takes
//! a search and whether it pages — and answers one page at a time. It is not a
//! media provider: it plays nothing, owns no library and is never asked for a
//! stream. Stremio's Cinemeta, TMDB's discover and any other browsable catalogue
//! live here, each one switchable in Settings → Watch → Sources (Catalogs).
//!
//! [`WatchProvider`]: crate::provider::WatchProvider

use crate::http::Transport;
use crate::provider::Credentials;
use crate::types::MovieItem;
use crate::Result;

/// Which page of which catalog: what the Movies and Series tabs ask for.
///
/// The source decides the page size (Cinemeta answers 50, TMDB 20); `skip` is
/// how many items of this catalog the caller already has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogRequest {
    /// `movie` or `series`.
    pub kind: String,
    /// The source's catalog id (`top`), empty for its default.
    pub catalog: String,
    /// Items to skip.
    pub skip: u32,
    /// A search of the source's catalog; `None` for the plain catalog.
    pub search: Option<String>,
    /// The order to ask for; a sort the descriptor does not declare is not sent.
    pub sort: CatalogSort,
    /// The chosen filters as `filter id → value` (`genre` → `Action`).
    pub filters: Vec<(String, String)>,
}

impl CatalogRequest {
    /// The first page of `kind`'s default catalog.
    pub fn first(kind: &str) -> Self {
        Self { kind: kind.to_owned(), ..Self::default() }
    }
}

/// How a catalog can be ordered by the source.
///
/// A closed set on purpose: the screens label and order these, and the source
/// picks the ones it can page by. A source that can only send its own order
/// declares none (or just [`Popular`](Self::Popular)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogSort {
    /// The source's own order (Cinemeta's "Popular", TMDB's `popularity.desc`).
    #[default]
    Popular,
    /// Best rated first.
    Rating,
    /// Newest first.
    Newest,
    /// Alphabetically by title.
    Title,
}

/// A filter a catalog takes, with the values it offers. The ids are the
/// source's (`genre`, `year`, …); the screens draw one row per filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogFilter {
    pub id: String,
    /// How the row is named; the screens localize the ids they know.
    pub label: String,
    pub values: Vec<String>,
    /// A value has to be chosen before the catalog can be asked.
    pub required: bool,
}

/// One catalog a source offers the Movies and Series tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogDescriptor {
    pub id: String,
    /// How the chip is named (localized by the screens for known ids).
    pub name: String,
    /// `movie` or `series`.
    pub kind: String,
    /// The orders it can page by; empty means only the order it sends.
    pub sorts: Vec<CatalogSort>,
    pub filters: Vec<CatalogFilter>,
    /// Whether it takes a search.
    pub search: bool,
    /// Whether it honours `skip`.
    pub paging: bool,
}

/// One page of a catalog, and whether more follows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogPage {
    pub items: Vec<MovieItem>,
    /// The source's own answer to "is there another page"; an empty page is the
    /// end even when it says otherwise.
    pub more: bool,
    /// How many of the source's own results this page paged through. It is what
    /// the caller adds to `skip`: a source that drops rows it cannot use
    /// (TMDB, without an IMDb id) has still moved on by them, and one whose
    /// pages have a fixed size must be asked for the next page, not asked again.
    pub consumed: u32,
}

/// What Settings lists a catalog source by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogSourceDescriptor {
    /// Stable id (`cinemeta`, `tmdb`): the settings file stores these.
    pub id: &'static str,
    /// How Settings and a chip name it when more than one source is on.
    pub name: &'static str,
}

/// A small cache a catalogue may keep between runs. TMDB's own ids are stable,
/// and resolving one to an IMDb id costs a request: a page of twenty must not
/// cost twenty requests on every visit.
pub trait CatalogCache: Send + Sync {
    /// The IMDb id kept for one of the source's titles: `Some(None)` means it
    /// was asked before and has none, `None` that it was never asked.
    fn imdb_id(&self, source: &str, kind: &str, id: u64) -> Option<Option<String>>;

    /// Keeps (or remembers the absence of) one title's IMDb id.
    fn keep_imdb_id(&self, source: &str, kind: &str, id: u64, imdb: Option<&str>);
}

/// What a catalog source is given: the transport, its own credentials, and the
/// cache it may keep.
///
/// TMDB needs an API key; Cinemeta needs none. A source reads only its own
/// entry, and never sees a media provider's account.
pub struct CatalogCtx<'a> {
    pub http: &'a dyn Transport,
    pub credentials: &'a dyn Credentials,
    /// `None` in a test that does not care, or where the caller has no cache.
    pub cache: Option<&'a dyn CatalogCache>,
}

/// A source of browsable catalogs.
pub trait WatchCatalogSource: Send + Sync {
    fn descriptor(&self) -> &CatalogSourceDescriptor;

    /// The catalogs it offers, in the order their chips appear.
    fn catalogs(&self, ctx: &CatalogCtx<'_>) -> Result<Vec<CatalogDescriptor>>;

    /// One page, with the source's own answer to "is there more".
    fn catalog(&self, ctx: &CatalogCtx<'_>, request: &CatalogRequest) -> Result<CatalogPage>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source of the test's own, so the contract is shown to need nothing from
    /// any particular provider.
    struct Stub;

    const STUB: CatalogSourceDescriptor = CatalogSourceDescriptor { id: "stub", name: "Stub" };

    impl WatchCatalogSource for Stub {
        fn descriptor(&self) -> &CatalogSourceDescriptor {
            &STUB
        }
        fn catalogs(&self, _: &CatalogCtx<'_>) -> Result<Vec<CatalogDescriptor>> {
            Ok(vec![CatalogDescriptor {
                id: "shelf".into(),
                name: "My Shelf".into(),
                kind: "movie".into(),
                sorts: vec![CatalogSort::Newest],
                filters: vec![CatalogFilter { id: "lang".into(), label: "lang".into(), values: vec!["en".into()], required: false }],
                search: false,
                paging: true,
            }])
        }
        fn catalog(&self, _: &CatalogCtx<'_>, request: &CatalogRequest) -> Result<CatalogPage> {
            let more = request.skip == 0;
            Ok(CatalogPage { items: Vec::new(), more, consumed: 20 })
        }
    }

    struct NoCredentials;

    impl Credentials for NoCredentials {
        fn get(&self, _: &str) -> Result<Option<crate::provider::Secret>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &crate::provider::Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    struct Nothing;

    impl Transport for Nothing {
        fn get(&self, _: &url::Url) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn post_json(&self, _: &url::Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_source_declares_what_it_browses_and_answers_a_page() {
        let http = Nothing;
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };

        let catalogs = Stub.catalogs(&ctx).unwrap();
        assert_eq!(catalogs.len(), 1);
        assert_eq!((catalogs[0].kind.as_str(), catalogs[0].id.as_str()), ("movie", "shelf"));
        assert!(Stub.catalog(&ctx, &CatalogRequest::first("movie")).unwrap().more);
        assert!(!Stub.catalog(&ctx, &CatalogRequest { skip: 20, ..CatalogRequest::first("movie") }).unwrap().more);
        assert_eq!(Stub.descriptor().id, "stub");
    }
}
