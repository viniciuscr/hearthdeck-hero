//! The Stremio provider: the linked account's library and catalogs, its stream
//! addons, the local service, and its datastore.
//!
//! Everything arrives through `Ctx`; the provider reads no file.

use anyhow::{anyhow, bail, Context, Result};
use url::Url;

use heronite_watch_addon::addon::AddonClient;
use heronite_watch_core::auth::AuthKey;
use heronite_watch_core::provider::{CatalogDescriptor, Ctx, Rail, WatchProvider};
use heronite_watch_core::types::{MovieItem, MovieProgress, StreamChoice};

use crate::account::addon_collection;
use crate::catalog::{self, CINEMETA};
use crate::datastore::{self, DatastoreClient};
use crate::endpoints::Endpoints;
use crate::streaming::StreamingServer;
use crate::ID;

const CONTINUE_WATCHING: CatalogDescriptor = CatalogDescriptor { id: "continue-watching", kind: "" };

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
            rails.push(Rail { catalog: CatalogDescriptor { id: "top", kind }, items });
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
                Ok(mut streams) => {
                    for stream in &mut streams {
                        stream.addon.clone_from(&manifest.name);
                    }
                    choices.append(&mut streams);
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

    fn resolve(&self, ctx: &Ctx<'_>, stream: &StreamChoice) -> Result<Url> {
        let endpoints = Endpoints::default();
        endpoints.validate()?;
        if let Some(url) = &stream.url {
            return Url::parse(url).context("stream URL");
        }
        let hash = stream.info_hash.as_deref().ok_or_else(|| anyhow!("This stream has nothing to play."))?;
        let server = StreamingServer::new(ctx.http, endpoints.streaming_server.clone())?;
        let base = server.base_url().context("The Stremio service is not running")?;
        server.create(hash, &[]).context("The Stremio service refused the torrent")?;
        StreamingServer::stream_url(&base, hash, stream.file_idx)
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
