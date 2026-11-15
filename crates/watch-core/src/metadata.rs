//! What *describes* a title: the metadata-source contract.
//!
//! Two different things are named differently on purpose. A **media provider**
//! ([`WatchProvider`](crate::provider::WatchProvider): Stremio, YouTube) says
//! where a title comes from and how to play it; a **metadata source**
//! ([`WatchMetadataSource`]: Cinemeta, later a TMDB addon) says what the title
//! is. A source is never given an account, a stream or a player, and a media
//! provider is never asked to describe a title. They have their own registries
//! and, later, their own settings.

use serde::{Deserialize, Serialize};

use crate::http::Transport;
use crate::Result;

/// Which title a source is asked about, and by which media provider it is
/// known. It is also the cache key: a title's details belong to the provider
/// that owns it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleRef {
    /// The media provider that owns the title (`stremio`).
    pub provider: String,
    /// That provider's own id (`tt0133093`).
    pub id: String,
    /// `movie` or `series`.
    pub kind: String,
    /// The title as the library shows it, for a source that searches by name.
    #[serde(default)]
    pub title: String,
    /// The catalog year (`1999`, `2011–2019`), for the same.
    #[serde(default)]
    pub year: Option<String>,
}

impl TitleRef {
    pub fn new(provider: &str, id: &str, kind: &str) -> Self {
        Self { provider: provider.to_owned(), id: id.to_owned(), kind: kind.to_owned(), title: String::new(), year: None }
    }

    /// The same reference with the library's title and year, for a source that
    /// has to search.
    pub fn named(mut self, title: &str, year: Option<&str>) -> Self {
        self.title = title.to_owned();
        self.year = year.map(str::to_owned);
        self
    }

    /// Whether the reference can be asked for at all.
    pub fn is_complete(&self) -> bool {
        !self.provider.is_empty() && !self.id.is_empty() && !self.kind.is_empty()
    }
}

/// What a metadata source is, for the registry and (later) the settings rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetaDescriptor {
    /// Stable id (`cinemeta`); also the value a settings file will store.
    pub id: &'static str,
    /// How a person-facing line names it.
    pub name: &'static str,
    /// The media providers it can be asked about; empty means any.
    pub applies_to: &'static [&'static str],
}

impl MetaDescriptor {
    /// Whether this source can answer for a title of `provider`.
    pub fn covers(&self, provider: &str) -> bool {
        self.applies_to.is_empty() || self.applies_to.contains(&provider)
    }
}

/// What a source may use besides the title: the transport, and nothing else.
pub struct MetaCtx<'a> {
    pub http: &'a dyn Transport,
}

impl<'a> MetaCtx<'a> {
    pub fn new(http: &'a dyn Transport) -> Self {
        Self { http }
    }
}

/// What describes a title.
///
/// A source says what a title is; it never browses, streams or plays. The
/// registry (`heronite-watch-metadata`) asks its sources in order and the
/// facade (`heronite-watch`) caches the answer.
pub trait WatchMetadataSource: Send + Sync {
    fn descriptor(&self) -> &MetaDescriptor;

    /// What the source knows about `title`: `None` when it does not serve the
    /// title (a 404), an error when it could not be asked at all.
    fn details(&self, ctx: &MetaCtx<'_>, title: &TitleRef) -> Result<Option<TitleDetails>>;
}

/// Everything a source knows about one title, as the screens show it.
///
/// A source fills what it has; an absent field is not the same as an empty one
/// and the page leaves it out. The protocol's wire shapes stay in the crate
/// that speaks them (`heronite-watch-addon`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleDetails {
    /// The source that answered (`cinemeta`); the registry fills it.
    #[serde(default)]
    pub source: String,
    pub id: String,
    pub kind: String,
    pub name: String,
    /// `1999`, or a series' `2011–2019`.
    #[serde(default)]
    pub year: Option<String>,
    /// The first release (`1999-03-31`), as the source prints it.
    #[serde(default)]
    pub released: Option<String>,
    /// `136 min`.
    #[serde(default)]
    pub runtime: Option<String>,
    /// `8.7`, as the source prints it.
    #[serde(default)]
    pub rating: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub directors: Vec<String>,
    #[serde(default)]
    pub writers: Vec<String>,
    #[serde(default)]
    pub cast: Vec<String>,
    /// The countries of production, split from the source's list.
    #[serde(default)]
    pub countries: Vec<String>,
    #[serde(default)]
    pub awards: Option<String>,
    /// Series only: `Ended`, `Continuing`.
    #[serde(default)]
    pub series_status: Option<String>,
    /// Series only: every episode the source lists, ordered by season and
    /// number. Season 0 (the specials) is not here.
    #[serde(default)]
    pub episodes: Vec<Episode>,
    /// Art the source knows, when the library record lacks it.
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    /// Display-only facts a source knows and the page shows as they come: a
    /// tagline, a budget, a network, where it can be watched. A source that knows
    /// more adds more rows without the page changing.
    #[serde(default)]
    pub extras: Vec<Fact>,
}

/// One display-only fact: a label and its value, and the source that gave it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    /// Stable id (`tagline`, `budget`, `network`): the first source to give one
    /// keeps it, and the screens may order or localize the ids they know.
    pub id: String,
    /// How the row is named when the screens do not know the id.
    pub label: String,
    pub value: String,
    /// The source's id (`tmdb`), for provenance.
    #[serde(default)]
    pub source: String,
}

/// One episode of a series: what the picker row and the stream request need.
///
/// `id` is the addon's own video id (`tt0944947:1:1`): it is what the `stream`
/// resource is asked for and what progress stores as `video_id`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub season: u32,
    pub number: u32,
    /// The episode's name; empty when the source has none.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub overview: Option<String>,
    /// A 16:9 still, when the source has one.
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// The air date (`2011-04-18`).
    #[serde(default)]
    pub released: Option<String>,
    /// `8.2`, as the source prints it; `0` when it has none.
    #[serde(default)]
    pub rating: Option<String>,
}

impl Episode {
    /// `S1E2`, the label a picker row and the player's title use.
    pub fn code(&self) -> String {
        format!("S{}E{}", self.season, self.number)
    }
}

impl TitleDetails {
    /// Fills what this answer has not got from `other`: the first source to fill
    /// a field keeps it, a later one fills only what is still empty, and extras
    /// are joined with the first of each id winning. `source` stays the first
    /// source that answered — the extras carry their own.
    pub fn merge(&mut self, other: TitleDetails) {
        fn fill(mine: &mut Option<String>, theirs: Option<String>) {
            let blank = mine.as_deref().is_none_or(|value| value.trim().is_empty());
            if blank {
                *mine = theirs;
            }
        }
        fill(&mut self.year, other.year);
        fill(&mut self.released, other.released);
        fill(&mut self.runtime, other.runtime);
        fill(&mut self.rating, other.rating);
        fill(&mut self.description, other.description);
        fill(&mut self.awards, other.awards);
        fill(&mut self.series_status, other.series_status);
        fill(&mut self.poster, other.poster);
        fill(&mut self.background, other.background);
        fill(&mut self.logo, other.logo);
        for (mine, theirs) in [
            (&mut self.genres, other.genres),
            (&mut self.directors, other.directors),
            (&mut self.writers, other.writers),
            (&mut self.cast, other.cast),
            (&mut self.countries, other.countries),
        ] {
            if mine.is_empty() {
                *mine = theirs;
            }
        }
        // One source's episode list is what the picker pages through, not two
        // half-lists stitched together.
        if self.episodes.is_empty() {
            self.episodes = other.episodes;
        }
        for fact in other.extras {
            if !self.extras.iter().any(|kept| kept.id == fact.id) {
                self.extras.push(fact);
            }
        }
    }

    /// The source's description, or the library's when it has none.
    pub fn description_or<'a>(&'a self, fallback: Option<&'a str>) -> Option<&'a str> {
        self.description.as_deref().filter(|d| !d.trim().is_empty()).or(fallback)
    }

    /// The season numbers the source lists, ascending.
    pub fn seasons(&self) -> Vec<u32> {
        let mut seasons: Vec<u32> = self.episodes.iter().map(|episode| episode.season).collect();
        seasons.sort_unstable();
        seasons.dedup();
        seasons
    }

    /// How many seasons and episodes the source lists.
    pub fn season_count(&self) -> usize {
        self.seasons().len()
    }

    pub fn episode_count(&self) -> usize {
        self.episodes.len()
    }

    /// The episodes of one season, ordered.
    pub fn episodes_of(&self, season: u32) -> impl Iterator<Item = &Episode> {
        self.episodes.iter().filter(move |episode| episode.season == season)
    }

    /// The episode with this video id, when the source lists it.
    pub fn episode_with_id(&self, id: &str) -> Option<&Episode> {
        self.episodes.iter().find(|episode| episode.id == id)
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn fact(id: &str, value: &str, source: &str) -> Fact {
        Fact { id: id.into(), label: id.into(), value: value.into(), source: source.into() }
    }

    #[test]
    fn a_merge_keeps_the_first_answer_and_fills_the_gaps() {
        let mut first = TitleDetails {
            source: "tmdb".into(),
            id: "tt0133093".into(),
            kind: "movie".into(),
            name: "The Matrix".into(),
            runtime: Some("136 min".into()),
            cast: vec!["Keanu Reeves".into()],
            extras: vec![fact("tagline", "Free your mind", "tmdb")],
            ..TitleDetails::default()
        };
        let second = TitleDetails {
            source: "cinemeta".into(),
            id: "tt0133093".into(),
            kind: "movie".into(),
            name: "The Matrix".into(),
            runtime: Some("2h 16m".into()),
            rating: Some("8.7".into()),
            directors: vec!["Lana Wachowski".into(), "Lilly Wachowski".into()],
            extras: vec![fact("tagline", "Welcome to the real world", "cinemeta"), fact("awards", "4 Oscars", "cinemeta")],
            ..TitleDetails::default()
        };

        first.merge(second);

        assert_eq!(first.source, "tmdb", "the record keeps where it came from");
        assert_eq!(first.runtime.as_deref(), Some("136 min"), "the first answer keeps the field");
        assert_eq!(first.rating.as_deref(), Some("8.7"), "and the second fills what was empty");
        assert_eq!(first.directors.len(), 2, "a list the first source lacked is taken whole");
        assert_eq!(first.cast.len(), 1, "a list the first source gave is not replaced");
        let extras: Vec<(&str, &str)> = first.extras.iter().map(|fact| (fact.id.as_str(), fact.source.as_str())).collect();
        assert_eq!(extras, [("tagline", "tmdb"), ("awards", "cinemeta")], "extras are joined, the first of each id winning");
    }

    #[test]
    fn a_blank_field_counts_as_empty() {
        let mut first = TitleDetails { runtime: Some("  ".into()), ..TitleDetails::default() };
        first.merge(TitleDetails { runtime: Some("136 min".into()), ..TitleDetails::default() });
        assert_eq!(first.runtime.as_deref(), Some("136 min"));
    }
}
