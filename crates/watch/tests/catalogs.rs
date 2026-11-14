//! What a provider declares for the Movies and Series tabs. Cinemeta's manifest
//! is the fixture, so the test proves the descriptors come from the provider
//! and not from assumptions in the screens. No test reaches the network.

use std::sync::Mutex;

use anyhow::Result;
use serde_json::Value;
use url::Url;

use heronite_data::paths::AppPaths;
use heronite_watch_core::http::Transport;
use heronite_watch_core::provider::CatalogSort;

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

fn app(root: &std::path::Path) -> AppPaths {
    AppPaths { config: root.join("config"), cache: root.join("cache") }
}

#[test]
fn cinemeta_declares_three_catalogs_per_kind() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake { seen: Mutex::new(Vec::new()) };
    let catalogs = heronite_watch::catalog::catalogs_with(&fake, &app(dir.path()), "stremio").unwrap();
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
    let catalogs = heronite_watch::catalog::catalogs_with(&fake, &app(dir.path()), "stremio").unwrap();

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

/// A provider of the test's own: it declares a catalog Cinemeta has never heard
/// of, to prove the screens take what a provider says instead of assuming one.
struct Stub;

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

impl heronite_watch_core::provider::WatchProvider for Stub {
    fn id(&self) -> &'static str {
        "stub"
    }
    fn name(&self) -> &'static str {
        "Stub"
    }
    fn browse(&self, _: &heronite_watch_core::provider::Ctx<'_>) -> Result<Vec<heronite_watch_core::provider::Rail>> {
        Ok(Vec::new())
    }
    fn streams(
        &self,
        _: &heronite_watch_core::provider::Ctx<'_>,
        _: &str,
        _: &str,
    ) -> Result<Vec<heronite_watch_core::types::StreamChoice>> {
        Ok(Vec::new())
    }
    fn resolve(
        &self,
        _: &heronite_watch_core::provider::Ctx<'_>,
        _: &heronite_watch_core::types::StreamChoice,
    ) -> Result<heronite_watch_core::provider::ResolvedStream> {
        anyhow::bail!("a stub has nothing to play")
    }
    fn catalogs(&self, _: &heronite_watch_core::provider::Ctx<'_>) -> Result<Vec<heronite_watch_core::provider::CatalogDescriptor>> {
        Ok(vec![heronite_watch_core::provider::CatalogDescriptor {
            id: "shelf".into(),
            name: "My Shelf".into(),
            kind: "movie".into(),
            sorts: vec![CatalogSort::Newest, CatalogSort::Title],
            filters: vec![heronite_watch_core::provider::CatalogFilter {
                id: "lang".into(),
                label: "Language".into(),
                values: vec!["en".into(), "pt".into()],
                required: false,
            }],
            search: false,
            paging: true,
        }])
    }
    fn catalog(
        &self,
        _: &heronite_watch_core::provider::Ctx<'_>,
        _: &heronite_watch_core::provider::CatalogRequest,
    ) -> Result<heronite_watch_core::provider::CatalogPage> {
        Ok(heronite_watch_core::provider::CatalogPage { items: Vec::new(), more: false })
    }
}

#[test]
fn a_provider_of_its_own_is_all_the_screens_need() {
    use heronite_watch_core::provider::{CatalogRequest, Ctx, WatchProvider};

    let http = Fake { seen: Mutex::new(Vec::new()) };
    let credentials = NoCredentials;
    let ctx = Ctx { http: &http, credentials: &credentials };

    let catalogs = Stub.catalogs(&ctx).unwrap();
    assert_eq!(catalogs.len(), 1);
    let shelf = &catalogs[0];
    assert_eq!((shelf.kind.as_str(), shelf.id.as_str(), shelf.name.as_str()), ("movie", "shelf", "My Shelf"));
    assert_eq!(shelf.sorts, [CatalogSort::Newest, CatalogSort::Title], "the provider chooses from the closed set");
    assert_eq!(shelf.filters.len(), 1);
    assert_eq!((shelf.filters[0].id.as_str(), shelf.filters[0].values.len()), ("lang", 2), "its own filter, not Cinemeta's genre");
    assert!(!shelf.search && shelf.paging);

    let page = Stub.catalog(&ctx, &CatalogRequest::first("movie")).unwrap();
    assert!(page.items.is_empty() && !page.more, "a provider that browses nothing answers an empty last page");
}
