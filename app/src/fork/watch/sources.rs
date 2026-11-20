//! What a Watch library item is: its Stremio type, genres, provider and top
//! catalog membership. These are the only Watch rules the Collection console
//! needs from the provider.

use std::collections::BTreeSet;
use std::sync::Arc;

use heronite_data::CollectionGame;
use heronite_i18n::tr;
use heronite_watch::progress::WatchedState;
use heronite_watch::projection::{DURATION_KEY, FINISHED_KEY, POSITION_KEY, RELEASED_KEY, RUNTIME_KEY};
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

/// What the last stream probe found for the title: `Some(true)` an addon
/// offered a stream, `Some(false)` it answered with none, `None` never asked
/// (or the answer aged out). A catalogue listing is not a stream, so this is
/// the only fact that says a title can be played.
pub fn streams(game: &CollectionGame) -> Option<bool> {
    game.game.extra.as_ref()?.get(heronite_watch::projection::STREAMS_KEY)?.as_bool()
}

/// Whether a card is **dimmed**: a title no source here can play. A placeholder
/// never could; a title whose stream probe came back empty cannot now. Dimmed
/// whatever the theme's "grey uninstalled" says — that is information, not
/// decoration (`docs/FORK.md` F-019).
pub fn dimmed(game: &CollectionGame) -> bool {
    imported(game) || streams(game) == Some(false)
}

/// The catalogue or provider that listed the title (`extra.watchSource`):
/// `cinemeta`, `tmdb`, `stremio`; `None` when the record names none.
pub fn origin(game: &CollectionGame) -> Option<&str> {
    game.game.extra.as_ref()?.get(heronite_watch::projection::SOURCE_KEY)?.as_str().map(str::trim).filter(|source| !source.is_empty())
}

/// The name a source id is shown by: a catalogue's ("Cinemeta"), a stream
/// provider's ("Stremio"); `None` for an id neither registry knows.
fn source_name(id: &str) -> Option<&'static str> {
    heronite_watch::catalog::sources()
        .iter()
        .map(|source| source.descriptor())
        .find(|descriptor| descriptor.id == id)
        .map(|descriptor| descriptor.name)
        .or_else(|| heronite_watch::registry::providers().iter().find(|provider| provider.id() == id).map(|provider| provider.name()))
}

/// Where a title comes from, as the page and the tiles say it: the catalogue
/// that listed it first, then the stream provider **only when a stream is known
/// to exist**. A catalogue stamps a provider on every title it lists (the one
/// that would resolve a stream), which is a claim rather than a fact — a title
/// with no stream must not name a source that cannot play it. A title Stremio's
/// own account brought names Stremio, once.
pub fn origins(game: &CollectionGame) -> Vec<&'static str> {
    origins_with(game, None)
}

/// [`origins`] with a fresher answer than the projection's: the page's own
/// stream probe, made a moment ago, when there is one.
pub fn origins_with(game: &CollectionGame, live: Option<bool>) -> Vec<&'static str> {
    origin_names(origin(game).and_then(source_name), live.or_else(|| streams(game)), source_name(&game.runner))
}

/// [`origins`] over the names, so the rule has one home and a test.
fn origin_names(origin: Option<&'static str>, streams: Option<bool>, provider: Option<&'static str>) -> Vec<&'static str> {
    let Some(origin) = origin else {
        // A record neither registry names: its provider is all there is to say.
        return provider.into_iter().collect();
    };
    let mut names = vec![origin];
    if let Some(provider) = provider.filter(|provider| *provider != origin && streams == Some(true)) {
        names.push(provider);
    }
    names
}

/// The shelf snapshot of a record: what the person's flags store.
pub fn personal_item(game: &CollectionGame) -> heronite_watch::personal::ShelfTitle {
    title(game).to_shelf()
}

/// Where a title stands, as a card says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchState {
    Watched,
    /// Started and not finished. `percent` is `None` when the provider measured
    /// no length to take a share of.
    InProgress {
        percent: Option<u8>,
    },
    Unseen,
}

/// Where the title stands, from the record the projection wrote.
///
/// The provider's position and flag come first: Stremio leaves `timeWatched` at
/// 0 for a film that was only started, so minutes alone read as "never played".
/// A record written before the position was kept falls back to its minutes and
/// its stamp, which say it was started even when they cannot say how far.
pub fn watch_state(game: &CollectionGame) -> WatchState {
    state_from_extra(game.game.extra.as_ref(), game.playtime_minutes, game.last_played.is_some())
}

/// [`watch_state`] over the record's extra and the two fields the library
/// carries: the one place the projection's keys are read.
fn state_from_extra(extra: Option<&serde_json::Value>, minutes: u64, last_played: bool) -> WatchState {
    let number = |key: &str| extra.and_then(|extra| extra.get(key)).and_then(serde_json::Value::as_u64).unwrap_or(0);
    let finished = extra.and_then(|extra| extra.get(FINISHED_KEY)).and_then(serde_json::Value::as_bool);
    state_of(number(POSITION_KEY), number(DURATION_KEY), finished.unwrap_or(false), minutes, last_played)
}

/// [`state_from_extra`] over the four facts it reads, so the rule has one home.
fn state_of(position_ms: u64, duration_ms: u64, finished: bool, minutes: u64, last_played: bool) -> WatchState {
    match WatchedState::of(position_ms, duration_ms, finished) {
        WatchedState::Watched => WatchState::Watched,
        started @ WatchedState::InProgress { .. } => WatchState::InProgress { percent: started.percent() },
        WatchedState::Unwatched if minutes > 0 || last_played => WatchState::InProgress { percent: None },
        WatchedState::Unwatched => WatchState::Unseen,
    }
}

/// How long the title runs, in minutes: the catalogue's `173 min` (`2h 16m`
/// and a bare number are read too).
pub fn runtime_minutes(game: &CollectionGame) -> Option<u64> {
    runtime_of_extra(game.game.extra.as_ref())
}

/// [`runtime_minutes`] over the record's extra.
fn runtime_of_extra(extra: Option<&serde_json::Value>) -> Option<u64> {
    runtime_minutes_of(extra?.get(RUNTIME_KEY)?.as_str()?)
}

/// Minutes in a length a source wrote (`173 min`, `2h 16m`, a bare `136`): the
/// catalogue preview and the merged metadata do not agree on the spelling.
pub fn runtime_minutes_of(raw: &str) -> Option<u64> {
    let raw = raw.trim().to_ascii_lowercase();
    let number = |text: &str| text.trim().parse::<f64>().ok();
    // "2h 16m", "2 h 16 m", "2h".
    if let Some((hours, rest)) = raw.split_once('h') {
        let hours = number(hours)?;
        let minutes = rest.split_once('m').and_then(|(minutes, _)| number(minutes)).unwrap_or(0.);
        return Some((hours * 60. + minutes).round() as u64).filter(|minutes| *minutes > 0);
    }
    // "173 min", "173", "173.5".
    let digits: String = raw.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    Some(number(&digits)?.round() as u64).filter(|minutes| *minutes > 0)
}

/// A catalogue has no "added to your library" date, so "New" is a recent
/// release there. A week: long enough that a rail's arrival is noticed, short
/// enough that a whole catalogue does not wear the badge.
const NEW_RELEASE_DAYS: i64 = 7;

/// Whether the title came out in the last week, from the day the source gave.
pub fn is_new_release(game: &CollectionGame) -> bool {
    released_of_extra(game.game.extra.as_ref()).is_some_and(released_within_a_week)
}

/// [`is_new_release`]'s rule, over the day alone: this week, not a future date.
fn released_within_a_week(day: chrono::NaiveDate) -> bool {
    let age = chrono::Local::now().date_naive().signed_duration_since(day).num_days();
    (0..=NEW_RELEASE_DAYS).contains(&age)
}

fn released_of_extra(extra: Option<&serde_json::Value>) -> Option<chrono::NaiveDate> {
    parse_day(extra?.get(RELEASED_KEY)?.as_str()?)
}

/// `1999-03-31`, or the day of a full stamp.
fn parse_day(raw: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(raw.trim().get(..10)?, "%Y-%m-%d").ok()
}

/// Whether the title is a series: the length a source gives is one episode's,
/// never the whole show's.
pub fn is_series(game: &CollectionGame) -> bool {
    kind(game) == "series"
}

/// The label a card puts on the length: a film's runtime, or one episode's for a
/// series (the metadata sources give a show's per-episode length).
pub fn length_label(game: &CollectionGame) -> &'static str {
    length_label_of(kind(game))
}

/// [`length_label`] over the wire kind, so the words have one home.
fn length_label_of(kind: &str) -> &'static str {
    if kind == "series" {
        tr!("Episode runtime")
    } else {
        tr!("Runtime")
    }
}

/// The whole show at `per_episode` minutes an episode (`57 min` × 73): what
/// watching it from the first episode to the last takes. `None` without a count
/// or a length — the episode list is the merged metadata's to give.
pub fn binge_minutes(per_episode: Option<u64>, episodes: usize) -> Option<u64> {
    let episodes = u64::try_from(episodes).ok().filter(|episodes| *episodes > 0)?;
    per_episode?.checked_mul(episodes).filter(|minutes| *minutes > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use core::prelude::v1::test;

    #[test]
    fn a_titles_origins_name_the_catalogue_and_the_provider_only_once_a_stream_is_known() {
        // A catalogue listing alone: the catalogue, never the resolver it stamps.
        assert_eq!(origin_names(Some("Cinemeta"), None, Some("Stremio")), ["Cinemeta"]);
        // A probe found no stream: still the catalogue (and the card dims).
        assert_eq!(origin_names(Some("Cinemeta"), Some(false), Some("Stremio")), ["Cinemeta"]);
        // A stream exists: both, the catalogue first.
        assert_eq!(origin_names(Some("Cinemeta"), Some(true), Some("Stremio")), ["Cinemeta", "Stremio"]);
        // Stremio's own rails: one name, not the same one twice.
        assert_eq!(origin_names(Some("Stremio"), Some(true), Some("Stremio")), ["Stremio"]);
        // A record no registry names falls back to its provider.
        assert_eq!(origin_names(None, None, Some("Stremio")), ["Stremio"]);
        assert!(origin_names(None, None, None).is_empty());
    }

    #[test]
    fn a_runtime_is_read_in_the_spellings_a_catalogue_uses() {
        assert_eq!(runtime_minutes_of("173 min"), Some(173));
        assert_eq!(runtime_minutes_of("136"), Some(136));
        assert_eq!(runtime_minutes_of("2h 16m"), Some(136));
        assert_eq!(runtime_minutes_of("2 h 5 m"), Some(125));
        assert_eq!(runtime_minutes_of("2h"), Some(120));
        assert_eq!(runtime_minutes_of(""), None);
        assert_eq!(runtime_minutes_of("0 min"), None);
        assert_eq!(runtime_minutes_of("TBA"), None);
    }

    #[test]
    fn a_shows_length_is_one_episode_and_the_binge_is_all_of_them() {
        // The sources give a series one episode's length ("57 min"), never the
        // whole run: the label says which one it is, and the binge multiplies.
        assert_eq!(length_label_of("series"), "Episode runtime");
        assert_eq!(length_label_of("movie"), "Runtime");
        assert_eq!(binge_minutes(Some(57), 73), Some(4_161));
        assert_eq!(binge_minutes(Some(57), 0), None, "no episodes listed, nothing to watch");
        assert_eq!(binge_minutes(None, 73), None, "no length, no sum");
        assert_eq!(binge_minutes(Some(0), 73), None);
    }

    #[test]
    fn a_started_title_is_not_an_unseen_one() {
        // Stremio counts no minute for a film it only started: the position says
        // it was watched, and how far.
        assert_eq!(state_of(2_271_644, 6_628_768, false, 0, true), WatchState::InProgress { percent: Some(34) });
        assert_eq!(state_of(0, 0, false, 0, true), WatchState::InProgress { percent: None }, "an old record's stamp is enough");
        assert_eq!(state_of(0, 0, false, 42, false), WatchState::InProgress { percent: None }, "its minutes are too");
        assert_eq!(state_of(0, 0, true, 0, false), WatchState::Watched, "the provider's flag is its word");
        assert_eq!(state_of(0, 0, false, 0, false), WatchState::Unseen, "nothing played, nothing seen");
    }

    #[test]
    fn the_record_extra_is_read_through_the_projection_keys() {
        let extra = serde_json::json!({
            POSITION_KEY: 2_271_644u64,
            DURATION_KEY: 6_628_768u64,
            FINISHED_KEY: false,
            RUNTIME_KEY: "173 min",
            RELEASED_KEY: "2026-11-14T00:00:00.000Z",
        });
        assert_eq!(state_from_extra(Some(&extra), 0, true), WatchState::InProgress { percent: Some(34) });
        assert_eq!(runtime_of_extra(Some(&extra)), Some(173));
        assert_eq!(released_of_extra(Some(&extra)).map(|day| day.to_string()), Some("2026-11-14".to_owned()));

        let done = serde_json::json!({ FINISHED_KEY: true });
        assert_eq!(state_from_extra(Some(&done), 0, false), WatchState::Watched);
        assert_eq!(runtime_of_extra(Some(&done)), None, "no key, no answer");
        assert_eq!(released_of_extra(None), None, "a record from before the keys reads as nothing");
    }

    #[test]
    fn a_release_day_is_a_day_and_a_stamp_is_its_day() {
        assert_eq!(parse_day("2026-11-14").map(|day| day.to_string()), Some("2026-11-14".to_owned()));
        assert_eq!(parse_day("2026-11-14T00:00:00.000Z").map(|day| day.to_string()), Some("2026-11-14".to_owned()));
        assert_eq!(parse_day("2026"), None);
        assert_eq!(parse_day(""), None);
    }

    #[test]
    fn a_release_is_new_for_a_week_and_no_longer() {
        let today = chrono::Local::now().date_naive();
        let days = |n: i64| today - chrono::Duration::days(n);
        assert!(released_within_a_week(today), "out today");
        assert!(released_within_a_week(days(7)), "the last day of the window");
        assert!(!released_within_a_week(days(8)), "the games' two weeks would keep a whole catalogue new");
        assert!(!released_within_a_week(today + chrono::Duration::days(1)), "an unreleased title is not new yet");
    }
}
