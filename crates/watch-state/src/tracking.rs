//! The tracker outbox (`watch-tracking.json`): what happened here and still has
//! to reach a tracker.
//!
//! The local record (`movies.json`, `watch-sessions.json`) is written first and
//! never waits for a network; this file is the follower. An event is queued once,
//! delivered to every enabled tracker, retried on a short schedule when the fault
//! was temporary, and dropped when there is nothing left to do. A title a tracker
//! cannot match is **done** for it — retrying cannot change the answer — and its
//! name is kept in `notices` for the settings card to show.
//!
//! Folding is deliberate: the same title and episode happening again replaces the
//! event still waiting, so a half-watched entry never survives a later finished
//! one for the same thing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use heronite_data::json_store;
use heronite_data::paths::AppPaths;
use heronite_watch_core::tracker::{TrackEvent, TrackedTitle, TrackedView};

/// The file, under the app's config folder, like `movies.json`.
pub const FILE: &str = "watch-tracking.json";

/// Events kept before the oldest are dropped: a tracker that is off for months
/// must not grow the file without a bound.
pub const MAX_EVENTS: usize = 500;

/// Refusals remembered for the settings card, newest first.
pub const MAX_NOTICES: usize = 8;

/// Deliveries remembered for echo suppression, and for how long. Bounded because
/// it is a hint, not a record: an old delivery no longer needs suppressing.
pub const MAX_PUSHED: usize = 64;
pub const PUSHED_FOR: chrono::Duration = chrono::Duration::hours(24);

/// How long a queued event blocks an import for its title: the sum of the retry
/// ladder, after which the outbox is out of cheap options and the pull may run.
pub const PENDING_HORIZON: chrono::Duration = chrono::Duration::minutes(36);

/// How long to wait before asking again, in minutes, by attempt. After the list
/// runs out the wait stays at its last value; the next app start drains anyway.
pub const RETRY_MINUTES: [u64; 3] = [1, 5, 30];

/// The file's shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingFile {
    /// The next event number; events are numbered once and never reused.
    #[serde(default)]
    pub next_seq: u64,
    #[serde(default)]
    pub events: Vec<QueuedEvent>,
    /// Titles a tracker refused for good, newest first: a person reads these as
    /// "could not be matched", not as a fault.
    #[serde(default)]
    pub notices: Vec<String>,
    /// What was delivered recently: an import ignores a provider fact about one of
    /// these, because it is our own write coming back. Pruned by `prune`, unlike
    /// `events`.
    #[serde(default)]
    pub pushed: Vec<PushedRecord>,
    /// When each tracker was last backfilled: switching one on sends what happened
    /// since then, never the whole history again.
    #[serde(default)]
    pub backfilled: BTreeMap<String, DateTime<Utc>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One event and what every tracker said about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedEvent {
    pub event: TrackEvent,
    #[serde(default)]
    pub trackers: BTreeMap<String, TrackerState>,
    /// When it was queued (not when it happened): the clock the pending horizon
    /// runs on. Absent in a file written before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_at: Option<DateTime<Utc>>,
}

impl QueuedEvent {
    /// When it entered the outbox, falling back to the event's own time.
    pub fn since(&self) -> DateTime<Utc> {
        self.queued_at.unwrap_or(self.event.at)
    }
}

/// One delivery, kept so a pull can recognise its own echo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushedRecord {
    pub delivered_at: DateTime<Utc>,
    pub title: heronite_watch_core::tracker::TrackedTitle,
    pub view: heronite_watch_core::tracker::TrackedView,
}

/// A tracker's standing on one event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerState {
    /// Delivered, or refused for good: no more attempts.
    #[serde(default)]
    pub done: bool,
    /// The last attempt's failure, when it has not succeeded since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

/// The last temporary (or permanent) failure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    #[serde(default)]
    pub attempts: u32,
    /// When to ask again. Ignored for a permanent failure.
    pub next_at: DateTime<Utc>,
    pub error: String,
    #[serde(default)]
    pub permanent: bool,
}

pub fn path(app: &AppPaths) -> PathBuf {
    app.config.join(FILE)
}

/// The outbox as it stands; a corrupt file is no outbox, not a failure.
pub fn read(app: &AppPaths) -> TrackingFile {
    let path = path(app);
    json_store::read(&path).unwrap_or_else(|error| {
        log::warn!("{}: {error:#}", path.display());
        TrackingFile::default()
    })
}

pub fn write(app: &AppPaths, file: &TrackingFile) -> Result<()> {
    let path = path(app);
    json_store::write(&path, file).with_context(|| format!("writing {}", path.display()))
}

/// Read-modify-write of the outbox, serialized per path by `json_store`: a mark
/// queued while a drain is applying its results must not be overwritten by it.
pub fn update<R>(app: &AppPaths, f: impl FnOnce(&mut TrackingFile) -> Result<R>) -> Result<R> {
    let path = path(app);
    json_store::update(&path, f).with_context(|| format!("updating {}", path.display()))
}

/// Queues `event` and saves the file. The same title and episode still waiting
/// are replaced, not duplicated: what is queued is the newest state of a thing.
pub fn enqueue(app: &AppPaths, event: TrackEvent) -> Result<TrackEvent> {
    update(app, |file| Ok(queue(file, event)))
}

/// Queues `event` in memory: its sequence number, folded into the newest state.
/// Split from [`enqueue`] so a caller can queue a batch and save once.
pub fn queue(file: &mut TrackingFile, mut event: TrackEvent) -> TrackEvent {
    file.next_seq = file.next_seq.max(1);
    event.seq = file.next_seq;
    file.next_seq += 1;
    // Only an event nobody has consumed yet can be replaced: once a tracker has
    // it, the pair are two things that happened.
    let waiting =
        file.events.iter().position(|queued| queued.event.same_target(&event) && queued.trackers.values().all(|state| !state.done));
    let fresh = QueuedEvent { event: event.clone(), trackers: BTreeMap::new(), queued_at: Some(Utc::now()) };
    match waiting {
        Some(index) => file.events[index] = fresh,
        None => file.events.push(fresh),
    }
    trim(file);
    event
}

/// Whether `tracker` should be asked about `queued` now.
pub fn due(queued: &QueuedEvent, tracker: &str, now: DateTime<Utc>) -> bool {
    match queued.trackers.get(tracker) {
        Some(state) if state.done => false,
        Some(state) => state.failure.as_ref().is_none_or(|failure| failure.permanent || failure.next_at <= now),
        None => true,
    }
}

/// Marks an event as already had by `tracker`, without it ever being sent: what a
/// backfill does for the trackers that were on before, so switching a second one
/// on does not re-send the whole history to the first.
pub fn done_for(queued: &mut QueuedEvent, tracker: &str) {
    queued.trackers.insert(tracker.to_owned(), TrackerState { done: true, failure: None });
}

/// The tracker has it.
pub fn delivered(queued: &mut QueuedEvent, tracker: &str) {
    done_for(queued, tracker);
}

/// A temporary fault: kept, with the next attempt's time.
pub fn retry(queued: &mut QueuedEvent, tracker: &str, error: &str, after: Option<Duration>, now: DateTime<Utc>) {
    let attempts = queued.trackers.get(tracker).and_then(|state| state.failure.as_ref()).map_or(1, |failure| failure.attempts + 1);
    // The provider's own wait wins: a daily quota does not reset on our ladder.
    let wait = after.unwrap_or_else(|| retry_delay(attempts));
    // A hostile or broken `Retry-After` must not overflow the clock: falling back
    // to the ladder is better than panicking in the drain.
    let next_at = now.checked_add_signed(wait).unwrap_or_else(|| now + retry_delay(attempts));
    let failure = Failure { attempts, next_at, error: error.to_owned(), permanent: false };
    queued.trackers.insert(tracker.to_owned(), TrackerState { done: false, failure: Some(failure) });
}

/// The tracker refuses it for good: done, and worth saying out loud.
pub fn unmatched(file: &mut TrackingFile, seq: u64, tracker: &str, reason: &str, now: DateTime<Utc>) {
    let Some(queued) = file.events.iter_mut().find(|queued| queued.event.seq == seq) else { return };
    let failure = Failure { attempts: 1, next_at: now, error: reason.to_owned(), permanent: true };
    queued.trackers.insert(tracker.to_owned(), TrackerState { done: true, failure: Some(failure) });
    let notice = format!("{}: {reason}", queued.event.title.title);
    if !file.notices.iter().any(|known| known == &notice) {
        file.notices.insert(0, notice);
        file.notices.truncate(MAX_NOTICES);
    }
}

/// How long to wait before attempt `attempts` (1 is the first retry).
pub fn retry_delay(attempts: u32) -> Duration {
    let index = usize::try_from(attempts.saturating_sub(1)).unwrap_or(usize::MAX).min(RETRY_MINUTES.len() - 1);
    Duration::minutes(RETRY_MINUTES[index] as i64)
}

/// Drops what every enabled tracker is finished with, and bounds the file. An
/// event a *disabled* tracker still owes stays: turning that tracker back on
/// must not mean the history of those days is lost.
pub fn prune(file: &mut TrackingFile, enabled: &[String]) {
    if !enabled.is_empty() {
        file.events.retain(|queued| !enabled.iter().all(|tracker| queued.trackers.get(tracker).is_some_and(|state| state.done)));
    }
    trim(file);
}

fn trim(file: &mut TrackingFile) {
    if file.events.len() > MAX_EVENTS {
        let excess = file.events.len() - MAX_EVENTS;
        file.events.drain(..excess);
    }
    let now = Utc::now();
    file.pushed.retain(|record| now.signed_duration_since(record.delivered_at) < PUSHED_FOR);
    if file.pushed.len() > MAX_PUSHED {
        let excess = file.pushed.len() - MAX_PUSHED;
        file.pushed.drain(..excess);
    }
}

/// Notes a delivery: the import ignores a provider fact for this target while it
/// is recent (our own write coming back).
pub fn record_pushed(file: &mut TrackingFile, event: &TrackEvent, at: DateTime<Utc>) {
    file.pushed.push(PushedRecord { delivered_at: at, title: event.title.clone(), view: event.view.clone() });
    trim(file);
}

/// Whether this target was delivered inside `window` — the echo test.
pub fn pushed_within(file: &TrackingFile, title: &TrackedTitle, view: &TrackedView, window: chrono::Duration, now: DateTime<Utc>) -> bool {
    file.pushed.iter().any(|record| {
        record.view == *view
            && record.title.is(title.imdb.as_deref(), &title.title)
            && now.signed_duration_since(record.delivered_at) < window
    })
}

/// Whether a title or episode still has an event the outbox is working on. A
/// pull skips it: importing first would race our own write.
pub fn pending_for(file: &TrackingFile, title: &TrackedTitle, view: &TrackedView, now: DateTime<Utc>) -> bool {
    file.events.iter().any(|queued| {
        queued.event.title.is(title.imdb.as_deref(), &title.title)
            && queued.event.view == *view
            // An event nobody has attempted yet is pending too: an empty map is
            // not "everyone is done".
            && (queued.trackers.is_empty() || queued.trackers.values().any(|state| !state.done))
            && now.signed_duration_since(queued.since()) < PENDING_HORIZON
    })
}

/// How many events are still owed to at least one enabled tracker.
pub fn pending(file: &TrackingFile, enabled: &[String]) -> usize {
    file.events.iter().filter(|queued| enabled.iter().any(|tracker| queued.trackers.get(tracker).is_none_or(|state| !state.done))).count()
}

/// The newest **temporary** failure a person should hear about: it is the state
/// of the sync, so it goes away once a drain succeeds. A refused title is not a
/// failure of the sync and lives in [`notices`] instead.
pub fn last_error(file: &TrackingFile, enabled: &[String]) -> Option<String> {
    file.events
        .iter()
        .filter(|queued| enabled.iter().any(|tracker| queued.trackers.get(tracker).is_some_and(|state| !state.done)))
        .filter_map(|queued| {
            queued
                .trackers
                .values()
                .filter_map(|state| state.failure.as_ref())
                .find(|failure| !failure.permanent)
                .map(|failure| failure.error.clone())
        })
        .next()
}

/// The titles a tracker refused for good, newest first: shown beside the sync
/// state, never as it.
pub fn notices(file: &TrackingFile) -> &[String] {
    &file.notices
}

/// Forgets the refusals: they belong to the account that was signed in, so
/// signing out (or forgetting it) clears them.
pub fn clear_notices(app: &AppPaths) -> Result<()> {
    update(app, |file| {
        file.notices.clear();
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::tracker::{TrackedTitle, TrackedView, TrackedWatch};

    fn app(root: &std::path::Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    fn event(view: TrackedView, watch: TrackedWatch) -> TrackEvent {
        TrackEvent {
            seq: 0,
            at: Utc::now(),
            title: TrackedTitle { imdb: Some("tt1".into()), kind: "series".into(), title: "A Show".into(), year: Some("2011".into()) },
            view,
            watch,
        }
    }

    fn enabled(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn the_queue_folds_the_same_thing_and_keeps_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let episode = TrackedView::Episode { season: 1, number: 2 };

        let first = enqueue(&app, event(episode.clone(), TrackedWatch::InProgress { position_ms: 60_000, duration_ms: 600_000 })).unwrap();
        let second = enqueue(&app, event(episode.clone(), TrackedWatch::Watched)).unwrap();
        assert_eq!((first.seq, second.seq), (1, 2), "events are numbered");
        let file = read(&app);
        assert_eq!(file.events.len(), 1, "the newest state of the same episode replaced the older one");
        assert_eq!(file.events[0].event.watch, TrackedWatch::Watched);

        enqueue(&app, event(TrackedView::Episode { season: 1, number: 3 }, TrackedWatch::Watched)).unwrap();
        enqueue(&app, event(TrackedView::Movie, TrackedWatch::Watched)).unwrap();
        assert_eq!(read(&app).events.len(), 3, "another episode, another title");
    }

    #[test]
    fn an_event_a_tracker_already_has_is_no_longer_folded() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let movie = TrackedView::Movie;
        enqueue(&app, event(movie.clone(), TrackedWatch::Watched)).unwrap();
        let mut file = read(&app);
        delivered(&mut file.events[0], "simkl");
        write(&app, &file).unwrap();

        enqueue(&app, event(movie, TrackedWatch::Watched)).unwrap();
        assert_eq!(read(&app).events.len(), 2, "what happened twice is two things once one of them was delivered");
    }

    #[test]
    fn a_temporary_fault_is_retried_on_a_schedule_and_a_refusal_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let now = Utc::now();
        enqueue(&app, event(TrackedView::Movie, TrackedWatch::Watched)).unwrap();
        let mut file = read(&app);
        let seq = file.events[0].event.seq;

        retry(&mut file.events[0], "simkl", "connection reset", None, now);
        assert!(!due(&file.events[0], "simkl", now), "not before its time");
        assert_eq!(file.events[0].trackers["simkl"].failure.as_ref().unwrap().attempts, 1);
        assert!(due(&file.events[0], "simkl", now + retry_delay(1)));
        assert_eq!(retry_delay(1), Duration::minutes(1));
        assert_eq!(retry_delay(2), Duration::minutes(5));
        assert_eq!(retry_delay(9), Duration::minutes(30), "the last window is the last");
        assert_eq!(pending(&file, &enabled(&["simkl"])), 1);

        unmatched(&mut file, seq, "simkl", "not in the catalogue", now);
        assert!(file.events[0].trackers["simkl"].done, "a refusal ends it");
        assert!(file.events[0].trackers["simkl"].failure.as_ref().is_some_and(|failure| failure.permanent));
        assert_eq!(file.notices, ["A Show: not in the catalogue"]);
        assert_eq!(pending(&file, &enabled(&["simkl"])), 0);
        // A tracker that is not enabled is not owed anything.
        assert_eq!(pending(&file, &enabled(&["trakt"])), 1);
    }

    #[test]
    fn an_event_can_be_marked_as_had_by_one_tracker_without_sending_it() {
        let mut file = TrackingFile::default();
        queue(&mut file, event(TrackedView::Movie, TrackedWatch::Watched));
        done_for(&mut file.events[0], "simkl");
        let now = Utc::now();
        assert!(!due(&file.events[0], "simkl", now), "told already");
        assert!(due(&file.events[0], "trakt", now), "the other still owes it");
        assert_eq!(pending(&file, &enabled(&["simkl", "trakt"])), 1);
    }

    #[test]
    fn a_provider_wait_wins_over_the_ladder() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let now = Utc::now();
        enqueue(&app, event(TrackedView::Movie, TrackedWatch::Watched)).unwrap();
        let mut file = read(&app);

        retry(&mut file.events[0], "simkl", "daily quota", Some(Duration::minutes(42)), now);
        assert!(!due(&file.events[0], "simkl", now + Duration::minutes(41)), "the header's wait, not ours");
        assert!(due(&file.events[0], "simkl", now + Duration::minutes(42)));
        assert_eq!(file.events[0].trackers["simkl"].failure.as_ref().unwrap().attempts, 1);
    }

    #[test]
    fn a_refusal_is_a_notice_and_a_fault_is_the_sync_state() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let now = Utc::now();
        let mut file = TrackingFile::default();
        queue(&mut file, event(TrackedView::Movie, TrackedWatch::Watched));
        let seq = file.events[0].event.seq;
        unmatched(&mut file, seq, "simkl", "not in the catalogue", now);
        assert_eq!(notices(&file), ["A Show: not in the catalogue"]);
        assert_eq!(last_error(&file, &enabled(&["simkl"])), None, "a refusal is not a failure of the sync");

        // A temporary fault on an event still owed is.
        queue(&mut file, event(TrackedView::Episode { season: 1, number: 4 }, TrackedWatch::Watched));
        let seq = file.events[1].event.seq;
        if let Some(queued) = file.events.iter_mut().find(|queued| queued.event.seq == seq) {
            retry(queued, "simkl", "connection reset", None, now);
        }
        assert_eq!(last_error(&file, &enabled(&["simkl"])).as_deref(), Some("connection reset"));

        write(&app, &file).unwrap();
        clear_notices(&app).unwrap();
        assert!(notices(&read(&app)).is_empty(), "signing out forgets the refusals");
        assert!(!read(&app).events.is_empty(), "and keeps what is still owed");
    }

    #[test]
    fn pruning_drops_what_every_enabled_tracker_has_but_not_what_a_disabled_one_owes() {
        // This one never touches the disk: the file type is exercised in memory.
        let mut file = TrackingFile::default();
        queue(&mut file, event(TrackedView::Movie, TrackedWatch::Watched));
        queue(&mut file, event(TrackedView::Episode { season: 1, number: 1 }, TrackedWatch::Watched));
        delivered(&mut file.events[0], "simkl");
        delivered(&mut file.events[1], "simkl");

        prune(&mut file, &enabled(&["simkl", "trakt"]));
        assert_eq!(file.events.len(), 2, "trakt still owes both");
        delivered(&mut file.events[0], "trakt");
        delivered(&mut file.events[1], "trakt");
        prune(&mut file, &enabled(&["simkl", "trakt"]));
        assert!(file.events.is_empty(), "everybody is done");

        // With no tracker enabled nothing is pruned: the events wait for one.
        queue(&mut file, event(TrackedView::Movie, TrackedWatch::Watched));
        delivered(&mut file.events[0], "simkl");
        prune(&mut file, &[]);
        assert_eq!(file.events.len(), 1);
    }

    #[test]
    fn the_file_is_bounded_and_keeps_what_a_newer_build_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let mut file = TrackingFile::default();
        for number in 0..(MAX_EVENTS + 5) {
            queue(&mut file, event(TrackedView::Episode { season: 1, number: number as u32 }, TrackedWatch::Watched));
        }
        assert_eq!(file.events.len(), MAX_EVENTS, "the oldest are dropped");
        assert_eq!(file.next_seq as usize, MAX_EVENTS + 6, "numbers are never reused");

        let mut value = serde_json::to_value(&file).unwrap();
        value["somethingNew"] = serde_json::json!(true);
        json_store::write(&path(&app), &value).unwrap();
        let read_back = read(&app);
        assert_eq!(read_back.extra.get("somethingNew"), Some(&serde_json::json!(true)));
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use heronite_watch_core::tracker::TrackedWatch;

    fn title() -> TrackedTitle {
        TrackedTitle { imdb: Some("tt1".into()), kind: "series".into(), title: "A Show".into(), year: Some("2011".into()) }
    }

    fn event() -> TrackEvent {
        TrackEvent {
            seq: 0,
            at: Utc::now(),
            title: title(),
            view: TrackedView::Episode { season: 1, number: 2 },
            watch: TrackedWatch::Watched,
        }
    }

    fn episode() -> TrackedView {
        TrackedView::Episode { season: 1, number: 2 }
    }

    #[test]
    fn a_delivery_is_remembered_and_prune_keeps_the_evidence() {
        let mut file = TrackingFile::default();
        let event = queue(&mut file, event());
        delivered(&mut file.events[0], "simkl");
        record_pushed(&mut file, &event, Utc::now());
        prune(&mut file, &["simkl".to_owned()]);
        assert!(file.events.is_empty(), "the outbox is done with it");
        assert_eq!(file.pushed.len(), 1, "the delivery survives the prune");
    }

    #[test]
    fn a_pushed_target_is_recognised_inside_the_window_only() {
        let mut file = TrackingFile::default();
        record_pushed(&mut file, &event(), Utc::now());
        assert!(pushed_within(&file, &title(), &episode(), chrono::Duration::minutes(5), Utc::now()));
        assert!(
            !pushed_within(&file, &title(), &TrackedView::Movie, chrono::Duration::minutes(5), Utc::now()),
            "another view is another target"
        );
        let later = Utc::now() + chrono::Duration::minutes(10);
        assert!(!pushed_within(&file, &title(), &episode(), chrono::Duration::minutes(5), later), "the window closed");
    }

    #[test]
    fn the_pushed_log_is_bounded() {
        let mut file = TrackingFile::default();
        for _ in 0..(MAX_PUSHED + 10) {
            record_pushed(&mut file, &event(), Utc::now());
        }
        assert_eq!(file.pushed.len(), MAX_PUSHED);
    }

    #[test]
    fn pending_is_true_while_fresh_and_false_after_the_horizon() {
        let mut file = TrackingFile::default();
        queue(&mut file, event());
        assert!(pending_for(&file, &title(), &episode(), Utc::now()), "never attempted is pending");
        let later = Utc::now() + PENDING_HORIZON + chrono::Duration::minutes(1);
        assert!(!pending_for(&file, &title(), &episode(), later), "the ladder is spent, the pull may run");
        delivered(&mut file.events[0], "simkl");
        assert!(!pending_for(&file, &title(), &episode(), Utc::now()), "delivered is not pending");
    }

    #[test]
    fn another_title_is_not_pending() {
        let mut file = TrackingFile::default();
        queue(&mut file, event());
        let other = TrackedTitle { imdb: Some("tt9".into()), ..title() };
        assert!(!pending_for(&file, &other, &episode(), Utc::now()));
    }
}
