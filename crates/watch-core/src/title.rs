//! What Watch calls a title: a film or a series, the id space it is known in,
//! and the stream providers that can play it.
//!
//! This is the rework's centre. Watch used to borrow the collection's idea of a
//! *game from a store* — `runner`, `app_name`, `installed` — with its own data
//! smuggled through `extra`. The structure stays (library, rails, tiles,
//! progress); the words and the model become ours: **stream providers** and
//! **movies/series**.
//!
//! Nothing here is serialized. A `Title` lives in memory; the projection in
//! `heronite-watch-state` writes the shapes upstream reads, and the shelf's own
//! snapshot owns its serialization. `docs/WATCH-TITLES.md` owns the transition.

use std::collections::BTreeSet;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::collections::mark;
use crate::personal::ShelfTitle;
use crate::types::MovieItem;

/// The `addon` marker an imported placeholder carries: a title a provider knows
/// and no stream source here has.
pub const IMPORTED_ADDON: &str = "imported";

/// A film or a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TitleKind {
    Movie,
    Series,
}

impl TitleKind {
    /// The kind a media source's own word means ("movie", "series", "show", …).
    pub fn of(raw: &str) -> Self {
        match raw {
            "series" | "show" | "tv" => Self::Series,
            _ => Self::Movie,
        }
    }

    /// The word the library stores it by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }
}

/// A stream provider: the thing that can play a title (Stremio today, others as
/// they come). It is **not** a store, and it owns no library.
///
/// The only ids that should reach this type are the ones the provider registry
/// serves; an empty or blank one is refused rather than becoming an owner.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    /// The provider `id` names, or `None` when it names nothing.
    pub fn new(id: &str) -> Option<Self> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        Some(Self(id.to_owned()))
    }

    /// The id a provider crate declares as its own.
    ///
    /// Trusted input: the constants the provider and catalogue crates carry
    /// (`Stremio`'s `ID`, a source's `PLAYS_WITH`), never a wire payload. An
    /// empty one is a bug in the caller, not something the registry serves.
    pub fn declared(id: &'static str) -> Self {
        debug_assert!(!id.trim().is_empty(), "a provider crate declared an empty id");
        Self(id.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The space that issued a title's id: the IMDb catalogue, or one provider's
/// own namespace (a YouTube video id, say).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IdSpace {
    Imdb,
    Provider(ProviderId),
}

/// The id a title is known by, in the space that issued it.
///
/// The space is part of the id on purpose: a second provider may know the same
/// film under its own id, and matching the two is a lookup (name and release),
/// never a merge of identities.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TitleId {
    pub space: IdSpace,
    pub id: String,
}

impl TitleId {
    /// The IMDb id (`tt0133093`): the canonical space, which every provider's
    /// catalogue resolves to when it can.
    pub fn imdb(id: &str) -> Self {
        Self { space: IdSpace::Imdb, id: id.to_owned() }
    }

    /// An id `provider` issued for the title.
    pub fn of(provider: ProviderId, id: &str) -> Self {
        Self { space: IdSpace::Provider(provider), id: id.to_owned() }
    }

    pub fn as_str(&self) -> &str {
        &self.id
    }
}

/// A title's identity: what makes two observations the same title. The stream
/// provider is a source of a title, never part of what it is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TitleKey {
    pub kind: TitleKind,
    pub id: TitleId,
}

impl TitleKey {
    pub fn new(kind: TitleKind, id: TitleId) -> Self {
        Self { kind, id }
    }

    /// The identity a provider's rail item carries.
    ///
    /// An item whose id is not an IMDb id is known in its own provider's space;
    /// everything else resolves to the canonical one.
    pub fn of_item(item: &MovieItem) -> Self {
        Self::of_parts(&item.kind, &item.id, ProviderId::new(&item.provider))
    }

    /// The identity a source's own id names, under the provider that plays it.
    pub fn of_source(kind: &str, id: &str, provider: &ProviderId) -> Self {
        Self::of_parts(kind, id, Some(provider.clone()))
    }

    /// The identity a kind, an id and (when it has one) a provider make.
    fn of_parts(kind: &str, id: &str, provider: Option<ProviderId>) -> Self {
        let id = match provider {
            Some(provider) if !is_imdb(id) => TitleId::of(provider, id),
            _ => TitleId::imdb(id),
        };
        Self { kind: TitleKind::of(kind), id }
    }
}

/// What the screens draw for a title.
///
/// The only part of a title that is serialized: it is flattened into the shelf's
/// snapshot, where the old item wrote these fields at the top level.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Art {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

impl Art {
    /// Fills the fields this art is missing from `other`, keeping its own.
    fn fill_from(&mut self, other: Self) {
        if self.poster.is_none() {
            self.poster = other.poster;
        }
        if self.background.is_none() {
            self.background = other.background;
        }
        if self.logo.is_none() {
            self.logo = other.logo;
        }
    }
}

/// Where a title was listed. The first mark names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RailMark {
    /// A catalogue page: the source that answered, and its feed key.
    Feed { source: String, catalog: String },
    /// One of the rails the person turned on.
    Collection(String),
}

impl RailMark {
    pub fn feed(source: &str, catalog: &str) -> Self {
        Self::Feed { source: source.to_owned(), catalog: catalog.to_owned() }
    }

    pub fn collection(id: &str) -> Self {
        Self::Collection(id.to_owned())
    }

    /// The string the projection stores in `extra.watchCatalog`.
    ///
    /// A collection's mark keeps its prefix — the writer's `kept` reads it back
    /// off the wire — and a feed keeps the catalogue's own key.
    pub fn as_wire(&self) -> String {
        match self {
            Self::Collection(id) => mark(id),
            Self::Feed { catalog, .. } => catalog.clone(),
        }
    }

    /// The mark `wire` means, given the `addon` that supplied the item: the
    /// collection prefix says which kind of rail it was.
    pub fn from_wire(wire: &str, addon: &str) -> Self {
        match wire.strip_prefix(crate::collections::MARK_PREFIX) {
            Some(id) => Self::Collection(id.to_owned()),
            None => Self::Feed { source: addon.to_owned(), catalog: wire.to_owned() },
        }
    }
}

/// What the library knows about watching a title: when it was last watched, and
/// the minutes it has taken. The provider's own stamp, parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct Watched {
    pub at: Option<DateTime<Utc>>,
    pub minutes: f64,
}

/// One title in the Watch library.
///
/// Identity is the key; the providers that can play it are a set, because the
/// same film may play from more than one. No provider at all is a placeholder:
/// a title a provider knows and nothing here can play.
#[derive(Clone, Debug, PartialEq)]
pub struct Title {
    pub key: TitleKey,
    pub name: String,
    /// As the library prints it: `1999`, or a series' `2011–2019`.
    pub release: Option<String>,
    pub art: Art,
    pub description: Option<String>,
    pub genres: Vec<String>,
    /// The rails that brought it, first mark first.
    pub rails: Vec<RailMark>,
    /// A wire placement, not identity: Cinemeta's Top membership. Kept as a
    /// field because the single persisted mark cannot carry it.
    pub top: bool,
    /// The stream providers that can play it; empty for a placeholder.
    pub providers: BTreeSet<ProviderId>,
    pub watched: Option<Watched>,
}

impl Title {
    /// A title with nothing known about it yet, and no provider that plays it.
    pub fn new(key: TitleKey, name: &str) -> Self {
        Self {
            key,
            name: name.to_owned(),
            release: None,
            art: Art::default(),
            description: None,
            genres: Vec::new(),
            rails: Vec::new(),
            top: false,
            providers: BTreeSet::new(),
            watched: None,
        }
    }

    /// Records that `provider` can play it.
    #[must_use]
    pub fn plays(mut self, provider: ProviderId) -> Self {
        self.providers.insert(provider);
        self
    }

    /// Records the rail that brought it.
    #[must_use]
    pub fn listed(mut self, mark: RailMark) -> Self {
        if !self.rails.contains(&mark) {
            self.rails.push(mark);
        }
        self
    }

    /// The title a provider's rail item is: the boundary every source crosses.
    ///
    /// An item with no provider is a placeholder — a title a provider knows and
    /// nothing here can play. No item gets a provider it did not name: a
    /// catalogue source stamps the provider that plays its titles itself.
    pub fn of_item(item: &MovieItem) -> Self {
        Self {
            key: TitleKey::of_item(item),
            name: item.name.clone(),
            release: item.year.clone(),
            art: Art { poster: item.poster.clone(), background: item.background.clone(), logo: item.logo.clone() },
            description: item.description.clone(),
            genres: item.genres.clone(),
            rails: item.catalog.as_deref().map(|wire| RailMark::from_wire(wire, &item.addon)).into_iter().collect(),
            top: item.top,
            providers: ProviderId::new(&item.provider).into_iter().collect(),
            watched: watched_of(item),
        }
    }

    /// The title a kept snapshot is: the fields the person saw when they kept
    /// it, and the provider that played it then. A snapshot has no rail.
    pub fn of_shelf(shelf: &ShelfTitle) -> Self {
        Self {
            key: TitleKey::of_parts(&shelf.kind, &shelf.id, ProviderId::new(&shelf.provider)),
            name: shelf.name.clone(),
            release: shelf.release.clone(),
            art: shelf.art.clone(),
            description: shelf.description.clone(),
            genres: shelf.genres.clone(),
            rails: Vec::new(),
            top: false,
            providers: ProviderId::new(&shelf.provider).into_iter().collect(),
            watched: None,
        }
    }

    /// The shelf snapshot of this title: the fields a card needs, and the
    /// provider that played it as provenance.
    pub fn to_shelf(&self) -> ShelfTitle {
        ShelfTitle {
            provider: self.providers.iter().next().map(|id| id.as_str().to_owned()).unwrap_or_default(),
            kind: self.key.kind.as_str().to_owned(),
            id: self.key.id.as_str().to_owned(),
            name: self.name.clone(),
            release: self.release.clone(),
            art: self.art.clone(),
            description: self.description.clone(),
            genres: self.genres.clone(),
        }
    }

    /// Whether a stream provider here can play it at all. A placeholder cannot
    /// — that is what "not currently present in any available stream source"
    /// means, and what the tiles grey.
    pub fn available(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Whether `provider` is one of the providers that play this title.
    pub fn played_by(&self, provider: &ProviderId) -> bool {
        self.providers.contains(provider)
    }

    /// Folds a second observation of **the same title** into this one.
    ///
    /// One field at a time, the first non-empty answer wins (the
    /// `TitleDetails::merge` rule), so a provider that has a poster fills a
    /// title another left bare. Sets union: every provider that plays it, and
    /// every rail that named it, in the order they arrived. A later watch
    /// replaces an earlier one; a tie keeps this title's.
    ///
    /// Not commutative: `rails` order is the order of arrival, and that order
    /// names the title.
    ///
    /// Returns the observation it could not use: a different key is a different
    /// title, and folding it in would corrupt both.
    pub fn merge(&mut self, other: Title) -> Option<Title> {
        if self.key != other.key {
            return Some(other);
        }
        if self.name.trim().is_empty() {
            self.name = other.name;
        }
        if self.release.is_none() {
            self.release = other.release;
        }
        if self.description.is_none() {
            self.description = other.description;
        }
        if self.genres.is_empty() {
            self.genres = other.genres;
        }
        self.art.fill_from(other.art);
        for mark in other.rails {
            if !self.rails.contains(&mark) {
                self.rails.push(mark);
            }
        }
        self.providers.extend(other.providers);
        self.top |= other.top;
        let later = other.watched.as_ref().is_some_and(|seen| self.watched.as_ref().is_none_or(|own| seen.at > own.at));
        if later {
            self.watched = other.watched;
        }
        None
    }
}

/// Whether `id` is an IMDb id (`tt0133093`).
fn is_imdb(id: &str) -> bool {
    id.strip_prefix("tt").is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The watch a provider reports, parsed: a stamp, minutes, or both.
fn watched_of(item: &MovieItem) -> Option<Watched> {
    let at = item.last_watched.as_deref().and_then(parse_stamp);
    (at.is_some() || item.minutes_played > 0.0).then_some(Watched { at, minutes: item.minutes_played })
}

/// The instant a provider's stamp names; an unparseable one is no stamp.
fn parse_stamp(stamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(stamp).ok().map(|at| at.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str) -> ProviderId {
        ProviderId::new(id).expect("a provider id in a test")
    }

    fn key(id: &str) -> TitleKey {
        TitleKey::new(TitleKind::Movie, TitleId::imdb(id))
    }

    /// A film with one provider and nothing else filled in.
    fn title(id: &str, provider_id: &str) -> Title {
        Title {
            key: key(id),
            name: "A Film".into(),
            release: None,
            art: Art::default(),
            description: None,
            genres: Vec::new(),
            rails: Vec::new(),
            top: false,
            providers: BTreeSet::from([provider(provider_id)]),
            watched: None,
        }
    }

    fn at(when: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(when).expect("an rfc3339 instant").with_timezone(&Utc)
    }

    #[test]
    fn a_film_in_two_providers_is_one_title_with_two_providers() {
        let mut film = title("tt1", "stremio");
        assert!(film.merge(title("tt1", "youtube")).is_none(), "the same key merges");
        assert_eq!(film.providers.len(), 2, "{film:?}");
        assert!(film.available());
        assert!(film.played_by(&provider("youtube")));
        assert!(film.played_by(&provider("stremio")));
    }

    #[test]
    fn a_placeholder_has_no_provider_and_is_not_available() {
        let mut placeholder = title("tt9", "stremio");
        placeholder.providers.clear();
        assert!(!placeholder.available());
        assert!(!placeholder.played_by(&provider("stremio")), "nothing here plays it");
        assert_eq!(placeholder.key, key("tt9"));
    }

    #[test]
    fn a_series_is_a_series_and_a_show_counts_as_one() {
        assert_eq!(TitleKind::of("series"), TitleKind::Series);
        assert_eq!(TitleKind::of("show"), TitleKind::Series);
        assert_eq!(TitleKind::of("movie"), TitleKind::Movie);
        assert_eq!(TitleKind::Series.as_str(), "series");
        assert_eq!(TitleKind::Movie.as_str(), "movie");
    }

    #[test]
    fn merge_fills_what_the_first_observation_left_empty() {
        let mut first = title("tt1", "stremio");
        let mut second = title("tt1", "stremio");
        second.name = "Another Name".into();
        second.description = Some("A plot".into());
        second.genres = vec!["Drama".into()];
        second.art.poster = Some("https://img/poster.jpg".into());
        second.release = Some("1999".into());

        assert!(first.merge(second).is_none());
        assert_eq!(first.name, "A Film", "the first non-empty name wins");
        assert_eq!(first.description.as_deref(), Some("A plot"));
        assert_eq!(first.genres, ["Drama"]);
        assert_eq!(first.art.poster.as_deref(), Some("https://img/poster.jpg"));
        assert_eq!(first.release.as_deref(), Some("1999"));
    }

    #[test]
    fn merge_keeps_the_first_rail_and_adds_the_others() {
        let mut first = title("tt1", "stremio");
        first.rails.push(RailMark::feed("tmdb", "movie:top:Popular"));
        first.rails.push(RailMark::collection("tmdb:top-rated:movie"));
        let mut second = title("tt1", "stremio");
        second.rails = vec![RailMark::feed("tmdb", "movie:top:Popular"), RailMark::feed("cinemeta", "movie:top:Popular")];

        assert!(first.merge(second).is_none());
        assert_eq!(
            first.rails,
            [
                RailMark::feed("tmdb", "movie:top:Popular"),
                RailMark::collection("tmdb:top-rated:movie"),
                RailMark::feed("cinemeta", "movie:top:Popular"),
            ],
            "a mark already there is not added again, and the first still names the title"
        );
    }

    #[test]
    fn merge_keeps_the_later_watch_and_its_minutes() {
        let mut first = title("tt1", "stremio");
        first.watched = Some(Watched { at: Some(at("2026-11-12T20:00:00Z")), minutes: 42.0 });
        let mut second = title("tt1", "stremio");
        second.watched = Some(Watched { at: Some(at("2026-11-13T20:00:00Z")), minutes: 7.0 });

        assert!(first.merge(second).is_none());
        assert_eq!(first.watched.as_ref().and_then(|seen| seen.at), Some(at("2026-11-13T20:00:00Z")));
        assert_eq!(first.watched.as_ref().map(|seen| seen.minutes), Some(7.0), "the minutes travel with the stamp");
    }

    #[test]
    fn merge_keeps_the_left_watch_when_the_stamps_are_equal() {
        let same = at("2026-11-12T20:00:00Z");
        let mut first = title("tt1", "stremio");
        first.watched = Some(Watched { at: Some(same), minutes: 42.0 });
        let mut second = title("tt1", "stremio");
        second.watched = Some(Watched { at: Some(same), minutes: 7.0 });

        assert!(first.merge(second).is_none());
        assert_eq!(first.watched.as_ref().map(|seen| seen.minutes), Some(42.0));
    }

    #[test]
    fn a_shelf_snapshot_keeps_the_cards_fields_and_its_provider() {
        let mut title = title("tt0944947", "stremio");
        title.key.kind = TitleKind::Series;
        title.name = "Game of Thrones".into();
        title.release = Some("2011–2019".into());
        title.art = Art { poster: Some("https://img/poster.jpg".into()), background: Some("https://img/hero.jpg".into()), logo: None };
        title.description = Some("Winter is coming.".into());
        title.genres = vec!["Action".into(), "Sci-Fi".into()];

        let shelf = title.to_shelf();
        assert_eq!((shelf.provider.as_str(), shelf.kind.as_str(), shelf.id.as_str()), ("stremio", "series", "tt0944947"));
        assert_eq!(shelf.name, "Game of Thrones");
        assert_eq!(shelf.release.as_deref(), Some("2011–2019"));
        assert_eq!(shelf.art.poster.as_deref(), Some("https://img/poster.jpg"));
        assert_eq!(shelf.art.background.as_deref(), Some("https://img/hero.jpg"));
        assert_eq!(shelf.description.as_deref(), Some("Winter is coming."));
        assert_eq!(shelf.genres, ["Action", "Sci-Fi"]);
    }

    #[test]
    fn a_placeholder_snapshot_names_no_provider() {
        let mut placeholder = title("tt9", "stremio");
        placeholder.providers.clear();
        assert_eq!(placeholder.to_shelf().provider, "", "provenance of nothing playable");
    }

    #[test]
    fn a_dated_watch_beats_an_undated_one() {
        let mut first = title("tt1", "stremio");
        first.watched = Some(Watched { at: None, minutes: 42.0 });
        let mut second = title("tt1", "stremio");
        second.watched = Some(Watched { at: Some(at("2026-11-12T20:00:00Z")), minutes: 7.0 });

        assert!(first.merge(second).is_none());
        assert_eq!(first.watched.as_ref().and_then(|seen| seen.at), Some(at("2026-11-12T20:00:00Z")));
    }

    #[test]
    fn top_is_any_of_the_observations() {
        let mut first = title("tt1", "stremio");
        let mut second = title("tt1", "stremio");
        second.top = true;
        assert!(first.merge(second).is_none());
        assert!(first.top);
    }

    #[test]
    fn merging_a_different_key_hands_the_observation_back() {
        let mut film = title("tt1", "stremio");
        let rejected = film.merge(title("tt2", "youtube")).expect("a different key is not merged");
        assert_eq!(rejected.key, key("tt2"));
        assert_eq!(film.providers.len(), 1, "the film did not take the other title's provider");
        assert_eq!(film.key, key("tt1"));
    }

    #[test]
    fn an_id_space_tells_an_imdb_id_from_a_providers() {
        let imdb = TitleId::imdb("tt1");
        let video = TitleId::of(provider("youtube"), "dQw4w9WgXcQ");
        assert_ne!(imdb, video, "the same film under two ids is a lookup, not one identity");
        assert_eq!(imdb.space, IdSpace::Imdb);
        assert_eq!(video.space, IdSpace::Provider(provider("youtube")));

        let film = TitleKey::new(TitleKind::Movie, imdb.clone());
        let series = TitleKey::new(TitleKind::Series, imdb);
        assert_ne!(film, series, "a film and a series may share an id");
    }

    #[test]
    fn a_provider_id_needs_a_name() {
        assert!(ProviderId::new("").is_none());
        assert!(ProviderId::new("   ").is_none());
        assert_eq!(ProviderId::new(" stremio ").map(|id| id.as_str().to_owned()), Some("stremio".to_owned()));
        assert_eq!(provider("stremio").to_string(), "stremio");
    }

    fn item(id: &str, provider_id: &str, addon: &str) -> MovieItem {
        MovieItem {
            id: id.into(),
            kind: "movie".into(),
            name: "A Film".into(),
            provider: provider_id.into(),
            addon: addon.into(),
            ..MovieItem::default()
        }
    }

    #[test]
    fn an_item_is_a_title_with_the_provider_that_plays_it() {
        let title = Title::of_item(&item("tt1", "stremio", "cinemeta"));
        assert_eq!(title.key, TitleKey::new(TitleKind::Movie, TitleId::imdb("tt1")));
        assert!(title.available());
        assert!(title.played_by(&provider("stremio")));
    }

    #[test]
    fn an_item_without_a_provider_is_a_placeholder() {
        let title = Title::of_item(&item("tt9", "", IMPORTED_ADDON));
        assert!(!title.available());
        assert!(title.providers.is_empty());
        assert_eq!(title.key, TitleKey::new(TitleKind::Movie, TitleId::imdb("tt9")));
    }

    #[test]
    fn a_providers_own_id_stays_in_its_space() {
        let video = Title::of_item(&item("dQw4w9WgXcQ", "youtube", "youtube"));
        assert_eq!(video.key.id.space, IdSpace::Provider(provider("youtube")));
        let film = Title::of_item(&item("tt0133093", "stremio", ""));
        assert_eq!(film.key.id.space, IdSpace::Imdb);
    }

    #[test]
    fn a_mark_keeps_its_kind_across_the_wire() {
        let feed = RailMark::feed("tmdb", "movie:top:Popular");
        assert_eq!(feed.as_wire(), "movie:top:Popular");
        assert_eq!(RailMark::from_wire(&feed.as_wire(), "tmdb"), feed);

        let collection = RailMark::collection("tmdb:top-rated:movie");
        assert_eq!(collection.as_wire(), "collection:tmdb:top-rated:movie");
        assert_eq!(RailMark::from_wire(&collection.as_wire(), ""), collection);
    }

    #[test]
    fn an_item_without_a_provider_stays_in_the_canonical_space() {
        let title = Title::of_item(&item("not-an-imdb-id", "", ""));
        assert_eq!(title.key.id.space, IdSpace::Imdb, "with no provider to name a space, the canonical one is the only claim left");
        assert!(!title.available(), "and nothing here plays it");
    }

    #[test]
    fn the_catalogue_that_listed_it_is_a_rail_mark() {
        let mut listed = item("tt1", "stremio", "cinemeta");
        listed.catalog = Some("movie:top:Popular".into());
        listed.top = true;
        let title = Title::of_item(&listed);
        assert_eq!(title.rails, [RailMark::feed("cinemeta", "movie:top:Popular")]);
        assert!(title.top);
    }

    #[test]
    fn the_watch_a_provider_reports_is_a_title_fact() {
        let mut watched = item("tt1", "stremio", "");
        watched.last_watched = Some("2026-11-12T20:00:00Z".into());
        watched.minutes_played = 42.5;
        let title = Title::of_item(&watched);
        assert_eq!(title.watched.as_ref().and_then(|seen| seen.at), Some(at("2026-11-12T20:00:00Z")));
        assert_eq!(title.watched.as_ref().map(|seen| seen.minutes), Some(42.5));

        assert!(Title::of_item(&item("tt2", "stremio", "")).watched.is_none(), "nothing watched, nothing to carry");
    }
}
