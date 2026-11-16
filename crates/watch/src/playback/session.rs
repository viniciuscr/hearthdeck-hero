//! Turning one playback into the session record the recap reads.
//!
//! The player reports where it stopped, not how long it played: resuming at
//! 40 minutes and stopping at 55 is fifteen minutes of watching, so the start is
//! the position the record had (0 for a new title or a new episode) and the end
//! is what mpv reported. A session too short to be watching is not recorded.

use chrono::{Duration, Utc};
use heronite_data::paths::AppPaths;
use heronite_watch_core::types::MovieProgress;
use heronite_watch_player::player::PlaybackOutcome;

use heronite_watch_state::sessions::{self, WatchSession};

use super::PlaybackRequest;

/// Appends the session `outcome` ended, if it is one worth keeping.
pub(super) fn record(app: &AppPaths, request: &PlaybackRequest, previous: Option<&MovieProgress>, outcome: &PlaybackOutcome) {
    // mpv exited before reporting anything: there is nothing to say.
    if outcome.position_ms == 0 && outcome.duration_ms == 0 {
        return;
    }
    let from = start_of(previous, request);
    let elapsed = elapsed_seconds(from, outcome.position_ms);
    if elapsed < sessions::MIN_SESSION_SECS {
        return;
    }
    let ended_at = Utc::now();
    let session = WatchSession {
        provider: request.provider.clone(),
        kind: request.kind.clone(),
        id: request.id.clone(),
        video_id: request.video_id.clone(),
        title: request.title.clone(),
        started_at: ended_at - Duration::milliseconds((elapsed * 1000.).round() as i64),
        elapsed_seconds: elapsed,
        position_from_ms: from,
        position_to_ms: outcome.position_ms,
        duration_ms: outcome.duration_ms,
        ended_at,
        finished: super::finished(outcome.position_ms, outcome.duration_ms),
    };
    if let Err(error) = sessions::append(app, session) {
        // A recap is never worth failing a finished playback for.
        log::warn!("recording the watch session: {error:#}");
    }
}

/// Where this playback started: the stored position when it is the same episode,
/// else the beginning (a new title, or the next episode).
fn start_of(previous: Option<&MovieProgress>, request: &PlaybackRequest) -> u64 {
    let Some(progress) = previous else { return 0 };
    let same_episode = match request.video_id.as_deref() {
        Some(video) => progress.video_id.as_deref() == Some(video),
        None => progress.video_id.as_deref().is_none_or(|pointer| pointer == request.id),
    };
    if same_episode {
        progress.position_ms
    } else {
        0
    }
}

/// How long the session covered, in seconds. A player that reports a position
/// behind the one it started at (a seek back) covered nothing measurable, so the
/// position it reached is what counts.
fn elapsed_seconds(from_ms: u64, to_ms: u64) -> f64 {
    let covered = if to_ms >= from_ms { to_ms - from_ms } else { to_ms };
    covered as f64 / 1000.
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::types::StreamChoice;

    fn request(video: Option<&str>) -> PlaybackRequest {
        PlaybackRequest {
            provider: "stremio".into(),
            imdb: Some("tt1".into()),
            id: "tt1".into(),
            kind: if video.is_some() { "series".into() } else { "movie".into() },
            video_id: video.map(str::to_owned),
            title: "A Film".into(),
            stream: StreamChoice::default(),
        }
    }

    fn progress(position_ms: u64, video: Option<&str>) -> MovieProgress {
        MovieProgress {
            provider: "stremio".into(),
            id: "tt1".into(),
            position_ms,
            duration_ms: 6_000_000,
            updated_at: Utc::now(),
            finished: false,
            video_id: video.map(str::to_owned),
            imdb: Some("tt1".into()),
            episodes: Default::default(),
        }
    }

    #[test]
    fn a_resumed_film_counts_only_what_it_played() {
        let previous = progress(2_400_000, None);
        assert_eq!(start_of(Some(&previous), &request(None)), 2_400_000);
        assert!((elapsed_seconds(2_400_000, 3_300_000) - 900.).abs() < 1e-9, "fifteen minutes, not fifty-five");
    }

    #[test]
    fn a_new_episode_starts_from_zero() {
        let previous = progress(1_200_000, Some("ep1"));
        assert_eq!(start_of(Some(&previous), &request(Some("ep2"))), 0, "another episode is another watching");
        assert_eq!(start_of(Some(&previous), &request(Some("ep1"))), 1_200_000);
    }

    #[test]
    fn a_seek_back_is_measured_to_where_it_reached() {
        assert!((elapsed_seconds(600_000, 120_000) - 120.).abs() < 1e-9);
    }

    #[test]
    fn a_mis_click_is_not_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        record(&app, &request(None), None, &PlaybackOutcome { position_ms: 5_000, duration_ms: 6_000_000 });
        assert!(sessions::read(&app).sessions.is_empty(), "five seconds is not watching");
        record(&app, &request(None), None, &PlaybackOutcome { position_ms: 600_000, duration_ms: 6_000_000 });
        let kept = sessions::read(&app).sessions;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].position_to_ms, 600_000);
        assert!((kept[0].elapsed_seconds - 600.).abs() < 1e-9);
        assert!(!kept[0].finished, "ten minutes of a hundred is not finished");
    }
}
