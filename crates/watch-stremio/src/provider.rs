//! The Stremio provider: the linked account's library and catalogs, its stream
//! addons, the local service, and its datastore.
//!
//! Everything arrives through `Ctx`; the provider reads no file.

use anyhow::{anyhow, bail, Context, Result};
use url::Url;

use heronite_watch_addon::addon::{AddonClient, Catalog, StreamAnswer};
use heronite_watch_core::auth::AuthKey;
use heronite_watch_core::provider::{
    CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, Ctx, Rail, RailDescriptor, ResolvedStream, StreamOrigin,
    WatchProvider,
};
use heronite_watch_core::types::{MovieItem, MovieProgress, StreamChoice};
use serde_json::Value;

use crate::account::addon_collection;
use crate::catalog::{self, CINEMETA};
use crate::datastore::{self, DatastoreClient};
use crate::endpoints::Endpoints;
use crate::streaming::StreamingServer;
use crate::ID;

const CONTINUE_WATCHING: RailDescriptor = RailDescriptor { id: "continue-watching", kind: "" };

pub struct Stremio;

/// The session key, as the link flow or the credential file stores it.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredAuth {
    Object {
        #[serde(rename = "authKey")]
        key: String,
    },
    Bare(String),
}

fn key(ctx: &Ctx<'_>) -> Result<Option<AuthKey>> {
    let Some(secret) = ctx.credentials.get(ID)? else {
        return Ok(None);
    };
    let stored: StoredAuth = secret.decode()?;
    let key = match stored {
        StoredAuth::Object { key } | StoredAuth::Bare(key) => key,
    };
    Ok((!key.is_empty()).then(|| AuthKey::new(key)))
}

/// The linked account's films and series, in Continue Watching order.
fn continue_watching(ctx: &Ctx<'_>, endpoints: &Endpoints) -> Result<Vec<MovieItem>> {
    let Some(key) = key(ctx)? else {
        return Ok(Vec::new());
    };
    let client = DatastoreClient::new(ctx.http, endpoints, key);
    Ok(datastore::continue_watching(&client.get_all()?)
        .into_iter()
        .filter(|item| item.kind == "movie" || item.kind == "series")
        .map(catalog::from_library)
        .collect())
}

impl WatchProvider for Stremio {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Stremio"
    }

    fn browse(&self, ctx: &Ctx<'_>) -> Result<Vec<Rail>> {
        let endpoints = Endpoints::default();
        endpoints.validate()?;
        let mut rails = vec![Rail { catalog: CONTINUE_WATCHING, items: continue_watching(ctx, &endpoints)? }];

        let mut addon = AddonClient::new(ctx.http, CINEMETA)?;
        addon.manifest()?;
        for kind in ["movie", "series"] {
            let items = catalog::top(&addon, kind)?;
            rails.push(Rail { catalog: RailDescriptor { id: "top", kind }, items });
        }
        Ok(rails)
    }

    fn streams(&self, ctx: &Ctx<'_>, kind: &str, id: &str) -> Result<Vec<StreamChoice>> {
        let endpoints = Endpoints::default();
        endpoints.validate()?;
        let key = key(ctx)?.ok_or_else(|| anyhow!("Link your Stremio account in Settings → Watch to play."))?;
        let addons = addon_collection(ctx.http, &endpoints.api, &key).context("reading the addons of the linked account")?;

        let mut choices = Vec::new();
        let mut sources = 0;
        for addon in addons {
            let Some(manifest) = &addon.manifest else {
                log::warn!("watch addon: no manifest, skipped");
                continue;
            };
            if !addon.serves("stream") || !manifest.types.iter().any(|t| t == kind) {
                continue;
            }
            // A protected or half-configured addon has no transport to ask.
            if addon.transport_url.is_empty() {
                continue;
            }
            sources += 1;
            // The addon's transport URL may carry the person's configuration;
            // the error names the addon, never the URL.
            let client = match AddonClient::new(ctx.http, &addon.transport_url) {
                Ok(client) => client,
                Err(error) => {
                    log::warn!("watch addon {}: {error:#}", manifest.name);
                    continue;
                }
            };
            match client.stream(kind, id) {
                Ok(StreamAnswer::Streams(mut streams)) => {
                    for stream in &mut streams {
                        stream.addon.clone_from(&manifest.name);
                    }
                    choices.append(&mut streams);
                }
                Ok(StreamAnswer::NotServed) => {
                    log::debug!("watch addon {} ({}): does not serve {kind} {id}", manifest.name, client.host());
                }
                Err(error) => log::warn!("watch addon {}: {error:#}", manifest.name),
            }
        }

        if sources == 0 {
            bail!("The linked Stremio account has no addon that provides streams. Install Torrentio in Stremio.");
        }
        choices.retain(StreamChoice::is_playable);
        choices.sort_by(|a, b| {
            b.seeders
                .unwrap_or(0)
                .cmp(&a.seeders.unwrap_or(0))
                .then_with(|| b.size_bytes.unwrap_or(0).cmp(&a.size_bytes.unwrap_or(0)))
                .then_with(|| a.addon.cmp(&b.addon))
        });
        Ok(choices)
    }

    fn resolve(&self, ctx: &Ctx<'_>, stream: &StreamChoice) -> Result<ResolvedStream> {
        let endpoints = Endpoints::default();
        endpoints.validate()?;
        if let Some(url) = &stream.url {
            let url = Url::parse(url).context("stream URL")?;
            // The picker only offers what the player would accept.
            if !StreamOrigin::Remote.allows(&url) {
                bail!("refusing a stream that is not https: {}://{}", url.scheme(), url.host_str().unwrap_or("?"));
            }
            return Ok(ResolvedStream { url, origin: StreamOrigin::Remote });
        }
        let hash = stream.info_hash.as_deref().ok_or_else(|| anyhow!("This stream has nothing to play."))?;
        let server = StreamingServer::new(ctx.http, endpoints.streaming_server.clone())?;
        let base = server.base_url().context("The Stremio service is not running")?;
        server.create(hash, &[]).context("The Stremio service refused the torrent")?;
        let url = StreamingServer::stream_url(&base, hash, stream.file_idx)?;
        Ok(ResolvedStream { url, origin: StreamOrigin::LocalService })
    }

    /// Cinemeta's catalogs for the Movies and Series tabs, straight from its
    /// manifest: `top` (its own order), `imdbRating` (best rated) and `year`
    /// (newest, which needs a year). The extras the manifest declares become the
    /// filters, and decide the search and paging flags.
    fn catalogs(&self, ctx: &Ctx<'_>) -> Result<Vec<CatalogDescriptor>> {
        let endpoints = Endpoints::default();
        endpoints.validate()?;
        let mut addon = AddonClient::new(ctx.http, CINEMETA)?;
        let manifest = addon.manifest()?;
        Ok(manifest.catalogs.iter().filter_map(descriptor_of).collect())
    }

    /// A page of Cinemeta's catalog. The sort picks the catalog (`top` for the
    /// provider's own order, `imdbRating` best rated, `year` newest); `skip`,
    /// `search` and the provider's `genre`/year filter are extras.
    fn catalog(&self, ctx: &Ctx<'_>, request: &CatalogRequest) -> Result<CatalogPage> {
        let endpoints = Endpoints::default();
        endpoints.validate()?;
        if !matches!(request.kind.as_str(), "movie" | "series") {
            return Ok(CatalogPage::default());
        }
        let addon = AddonClient::new(ctx.http, CINEMETA)?;
        let skip = request.skip.to_string();
        let mut extra: Vec<(&str, &str)> = Vec::new();
        if request.skip > 0 {
            extra.push(("skip", skip.as_str()));
        }
        if let Some(search) = request.search.as_deref().map(str::trim).filter(|search| !search.is_empty()) {
            extra.push(("search", search));
        }
        // The screen's filters are the provider's: `genre`, or the year the
        // `year` catalog carries in the same slot.
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
                movie.provider = ID.into();
                movie
            })
            .collect();
        // Cinemeta does not say how many a catalog holds: an empty page is the
        // end, and one request more finds it.
        let more = !items.is_empty();
        Ok(CatalogPage { items, more })
    }

    /// Read-merge-write of the full record; a failed write is the caller's to
    /// keep locally and retry. Without a linked account there is nothing to do.
    fn read_progress(&self, ctx: &Ctx<'_>, id: &str) -> Result<Option<MovieProgress>> {
        let Some(key) = key(ctx)? else {
            return Ok(None);
        };
        let endpoints = Endpoints::default();
        let client = DatastoreClient::new(ctx.http, &endpoints, key);
        let Some(item) = client.get_all()?.into_iter().find(|item| item.id == id) else {
            return Ok(None);
        };
        Ok(Some(MovieProgress {
            provider: ID.into(),
            id: item.id.clone(),
            position_ms: item.state.time_offset,
            duration_ms: item.state.duration,
            updated_at: item.state.last_watched.unwrap_or_else(chrono::Utc::now),
            finished: false,
            video_id: item.state.video_id.clone(),
        }))
    }

    fn write_progress(&self, ctx: &Ctx<'_>, progress: &MovieProgress) -> Result<()> {
        let Some(key) = key(ctx)? else {
            return Ok(());
        };
        let endpoints = Endpoints::default();
        let client = DatastoreClient::new(ctx.http, &endpoints, key);
        datastore::write_progress(&client, progress).map(|_| ())
    }
}

/// One manifest catalog as a descriptor, for the catalogs the tabs can page:
/// `top`, `imdbRating` and `year`. Anything else is left out (a catalog without
/// a sort we can label has no chip to render).
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
                ("genre", _) => ("genre", "genre"),
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
