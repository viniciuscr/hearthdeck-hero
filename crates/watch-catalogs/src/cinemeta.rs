//! Cinemeta as a catalog source: Stremio's own catalogue, browsed on its own.
//!
//! It declares the catalogs its manifest carries — `top` (its order),
//! `imdbRating` (best rated) and `year` (newest, with a required year) — and
//! answers a page by asking the addon. It plays nothing; the items name the
//! media provider that can play them (`stremio`).

use anyhow::Result;
use heronite_watch_addon::addon::{AddonClient, Catalog};
use heronite_watch_core::catalog::{
    CatalogCtx, CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor, WatchCatalogSource,
};
use heronite_watch_core::types::MovieItem;
use serde_json::Value;

/// Where Cinemeta lives: its manifest. The source keeps its own copy, so it
/// depends on no media provider crate.
pub const MANIFEST: &str = "https://v3-cinemeta.strem.io/manifest.json";
/// The media provider that can play what Cinemeta can describe.
const PLAYS_WITH: &str = "stremio";

const DESCRIPTOR: CatalogSourceDescriptor = CatalogSourceDescriptor { id: "cinemeta", name: "Cinemeta" };

/// The catalogue of one Cinemeta deployment.
pub struct Cinemeta {
    manifest: &'static str,
}

impl Cinemeta {
    /// A deployment of it, for a test or a mirror.
    pub const fn at(manifest: &'static str) -> Self {
        Self { manifest }
    }

    /// The public Cinemeta.
    pub const fn builtin() -> Self {
        Self::at(MANIFEST)
    }
}

impl WatchCatalogSource for Cinemeta {
    fn descriptor(&self) -> &CatalogSourceDescriptor {
        &DESCRIPTOR
    }

    fn catalogs(&self, ctx: &CatalogCtx<'_>) -> Result<Vec<CatalogDescriptor>> {
        let mut addon = AddonClient::new(ctx.http, self.manifest)?;
        let manifest = addon.manifest()?;
        Ok(manifest.catalogs.iter().filter_map(descriptor_of).collect())
    }

    /// A page of Cinemeta's catalog. The sort picks the catalog (`top` for its
    /// own order, `imdbRating` best rated, `year` newest); `skip`, `search` and
    /// the `genre`/year filter are extras.
    fn catalog(&self, ctx: &CatalogCtx<'_>, request: &CatalogRequest) -> Result<CatalogPage> {
        if !matches!(request.kind.as_str(), "movie" | "series") {
            return Ok(CatalogPage::default());
        }
        let addon = AddonClient::new(ctx.http, self.manifest)?;
        let skip = request.skip.to_string();
        let mut extra: Vec<(&str, &str)> = Vec::new();
        if request.skip > 0 {
            extra.push(("skip", skip.as_str()));
        }
        if let Some(search) = request.search.as_deref().map(str::trim).filter(|search| !search.is_empty()) {
            extra.push(("search", search));
        }
        // The screen's filters are the source's: `genre`, or the year the `year`
        // catalog carries in the same slot.
        for (id, value) in &request.filters {
            let value = value.trim();
            if !value.is_empty() && (id == "genre" || id == "year") {
                extra.push(("genre", value));
            }
        }
        let catalog = match request.catalog.trim() {
            "" => match request.sort {
                CatalogSort::Rating => "imdbRating",
                CatalogSort::Newest => "year",
                CatalogSort::Popular | CatalogSort::Title => "top",
            },
            named => named,
        };
        let items: Vec<MovieItem> = addon
            .catalog(&request.kind, catalog, &extra)?
            .into_iter()
            .map(|mut movie| {
                movie.provider = PLAYS_WITH.into();
                movie.addon = DESCRIPTOR.id.into();
                movie
            })
            .collect();
        // Cinemeta does not say how many a catalog holds: an empty page is the
        // end, and one request more finds it.
        let more = !items.is_empty();
        let consumed = items.len() as u32;
        Ok(CatalogPage { items, more, consumed })
    }
}

/// One manifest catalog as a descriptor, for the catalogs the tabs can page:
/// `top`, `imdbRating` and `year`. Anything else is left out (a catalog without
/// an order we can label has no chip to render).
fn descriptor_of(catalog: &Catalog) -> Option<CatalogDescriptor> {
    if !matches!(catalog.kind.as_str(), "movie" | "series") {
        return None;
    }
    let sorts = match catalog.id.as_str() {
        "top" => vec![CatalogSort::Popular],
        "imdbRating" => vec![CatalogSort::Rating],
        "year" => vec![CatalogSort::Newest],
        _ => return None,
    };
    let extras: &[Value] = catalog.extra.as_array().map_or(&[], Vec::as_slice);
    let named = |name: &str| extras.iter().any(|extra| extra["name"].as_str() == Some(name));
    let filters = extras
        .iter()
        .filter_map(|extra| {
            let id = extra["name"].as_str()?;
            // The `year` catalog carries its year in the protocol's `genre`
            // slot; the screens call that filter a year.
            let (id, label) = match (catalog.id.as_str(), id) {
                ("year", "genre") => ("year", "year"),
                (_, "genre") => ("genre", "genre"),
                _ => return None,
            };
            let values: Vec<String> = extra["options"]
                .as_array()
                .map(|options| options.iter().filter_map(|value| value.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            if values.is_empty() {
                return None;
            }
            Some(CatalogFilter {
                id: id.to_owned(),
                label: label.to_owned(),
                values,
                required: extra["isRequired"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    Some(CatalogDescriptor {
        id: catalog.id.clone(),
        name: catalog.name.clone().unwrap_or_else(|| catalog.id.clone()),
        kind: catalog.kind.clone(),
        sorts,
        filters,
        search: named("search"),
        paging: named("skip"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::{Credentials, Secret};
    use std::sync::Mutex;
    use url::Url;

    const MANIFEST: &str = include_str!("../tests/fixtures/cinemeta_manifest.json");

    struct Fake {
        seen: Mutex<Vec<String>>,
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            if url.path().ends_with("manifest.json") {
                return Ok(MANIFEST.as_bytes().to_vec());
            }
            Ok(br#"{"metas":[{"id":"tt0133093","type":"movie","name":"The Matrix"}]}"#.to_vec())
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

    #[test]
    fn the_manifest_decides_the_catalogs_and_their_controls() {
        let http = Fake { seen: Mutex::new(Vec::new()) };
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };
        let catalogs = Cinemeta::builtin().catalogs(&ctx).unwrap();
        let kinds: Vec<(&str, &str)> = catalogs.iter().map(|catalog| (catalog.kind.as_str(), catalog.id.as_str())).collect();
        assert_eq!(
            kinds,
            [("movie", "top"), ("movie", "imdbRating"), ("movie", "year"), ("series", "top"), ("series", "imdbRating"), ("series", "year")]
        );
        let top = &catalogs[0];
        assert_eq!(top.sorts, [CatalogSort::Popular]);
        assert!(top.search && top.paging);
        let year = catalogs.iter().find(|catalog| catalog.id == "year").unwrap();
        let filter = year.filters.first().unwrap();
        assert!(filter.required && filter.id == "year", "the year catalog needs a year, read from the genre slot");
    }

    #[test]
    fn a_page_marks_the_provider_that_plays_it_and_asks_for_the_extras() {
        let http = Fake { seen: Mutex::new(Vec::new()) };
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };
        let mut request = CatalogRequest::first("movie");
        request.sort = CatalogSort::Rating;
        request.filters = vec![("genre".into(), "Sci-Fi".into())];
        request.skip = 50;
        let page = Cinemeta::builtin().catalog(&ctx, &request).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].provider, "stremio", "Cinemeta describes; Stremio plays");
        assert_eq!(page.items[0].addon, "cinemeta", "and says which catalogue it came from");
        assert_eq!(page.consumed, 1);
        assert!(page.more, "a page with items says another may follow");
        let asked = http.seen.lock().unwrap().clone();
        assert!(asked.iter().any(|url| url.ends_with("catalog/movie/imdbRating/skip=50&genre=Sci-Fi.json")), "{asked:?}");
    }
}
