//! Playback: a provider's streams, its resolved URL and the player.
//!
//! `streams` resolves a title through the provider that owns it and sorts the
//! result for the picker. `play` asks the provider for the URL, hands it to mpv
//! and records the position. Everything here is blocking: the app calls
//! `streams` on a background task and `play` on its own thread, and flips
//! `stop` to end a session.

use std::sync::atomic::AtomicBool;

use anyhow::{anyhow, Result};
use chrono::Utc;
use heronite_data::paths::AppPaths;

use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::{Ctx, ResolvedStream, WatchProvider};
use heronite_watch_core::types::{MovieProgress, StreamChoice};
use heronite_watch_player::player::PlaybackOutcome;
#[cfg(unix)]
use heronite_watch_player::player::{MpvSession, PlayRequest};
use heronite_watch_state::store;
use heronite_watch_state::store::StoreCredentials;

use crate::registry;

/// What to play: one normalized stream of a title.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackRequest {
    /// The provider that owns the title, as `GameInfo.runner` names it.
    pub provider: String,
    /// The provider's id for the title, also the local progress key.
    pub id: String,
    /// `movie` in v1.
    pub kind: String,
    pub title: String,
    pub stream: StreamChoice,
}

/// The playable streams of a title, best first.
pub fn streams(app: &AppPaths, provider: &str, kind: &str, id: &str) -> Result<Vec<StreamChoice>> {
    let http = UreqTransport::new();
    streams_with(&http, app, provider, kind, id)
}

/// [`streams`] with an injected transport, for tests.
pub fn streams_with(http: &dyn Transport, app: &AppPaths, provider: &str, kind: &str, id: &str) -> Result<Vec<StreamChoice>> {
    let credentials = StoreCredentials::new(app);
    let ctx = Ctx { http, credentials: &credentials };
    registry::find(provider).ok_or_else(|| anyhow!("Unknown Watch provider: {provider}"))?.streams(&ctx, kind, id)
}

/// Plays `request` to the end (or until `stop`), then records the position.
/// Blocking: the caller owns a thread.
pub fn play(app: &AppPaths, request: &PlaybackRequest, stop: &AtomicBool) -> Result<PlaybackOutcome> {
    let http = UreqTransport::new();
    let credentials = StoreCredentials::new(app);
    let ctx = Ctx { http: &http, credentials: &credentials };
    let provider = registry::find(&request.provider).ok_or_else(|| anyhow!("Unknown Watch provider: {}", request.provider))?;
    let resolved = provider.resolve(&ctx, &request.stream)?;
    let previous = progress_record(app, provider, &ctx, &request.id);
    let start_ms = previous.as_ref().and_then(resume_at);
    let outcome = play_url(request, resolved, start_ms, stop)?;
    record(app, provider, &ctx, &request.id, previous.as_ref(), &outcome);
    Ok(outcome)
}

#[cfg(unix)]
fn play_url(request: &PlaybackRequest, resolved: ResolvedStream, start_ms: Option<u64>, stop: &AtomicBool) -> Result<PlaybackOutcome> {
    let mpv = heronite_watch_player::player::find_mpv().ok_or_else(|| anyhow!("mpv is not installed."))?;
    let gamepad = heronite_watch_player::player::gamepad_supported(&mpv);
    let play = PlayRequest { url: resolved.url, origin: resolved.origin, start_ms, title: Some(request.title.clone()) };
    let mut session = MpvSession::start(&mpv, &play, gamepad)?;
    session.wait_until(stop)
}

#[cfg(not(unix))]
fn play_url(_request: &PlaybackRequest, _resolved: ResolvedStream, _start_ms: Option<u64>, _stop: &AtomicBool) -> Result<PlaybackOutcome> {
    anyhow::bail!("Playback is only available on Linux.")
}

/// The stored progress of a title: the local record first, else the provider's
/// own (another device may have watched further).
fn progress_record(app: &AppPaths, provider: &dyn WatchProvider, ctx: &Ctx<'_>, id: &str) -> Option<MovieProgress> {
    let movies = store::read_movies(app);
    if let Some(progress) = store::progress_for(&movies, provider.id(), id) {
        return Some(progress.clone());
    }
    match provider.read_progress(ctx, id) {
        Ok(progress) => progress,
        Err(error) => {
            log::warn!("reading watch progress from {}: {error:#}", provider.name());
            None
        }
    }
}

/// A position worth resuming: started, and not near its end.
fn resume_at(progress: &MovieProgress) -> Option<u64> {
    if progress.finished || progress.position_ms == 0 {
        return None;
    }
    if progress.duration_ms > 0 && u128::from(progress.position_ms) * 100 >= u128::from(progress.duration_ms) * 95 {
        return None;
    }
    Some(progress.position_ms)
}

/// Stremio's finished threshold (`WATCHED_THRESHOLD_COEF`).
fn finished(position_ms: u64, duration_ms: u64) -> bool {
    duration_ms > 0 && u128::from(position_ms) * 100 >= u128::from(duration_ms) * 70
}

/// Local progress first, then the provider's. A failed remote write keeps the
/// local record for the next attempt; it never deletes anything.
fn record(
    app: &AppPaths,
    provider: &dyn WatchProvider,
    ctx: &Ctx<'_>,
    id: &str,
    previous: Option<&MovieProgress>,
    outcome: &PlaybackOutcome,
) {
    // mpv exited before reporting anything: keep the position we had instead
    // of resetting it.
    if outcome.position_ms == 0 && outcome.duration_ms == 0 {
        return;
    }
    let progress = MovieProgress {
        provider: provider.id().to_owned(),
        id: id.to_owned(),
        position_ms: outcome.position_ms,
        duration_ms: outcome.duration_ms,
        updated_at: Utc::now(),
        finished: finished(outcome.position_ms, outcome.duration_ms),
        // A series' episode pointer comes from the stored record; a film's is
        // its own id.
        video_id: previous.and_then(|progress| progress.video_id.clone()).or_else(|| Some(id.to_owned())),
    };
    let mut movies = store::read_movies(app);
    store::upsert_progress(&mut movies, progress.clone());
    if let Err(error) = store::write_movies(app, &movies) {
        log::warn!("saving watch progress: {error:#}");
        return;
    }
    if let Err(error) = provider.write_progress(ctx, &progress) {
        log::warn!("writing watch progress to {}: {error:#}", provider.name());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(position_ms: u64, duration_ms: u64, finished: bool) -> MovieProgress {
        MovieProgress {
            provider: "stremio".into(),
            id: "tt1".into(),
            position_ms,
            duration_ms,
            updated_at: Utc::now(),
            finished,
            video_id: None,
        }
    }

    fn app(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    #[test]
    fn a_started_film_resumes_and_a_finished_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let mut movies = store::MoviesFile::default();
        store::upsert_progress(&mut movies, progress(60_000, 600_000, false));
        store::write_movies(&app, &movies).unwrap();
        let stored = store::read_movies(&app);
        assert_eq!(store::progress_for(&stored, "stremio", "tt1").and_then(resume_at), Some(60_000));

        store::upsert_progress(&mut movies, progress(0, 600_000, false));
        store::write_movies(&app, &movies).unwrap();
        let stored = store::read_movies(&app);
        assert_eq!(store::progress_for(&stored, "stremio", "tt1").and_then(resume_at), None);

        store::upsert_progress(&mut movies, progress(590_000, 600_000, false));
        store::write_movies(&app, &movies).unwrap();
        let stored = store::read_movies(&app);
        assert_eq!(store::progress_for(&stored, "stremio", "tt1").and_then(resume_at), None);
    }

    #[test]
    fn seventy_percent_counts_as_watched() {
        assert!(!finished(0, 0));
        assert!(!finished(419, 600));
        assert!(finished(420, 600));
        assert!(finished(600, 600));
    }

    #[test]
    fn a_stream_without_a_url_or_hash_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let credentials = StoreCredentials::new(&app);
        let http = UreqTransport::new();
        let ctx = Ctx { http: &http, credentials: &credentials };
        let stream = StreamChoice::default();
        assert!(!stream.is_playable());
        let err = registry::find("stremio").unwrap().resolve(&ctx, &stream).unwrap_err().to_string();
        assert!(err.contains("nothing to play"), "{err}");
    }

    #[test]
    fn an_unlinked_account_is_told_where_to_link() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let http = UreqTransport::new();
        let err = streams_with(&http, &app, "stremio", "movie", "tt1").unwrap_err().to_string();
        assert!(err.contains("Settings → Watch"), "{err}");
    }

    #[test]
    fn an_unknown_provider_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let http = UreqTransport::new();
        let err = streams_with(&http, &app, "nowhere", "movie", "tt1").unwrap_err().to_string();
        assert!(err.contains("Unknown Watch provider"), "{err}");
    }

    struct NoopProvider;

    impl WatchProvider for NoopProvider {
        fn id(&self) -> &'static str {
            "stremio"
        }
        fn name(&self) -> &'static str {
            "Stremio"
        }
        fn browse(&self, _: &Ctx<'_>) -> Result<Vec<heronite_watch_core::provider::Rail>> {
            Ok(Vec::new())
        }
        fn streams(&self, _: &Ctx<'_>, _: &str, _: &str) -> Result<Vec<StreamChoice>> {
            Ok(Vec::new())
        }
        fn resolve(&self, _: &Ctx<'_>, _: &StreamChoice) -> Result<ResolvedStream> {
            anyhow::bail!("not used")
        }
    }

    #[test]
    fn recording_keeps_the_episode_pointer_it_had() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let credentials = StoreCredentials::new(&app);
        let http = UreqTransport::new();
        let ctx = Ctx { http: &http, credentials: &credentials };
        let previous = MovieProgress {
            provider: "stremio".into(),
            id: "tt1".into(),
            position_ms: 1,
            duration_ms: 2,
            updated_at: Utc::now(),
            finished: false,
            video_id: Some("tt1:1:2".into()),
        };
        record(&app, &NoopProvider, &ctx, "tt1", Some(&previous), &PlaybackOutcome { position_ms: 500, duration_ms: 1000 });

        let stored = store::read_movies(&app);
        assert_eq!(store::progress_for(&stored, "stremio", "tt1").unwrap().video_id.as_deref(), Some("tt1:1:2"));
    }

    #[test]
    fn absurd_durations_do_not_overflow() {
        let huge = MovieProgress {
            provider: "stremio".into(),
            id: "tt1".into(),
            position_ms: u64::MAX,
            duration_ms: u64::MAX,
            updated_at: Utc::now(),
            finished: false,
            video_id: None,
        };
        assert_eq!(resume_at(&huge), None);
        assert!(finished(u64::MAX, u64::MAX));
        assert!(!finished(1, u64::MAX));
    }
}
