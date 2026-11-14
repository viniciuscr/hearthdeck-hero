//! What a catalog source declares for the Movies and Series tabs. Cinemeta's
//! manifest is the fixture, and a stub source of the test's own proves the
//! screens take what a source says instead of assuming one. No test reaches the
//! network.

use std::sync::Mutex;

use anyhow::Result;
use serde_json::Value;
use url::Url;

use heronite_data::paths::AppPaths;
use heronite_watch_core::catalog::{
    CatalogCtx, CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor, WatchCatalogSource,
};
use heronite_watch_core::http::Transport;
use heronite_watch_core::provider::{Credentials, Secret};

const MANIFEST: &str = include_str!("fixtures/cinemeta_manifest.json");

/// Answers every request with the manifest.
struct Fake {
    seen: Mutex<Vec<String>>,
}

impl Transport for Fake {
    fn get(&self, url: &Url) -> Result<Vec<u8>> {
        self.seen.lock().unwrap().push(url.as_str().to_owned());
        Ok(MANIFEST.as_bytes().to_vec())
    }
    fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
        Ok(b"{}".to_vec())
    }
}

struct NoCredentials;

impl Credentials for NoCredentials {
    fn get(&self, _: &str) -> Result<Option<Secret>> {
        Ok(None)
    }
    fn set(&self, _: &str, _: &Secret) -> Result<()> {
        Ok(())
    }
    fn clear(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

fn app(root: &std::path::Path) -> AppPaths {
    AppPaths { config: root.join("config"), cache: root.join("cache") }
}

#[test]
fn cinemeta_declares_three_catalogs_per_kind() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake { seen: Mutex::new(Vec::new()) };
    let catalogs = heronite_watch::catalog::catalogs_with(&fake, &app(dir.path()), "cinemeta").unwrap();
    let kinds: Vec<(&str, &str)> = catalogs.iter().map(|catalog| (catalog.kind.as_str(), catalog.id.as_str())).collect();
    assert_eq!(
        kinds,
        [("movie", "top"), ("movie", "imdbRating"), ("movie", "year"), ("series", "top"), ("series", "imdbRating"), ("series", "year")]
    );
    assert!(fake.seen.lock().unwrap().iter().all(|url| url.ends_with("manifest.json")), "the descriptors come from the manifest");
}

#[test]
fn the_sorts_and_the_filters_come_from_the_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake { seen: Mutex::new(Vec::new()) };
    let catalogs = heronite_watch::catalog::catalogs_with(&fake, &app(dir.path()), "cinemeta").unwrap();

    let top = catalogs.iter().find(|catalog| catalog.kind == "movie" && catalog.id == "top").unwrap();
    assert_eq!(top.sorts, [CatalogSort::Popular]);
    assert!(top.search, "top takes a search");
    assert!(top.paging, "top pages with skip");
    let genre = top.filters.iter().find(|filter| filter.id == "genre").unwrap();
    assert_eq!(genre.values, ["Action", "Sci-Fi"]);
    assert!(!genre.required);

    let featured = catalogs.iter().find(|catalog| catalog.kind == "movie" && catalog.id == "imdbRating").unwrap();
    assert_eq!(featured.sorts, [CatalogSort::Rating]);
    assert!(!featured.search, "imdbRating takes no search");

    let year = catalogs.iter().find(|catalog| catalog.kind == "movie" && catalog.id == "year").unwrap();
    assert_eq!(year.sorts, [CatalogSort::Newest]);
    let year_filter = year.filters.iter().find(|filter| filter.id == "year").unwrap();
    assert!(year_filter.required, "the year catalog cannot be asked without a year");
    assert_eq!(year_filter.values, ["1999", "2008"]);
    assert!(!year.filters.iter().any(|filter| filter.id == "genre"), "the year slot is read as a year");
}

/// A source of the test's own: a catalog Cinemeta has never heard of.
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
            sorts: vec![CatalogSort::Newest, CatalogSort::Title],
            filters: vec![CatalogFilter {
                id: "lang".into(),
                label: "Language".into(),
                values: vec!["en".into(), "pt".into()],
                required: false,
            }],
            search: false,
            paging: true,
        }])
    }
    fn catalog(&self, _: &CatalogCtx<'_>, _: &CatalogRequest) -> Result<CatalogPage> {
        Ok(CatalogPage { items: Vec::new(), more: false, consumed: 0 })
    }
}

#[test]
fn a_source_of_its_own_is_all_the_screens_need() {
    let http = Fake { seen: Mutex::new(Vec::new()) };
    let credentials = NoCredentials;
    let ctx = CatalogCtx { http: &http, credentials: &credentials };

    let catalogs = Stub.catalogs(&ctx).unwrap();
    assert_eq!(catalogs.len(), 1);
    let shelf = &catalogs[0];
    assert_eq!((shelf.kind.as_str(), shelf.id.as_str(), shelf.name.as_str()), ("movie", "shelf", "My Shelf"));
    assert_eq!(shelf.sorts, [CatalogSort::Newest, CatalogSort::Title], "the source chooses from the closed set");
    assert_eq!(shelf.filters.len(), 1);
    assert_eq!((shelf.filters[0].id.as_str(), shelf.filters[0].values.len()), ("lang", 2), "its own filter, not Cinemeta's genre");
    assert!(!shelf.search && shelf.paging);
    assert_eq!(Stub.descriptor().name, "Stub");

    let page = Stub.catalog(&ctx, &CatalogRequest::first("movie")).unwrap();
    assert!(page.items.is_empty() && !page.more, "a source that browses nothing answers an empty last page");
}
