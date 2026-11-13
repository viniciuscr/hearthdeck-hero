//! Cinemeta catalogs and the linked account's Continue Watching, normalized.
//!
//! Everything is reached through the transport and the session key the caller
//! passes in; this module reads no file.

use std::collections::HashSet;

use anyhow::Result;
use heronite_watch_addon::addon::AddonClient;
use heronite_watch_core::auth::AuthKey;
use heronite_watch_core::http::Transport;
use heronite_watch_core::types::MovieItem;

use crate::datastore::{continue_watching, DatastoreClient, LibraryItem};
use crate::endpoints::Endpoints;

pub const CINEMETA: &str = "https://v3-cinemeta.strem.io/manifest.json";

/// The linked account's films, then Cinemeta's Top. Series are out of scope in
/// v1; the catalog is films only.
pub fn fetch(http: &dyn Transport, endpoints: &Endpoints, key: Option<AuthKey>) -> Result<Vec<MovieItem>> {
    let mut movies = Vec::new();
    if let Some(key) = key {
        let client = DatastoreClient::new(http, endpoints, key);
        for item in continue_watching(&client.get_all()?) {
            if item.kind == "movie" || item.kind == "series" {
                movies.push(movie_from_library(item));
            }
        }
    }
    let mut addon = AddonClient::new(http, CINEMETA)?;
    addon.manifest()?;
    for kind in ["movie", "series"] {
        movies.extend(addon.catalog(kind, "top", &[])?.into_iter().map(|mut movie| {
            movie.top = true;
            movie.provider = "stremio".into();
            movie
        }));
    }
    let mut seen = HashSet::new();
    movies.retain(|movie| seen.insert(format!("{}:{}", movie.kind, movie.id)));
    Ok(movies)
}

fn movie_from_library(item: &LibraryItem) -> MovieItem {
    MovieItem {
        id: item.id.clone(),
        kind: item.kind.clone(),
        name: item.name.clone(),
        poster: item.poster.clone(),
        last_watched: item.state.last_watched.map(|date| date.to_rfc3339()),
        minutes_played: item.state.time_watched as f64 / 60_000.0,
        provider: "stremio".into(),
        ..MovieItem::default()
    }
}
