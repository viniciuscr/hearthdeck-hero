//! The Watch grid's filtering and ordering: type, genre, provider and sort.
//! The Collection grid supplies the candidate items; this module owns the
//! Watch rules.

use std::sync::Arc;

use heronite_data::CollectionGame;

use super::sources;

pub fn retain_and_sort(
    all: &[Arc<CollectionGame>],
    games: &mut Vec<&CollectionGame>,
    kind: usize,
    genre: usize,
    provider: usize,
    sort: usize,
    fixed_kind: Option<&str>,
) {
    let genres = sources::genres(all);
    let providers = sources::providers(all);
    if fixed_kind.is_none() {
        games.retain(|g| match kind {
            1 => sources::kind(g) == "movie",
            2 => sources::kind(g) == "series",
            _ => true,
        });
    }
    if let Some(selected) = genre.checked_sub(1).and_then(|i| genres.get(i)) {
        games.retain(|g| sources::has_genre(g, selected));
    }
    if let Some(selected) = provider.checked_sub(1).and_then(|i| providers.get(i)) {
        games.retain(|g| g.source_label == *selected);
    }
    match sort {
        1 => games.sort_by(|a, b| release_key(b).cmp(release_key(a)).then_with(|| a.title_lower.cmp(&b.title_lower))),
        2 => games.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower))),
        _ => games.sort_by(|a, b| a.title_lower.cmp(&b.title_lower)),
    }
}

fn release_key(game: &CollectionGame) -> &str {
    game.metadata.as_ref().and_then(|metadata| metadata.release_date.as_deref()).unwrap_or("")
}
