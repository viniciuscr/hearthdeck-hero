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
    /// Stremio's last watch, for the “Continue watching” rail.
    #[serde(default)]
    pub last_watched: Option<String>,
    /// Stremio's watched minutes (`timeWatched`).
    #[serde(default)]
    pub minutes_played: f64,
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

/// Local, non-secret progress. Stremio remains the cross-device source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MovieProgress {
    pub id: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub video_id: Option<String>,
}
