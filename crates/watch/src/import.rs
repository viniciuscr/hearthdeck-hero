//! Applying a pull to the local records.
//!
//! The local files stay the truth: an import writes into them **with provenance**,
//! never un-finishes a record, never touches an `unwatched` marker, never races the
//! outbox, and never overwrites a position the person set here since the last pull.
//! The provider's snapshot is saved last, and only if every write landed.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};

use heronite_data::paths::AppPaths;
use heronite_watch_core::http::UreqTransport;
use heronite_watch_core::personal::{ListDecision, Membership, FAVOURITES, WATCHLIST};
use heronite_watch_core::provider::Ctx;
use heronite_watch_core::pull::{Pulled, RemoteMembership, RemotePosition, RemoteWatch, WatchSync};
use heronite_watch_core::sync_rules::{reconcile, Action, Local, Remote, Seen};
use heronite_watch_core::title::IMPORTED_ADDON;
use heronite_watch_core::tracker::{Delivery, ListChange, TrackedTitle, TrackedView};
use heronite_watch_core::types::{EpisodeProgress, MovieItem, MovieProgress, Origin};
use heronite_watch_simkl::pull::SimklPull;
use heronite_watch_state::import as cursor;
use heronite_watch_state::placeholders::{self, Placeholder};
use heronite_watch_state::sessions::{self, WatchSession};
use heronite_watch_state::store;
use heronite_watch_state::store::StoreCredentials;

use crate::trackers;

/// How long after a delivery our own write is still "not news".
const ECHO: chrono::Duration = chrono::Duration::minutes(5);

/// What the current pull asks for. Bumped when a pull's shape changes, so a cursor
/// written by an older build is refetched as a baseline: a snapshot sends only
/// deltas, and the rows a previous build skipped would never arrive otherwise.
pub const CURSOR_SCHEMA: u32 = 1;

/// What one import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub watched: usize,
    pub positions: usize,
    /// Provider items this build cannot use (no IMDb id, a show-level fact).
    pub skipped: usize,
    /// Titles the provider knows and the library does not: they arrive as greyed
    /// placeholders, and the count is here so a screen can say how many.
    pub no_local: usize,
    /// Things left alone on purpose: unwatched markers, finished records, a
    /// pending push, our own echo, a local position newer than the pull.
    pub blocked: usize,
    /// The change query said nothing moved: no pull, no writes.
    pub unchanged: bool,
    /// List membership applied here: added, removed, and what a local change
    /// still owes the providers (`lists_push`, the send side).
    pub lists_added: usize,
    pub lists_removed: usize,
    pub lists_push: usize,
    /// What a tracker took during this run. A change that was not taken stays owed
    /// and is derived again by the next tick, so there is no second outbox.
    pub lists_delivered: usize,
    /// The list changes this pull derived and still owes: delivered by [`run`].
    #[doc(hidden)]
    pub pushes: Vec<ListChange>,
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
    let mut report = run_with(app, sync, &ctx)?;
    // The list changes the reconciliation derived, delivered here rather than in
    // an outbox: a failure stays owed (the ledger still says `in`/`out`) and the
    // next tick derives it again.
    for change in std::mem::take(&mut report.pushes) {
        for (tracker, delivery) in crate::trackers::deliver_list(app, &change) {
            match delivery {
                Delivery::Delivered => report.lists_delivered += 1,
                Delivery::Retry { reason, .. } => log::warn!("{tracker} list change: {reason}"),
                Delivery::Unmatched(reason) => log::info!("{tracker} list change: {reason}"),
            }
        }
    }
    Ok(report)
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
                skipped: report.skipped,
                placeholders: report.no_local,
                error: None,
            });
        }
        Err(error) => {
            cursor.attempts = cursor.attempts.saturating_add(1);
            cursor.next_at = Some(now + backoff(cursor.attempts));
            cursor.last = Some(cursor::LastImport {
                at: now,
                watched: 0,
                positions: 0,
                unchanged: false,
                skipped: 0,
                placeholders: 0,
                error: Some(format!("{error:#}")),
            });
        }
    }
}

/// How a puller is called on a screen.
pub fn name(id: &str) -> &'static str {
    providers().iter().find(|sync| sync.id() == id).map(|sync| sync.name()).unwrap_or("Provider")
}

/// The pullers the person turned on.
pub fn enabled(app: &AppPaths) -> Vec<&'static str> {
    let ids = heronite_watch_state::settings::read(app).import_providers;
    providers().iter().filter(|sync| ids.iter().any(|id| id == sync.id())).map(|sync| sync.id()).collect()
}

/// Turns one provider's automatic pull on or off.
pub fn set_enabled(app: &AppPaths, provider: &str, on: bool) -> Result<()> {
    let mut settings = heronite_watch_state::settings::read(app);
    settings.import_providers.retain(|id| id != provider);
    if on {
        settings.import_providers.push(provider.to_owned());
        // On means "bring what happened", not "resume where a previous pull
        // stopped": the whole history is fetched once, then the schedule keeps its
        // deltas. It is also the fix for anything a past pull got wrong.
        cursor::reset_baseline(app, provider)?;
    }
    heronite_watch_state::settings::write(app, &settings)?;
    log::info!("{provider}: import {} (the next pull fetches the whole history)", if on { "on" } else { "off" });
    Ok(())
}

/// What the last pull did, for the settings row.
pub fn status(app: &AppPaths, provider: &str) -> Option<cursor::LastImport> {
    cursor::read(app, provider).last
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
            // A tick that decides not to ask says so: "no logs" must never be the
            // same trace as "the schedule was silent about skipping".
            match stored.next_at {
                Some(at) => log::info!("{}: not due yet, next pull at {}", sync.id(), at.to_rfc3339()),
                None => log::info!("{}: not due yet", sync.id()),
            }
            continue;
        }
        log::info!("{}: pulling now", sync.id());
        let result = run(app, *sync);
        if let Err(error) = &result {
            log::warn!("{}: the pull failed: {error:#}", sync.id());
        }
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
    let stale = stored.schema < CURSOR_SCHEMA;
    if stale && !stored.snapshot.is_null() {
        log::info!("{provider}: the saved cursor predates this build's pull; fetching the whole history once");
    }
    let since = (!stale && !stored.snapshot.is_null()).then(|| stored.snapshot.clone());

    match since.as_ref() {
        Some(stamp) => {
            log::info!("{provider}: pulling changes since {}", stamp.get("all").and_then(|all| all.as_str()).unwrap_or("the last pull"))
        }
        None => log::info!("{provider}: pulling the whole history (a baseline)"),
    }

    if let Some(since) = since.as_ref() {
        if sync.unchanged(since, &now) {
            log::info!("{provider}: nothing moved since the last pull; not fetching anything");
            // The cheap path: one request, and the pull did not even run.
            cursor::update(app, provider, |cursor| {
                cursor.last_pull_at = Some(Utc::now());
                Ok(())
            })?;
            return Ok(Report { unchanged: true, ..Report::default() });
        }
    }

    let pulled = sync.pull(ctx, since.as_ref())?;
    let (report, seen) = apply(app, provider, &pulled, stored.last_pull_at, &stored.list_ids)?;
    log::info!(
        "{provider} pull: {} watched, {} positions, {} skipped, {} added to a list, {} removed, {} list changes delivered",
        report.watched,
        report.positions,
        report.skipped,
        report.lists_added,
        report.lists_removed,
        report.lists_delivered
    );
    // Last, and only now: a failed apply leaves the cursor where it was, so the
    // next run asks for the same window again.
    cursor::update(app, provider, |cursor| {
        cursor.schema = CURSOR_SCHEMA;
        cursor.snapshot = now;
        // What the lists hold *now*: the base the next removal is found against.
        cursor.list_ids = seen;
        cursor.last_pull_at = Some(Utc::now());
        Ok(())
    })?;
    Ok(report)
}

/// Writes what the pull brought into `movies.json` and `watch-personal.json`,
/// with provenance, and says what each list holds now.
fn apply(
    app: &AppPaths,
    provider: &str,
    pulled: &Pulled,
    last_pull_at: Option<DateTime<Utc>>,
    last_seen: &BTreeMap<String, Vec<String>>,
) -> Result<(Report, BTreeMap<String, Vec<String>>)> {
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
    let (added, removed, pushes, seen) = apply_lists(app, provider, pulled, last_seen, now)?;
    report.lists_added = added;
    report.lists_removed = removed;
    report.lists_push = pushes.len();
    report.pushes = pushes;

    if watches.is_empty() && positions.is_empty() {
        return Ok((report, seen));
    }

    let mut watched = 0usize;
    let mut placed = 0usize;
    let mut held = 0usize;
    // Titles the library does not have, and titles it just gained: written after
    // the movies lock is released (a different file).
    let mut missing: Vec<(String, String, String, Option<String>)> = Vec::new();
    let mut present: Vec<(String, String)> = Vec::new();
    // A pull is a playback this machine missed. Each watched title also becomes a
    // session, so the recap and anything else reading sessions sees it like a watch
    // made here. `covered` is what a session already holds: a re-pull adds none.
    let mut covered: std::collections::HashSet<(String, Option<String>)> =
        sessions::read(app).sessions.iter().map(|session| (session.id.clone(), session.video_id.clone())).collect();
    let mut played: Vec<WatchSession> = Vec::new();

    store::update_movies(app, |movies| {
        for watch in &watches {
            let Some(imdb) = watch.title.imdb.as_deref() else {
                continue;
            };
            let (kind, view) = view_of(&watch.view);
            let Some(record) = find(movies, imdb) else {
                // The library does not have this title, but the watch is still a
                // fact: a placeholder draws the title, and an episode record is what
                // makes its episodes read as watched when the page opens. Without
                // it a whole imported series was invisible.
                log::info!("{provider}: {} has no record here yet; kept as a placeholder too", watch.title.title);
                missing.push((imdb.to_owned(), kind, watch.title.title.clone(), watch.title.year.clone()));
                if let TrackedView::Episode { season, number } = view {
                    let mut record = MovieProgress {
                        provider: media_provider(),
                        id: imdb.to_owned(),
                        imdb: Some(imdb.to_owned()),
                        updated_at: now,
                        ..MovieProgress::default()
                    };
                    record.episodes.insert(
                        format!("{imdb}:{season}:{number}"),
                        EpisodeProgress {
                            season: Some(season),
                            number: Some(number),
                            finished: true,
                            updated_at: now,
                            origin: Origin::Imported,
                            imported_from: Some(provider.to_owned()),
                            imported_at: Some(now),
                            remote_updated_at: watch.at,
                            ..EpisodeProgress::default()
                        },
                    );
                    log::info!("{provider}: {} S{season}E{number} watched (its first record here)", watch.title.title);
                    movies.progress.push(record);
                    note_played(&mut covered, &mut played, watch, now);
                    watched += 1;
                }
                continue;
            };
            present.push((imdb.to_owned(), kind));
            // A title marked "not watched" here stays that way: the person said so.
            if record.unwatched {
                log::info!("{provider}: {} left alone (marked not watched here)", watch.title.title);
                held += 1;
                continue;
            }
            match view {
                TrackedView::Movie => {
                    if record.finished {
                        log::info!("{provider}: {} left alone (already watched here)", watch.title.title);
                        note_played(&mut covered, &mut played, watch, now);
                        held += 1;
                        continue;
                    }
                    record.finished = true;
                    stamp(record, provider, now, watch.at);
                    log::info!("{provider}: {} watched", watch.title.title);
                    note_played(&mut covered, &mut played, watch, now);
                    watched += 1;
                }
                TrackedView::Episode { season, number } => {
                    // A series keeps one record per episode. Playback keys them by
                    // the provider's episode id, so an entry is matched by its own
                    // season and number before a new one is made.
                    let key = record
                        .episodes
                        .iter()
                        .find(|(_, episode)| episode.season == Some(season) && episode.number == Some(number))
                        .map(|(key, _)| key.clone())
                        // The same shape every other writer uses (`tt1:1:2`), so a
                        // later local play of this episode finds this entry instead
                        // of adding a second one.
                        .unwrap_or_else(|| format!("{imdb}:{season}:{number}"));
                    let episode = record.episodes.entry(key).or_default();
                    if episode.unwatched || episode.finished {
                        if episode.finished && !episode.unwatched {
                            note_played(&mut covered, &mut played, watch, now);
                        }
                        held += 1;
                        continue;
                    }
                    episode.season = Some(season);
                    episode.number = Some(number);
                    episode.finished = true;
                    // Ours, like the film path: when this machine learned it. The
                    // provider's own clock lives in `remote_updated_at`.
                    episode.updated_at = now;
                    episode.origin = Origin::Imported;
                    episode.imported_from = Some(provider.to_owned());
                    episode.imported_at = Some(now);
                    episode.remote_updated_at = watch.at;
                    log::info!("{provider}: {} S{season}E{number} watched", watch.title.title);
                    note_played(&mut covered, &mut played, watch, now);
                    watched += 1;
                }
            }
        }
        for position in &positions {
            let Some(imdb) = position.title.imdb.as_deref() else {
                continue;
            };
            let (kind, _) = view_of(&position.view);
            let Some(record) = find(movies, imdb) else {
                missing.push((imdb.to_owned(), kind, position.title.title.clone(), position.title.year.clone()));
                continue;
            };
            present.push((imdb.to_owned(), kind));
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

    // A title no source here has becomes a library entry of its own: it appears
    // where titles appear, greyed, and leaves when the library gains it.
    for session in played {
        if let Err(error) = sessions::append(app, session) {
            log::warn!("import: keeping a session of what was imported: {error:#}");
        }
    }

    for (imdb, kind, title, year) in missing {
        placeholders::upsert(app, Placeholder { imdb, kind, title, year, provider: provider.to_owned(), first_seen: Some(now) })?;
        report.no_local += 1;
    }
    for (imdb, kind) in present {
        placeholders::remove(app, &imdb, &kind)?;
    }

    report.watched = watched;
    report.positions = placed;
    report.blocked += held;
    Ok((report, seen))
}

/// Reconciles each list against the ledger with the three-way table
/// (`docs/WATCH-SYNC-RULES.md`): the local decision, what the last tick saw the
/// provider holding, and what it holds now. Absence alone never removes.
fn apply_lists(
    app: &AppPaths,
    provider: &str,
    pulled: &Pulled,
    last_seen: &BTreeMap<String, Vec<String>>,
    now: DateTime<Utc>,
) -> Result<(usize, usize, Vec<ListChange>, BTreeMap<String, Vec<String>>)> {
    let mut library = heronite_watch_state::personal::read(app);
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut pushes: Vec<ListChange> = Vec::new();

    // What the provider says each list holds now.
    let mut remote: BTreeMap<String, Vec<&RemoteMembership>> = BTreeMap::new();
    for membership in &pulled.memberships {
        if membership.member {
            remote.entry(membership.list.clone()).or_default().push(membership);
        }
    }
    // Both kinds, always: a local entry the provider has never mentioned is a
    // change to push, so the loop cannot depend on what was reported.
    let mut held_now: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for kind in [WATCHLIST, FAVOURITES] {
        let there: Vec<&RemoteMembership> = remote.get(kind).cloned().unwrap_or_default();
        let was: Vec<String> = last_seen.get(kind).cloned().unwrap_or_default();

        let mut now_ids: Vec<String> = there.iter().filter_map(|m| m.title.imdb.clone()).collect();
        now_ids.sort();
        now_ids.dedup();

        // Everything either side knows about, plus what the local list holds (a
        // local add with nothing remote must be pushed, not dropped).
        let mut universe: BTreeSet<String> = now_ids.iter().cloned().collect();
        universe.extend(was.iter().cloned());
        for item in local_items(&library, kind) {
            universe.insert(item.id.clone());
        }

        for imdb in universe {
            let local = match library.membership_any(kind, &imdb) {
                Some(Membership::In) => Local::In,
                Some(Membership::Out) => Local::Out,
                None => Local::Unset,
            };
            let seen = if was.iter().any(|id| id == &imdb) { Seen::In } else { Seen::Absent };
            let is = if now_ids.iter().any(|id| id == &imdb) { Remote::In } else { Remote::Absent };
            let title = there.iter().find(|m| m.title.imdb.as_deref() == Some(imdb.as_str())).map(|m| m.title.clone());

            let decision = |state: Membership| ListDecision {
                state,
                changed_at: now,
                origin: Origin::Imported,
                imported_from: Some(provider.to_owned()),
            };
            let action = reconcile(local, seen, is);
            match action {
                Action::AddLocal => {
                    let Some(title) = title else { continue };
                    // The kind decides which shelf: a series from the provider's
                    // favourites is a favourite here, not a watchlist entry.
                    match kind {
                        FAVOURITES => library.add_favourite(&list_item(&title)),
                        _ => library.add_watchlist(&list_item(&title)),
                    };
                    library.decide(kind, "", &imdb, decision(Membership::In));
                    added += 1;
                }
                Action::RemoveLocal => {
                    // By id: the entry a sync removes may have been kept with
                    // another kind or no provider at all.
                    if library.remove_any(kind, &imdb) {
                        removed += 1;
                    }
                    library.decide(kind, "", &imdb, decision(Membership::Out));
                }
                // A local decision the provider has not been told about yet.
                Action::PushAdd | Action::PushRemove => {
                    let target = title.clone().or_else(|| local_tracked(&library, kind, &imdb));
                    if let Some(target) = target {
                        pushes.push(ListChange { title: target, list: kind.to_owned(), member: matches!(action, Action::PushAdd) });
                    }
                }
                // Both moved: the local state wins until the send side exists.
                Action::Conflict | Action::None => {}
            }
        }
        held_now.insert(kind.to_owned(), now_ids);
    }

    if added + removed > 0 {
        heronite_watch_state::personal::write(app, &library)?;
    }
    Ok((added, removed, pushes, held_now))
}

/// The id the media provider uses: a record or session written here belongs to it,
/// exactly as if the title had been played in this app.
fn media_provider() -> String {
    crate::registry::providers().first().map(|provider| provider.id().to_owned()).unwrap_or_default()
}

/// The episode key every writer uses (`tt1:1:2`); `None` for a film.
fn video_id_of(view: &TrackedView, imdb: &str) -> Option<String> {
    match view {
        TrackedView::Movie => None,
        TrackedView::Episode { season, number } => Some(format!("{imdb}:{season}:{number}")),
    }
}

/// A watched fact as the session a playback would have written.
fn session_of(watch: &RemoteWatch, now: DateTime<Utc>) -> WatchSession {
    let imdb = watch.title.imdb.clone().unwrap_or_default();
    let at = watch.at.unwrap_or(now);
    let (video_id, season, number, title) = match watch.view {
        TrackedView::Movie => (None, None, None, watch.title.title.clone()),
        TrackedView::Episode { season, number } => {
            (Some(format!("{imdb}:{season}:{number}")), Some(season), Some(number), format!("S{season}E{number} · {}", watch.title.title))
        }
    };
    WatchSession {
        provider: media_provider(),
        kind: if watch.title.kind == "series" { "series".into() } else { "movie".into() },
        id: imdb,
        video_id,
        season,
        number,
        name: watch.title.title.clone(),
        year: watch.title.year.clone(),
        title,
        started_at: at,
        ended_at: at,
        // How long it ran is the provider's business and it does not say: the
        // session records that it happened, which is what a recap counts.
        elapsed_seconds: 0.0,
        position_from_ms: 0,
        position_to_ms: 0,
        duration_ms: 0,
        finished: true,
    }
}

/// Records the session for a watched fact, once: `covered` is what is already there.
fn note_played(
    covered: &mut std::collections::HashSet<(String, Option<String>)>,
    played: &mut Vec<WatchSession>,
    watch: &RemoteWatch,
    now: DateTime<Utc>,
) {
    let imdb = watch.title.imdb.clone().unwrap_or_default();
    if covered.insert((imdb.clone(), video_id_of(&watch.view, &imdb))) {
        played.push(session_of(watch, now));
    }
}

/// A local list entry as a tracker's title: what a push is about when the
/// provider has never mentioned it.
fn local_tracked(library: &heronite_watch_core::personal::PersonalLibrary, kind: &str, imdb: &str) -> Option<TrackedTitle> {
    local_items(library, kind).iter().find(|item| item.id == imdb).map(|item| TrackedTitle {
        imdb: Some(item.id.clone()),
        kind: item.kind.clone(),
        title: item.name.clone(),
        year: item.year.clone(),
    })
}

/// The items a local list holds.
fn local_items<'a>(library: &'a heronite_watch_core::personal::PersonalLibrary, kind: &str) -> &'a [MovieItem] {
    match kind {
        FAVOURITES => &library.favourites,
        _ => &library.watchlist,
    }
}

/// A list entry for a title, marked as imported: it is a title we may not have,
/// so it travels as a placeholder and the tiles treat it as unavailable.
fn list_item(title: &TrackedTitle) -> MovieItem {
    MovieItem {
        id: title.imdb.clone().unwrap_or_default(),
        kind: if title.kind == "series" { "series".into() } else { "movie".into() },
        name: title.title.clone(),
        year: title.year.clone(),
        provider: String::new(),
        addon: IMPORTED_ADDON.to_owned(),
        ..MovieItem::default()
    }
}

/// The library kind and the view a remote fact carries.
fn view_of(view: &TrackedView) -> (String, TrackedView) {
    let kind = match view {
        TrackedView::Movie => "movie",
        TrackedView::Episode { .. } => "series",
    };
    (kind.to_owned(), view.clone())
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
        let pulled = Pulled { watches: vec![watch("tt1")], positions: vec![position("tt2", 40.0)], skipped: 2, ..Pulled::default() };

        let report = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap().0;
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
        let report = apply(&app, "simkl", &Pulled { watches: vec![watch("tt1")], ..Default::default() }, None, &BTreeMap::new()).unwrap().0;
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
        let report = apply(&app, "simkl", &Pulled { positions: vec![position("tt1", 30.0)], ..Default::default() }, None, &BTreeMap::new())
            .unwrap()
            .0;
        assert_eq!(report.positions, 0);
        assert_eq!(report.blocked, 1);
        assert!(stored(&app, "tt1").finished);
    }

    #[test]
    fn a_title_the_library_does_not_have_becomes_a_placeholder() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        let report = apply(&app, "simkl", &Pulled { watches: vec![watch("tt9")], ..Default::default() }, None, &BTreeMap::new()).unwrap().0;
        assert_eq!(report.no_local, 1);
        assert!(store::read_movies(&app).progress.is_empty(), "no watch record is invented");
        let placeholders = placeholders::read(&app).items;
        assert_eq!(placeholders.len(), 1);
        assert_eq!(placeholders[0].imdb, "tt9");
        assert_eq!(placeholders[0].provider, "simkl");
    }

    #[test]
    fn a_placeholder_leaves_when_the_library_gains_the_title() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        apply(&app, "simkl", &Pulled { watches: vec![watch("tt9")], ..Default::default() }, None, &BTreeMap::new()).unwrap();
        assert_eq!(placeholders::read(&app).items.len(), 1);

        record(&app, "tt9", 6_000_000);
        apply(&app, "simkl", &Pulled { watches: vec![watch("tt9")], ..Default::default() }, None, &BTreeMap::new()).unwrap();
        assert!(placeholders::read(&app).items.is_empty(), "the library has it now");
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
        let report = apply(&app, "simkl", &Pulled { watches: vec![watch("tt1")], ..Default::default() }, None, &BTreeMap::new()).unwrap().0;
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
        let report = apply(&app, "simkl", &Pulled { positions: vec![position("tt1", 40.0)], ..Default::default() }, None, &BTreeMap::new())
            .unwrap()
            .0;
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
        let report = apply(
            &app,
            "simkl",
            &Pulled { positions: vec![position("tt1", 40.0)], ..Default::default() },
            Some(last_pull),
            &BTreeMap::new(),
        )
        .unwrap()
        .0;
        assert_eq!(report.blocked, 1);
        assert_eq!(stored(&app, "tt1").position_ms, 4_800_000);
    }

    fn episode(imdb: &str, season: u32, number: u32) -> RemoteWatch {
        RemoteWatch { title: title(imdb), view: TrackedView::Episode { season, number }, at: Some(Utc::now()) }
    }

    #[test]
    fn an_imported_episode_is_a_session_as_if_played_here() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        let pulled = Pulled { watches: vec![episode("tt9", 2, 4)], ..Default::default() };
        let _ = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();

        let sessions = heronite_watch_state::sessions::read(&app).sessions;
        assert_eq!(sessions.len(), 1, "a session, like any playback writes");
        let session = &sessions[0];
        assert!(session.finished);
        assert_eq!(session.id, "tt9");
        assert_eq!(session.video_id.as_deref(), Some("tt9:2:4"), "numbered like every other writer");
        let media = crate::registry::providers().first().map(|provider| provider.id()).unwrap_or("");
        assert_eq!(session.provider, media, "the media provider's, not a world of its own");
    }

    #[test]
    fn a_second_pull_adds_no_second_session() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        let pulled = Pulled { watches: vec![episode("tt9", 2, 4)], ..Default::default() };
        let _ = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();
        let _ = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();

        assert_eq!(heronite_watch_state::sessions::read(&app).sessions.len(), 1, "counted once");
    }

    #[test]
    fn a_show_the_library_lacks_still_keeps_its_episodes() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        // Nothing recorded for this show, and no rail carrying it either.
        let pulled = Pulled { watches: vec![episode("tt9", 3, 7)], ..Default::default() };
        let (report, _) = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();
        assert_eq!(report.watched, 1, "{report:?}");
        assert_eq!(report.no_local, 1, "and the title is a placeholder");

        let record = stored(&app, "tt9");
        let entry = record.episodes.values().next().expect("the episode is a record");
        assert_eq!((entry.season, entry.number), (Some(3), Some(7)));
        assert!(entry.finished, "a series nobody here has can still be partly watched");
    }

    #[test]
    fn an_episode_watched_elsewhere_finishes_here() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt9", 6_000_000);

        let pulled = Pulled { watches: vec![episode("tt9", 2, 4)], ..Default::default() };
        let (report, _) = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();
        assert_eq!(report.watched, 1, "{report:?}");

        let stored = stored(&app, "tt9");
        let key = stored.episodes.keys().next().expect("one episode");
        assert_eq!(key, "tt9:2:4", "the key other writers use");
        let entry = stored.episodes.values().next().expect("one episode");
        assert_eq!((entry.season, entry.number), (Some(2), Some(4)), "which episode it was");
        assert!(entry.finished, "watched elsewhere, watched here");
        assert_eq!(entry.origin, Origin::Imported);
        assert_eq!(entry.imported_from.as_deref(), Some("simkl"));
    }

    #[test]
    fn an_episode_already_finished_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        record(&app, "tt9", 6_000_000);
        let pulled = Pulled { watches: vec![episode("tt9", 2, 4)], ..Default::default() };
        let _ = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();

        let (second, _) = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();
        assert_eq!(second.watched, 0, "nothing new to record");
        assert_eq!(second.blocked, 1, "counted as held, not rewritten");
        assert_eq!(stored(&app, "tt9").episodes.len(), 1, "still one entry");
    }

    fn membership(imdb: &str) -> RemoteMembership {
        RemoteMembership { title: title(imdb), list: "watchlist".into(), member: true }
    }

    fn watchlist(app: &AppPaths) -> Vec<String> {
        heronite_watch_state::personal::read(app).watchlist.iter().map(|item| item.id.clone()).collect()
    }

    fn keep_local(app: &AppPaths, imdb: &str) {
        let mut library = heronite_watch_state::personal::read(app);
        library.add_watchlist(&MovieItem { id: imdb.into(), kind: "movie".into(), name: "A Film".into(), ..MovieItem::default() });
        heronite_watch_state::personal::write(app, &library).unwrap();
    }

    #[test]
    fn a_title_added_on_the_phone_enters_the_watchlist() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        let pulled = Pulled { memberships: vec![membership("tt9")], ..Default::default() };
        let (report, seen) = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();
        assert_eq!(report.lists_added, 1, "{report:?}");
        assert_eq!(watchlist(&app), ["tt9"]);
        assert_eq!(seen.get("watchlist"), Some(&vec!["tt9".to_owned()]), "the tick remembers what it saw");
    }

    #[test]
    fn a_favourite_lands_on_the_favourites_shelf() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        let pulled = Pulled {
            memberships: vec![RemoteMembership { title: title("tt9"), list: "favourites".into(), member: true }],
            ..Default::default()
        };
        let _ = apply(&app, "simkl", &pulled, None, &BTreeMap::new()).unwrap();

        let library = heronite_watch_state::personal::read(&app);
        assert_eq!(library.favourites.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["tt9"]);
        assert!(library.watchlist.is_empty(), "a favourite is not a watchlist entry");
    }

    #[test]
    fn a_title_removed_on_the_phone_leaves_the_watchlist() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        keep_local(&app, "tt1");
        let seen = BTreeMap::from([("watchlist".to_owned(), vec!["tt1".to_owned()])]);

        let (report, after) = apply(&app, "simkl", &Pulled::default(), None, &seen).unwrap();
        assert_eq!(report.lists_removed, 1, "{report:?}");
        assert!(watchlist(&app).is_empty(), "gone here too");
        assert!(after.get("watchlist").is_none_or(|ids| ids.is_empty()));
    }

    #[test]
    fn a_title_added_here_and_absent_there_is_pushed_not_removed() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        keep_local(&app, "tt1");

        let (report, _) = apply(&app, "simkl", &Pulled::default(), None, &BTreeMap::new()).unwrap();
        assert_eq!(report.lists_push, 1, "{report:?}");
        assert_eq!(watchlist(&app), ["tt1"], "absence is not a removal");
        assert_eq!(report.pushes.len(), 1, "the change is owed to the provider");
        assert_eq!(report.pushes[0].list, "watchlist");
        assert!(report.pushes[0].member, "an add, not a removal");
        assert_eq!(report.pushes[0].title.imdb.as_deref(), Some("tt1"), "the local entry names it");
    }

    /// A puller that remembers the window it was asked for.
    struct Recording {
        pulled: Pulled,
        since: std::sync::Mutex<Option<Value>>,
    }

    impl WatchSync for Recording {
        fn id(&self) -> &'static str {
            "simkl"
        }
        fn name(&self) -> &'static str {
            "Simkl"
        }
        fn snapshot(&self, _: &Ctx<'_>) -> Result<Value> {
            Ok(serde_json::json!({"all": "2026-02-01T00:00:00Z"}))
        }
        fn unchanged(&self, _: &Value, _: &Value) -> bool {
            false
        }
        fn pull(&self, _: &Ctx<'_>, since: Option<&Value>) -> Result<Pulled> {
            *self.since.lock().unwrap() = since.cloned();
            Ok(self.pulled.clone())
        }
    }

    #[test]
    fn a_cursor_from_an_older_build_is_fetched_as_a_baseline() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        // What the build before episodes left behind: a snapshot and no schema.
        cursor::update(&app, "simkl", |cursor| {
            cursor.snapshot = serde_json::json!({"all": "2026-01-01T00:00:00Z"});
            cursor.last_pull_at = Some(Utc::now());
            Ok(())
        })
        .unwrap();

        let sync = Recording { pulled: Pulled::default(), since: Default::default() };
        let _ = run(&app, &sync).unwrap();
        assert!(sync.since.lock().unwrap().is_none(), "an old cursor must not narrow the pull to a delta");
        assert_eq!(cursor::read(&app, "simkl").schema, CURSOR_SCHEMA, "and the cursor is stamped");
    }

    #[test]
    fn switching_the_import_on_asks_for_the_whole_history() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        // A cursor that would otherwise make the next pull a delta.
        cursor::update(&app, "simkl", |cursor| {
            cursor.schema = CURSOR_SCHEMA;
            cursor.snapshot = serde_json::json!({"all": "2026-01-01T00:00:00Z"});
            Ok(())
        })
        .unwrap();

        set_enabled(&app, "simkl", true).unwrap();
        let sync = Recording { pulled: Pulled::default(), since: Default::default() };
        let _ = run(&app, &sync).unwrap();
        assert!(sync.since.lock().unwrap().is_none(), "on means the whole history, not a window");
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
