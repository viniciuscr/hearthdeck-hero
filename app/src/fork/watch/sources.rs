//! What a Watch library item is: its Stremio type, genres, provider and top
//! catalog membership. These are the only Watch rules the Collection console
//! needs from the provider.

use std::collections::BTreeSet;
use std::sync::Arc;

use heronite_data::CollectionGame;

/// `movie` or `series`; items without a Stremio type are movies.
pub fn kind(game: &CollectionGame) -> &str {
    game.game.extra.as_ref().and_then(|extra| extra.get("stremioType")).and_then(serde_json::Value::as_str).unwrap_or("movie")
}

/// Whether the provider put the item in its top catalog.
pub fn top(game: &CollectionGame) -> bool {
    game.game.extra.as_ref().and_then(|extra| extra.get("stremioTop")).and_then(serde_json::Value::as_bool).unwrap_or(false)
}

pub fn has_genre(game: &CollectionGame, value: &str) -> bool {
    game.metadata.as_ref().is_some_and(|metadata| metadata.genres.iter().any(|genre| genre.eq_ignore_ascii_case(value)))
}

/// The genres present in a Watch library, sorted.
pub fn genres(games: &[Arc<CollectionGame>]) -> Vec<String> {
    let mut genres = BTreeSet::new();
    for game in games {
        if let Some(metadata) = &game.metadata {
            genres.extend(metadata.genres.iter().filter(|genre| !genre.trim().is_empty()).cloned());
        }
    }
    genres.into_iter().collect()
}

/// The providers present in a Watch library, sorted.
pub fn providers(games: &[Arc<CollectionGame>]) -> Vec<String> {
    let mut providers = BTreeSet::new();
    for game in games {
        if !game.source_label.is_empty() {
            providers.insert(game.source_label.to_string());
        }
    }
    providers.into_iter().collect()
}
