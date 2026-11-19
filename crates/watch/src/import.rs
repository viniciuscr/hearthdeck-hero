//! Applying a pull to the local records.
//!
//! The local files stay the truth: an import writes into them **with provenance**,
//! never un-finishes a record, never touches an `unwatched` marker, never races the
//! outbox, and never overwrites a position the person set here since the last pull.
//! The provider's snapshot is saved last, and only if every write landed.

use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};

use heronite_data::paths::AppPaths;
use heronite_watch_core::http::UreqTransport;
use heronite_watch_core::provider::Ctx;
use heronite_watch_core::pull::{Pulled, RemotePosition, RemoteWatch, WatchSync};
use heronite_watch_core::tracker::{TrackedTitle, TrackedView};
use heronite_watch_core::types::{MovieProgress, Origin};
use heronite_watch_simkl::pull::SimklPull;
use heronite_watch_state::import as cursor;
use heronite_watch_state::store;
use heronite_watch_state::store::StoreCredentials;

use crate::trackers;

/// How long after a delivery our own write is still "not news".
const ECHO: chrono::Duration = chrono::Duration::minutes(5);

/// What one import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub watched: usize,
    pub positions: usize,
    /// Provider items this build cannot use (no IMDb id, a show-level fact).
    pub skipped: usize,
    /// Titles the provider knows and the library does not — placeholders are a
    /// later slice, so these are only counted for now.
    pub no_local: usize,
    /// Things left alone on purpose: unwatched markers, finished records, a
    /// pending push, our own echo, a local position newer than the pull.
    pub blocked: usize,
    /// The change query said nothing moved: no pull, no writes.
    pub unchanged: bool,
}

impl Report {
    /// Whether the import put anything into the files.
    pub fn wrote(&self) -> bool {
        self.watched + self.positions > 0
    }
}

/// How often an automatic pull runs while the app is open. One change-query
/// request when nothing moved, so a day costs at most 96 — inside the free plan's
/// 500 and leaving room for writes.
pub const INTERVAL: chrono::Duration = chrono::Duration::minutes(15);

/// Waits after consecutive failures, in minutes.
const BACKOFF_MINUTES: [i64; 4] = [1, 5, 30, 60];

/// The pullers this build ships, in registry order.
pub fn providers() -> &'static [&'static dyn WatchSync] {
    static SIMKL: SimklPull = SimklPull;
    static ALL: [&'static dyn WatchSync; 1] = [&SIMKL];
    &ALL
}

/// Runs one import for Simkl with the app's own transport and credentials.
pub fn run_simkl(app: &AppPaths) -> Result<Report> {
    run(app, providers()[0])
}

/// [`run_simkl`] for any provider, with the app's own transport and credentials.
pub fn run(app: &AppPaths, sync: &dyn WatchSync) -> Result<Report> {
    // The baseline may carry episode rows: a larger ceiling than the 1 MiB default.
    let http = UreqTransport::with_limits(Duration::from_secs(60), 64 << 20);
    let credentials = StoreCredentials::new(app);
    let ctx = Ctx { http: &http, credentials: &credentials };
    run_with(app, sync, &ctx)
}

/// Whether a provider's next automatic pull is due.
pub fn due(cursor: &cursor::Cursor, now: DateTime<Utc>) -> bool {
    cursor.next_at.is_none_or(|at| at <= now)
}

/// How long to wait after `attempts` consecutive failures.
pub fn backoff(attempts: u32) -> chrono::Duration {
    let index = usize::try_from(attempts.saturating_sub(1)).unwrap_or(usize::MAX).min(BACKOFF_MINUTES.len() - 1);
    chrono::Duration::minutes(BACKOFF_MINUTES[index])
}

/// The schedule and the row's sentence, after one attempt.
pub fn record_result(cursor: &mut cursor::Cursor, result: &Result<Report>, now: DateTime<Utc>) {
    match result {
        Ok(report) => {
            cursor.attempts = 0;
            cursor.next_at = Some(now + INTERVAL);
            cursor.last = Some(cursor::LastImport {
                at: now,
                watched: report.watched,
                positions: report.positions,
                unchanged: report.unchanged,
                error: None,
            });
        }
        Err(error) => {
            cursor.attempts = cursor.attempts.saturating_add(1);
            cursor.next_at = Some(now + backoff(cursor.attempts));
            cursor.last =
                Some(cursor::LastImport { at: now, watched: 0, positions: 0, unchanged: false, error: Some(format!("{error:#}")) });
        }
    }
}

/// Every enabled provider whose pull is due, run once. Called from the app on a
/// timer, on opening Watch, and after a playback: never from a background service.
pub fn tick(app: &AppPaths) -> Vec<(&'static str, Result<Report>)> {
    let enabled = heronite_watch_state::settings::read(app).import_providers;
    let mut ran = Vec::new();
    for sync in providers() {
        if !enabled.iter().any(|id| id == sync.id()) {
            continue;
        }
        let stored = cursor::read(app, sync.id());
        if !due(&stored, Utc::now()) {
            continue;
        }
        let result = run(app, *sync);
        let now = Utc::now();
        // The schedule is kept even when the pull failed: a failure is a schedule
        // fact, not a reason to hammer the provider.
        let _ = cursor::update(app, sync.id(), |cursor| {
            record_result(cursor, &result, now);
            Ok(())
        });
        ran.push((sync.id(), result));
    }
    ran
}

/// [`run_simkl`] with an injected provider and transport, for tests.
pub fn run_with(app: &AppPaths, sync: &dyn WatchSync, ctx: &Ctx<'_>) -> Result<Report> {
    let provider = sync.id();
    let stored = cursor::read(app, provider);
    let now = sync.snapshot(ctx)?;
    let since = (!stored.snapshot.is_null()).then(|| stored.snapshot.clone());

    if let Some(since) = since.as_ref() {
        if sync.unchanged(since, &now) {
            // The cheap path: one request, and the pull did not even run.
            cursor::update(app, provider, |cursor| {
                cursor.last_pull_at = Some(Utc::now());
                Ok(())
            })?;
            return Ok(Report { unchanged: true, ..Report::default() });
        }
    }

    let pulled = sync.pull(ctx, since.as_ref())?;
    let report = apply(app, provider, &pulled, stored.last_pull_at)?;
    // Last, and only now: a failed apply leaves the cursor where it was, so the
    // next run asks for the same window again.
    cursor::update(app, provider, |cursor| {
        cursor.snapshot = now;
        cursor.last_pull_at = Some(Utc::now());
        Ok(())
    })?;
    Ok(report)
}

/// Writes what the pull brought into `movies.json`, with provenance.
fn apply(app: &AppPaths, provider: &str, pulled: &Pulled, last_pull_at: Option<DateTime<Utc>>) -> Result<Report> {
    let now = Utc::now();
    let mut report = Report { skipped: pulled.skipped, ..Report::default() };

    // The guards read the outbox, which is a different file: settled before the
    // movies lock is taken.
    let mut watches: Vec<&RemoteWatch> = Vec::new();
    for watch in &pulled.watches {
        if blocked(app, &watch.title, &watch.view) {
            report.blocked += 1;
        } else {
            watches.push(watch);
        }
    }
    let mut positions: Vec<&RemotePosition> = Vec::new();
    for position in &pulled.positions {
        if blocked(app, &position.title, &position.view) {
            report.blocked += 1;
        } else {
            positions.push(position);
        }
    }
    if watches.is_empty() && positions.is_empty() {
        return Ok(report);
    }

    let mut watched = 0usize;
    let mut placed = 0usize;
    let mut no_local = 0usize;
    let mut held = 0usize;
    store::update_movies(app, |movies| {
        for watch in &watches {
            let Some(imdb) = watch.title.imdb.as_deref() else {
                continue;
            };
            let Some(record) = find(movies, imdb) else {
                no_local += 1;
                continue;
            };
            // An episode's watched state comes with the extended pull; a film is
            // ours to finish.
            if !matches!(watch.view, TrackedView::Movie) {
                held += 1;
                continue;
            }
            if record.unwatched || record.finished {
                held += 1;
                continue;
            }
            record.finished = true;
            stamp(record, provider, now, watch.at);
            watched += 1;
        }
        for position in &positions {
            let Some(imdb) = position.title.imdb.as_deref() else {
                continue;
            };
            let Some(record) = find(movies, imdb) else {
                no_local += 1;
                continue;
            };
            if record.finished || record.unwatched || record.duration_ms == 0 {
                held += 1;
                continue;
            }
            // A position set here since the last pull is the person's, not the
            // provider's.
            if record.origin == Origin::Local && last_pull_at.is_some_and(|last| record.updated_at > last) {
                held += 1;
                continue;
            }
            if position.percent >= 80.0 {
                record.finished = true;
            } else {
                record.position_ms = (record.duration_ms as f64 * position.percent / 100.0).round().max(0.0) as u64;
            }
            stamp(record, provider, now, position.at);
            placed += 1;
        }
        Ok(())
    })?;

    report.watched = watched;
    report.positions = placed;
    report.no_local = no_local;
    report.blocked += held;
    Ok(report)
}

/// Whether the outbox or the echo window says "leave this title alone".
fn blocked(app: &AppPaths, title: &TrackedTitle, view: &TrackedView) -> bool {
    trackers::pending_for(app, title, view) || trackers::pushed_within(app, title, view, ECHO)
}

/// The record a remote fact is about: the local one carrying the same IMDb id.
fn find<'a>(movies: &'a mut heronite_watch_state::store::MoviesFile, imdb: &str) -> Option<&'a mut MovieProgress> {
    movies.progress.iter_mut().find(|record| record.imdb.as_deref() == Some(imdb))
}

/// Who wrote it, and when: provenance, never a change of behaviour.
fn stamp(record: &mut MovieProgress, provider: &str, now: DateTime<Utc>, remote: Option<DateTime<Utc>>) {
    record.origin = Origin::Imported;
    record.imported_from = Some(provider.to_owned());
    record.imported_at = Some(now);
    record.remote_updated_at = remote;
    record.updated_at = now;
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::{Credentials, Secret};
    use heronite_watch_core::tracker::{TrackEvent, TrackedWatch};
    use heronite_watch_state::tracking;
    use serde_json::Value;

    fn app(root: &std::path::Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    fn record(app: &AppPaths, imdb: &str, duration_ms: u64) {
        store::update_movies(app, |movies| {
            movies.progress.push(MovieProgress {
                provider: "stremio".into(),
                id: imdb.into(),
                duration_ms,
                imdb: Some(imdb.into()),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    }

    fn title(imdb: &str) -> TrackedTitle {
        TrackedTitle { imdb: Some(imdb.into()), kind: "movie".into(), title: "A Film".into(), year: None }
    }

    fn watch(imdb: &str) -> RemoteWatch {
        RemoteWatch { title: title(imdb), view: TrackedView::Movie, at: Some(Utc::now()) }
    }

    fn position(imdb: &str, percent: f64) -> RemotePosition {
        RemotePosition { title: title(imdb), view: TrackedView::Movie, percent, at: Some(Utc::now()) }
    }

    fn stored(app: &AppPaths, imdb: &str) -> MovieProgress {
        store::read_movies(app).progress.into_iter().find(|record| record.imdb.as_deref() == Some(imdb)).expect("the record")
    }

    #[test]
    fn a_pull_writes_a_watch_and_a_position_with_provenance() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt1", 6_000_000);
        record(&app, "tt2", 6_000_000);
        let pulled = Pulled { watches: vec![watch("tt1")], positions: vec![position("tt2", 40.0)], skipped: 2 };

        let report = apply(&app, "simkl", &pulled, None).unwrap();
        assert_eq!((report.watched, report.positions, report.skipped), (1, 1, 2), "{report:?}");
        let watched = stored(&app, "tt1");
        assert!(watched.finished);
        assert_eq!(watched.origin, Origin::Imported);
        assert_eq!(watched.imported_from.as_deref(), Some("simkl"));
        assert!(!watched.unwatched, "an imported mark is a mark, not a veto");
        let placed = stored(&app, "tt2");
        assert_eq!(placed.position_ms, 2_400_000, "40 % of the local length");
        assert!(!placed.finished);
    }

    #[test]
    fn an_unwatched_marker_wins_over_the_provider() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt1", 6_000_000);
        store::update_movies(&app, |movies| {
            movies.progress[0].unwatched = true;
            Ok(())
        })
        .unwrap();
        let report = apply(&app, "simkl", &Pulled { watches: vec![watch("tt1")], ..Default::default() }, None).unwrap();
        assert_eq!(report.watched, 0);
        assert_eq!(report.blocked, 1);
        assert!(!stored(&app, "tt1").finished);
    }

    #[test]
    fn a_finished_record_is_not_downgraded_by_a_position() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt1", 6_000_000);
        store::update_movies(&app, |movies| {
            movies.progress[0].finished = true;
            Ok(())
        })
        .unwrap();
        let report = apply(&app, "simkl", &Pulled { positions: vec![position("tt1", 30.0)], ..Default::default() }, None).unwrap();
        assert_eq!(report.positions, 0);
        assert_eq!(report.blocked, 1);
        assert!(stored(&app, "tt1").finished);
    }

    #[test]
    fn a_title_the_library_does_not_have_is_counted_not_created() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        let report = apply(&app, "simkl", &Pulled { watches: vec![watch("tt9")], ..Default::default() }, None).unwrap();
        assert_eq!(report.no_local, 1);
        assert!(store::read_movies(&app).progress.is_empty(), "no record is invented yet");
    }

    #[test]
    fn a_pending_push_blocks_the_title() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt1", 6_000_000);
        tracking::enqueue(
            &app,
            TrackEvent { seq: 0, at: Utc::now(), title: title("tt1"), view: TrackedView::Movie, watch: TrackedWatch::Watched },
        )
        .unwrap();
        let report = apply(&app, "simkl", &Pulled { watches: vec![watch("tt1")], ..Default::default() }, None).unwrap();
        assert_eq!(report.blocked, 1, "{report:?}");
        assert!(!stored(&app, "tt1").finished);
    }

    #[test]
    fn our_own_echo_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt1", 6_000_000);
        tracking::update(&app, |file| {
            let event = TrackEvent { seq: 1, at: Utc::now(), title: title("tt1"), view: TrackedView::Movie, watch: TrackedWatch::Watched };
            tracking::record_pushed(file, &event, Utc::now());
            Ok(())
        })
        .unwrap();
        let report = apply(&app, "simkl", &Pulled { positions: vec![position("tt1", 40.0)], ..Default::default() }, None).unwrap();
        assert_eq!(report.blocked, 1, "{report:?}");
        assert_eq!(stored(&app, "tt1").position_ms, 0);
    }

    #[test]
    fn a_local_position_newer_than_the_pull_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt1", 6_000_000);
        store::update_movies(&app, |movies| {
            movies.progress[0].position_ms = 4_800_000;
            movies.progress[0].updated_at = Utc::now();
            Ok(())
        })
        .unwrap();
        let last_pull = Utc::now() - chrono::Duration::hours(3);
        let report =
            apply(&app, "simkl", &Pulled { positions: vec![position("tt1", 40.0)], ..Default::default() }, Some(last_pull)).unwrap();
        assert_eq!(report.blocked, 1);
        assert_eq!(stored(&app, "tt1").position_ms, 4_800_000);
    }

    struct FakeSync {
        snapshot: Value,
        pulled: Pulled,
    }

    impl WatchSync for FakeSync {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn snapshot(&self, _: &Ctx<'_>) -> Result<Value> {
            Ok(self.snapshot.clone())
        }
        fn unchanged(&self, since: &Value, now: &Value) -> bool {
            since == now
        }
        fn pull(&self, _: &Ctx<'_>, _: Option<&Value>) -> Result<Pulled> {
            Ok(self.pulled.clone())
        }
    }

    struct NoNet;
    impl Transport for NoNet {
        fn get(&self, _: &url::Url) -> Result<Vec<u8>> {
            unreachable!()
        }
        fn post_json(&self, _: &url::Url, _: &Value) -> Result<Vec<u8>> {
            unreachable!()
        }
    }
    struct NoCreds;
    impl Credentials for NoCreds {
        fn get(&self, _: &str) -> Result<Option<Secret>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_cursor_without_a_schedule_is_due_and_a_failure_backs_off() {
        let mut cursor = cursor::Cursor::default();
        let now = Utc::now();
        assert!(due(&cursor, now), "never pulled is due");

        record_result(&mut cursor, &Ok(Report { watched: 1, positions: 2, ..Report::default() }), now);
        assert!(!due(&cursor, now), "a success schedules the next one");
        assert!(due(&cursor, now + INTERVAL));
        let last = cursor.last.clone().expect("a result is kept");
        assert_eq!((last.watched, last.positions, last.unchanged), (1, 2, false));

        record_result(&mut cursor, &Err(anyhow::anyhow!("network down")), now);
        assert_eq!(cursor.attempts, 1);
        assert_eq!(cursor.next_at, Some(now + backoff(1)));
        assert!(cursor.last.as_ref().unwrap().error.as_deref().is_some_and(|error| error.contains("network down")));

        record_result(&mut cursor, &Err(anyhow::anyhow!("again")), now);
        assert_eq!(cursor.next_at, Some(now + backoff(2)));
        assert!(backoff(2) > backoff(1), "the ladder grows");
        record_result(&mut cursor, &Ok(Report::default()), now);
        assert_eq!(cursor.attempts, 0, "a success resets the ladder");
    }

    #[test]
    fn the_same_snapshot_stops_after_one_request_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt1", 6_000_000);
        let sync = FakeSync {
            snapshot: serde_json::json!({"all": "2026-01-01T00:00:00Z"}),
            pulled: Pulled { watches: vec![watch("tt1")], ..Default::default() },
        };
        let credentials = NoCreds;
        let ctx = Ctx { http: &NoNet, credentials: &credentials };

        let first = run_with(&app, &sync, &ctx).unwrap();
        assert_eq!(first.watched, 1, "{first:?}");
        assert!(!first.unchanged);
        assert!(stored(&app, "tt1").finished, "the first pull wrote");

        // A pulled watch makes the next tick see a different picture only through
        // its snapshot; here the snapshot is identical, so nothing runs.
        let second = run_with(&app, &sync, &ctx).unwrap();
        assert!(second.unchanged, "{second:?}");
        assert_eq!(second.watched, 0);
        let cursor = cursor::read(&app, "fake");
        assert_eq!(cursor.snapshot, sync.snapshot, "the snapshot was saved");
        assert!(cursor.last_pull_at.is_some());
    }
}
