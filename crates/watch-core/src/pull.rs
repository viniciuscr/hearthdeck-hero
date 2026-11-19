//! The **import** side of a provider: pulling its history and resume positions
//! into our words.
//!
//! The core never sees a provider's vocabulary (§2.5 of
//! `docs/WATCH-IMPORT.md`): it asks for a change query, an opaque snapshot and
//! three neutral lists. Simkl's `activities`, Trakt's `last_activities` and
//! Stremio's `_mtime` are implementations of that, not part of the contract.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::provider::Ctx;
use crate::tracker::{TrackedTitle, TrackedView};
use crate::Result;

/// One watched fact a provider reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteWatch {
    pub title: TrackedTitle,
    pub view: TrackedView,
    /// The provider's own stamp, when it has one (Simkl's `last_watched_at`).
    pub at: Option<DateTime<Utc>>,
}

/// One resume position a provider reports.
#[derive(Clone, Debug, PartialEq)]
pub struct RemotePosition {
    pub title: TrackedTitle,
    pub view: TrackedView,
    /// 0–100 as the provider reports it; it may be rounded, so it is a hint.
    pub percent: f64,
    pub at: Option<DateTime<Utc>>,
}

/// What one pull brought.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pulled {
    pub watches: Vec<RemoteWatch>,
    pub positions: Vec<RemotePosition>,
    /// Items the provider had that cannot be used here (no IMDb id, a
    /// show-level fact): counted, so a pull can say what it left behind.
    pub skipped: usize,
}

/// A provider that can be pulled from.
pub trait WatchSync: Send + Sync {
    /// The id the cursor and the settings name it by (`simkl`).
    fn id(&self) -> &'static str;
    /// How a screen or a log calls it.
    fn name(&self) -> &'static str;
    /// The provider's change query, kept opaque.
    fn snapshot(&self, ctx: &Ctx<'_>) -> Result<Value>;
    /// Whether `now` says nothing moved since `since` — the cheap path.
    fn unchanged(&self, since: &Value, now: &Value) -> bool;
    /// Pulls the axes that moved; `since` is `None` on the first pull.
    fn pull(&self, ctx: &Ctx<'_>, since: Option<&Value>) -> Result<Pulled>;
}
