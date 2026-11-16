//! The feature's own data types. Wire types stay in the module that speaks
//! their protocol; these are what the UI sees.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PosterShape {
    #[default]
    Poster,
    Landscape,
    Square,
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
    #[serde(default)]
    pub runtime: Option<Value>,
    #[serde(default)]
    pub genres: Vec<String>,
    /// Cinemeta's Top catalog membership, for the “Top movies” rail.
    #[serde(default)]
    pub top: bool,
    /// Fetched from a catalog page (the Movies and Series tabs), not from a
    /// rail: a rails refresh keeps these.
    #[serde(default)]
    pub catalog: Option<String>,
    /// Stremio's last watch, for the “Continue watching” rail.
    #[serde(default)]
    pub last_watched: Option<String>,
    /// Stremio's watched minutes (`timeWatched`).
    #[serde(default)]
    pub minutes_played: f64,
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
}
