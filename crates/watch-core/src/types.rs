//! The feature's own data types. Wire types stay in the module that speaks
//! their protocol; these are what the UI sees.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PosterShape {
    #[default]
    Poster,
    Landscape,
    Square,
}

/// One mark of a record, as the projection writes it: the wire (`catalog`) and
/// the source that listed the title (empty for a collection's mark, which names
/// itself in the wire).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireMark {
    #[serde(default)]
    pub source: String,
    pub catalog: String,
}

/// A catalog or meta item, normalized from Cinemeta or an installed addon.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MovieItem {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// How long it runs, in the source's own words (`136 min`).
    #[serde(default)]
    pub runtime: Option<String>,
    /// The day it came out (`1999-03-31`), when the source says: what "New"
    /// measures in a catalogue that has no added date.
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    /// Cinemeta's Top catalog membership, for the “Top movies” rail.
    #[serde(default)]
    pub top: bool,
    /// Fetched from a catalog page (the Movies and Series tabs), not from a
    /// rail: a rails refresh keeps these.
    #[serde(default)]
    pub catalog: Option<String>,
    /// **Every** mark the title carried, each with the source that made it. A
    /// title can be listed by a catalogue page and kept by a film page's rails;
    /// a record with one mark lost the other, and the next keep deleted the
    /// catalogue's copy. `catalog` stays as the first of these, for a reader that
    /// knows only the one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub catalogs: Vec<WireMark>,
    /// Stremio's last watch, for the “Continue watching” rail.
    #[serde(default)]
    pub last_watched: Option<String>,
    /// Stremio's watched minutes (`timeWatched`). It is 0 for a title that was
    /// only started, so it is not what says whether anything was watched.
    #[serde(default)]
    pub minutes_played: f64,
    /// Where the provider left off and how long the title is (`timeOffset`,
    /// `duration`): the two fields that say "in progress" when no minute was
    /// counted.
    #[serde(default)]
    pub position_ms: u64,
    #[serde(default)]
    pub duration_ms: u64,
    /// The provider says it was finished (`flaggedWatched`).
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub provider: String,
    /// The addon that supplied it, for provenance and error messages.
    #[serde(default)]
    pub addon: String,
}

/// One stream from an addon, normalized. Exactly one of `url` and `info_hash`
/// is set for a playable entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamChoice {
    pub addon: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub info_hash: Option<String>,
    #[serde(default)]
    pub file_idx: Option<u16>,
    #[serde(default)]
    pub behavior_hints: Value,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub seeders: Option<u32>,
}

impl StreamChoice {
    /// Whether there is something to hand to the player.
    pub fn is_playable(&self) -> bool {
        self.url.is_some() || self.info_hash.is_some()
    }
}

/// Who wrote a fact: this machine, or a provider it was pulled from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// This machine wrote it (a playback, a mark).
    #[default]
    Local,
    /// A pull wrote it from a provider.
    Imported,
}

/// `true` is the interesting value; a false one is left out of the file.
fn is_false(value: &bool) -> bool {
    !*value
}

/// Local, non-secret progress. Stremio remains the cross-device source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MovieProgress {
    /// The IMDb id the title is known by elsewhere (`tt0944947`), saved with the
    /// record so it can still be matched after a provider changes its own ids.
    /// Empty for a record written before it was kept.
    #[serde(default)]
    pub imdb: Option<String>,
    /// A series: one record per episode watched, by its episode id. The title's
    /// own position is only where playback resumes, so this is the watched
    /// state; a film has none and uses the title's `finished`.
    #[serde(default)]
    pub episodes: std::collections::BTreeMap<String, EpisodeProgress>,
    /// The provider that owns the title. Records written before providers were
    /// recorded have an empty value and belong to Stremio.
    #[serde(default)]
    pub provider: String,
    pub id: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub video_id: Option<String>,
    /// Who last wrote this record: `local` for anything this machine wrote,
    /// `imported` for a pull. Informational — playing never reads it.
    #[serde(default)]
    pub origin: Origin,
    /// The provider a fact came from, when imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<DateTime<Utc>>,
    /// The provider's own stamp for the fact — never our clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_updated_at: Option<DateTime<Utc>>,
    /// Marked "not watched" by hand here: an import must never re-finish it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unwatched: bool,
    /// Fields a newer build wrote; kept so a downgrade cannot drop them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for MovieProgress {
    fn default() -> Self {
        Self {
            imdb: None,
            episodes: Default::default(),
            provider: String::new(),
            id: String::new(),
            position_ms: 0,
            duration_ms: 0,
            updated_at: DateTime::<Utc>::MIN_UTC,
            finished: false,
            video_id: None,
            origin: Origin::Local,
            imported_from: None,
            imported_at: None,
            remote_updated_at: None,
            unwatched: false,
            extra: Map::new(),
        }
    }
}

/// One episode's state: where it stands and whether it was finished. Kept per
/// episode so finishing one does not erase another — and so moving to the next
/// episode does not un-watch the one before it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeProgress {
    #[serde(default)]
    pub position_ms: u64,
    #[serde(default)]
    pub duration_ms: u64,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub finished: bool,
    /// The season and number the page knew when it played: what a tracker asks
    /// for, without parsing the provider's own `video_id` shape. `None` for a
    /// record written before they were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    /// Who last wrote this episode: `local` or `imported` (see `MovieProgress`).
    #[serde(default)]
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_updated_at: Option<DateTime<Utc>>,
    /// Marked "not watched" by hand: an import must never re-finish it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unwatched: bool,
    /// Fields a newer build wrote; kept so a downgrade cannot drop them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for EpisodeProgress {
    fn default() -> Self {
        Self {
            position_ms: 0,
            duration_ms: 0,
            updated_at: DateTime::<Utc>::MIN_UTC,
            finished: false,
            season: None,
            number: None,
            origin: Origin::Local,
            imported_from: None,
            imported_at: None,
            remote_updated_at: None,
            unwatched: false,
            extra: Map::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_written_before_the_new_fields_still_reads() {
        let old = r#"{"provider":"stremio","id":"tt1","position_ms":10,"duration_ms":20,
                      "updated_at":"2026-01-01T00:00:00Z","finished":false}"#;
        let record: MovieProgress = serde_json::from_str(old).expect("an old file still reads");
        assert_eq!(record.origin, Origin::Local);
        assert!(!record.unwatched);
        assert!(record.imported_from.is_none() && record.imported_at.is_none());
        assert!(record.extra.is_empty());
    }

    #[test]
    fn a_field_from_a_newer_build_survives_a_write() {
        let future = r#"{"provider":"stremio","id":"tt1","position_ms":0,"duration_ms":0,
                         "updated_at":"2026-01-01T00:00:00Z","finished":false,"whatever":7}"#;
        let record: MovieProgress = serde_json::from_str(future).expect("reads");
        assert_eq!(record.extra.get("whatever"), Some(&serde_json::json!(7)));
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains("\"whatever\":7"), "{text}");
    }

    #[test]
    fn an_imported_episode_says_so_in_camel_case() {
        let mut episode = EpisodeProgress { finished: true, ..EpisodeProgress::default() };
        episode.origin = Origin::Imported;
        episode.imported_from = Some("simkl".into());
        episode.remote_updated_at = Some(DateTime::<Utc>::UNIX_EPOCH);
        let text = serde_json::to_string(&episode).unwrap();
        assert!(text.contains("\"origin\":\"imported\""), "{text}");
        assert!(text.contains("\"importedFrom\":\"simkl\""), "{text}");
        assert!(text.contains("\"remoteUpdatedAt\""), "{text}");
        assert!(!text.contains("\"unwatched\""), "a false marker is left out: {text}");
    }

    #[test]
    fn an_unwatched_mark_is_written() {
        let record = MovieProgress { unwatched: true, ..Default::default() };
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains("\"unwatched\":true"), "{text}");
    }
}
