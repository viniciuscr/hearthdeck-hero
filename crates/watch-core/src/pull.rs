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

/// One list membership a provider reports.
///
/// A pull reports what **is** in the list; a removal is the absence of a title
/// that was there, which only the caller's last-seen set can tell apart from a
/// title that never was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMembership {
    pub title: TrackedTitle,
    /// The list kind: `watchlist`, `favourites`.
    pub list: String,
    /// Whether it is in that list now.
    pub member: bool,
}

/// What a pull's list membership is worth.
///
/// The type exists so the dangerous reading cannot be typed by accident: a pull
/// that hears about some rows says [`ListAnswer::Delta`] — the default — and a
/// caller may only read a missing title as "removed there" from
/// [`ListAnswer::Whole`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListAnswer {
    /// Some of what a list holds: the rows this pull heard about, and nothing
    /// said about the rest.
    #[default]
    Delta,
    /// The whole contents of each list: a title missing from it is not there.
    Whole,
}

/// What one pull brought.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pulled {
    pub watches: Vec<RemoteWatch>,
    pub positions: Vec<RemotePosition>,
    #[doc = "List membership: what the provider says is in each list today."]
    pub memberships: Vec<RemoteMembership>,
    /// What `memberships` is worth: the whole of each list, or only the rows this
    /// pull heard about.
    ///
    /// A provider asked for "what changed since" answers with a delta: a title
    /// missing from it says nothing about the list, so only [`ListAnswer::Whole`]
    /// lets the caller read an absent title as removed there — the rule
    /// `docs/WATCH-SYNC-RULES.md` calls "absence never removes by itself".
    pub lists: ListAnswer,
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
    /// Pulls the axes that moved; `since` is `None` on the first pull and `now`
    /// is the snapshot this run took, so a provider can tell whether a whole list
    /// has to be fetched (a list that moved) or a delta will do.
    fn pull(&self, ctx: &Ctx<'_>, since: Option<&Value>, now: &Value) -> Result<Pulled>;
}
