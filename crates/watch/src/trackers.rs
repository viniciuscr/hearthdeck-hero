//! Keeping the person's watch history in step with the trackers they enabled.
//!
//! **The local record is the truth.** Playback and manual marks write
//! `movies.json` / `watch-sessions.json` first, then queue what happened in the
//! outbox ([`heronite_watch_state::tracking`]). This module drains that outbox —
//! on a background thread, never on the way of a playback or a page — and turns
//! what it cannot deliver into a pending count a screen can show.
//!
//! A tracker nobody signed in to is not queued for: the outbox only grows while
//! a destination exists, and [`backfill`] is there for the history from before.

use anyhow::Result;
use chrono::{DateTime, Utc};
use heronite_data::paths::AppPaths;
use heronite_watch_core::http::UreqTransport;
use heronite_watch_core::progress::{finished, WatchedState};
use heronite_watch_core::tracker::{Delivery, ListChange, TrackEvent, TrackedTitle, TrackedView, TrackedWatch, WatchTracker};
use heronite_watch_core::types::MovieProgress;
use heronite_watch_player::player::PlaybackOutcome;
use heronite_watch_state::sessions::{self, WatchSession};
use heronite_watch_state::tracking;

use crate::playback::PlaybackRequest;

/// What the facade needs to know about one tracker: how to tell it is usable, and
/// how to build the deliverer for a drain. A second tracker is another module
/// implementing this and one line in [`adapters`] — no branch anywhere else.
pub trait TrackerAdapter: Sync {
    fn id(&self) -> &'static str;
    /// How a screen or a log calls it.
    fn name(&self) -> &'static str;
    /// Whether this machine can use it right now (signed in, credentials there).
    fn ready(&self, app: &AppPaths) -> bool;
    /// The deliverer for one drain; `Err` when it cannot be built (a dead session).
    fn build<'a>(&self, app: &AppPaths, http: &'a UreqTransport) -> Result<Box<dyn WatchTracker + 'a>>;
}

/// Every tracker this build ships, in registry order.
pub fn adapters() -> &'static [&'static dyn TrackerAdapter] {
    static ALL: [&'static dyn TrackerAdapter; 1] = [&crate::simkl_adapter::SIMKL];
    &ALL
}

/// How a tracker is called on a screen.
pub fn name(id: &str) -> &'static str {
    adapters().iter().find(|adapter| adapter.id() == id).map(|adapter| adapter.name()).unwrap_or("Tracker")
}

/// What a drain did, and how much is left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Events at least one tracker took.
    pub delivered: usize,
    /// Events a tracker refused (the title is not in its catalogue).
    pub unmatched: usize,
    /// Still owed to at least one enabled tracker.
    pub pending: usize,
    /// The last failure a screen should show, when one happened.
    pub failed: Option<String>,
}

/// What is waiting, for the settings card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub count: usize,
    /// The last **temporary** fault: it goes away once a drain succeeds.
    pub last_error: Option<String>,
    /// Titles a tracker refused for good, newest first: not a sync fault.
    pub unmatched: Vec<String>,
}

/// Events one drain delivers. Each may be two requests (a paused playback is
/// one), so the run stays bounded and a hand full of marks does not turn into
/// minutes of background work.
pub const MAX_PER_RUN: usize = 40;

/// The trackers this build can use **now**: signed in, with the app's credential.
pub fn available(app: &AppPaths) -> Vec<String> {
    adapters().iter().filter(|adapter| adapter.ready(app)).map(|adapter| adapter.id().to_owned()).collect()
}

/// The trackers the person turned on, among the ones this build can use.
pub fn enabled(app: &AppPaths) -> Vec<String> {
    let chosen = heronite_watch_state::settings::read(app).trackers;
    let ready = available(app);
    chosen.into_iter().filter(|id| ready.contains(id)).collect()
}

/// Queues what one playback did, after the local files were written. Local only:
/// nothing here touches the network.
pub fn record_playback(app: &AppPaths, request: &PlaybackRequest, previous: Option<&MovieProgress>, outcome: &PlaybackOutcome) {
    if outcome.position_ms == 0 && outcome.duration_ms == 0 {
        return;
    }
    let Some(view) = view_of(request) else {
        log::debug!("not queueing {}: the episode's season and number are unknown", request.id);
        return;
    };
    let state = WatchedState::of(outcome.position_ms, outcome.duration_ms, finished(outcome.position_ms, outcome.duration_ms));
    let watch = match state {
        WatchedState::Unwatched => return,
        WatchedState::Watched => TrackedWatch::Watched,
        // Leaving a title part-way is a paused position — but not for something
        // that was already watched: a rewatch stopped early is not a new watch.
        WatchedState::InProgress { .. } if was_watched(request, previous) => {
            log::debug!("not queueing a rewatch of {} that stopped part-way", request.id);
            return;
        }
        WatchedState::InProgress { position_ms, duration_ms } => TrackedWatch::InProgress { position_ms, duration_ms },
    };
    queue(app, TrackEvent { seq: 0, at: Utc::now(), title: title_of(request), view, watch });
}

/// Queues a mark the person made by hand (a tick, or taking the tick away).
pub fn record_manual(app: &AppPaths, title: TrackedTitle, view: TrackedView, watch: TrackedWatch) {
    // A series marked before its episode list is known has nothing a tracker can
    // understand: a film's mark on a show would be posted as a movie entry.
    if view == TrackedView::Movie && title.kind == "series" {
        log::debug!("not queueing a film mark for the series {}", title.title);
        return;
    }
    queue(app, TrackEvent { seq: 0, at: Utc::now(), title, view, watch });
}

/// Whether the outbox is still working on this title or episode: a pull skips it
/// rather than racing our own write.
pub fn pending_for(app: &AppPaths, title: &TrackedTitle, view: &TrackedView) -> bool {
    tracking::pending_for(&tracking::read(app), title, view, Utc::now())
}

/// Whether this target was delivered inside `window` — the echo test a pull uses
/// to recognise its own write.
pub fn pushed_within(app: &AppPaths, title: &TrackedTitle, view: &TrackedView, window: chrono::Duration) -> bool {
    tracking::pushed_within(&tracking::read(app), title, view, window, Utc::now())
}

/// What is waiting, for a screen to show.
pub fn pending(app: &AppPaths) -> Pending {
    let enabled = enabled(app);
    let file = tracking::read(app);
    Pending {
        count: tracking::pending(&file, &enabled),
        last_error: tracking::last_error(&file, &enabled),
        unmatched: tracking::notices(&file).to_vec(),
    }
}

/// Forgets the titles a tracker refused: they belong to the account that was
/// signed in, so signing out or forgetting it clears them.
pub fn clear_notices(app: &AppPaths) -> Result<()> {
    tracking::clear_notices(app)
}

/// Sends what is due, and reports what is left. Blocking: call it off the UI
/// thread. A tracker that answers "wrong token" stops for this run rather than
/// hammering the API — the card says to sign in again.
pub fn drain(app: &AppPaths) -> Result<Report> {
    let enabled = enabled(app);
    let mut report = Report::default();
    if enabled.is_empty() {
        report.pending = tracking::pending(&tracking::read(app), &enabled);
        return Ok(report);
    }

    let http = UreqTransport::new();
    let deliverers = deliverers(app, &http, &enabled);
    if deliverers.is_empty() {
        report.pending = tracking::pending(&tracking::read(app), &enabled);
        return Ok(report);
    }

    // What to send, from a snapshot: the file is not held open while the network
    // answers, so a mark queued meanwhile is applied to the same file afterwards.
    let now = Utc::now();
    let snapshot = tracking::read(app);
    let mut work: Vec<(&'static str, &dyn WatchTracker, TrackEvent)> = Vec::new();
    'collect: for (id, tracker) in &deliverers {
        for queued in snapshot.events.iter().filter(|queued| tracking::due(queued, id, now)) {
            if work.len() >= MAX_PER_RUN {
                break 'collect;
            }
            work.push((id, tracker.as_ref(), queued.event.clone()));
        }
    }

    let mut results: Vec<(&'static str, u64, Delivery)> = Vec::new();
    let mut failed: Option<String> = None;
    for (id, tracker, event) in work {
        match tracker.deliver(&event) {
            Ok(delivery) => {
                if let Delivery::Retry { reason, .. } = &delivery {
                    failed = Some(reason.clone());
                }
                results.push((id, event.seq, delivery));
            }
            // The tracker cannot be used (a dead token, no header): stop this one
            // for the run and keep the rest of its events pending.
            Err(error) => {
                let reason = format!("{error:#}");
                log::warn!("tracker {id}: {reason}");
                failed = Some(reason.clone());
                results.push((id, event.seq, Delivery::Retry { reason, after: None }));
                break;
            }
        }
    }

    report.delivered = results.iter().filter(|(_, _, delivery)| matches!(delivery, Delivery::Delivered)).count();
    report.unmatched = results.iter().filter(|(_, _, delivery)| matches!(delivery, Delivery::Unmatched(_))).count();
    report.failed = failed;

    tracking::update(app, |file| {
        let mut delivered: Vec<TrackEvent> = Vec::new();
        for (id, seq, delivery) in results {
            match delivery {
                Delivery::Delivered => {
                    if let Some(queued) = queued_mut(file, seq) {
                        tracking::delivered(queued, id);
                        delivered.push(queued.event.clone());
                    }
                }
                Delivery::Unmatched(reason) => tracking::unmatched(file, seq, id, &reason, now),
                Delivery::Retry { reason, after } => {
                    if let Some(queued) = queued_mut(file, seq) {
                        tracking::retry(queued, id, &reason, after, now);
                    }
                }
            }
        }
        // Our own write, remembered so a pull ignores it coming back — and noted
        // before `prune`, which would otherwise erase the evidence.
        for event in delivered {
            tracking::record_pushed(file, &event, now);
        }
        tracking::prune(file, &enabled);
        report.pending = tracking::pending(file, &enabled);
        Ok(())
    })?;
    Ok(report)
}

/// Queues the history already on this machine (every finished session), then
/// drains it: what makes a library that predates the tracker show up there. A
/// tracker receives each title/episode once — rewatch counts are Simkl's own.
/// Queues the finished history already on this machine for **one** tracker: the
/// one just switched on. The others were told already; the outbox marks these
/// events done for them, so nothing is re-sent and nothing is delivered twice.
pub fn backfill_for(app: &AppPaths, tracker: &str) -> Result<usize> {
    let on = enabled(app);
    if !on.iter().any(|id| id == tracker) {
        return Ok(0);
    }
    let sessions = sessions::read(app).sessions;
    let queued = tracking::update(app, |file| {
        let mut queued = 0;
        let mut seqs = Vec::new();
        for session in sessions.iter().filter(|session| session.finished) {
            let Some(event) = event_of_session(session) else { continue };
            // `queue` assigns the seq, and may fold into an event still waiting.
            seqs.push(tracking::queue(file, event).seq);
            queued += 1;
        }
        for seq in seqs {
            if let Some(queued) = queued_mut(file, seq) {
                for other in on.iter().filter(|id| id.as_str() != tracker) {
                    tracking::done_for(queued, other);
                }
            }
        }
        Ok(queued)
    })?;
    Ok(queued)
}

/// Queues the finished history for every tracker that is on.
pub fn backfill(app: &AppPaths) -> Result<usize> {
    if enabled(app).is_empty() {
        return Ok(0);
    }
    let sessions = sessions::read(app).sessions;
    let queued = tracking::update(app, |file| {
        let mut queued = 0;
        for session in sessions.iter().filter(|session| session.finished) {
            let Some(event) = event_of_session(session) else { continue };
            tracking::queue(file, event);
            queued += 1;
        }
        Ok(queued)
    })?;
    let report = drain(app)?;
    log::info!("sent {queued} watch sessions to the trackers, {} still waiting", report.pending);
    Ok(queued)
}

/// Delivers one list change to every enabled tracker that keeps lists, in the
/// same order and one at a time as a watch event.
///
/// There is no list outbox: a change a tracker did not take stays owed (the ledger
/// still says `in`/`out`, the provider's list still says otherwise) and the next
/// import derives it again. That is why a failure here is a log line, not state.
pub fn deliver_list(app: &AppPaths, change: &ListChange) -> Vec<(&'static str, Delivery)> {
    let http = UreqTransport::new();
    let enabled = enabled(app);
    deliverers(app, &http, &enabled)
        .into_iter()
        .map(|(id, tracker)| {
            // A tracker that cannot be used (a dead token, no session) is a retry,
            // exactly like a watch event: the change is derived again next tick.
            let delivery = match tracker.sync_list(change) {
                Ok(delivery) => delivery,
                Err(error) => Delivery::Retry { reason: format!("{error:#}"), after: None },
            };
            (id, delivery)
        })
        .collect()
}

/// The trackers that can deliver right now, and the ids the outbox knows them by.
fn deliverers<'a>(app: &AppPaths, http: &'a UreqTransport, enabled: &[String]) -> Vec<(&'static str, Box<dyn WatchTracker + 'a>)> {
    let mut out: Vec<(&'static str, Box<dyn WatchTracker + 'a>)> = Vec::new();
    for adapter in adapters().iter().filter(|adapter| enabled.iter().any(|id| id == adapter.id())) {
        match adapter.build(app, http) {
            Ok(tracker) => out.push((adapter.id(), tracker)),
            // A tracker that cannot be built now (a dead session) is said once and
            // skipped; the outbox keeps its events.
            Err(error) => log::warn!("{}: {error:#}; not delivering to it", adapter.name()),
        }
    }
    out
}

fn queued_mut(file: &mut tracking::TrackingFile, seq: u64) -> Option<&mut tracking::QueuedEvent> {
    file.events.iter_mut().find(|queued| queued.event.seq == seq)
}

/// Queues `event`, when there is somewhere to send it. A tracker turned on later
/// gets the history through [`backfill`], not by replaying an unbounded log.
fn queue(app: &AppPaths, event: TrackEvent) {
    if enabled(app).is_empty() {
        return;
    }
    if let Err(error) = tracking::enqueue(app, event) {
        log::warn!("queueing the watch event: {error:#}");
    }
}

fn title_of(request: &PlaybackRequest) -> TrackedTitle {
    let name = if request.name.trim().is_empty() { request.title.clone() } else { request.name.clone() };
    TrackedTitle { imdb: request.imdb.clone(), kind: request.kind.clone(), title: name, year: request.year.clone() }
}

/// Which part of the title the request played. A series without its episode's
/// numbering cannot be told to a tracker, so it is left out rather than guessed
/// from the provider's id shape.
fn view_of(request: &PlaybackRequest) -> Option<TrackedView> {
    if request.kind == "series" {
        return Some(TrackedView::Episode { season: request.season?, number: request.number? });
    }
    Some(TrackedView::Movie)
}

/// Whether this exact thing was already watched before: a film by its own
/// record, an episode by its own entry.
fn was_watched(request: &PlaybackRequest, previous: Option<&MovieProgress>) -> bool {
    let Some(previous) = previous else { return false };
    match request.video_id.as_deref() {
        Some(video) => previous.episodes.get(video).is_some_and(|episode| episode.finished),
        None => previous.finished,
    }
}

/// A finished session as an event, for the history backfill.
fn event_of_session(session: &WatchSession) -> Option<TrackEvent> {
    let view = if session.kind == "series" {
        TrackedView::Episode { season: session.season?, number: session.number? }
    } else {
        TrackedView::Movie
    };
    let at: DateTime<Utc> = session.ended_at;
    let title = if session.name.trim().is_empty() { session.title.clone() } else { session.name.clone() };
    Some(TrackEvent {
        seq: 0,
        at,
        title: TrackedTitle {
            imdb: Some(session.id.clone()).filter(|id| id.starts_with("tt")),
            kind: session.kind.clone(),
            title,
            year: session.year.clone(),
        },
        view,
        watch: TrackedWatch::Watched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use heronite_data::paths::DataPaths;
    use heronite_watch_core::provider::Credentials;

    fn app(dir: &std::path::Path) -> AppPaths {
        let app = AppPaths::for_data(&DataPaths::at(dir.to_path_buf()));
        // Nothing is on by default: switching the tracker on is what the person
        // does in Sources, and these tests are all about a switched-on tracker.
        let settings = heronite_watch_state::settings::Settings { trackers: vec!["simkl".to_owned()], ..Default::default() };
        heronite_watch_state::settings::write(&app, &settings).unwrap();
        app
    }

    /// Signing in is what makes the tracker available; the tests write the same
    /// credential the sign-in would.
    fn sign_in(app: &AppPaths) {
        heronite_watch_state::store::write_simkl_client_id(app, "client-1").unwrap();
        let secret = serde_json::json!({
            "clientId": "client-1",
            "accessToken": "simkl_at_x",
            "refreshToken": "simkl_rt_y",
            "scope": "media:read",
            "expiresAt": "2099-01-01T00:00:00Z",
        });
        heronite_watch_state::store::StoreCredentials::new(app).set("simkl", &heronite_watch_core::provider::Secret::new(secret)).unwrap();
    }

    fn request(video: Option<&str>) -> PlaybackRequest {
        PlaybackRequest {
            provider: "stremio".into(),
            imdb: Some("tt1".into()),
            id: "tt1".into(),
            kind: if video.is_some() { "series".into() } else { "movie".into() },
            video_id: video.map(str::to_owned),
            season: video.map(|_| 1),
            number: video.map(|_| 2),
            title: "S1E2 · A Show".into(),
            name: "A Show".into(),
            year: Some("2011".into()),
            stream: Default::default(),
        }
    }

    fn outcome(position_ms: u64, duration_ms: u64) -> PlaybackOutcome {
        PlaybackOutcome { position_ms, duration_ms }
    }

    #[test]
    fn nothing_is_queued_while_no_tracker_is_signed_in() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        assert!(enabled(&app).is_empty(), "signed out is not enabled");
        record_playback(&app, &request(None), None, &outcome(600_000, 6_000_000));
        assert_eq!(pending(&app).count, 0);
        assert!(tracking::read(&app).events.is_empty(), "no destination, nothing queued");
    }

    #[test]
    fn a_finished_episode_is_queued_with_its_season_and_number() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        sign_in(&app);
        record_playback(&app, &request(Some("tt1:1:2")), None, &outcome(5_000_000, 6_000_000));
        let file = tracking::read(&app);
        assert_eq!(file.events.len(), 1);
        let event = &file.events[0].event;
        assert_eq!(event.watch, TrackedWatch::Watched);
        assert_eq!(event.view, TrackedView::Episode { season: 1, number: 2 });
        assert_eq!(event.title.title, "A Show", "the show, not the episode label");
        assert_eq!(event.title.year.as_deref(), Some("2011"));
    }

    #[test]
    fn a_series_without_its_episode_numbering_is_not_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        sign_in(&app);
        let request = PlaybackRequest { season: None, number: None, ..request(Some("tt1:1:2")) };
        record_playback(&app, &request, None, &outcome(5_000_000, 6_000_000));
        assert!(tracking::read(&app).events.is_empty(), "no season, no event");
    }

    #[test]
    fn a_mis_click_is_not_queued_and_a_rewatch_is_not_a_new_watch() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        sign_in(&app);
        record_playback(&app, &request(None), None, &outcome(5_000, 6_000_000));
        assert!(tracking::read(&app).events.is_empty(), "five seconds is not watching");

        let mut previous = MovieProgress {
            provider: "stremio".into(),
            id: "tt1".into(),
            position_ms: 6_000_000,
            duration_ms: 6_000_000,
            updated_at: Utc::now(),
            finished: true,
            video_id: None,
            imdb: Some("tt1".into()),
            episodes: Default::default(),
            ..Default::default()
        };
        record_playback(&app, &request(None), Some(&previous), &outcome(600_000, 6_000_000));
        assert!(tracking::read(&app).events.is_empty(), "a rewatch stopped part-way is not a paused playback");

        previous.finished = false;
        record_playback(&app, &request(None), Some(&previous), &outcome(600_000, 6_000_000));
        let file = tracking::read(&app);
        assert_eq!(file.events[0].event.watch, TrackedWatch::InProgress { position_ms: 600_000, duration_ms: 6_000_000 });
    }

    #[test]
    fn a_manual_mark_is_queued_as_the_person_made_it() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        sign_in(&app);
        let title = TrackedTitle { imdb: Some("tt1".into()), kind: "movie".into(), title: "A Film".into(), year: None };
        record_manual(&app, title.clone(), TrackedView::Movie, TrackedWatch::Watched);
        record_manual(&app, title, TrackedView::Movie, TrackedWatch::Unwatched);
        let file = tracking::read(&app);
        assert_eq!(file.events.len(), 1, "the newest state of the same title");
        assert_eq!(file.events[0].event.watch, TrackedWatch::Unwatched);
    }

    #[test]
    fn a_series_is_never_queued_as_a_film() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        sign_in(&app);
        let show = TrackedTitle { imdb: Some("tt1".into()), kind: "series".into(), title: "A Show".into(), year: None };
        record_manual(&app, show, TrackedView::Movie, TrackedWatch::Watched);
        assert!(tracking::read(&app).events.is_empty(), "a film's mark on a show would be posted as a movie entry");
    }

    #[test]
    fn a_backfill_only_arms_the_tracker_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        sign_in(&app);
        let at = Utc::now();
        heronite_watch_state::sessions::append(
            &app,
            WatchSession {
                provider: "stremio".into(),
                kind: "movie".into(),
                id: "tt1".into(),
                video_id: None,
                season: None,
                number: None,
                name: "A Film".into(),
                year: Some("2010".into()),
                title: "A Film".into(),
                started_at: at,
                ended_at: at,
                elapsed_seconds: 6_000.,
                position_from_ms: 0,
                position_to_ms: 6_000_000,
                duration_ms: 6_000_000,
                finished: true,
            },
        )
        .unwrap();

        assert_eq!(backfill_for(&app, "trakt").unwrap(), 0, "a tracker that is off owes nothing");
        assert!(tracking::read(&app).events.is_empty());

        assert_eq!(backfill_for(&app, "simkl").unwrap(), 1);
        let file = tracking::read(&app);
        assert_eq!(file.events.len(), 1);
        assert!(tracking::due(&file.events[0], "simkl", Utc::now()), "it is the one being told");
    }

    #[test]
    fn the_backfill_queues_finished_sessions_and_skips_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        sign_in(&app);
        let at = Utc::now();
        for (episode, finished) in [(1u32, true), (2, false)] {
            let session = WatchSession {
                provider: "stremio".into(),
                kind: "series".into(),
                id: "tt1".into(),
                video_id: Some(format!("tt1:1:{episode}")),
                season: Some(1),
                number: Some(episode),
                name: "A Show".into(),
                year: Some("2011".into()),
                title: format!("S1E{episode} · A Show"),
                started_at: at,
                ended_at: at,
                elapsed_seconds: 1_800.,
                position_from_ms: 0,
                position_to_ms: 1_800_000,
                duration_ms: 2_400_000,
                finished,
            };
            heronite_watch_state::sessions::append(&app, session).unwrap();
        }
        let queued = backfill(&app).unwrap();
        assert_eq!(queued, 1, "only the finished one is history");
        let file = tracking::read(&app);
        assert_eq!(file.events.len(), 1, "the delivery failed (no network in tests), so it waits");
        assert_eq!(file.events[0].event.view, TrackedView::Episode { season: 1, number: 1 });
        assert_eq!(file.events[0].event.title.title, "A Show");
    }
}
