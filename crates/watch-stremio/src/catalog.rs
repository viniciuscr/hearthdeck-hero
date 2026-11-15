//! The account's records as Watch items.

use heronite_watch_core::types::MovieItem;

use crate::datastore::LibraryItem;
use crate::ID;

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
