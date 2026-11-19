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

/// Whether this title came in from a provider and **no stream source here has
/// it**: a placeholder. It shows where titles show, greyed (the library says it is
/// not installed), and there is nothing to play.
pub fn imported(game: &CollectionGame) -> bool {
    game.game.extra.as_ref().and_then(|extra| extra.get("imported")).and_then(serde_json::Value::as_bool).unwrap_or(false)
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

/// The fields a personal rail keeps of a title: the card's, and the key the rest
/// of Watch resolves it by. Taken as values so the mapping can be tested without
/// a whole library record.
pub struct PersonalFields<'a> {
    pub provider: &'a str,
    pub kind: &'a str,
    pub id: &'a str,
    pub name: &'a str,
    pub year: Option<&'a str>,
    pub poster: Option<&'a str>,
    pub background: Option<&'a str>,
    pub description: Option<&'a str>,
    pub genres: &'a [String],
}

/// The title as the person's shelf remembers it: the card's fields, keyed the way
/// the rest of Watch resolves a title (`provider`, kind, id).
pub fn personal_snapshot(fields: PersonalFields<'_>) -> heronite_watch::personal::MovieItem {
    heronite_watch::personal::MovieItem {
        provider: fields.provider.to_owned(),
        kind: fields.kind.to_owned(),
        id: fields.id.to_owned(),
        name: fields.name.to_owned(),
        year: fields.year.map(str::to_owned),
        poster: fields.poster.map(str::to_owned),
        background: fields.background.map(str::to_owned),
        description: fields.description.map(str::to_owned),
        genres: fields.genres.to_vec(),
        ..Default::default()
    }
}

/// [`personal_snapshot`] of a library record.
pub fn personal_item(game: &CollectionGame) -> heronite_watch::personal::MovieItem {
    personal_snapshot(PersonalFields {
        provider: &game.runner,
        kind: kind(game),
        id: id(game),
        name: &game.title,
        year: year(game),
        poster: game.game.art_cover.as_deref(),
        background: game.game.art_background.as_deref(),
        description: game.metadata.as_ref().and_then(|metadata| metadata.description.as_deref()),
        genres: game.metadata.as_ref().map_or(&[], |metadata| metadata.genres.as_slice()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_keeps_the_cards_fields_and_its_key() {
        let genres = ["Action".to_owned(), "Sci-Fi".to_owned()];
        let item = personal_snapshot(PersonalFields {
            provider: "stremio",
            kind: "series",
            id: "tt0944947",
            name: "Game of Thrones",
            year: Some("2011–2019"),
            poster: Some("https://img/poster.jpg"),
            background: Some("https://img/hero.jpg"),
            description: Some("Winter is coming."),
            genres: &genres,
        });
        assert_eq!((item.provider.as_str(), item.kind.as_str(), item.id.as_str()), ("stremio", "series", "tt0944947"));
        assert_eq!(item.name, "Game of Thrones");
        assert_eq!(item.year.as_deref(), Some("2011–2019"));
        assert_eq!(item.poster.as_deref(), Some("https://img/poster.jpg"));
        assert_eq!(item.background.as_deref(), Some("https://img/hero.jpg"));
        assert_eq!(item.description.as_deref(), Some("Winter is coming."));
        assert_eq!(item.genres, ["Action", "Sci-Fi"]);
        assert!(!item.top, "a snapshot is not a placement in a provider rail");
    }

    #[test]
    fn a_snapshot_without_a_year_or_art_is_still_a_title() {
        let item = personal_snapshot(PersonalFields {
            provider: "stremio",
            kind: "movie",
            id: "tt0133093",
            name: "The Matrix",
            year: None,
            poster: None,
            background: None,
            description: None,
            genres: &[],
        });
        assert_eq!(item.name, "The Matrix");
        assert!(item.year.is_none() && item.poster.is_none() && item.background.is_none() && item.genres.is_empty());
    }
}
