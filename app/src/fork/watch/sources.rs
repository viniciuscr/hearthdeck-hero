//! What a Watch library item is: its Stremio type, genres, provider and top
//! catalog membership. These are the only Watch rules the Collection console
//! needs from the provider.

use std::collections::BTreeSet;
use std::sync::Arc;

use heronite_data::CollectionGame;
use heronite_watch::types::Title;

/// The provider's own id for the title: what its API and its progress take.
/// `extra.watchId` keeps the raw id while `app_name` is provider-prefixed.
pub fn id(game: &CollectionGame) -> &str {
    game.game.extra.as_ref().and_then(|extra| extra.get("watchId")).and_then(serde_json::Value::as_str).unwrap_or(&game.app_name)
}

/// `movie` or `series`; items without a Stremio type are movies.
pub fn kind(game: &CollectionGame) -> &str {
    game.game.extra.as_ref().and_then(|extra| extra.get("stremioType")).and_then(serde_json::Value::as_str).unwrap_or("movie")
}

/// The title a library record is, as Watch sees it.
///
/// The accessors below are the cheap views of the same record; this is the typed
/// one for a caller that needs several fields at once, and it comes from the
/// adapter that owns the projection's keys (`heronite_watch::library`).
pub fn title(game: &CollectionGame) -> Title {
    heronite_watch::library::title_of_game(game)
}

/// The rail that brought the title in (`extra.watchCatalog`): a catalogue page's
/// feed key, or `collection:<id>` for one of the rails the person turned on.
pub fn rail_marker(game: &CollectionGame) -> Option<&str> {
    game.game.extra.as_ref().and_then(|extra| extra.get("watchCatalog")).and_then(serde_json::Value::as_str)
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

/// Whether this title came from a provider and **no stream source here has it**:
/// a placeholder. It shows where titles show, greyed (the library says it is not
/// installed), and there is nothing to play. A record marks it explicitly; one
/// from before the marker existed is imported iff its runner is the owner.
pub fn imported(game: &CollectionGame) -> bool {
    game.game
        .extra
        .as_ref()
        .and_then(|extra| extra.get("imported"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(game.runner == heronite_watch::types::IMPORTED_ADDON)
}

/// The shelf snapshot of a record: what the person's flags store.
pub fn personal_item(game: &CollectionGame) -> heronite_watch::personal::ShelfTitle {
    title(game).to_shelf()
}
