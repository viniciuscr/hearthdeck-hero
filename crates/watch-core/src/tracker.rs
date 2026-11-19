//! The port a **tracker** implements: a service that keeps a person's watch
//! history in step (Simkl today).
//!
//! A tracker is not a media provider: it plays nothing, it owns no titles, and it
//! is never on the way of a playback. The facade queues what happened in the
//! local outbox and hands it one event at a time, whenever the network is there.
//! That is what makes the local record the truth and the tracker a follower of it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Result;

/// What an event is about. The IMDb id is the identity Watch resolves titles by
/// (and the one every tracker here accepts); the name and year are the fallback
/// for a title whose id a tracker cannot match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedTitle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imdb: Option<String>,
    /// `movie` or `series`.
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
}

/// Which part of a title the event is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "view")]
pub enum TrackedView {
    Movie,
    Episode { season: u32, number: u32 },
}

/// What happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "watch")]
pub enum TrackedWatch {
    /// Finished: the tracker's history gets a watch event, stamped with
    /// [`TrackEvent::at`].
    Watched,
    /// Started, not finished: only a tracker with a place for a resume position
    /// (Simkl's paused playback) has anything to do with it.
    InProgress { position_ms: u64, duration_ms: u64 },
    /// The person un-marked it: whatever the tracker recorded goes away.
    Unwatched,
}

/// One thing that happened, as the outbox keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackEvent {
    /// Assigned by the outbox; used to tell one attempt from the next.
    pub seq: u64,
    /// When it happened (the session's end, the click), which is the timestamp a
    /// tracker records with the event — not when it was delivered.
    pub at: DateTime<Utc>,
    pub title: TrackedTitle,
    pub view: TrackedView,
    pub watch: TrackedWatch,
}

impl TrackedTitle {
    /// Whether this names the same title as `imdb`/`title` — the identity rule
    /// [`TrackEvent::same_target`] uses, exposed for callers holding only a
    /// record (the pushed log, an import).
    pub fn is(&self, imdb: Option<&str>, title: &str) -> bool {
        match (&self.imdb, imdb) {
            (Some(one), Some(two)) => one == two,
            _ => self.title == title,
        }
    }
}

impl TrackEvent {
    /// Whether `other` describes the same thing: the same title, the same episode
    /// (or the same film). The outbox folds those into the newest, so a
    /// half-watched event never survives a later finished one.
    ///
    /// The identity is the IMDb id when both have one, and the name otherwise —
    /// a title without an id is still one title.
    pub fn same_target(&self, other: &Self) -> bool {
        if self.title.kind != other.title.kind || self.view != other.view {
            return false;
        }
        match (&self.title.imdb, &other.title.imdb) {
            (Some(one), Some(two)) => one == two,
            _ => self.title.title == other.title.title,
        }
    }
}

/// One list membership change to deliver: a title entering or leaving a list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChange {
    pub title: TrackedTitle,
    /// The list kind: `watchlist`, `favourites`.
    pub list: String,
    /// `true` to add, `false` to remove.
    pub member: bool,
}

/// How one delivery went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The tracker has it.
    Delivered,
    /// The tracker does not know this title, or refuses this kind of write:
    /// asking again cannot help. The message is shown, never retried.
    Unmatched(String),
    /// Temporary (a network fault, a 429, a 5xx): the outbox keeps it and asks
    /// again later. `after` is the wait the provider named (`Retry-After`), when
    /// it named one: a daily quota resets on its own clock, not on our ladder.
    Retry { reason: String, after: Option<chrono::Duration> },
}

/// A service that keeps a person's watch history in step.
pub trait WatchTracker: Send + Sync {
    /// The value the outbox and the credentials name it by (`simkl`).
    fn id(&self) -> &'static str;
    /// How an error or a log line calls it.
    fn name(&self) -> &'static str;
    /// Delivers one event. Blocking: the facade calls it off the UI thread.
    fn deliver(&self, event: &TrackEvent) -> Result<Delivery>;

    /// Delivers one list membership change. A tracker that keeps no lists — or
    /// not that kind of list — answers [`Delivery::Unmatched`] instead of
    /// pretending, so the caller can say so once and move on.
    fn sync_list(&self, change: &ListChange) -> Result<Delivery> {
        let _ = change;
        Ok(Delivery::Unmatched("this tracker keeps no lists".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(imdb: Option<&str>, name: &str, view: TrackedView, watch: TrackedWatch) -> TrackEvent {
        TrackEvent {
            seq: 1,
            at: Utc::now(),
            title: TrackedTitle { imdb: imdb.map(str::to_owned), kind: "series".into(), title: name.into(), year: Some("2011".into()) },
            view,
            watch,
        }
    }

    #[test]
    fn the_same_target_is_the_same_title_and_episode() {
        let first = event(Some("tt1"), "A Show", TrackedView::Episode { season: 1, number: 2 }, TrackedWatch::Watched);
        let same = event(
            Some("tt1"),
            "Renamed",
            TrackedView::Episode { season: 1, number: 2 },
            TrackedWatch::InProgress { position_ms: 1, duration_ms: 2 },
        );
        assert!(first.same_target(&same), "the id identifies the title, the name does not");
        assert!(!first.same_target(&event(Some("tt1"), "A Show", TrackedView::Episode { season: 1, number: 3 }, TrackedWatch::Watched)));
        assert!(!first.same_target(&event(Some("tt2"), "A Show", TrackedView::Episode { season: 1, number: 2 }, TrackedWatch::Watched)));
    }

    #[test]
    fn a_title_without_an_id_is_identified_by_its_name() {
        let first = event(None, "A Film", TrackedView::Movie, TrackedWatch::Watched);
        assert!(first.same_target(&event(None, "A Film", TrackedView::Movie, TrackedWatch::Unwatched)));
        assert!(!first.same_target(&event(None, "Another Film", TrackedView::Movie, TrackedWatch::Watched)));
        // When only one side carries an id the name decides: the same library
        // record either has one or not, so folding by name loses nothing.
        assert!(first.same_target(&event(Some("tt1"), "A Film", TrackedView::Movie, TrackedWatch::Watched)));
    }
}
