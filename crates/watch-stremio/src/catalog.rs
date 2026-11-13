//! Cinemeta's Top catalogs and its records as Watch items.

use anyhow::Result;
use heronite_watch_addon::addon::AddonClient;
use heronite_watch_core::types::MovieItem;

use crate::datastore::LibraryItem;
use crate::ID;

pub const CINEMETA: &str = "https://v3-cinemeta.strem.io/manifest.json";

/// Cinemeta's Top catalog of `kind`, every item marked as such.
pub fn top(addon: &AddonClient<'_>, kind: &str) -> Result<Vec<MovieItem>> {
    Ok(addon
        .catalog(kind, "top", &[])?
        .into_iter()
        .map(|mut movie| {
            movie.top = true;
            movie.provider = ID.into();
            movie
        })
        .collect())
}

/// A datastore record as a Watch item.
pub fn from_library(item: &LibraryItem) -> MovieItem {
    MovieItem {
        id: item.id.clone(),
        kind: item.kind.clone(),
        name: item.name.clone(),
        poster: item.poster.clone(),
        last_watched: item.state.last_watched.map(|date| date.to_rfc3339()),
        minutes_played: item.state.time_watched as f64 / 60_000.0,
        provider: ID.into(),
        ..MovieItem::default()
    }
}
