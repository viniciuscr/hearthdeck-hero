//! What a Watch library item is: its Stremio type, genres, provider and top
//! catalog membership. These are the only Watch rules the Collection console
//! needs from the provider.

use std::collections::BTreeSet;
use std::sync::Arc;

use heronite_data::CollectionGame;

/// The provider's own id for the title: what its API and its progress take.
/// `extra.watchId` keeps the raw id while `app_name` is provider-prefixed.
pub fn id(game: &CollectionGame) -> &str {
    game.game.extra.as_ref().and_then(|extra| extra.get("watchId")).and_then(serde_json::Value::as_str).unwrap_or(&game.app_name)
}

/// `movie` or `series`; items without a Stremio type are movies.
pub fn kind(game: &CollectionGame) -> &str {
    game.game.extra.as_ref().and_then(|extra| extra.get("stremioType")).and_then(serde_json::Value::as_str).unwrap_or("movie")
}

/// Whether the provider put the item in its top catalog.
pub fn top(game: &CollectionGame) -> bool {
    game.game.extra.as_ref().and_then(|extra| extra.get("stremioTop")).and_then(serde_json::Value::as_bool).unwrap_or(false)
}

/// The year the library knows ("1999", "2011–2019"), for a metadata source
/// that has to search by name.
pub fn year(game: &CollectionGame) -> Option<&str> {
    game.details.release_date.as_deref().map(str::trim).filter(|date| !date.is_empty())
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

/// The title as the person's shelf remembers it: the fields the card draws, keyed
/// the way the rest of Watch resolves a title (`runner`, kind, id).
pub fn personal_item(game: &CollectionGame) -> heronite_watch::personal::MovieItem {
    heronite_watch::personal::MovieItem {
        provider: game.runner.clone(),
        kind: kind(game).to_owned(),
        id: id(game).to_owned(),
        name: game.title.clone(),
        year: year(game).map(str::to_owned),
        poster: game.game.art_cover.clone(),
        background: game.game.art_background.clone(),
        description: game.metadata.as_ref().and_then(|metadata| metadata.description.clone()),
        genres: game.metadata.as_ref().map(|metadata| metadata.genres.clone()).unwrap_or_default(),
        ..Default::default()
    }
}
