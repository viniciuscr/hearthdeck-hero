//! Playback: the linked account's stream addons, the local Stremio service
//! and mpv.
//!
//! `streams` resolves a title through the addons the account already has
//! installed (Torrentio and friends) and sorts the result for the picker.
//! `play` fetches the torrent into the local service, hands the resulting
//! HTTP stream to mpv and records the position. Everything here is blocking:
//! the app calls `streams` on a background task and `play` on its own thread,
//! and flips `stop` to end a session.

use std::sync::atomic::AtomicBool;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use heronite_data::paths::AppPaths;
use url::Url;

use heronite_watch_addon::addon::AddonClient;
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::types::{MovieProgress, StreamChoice};
use heronite_watch_player::player::{MpvSession, PlayRequest, PlaybackOutcome};
use heronite_watch_state::store;
use heronite_watch_stremio::account::addon_collection;
use heronite_watch_stremio::datastore::{write_progress, DatastoreClient};
use heronite_watch_stremio::endpoints::Endpoints;
use heronite_watch_stremio::streaming::StreamingServer;

/// What to play: one normalized stream of a title.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackRequest {
    /// The Stremio id (`tt…`), also the local progress key.
    pub id: String,
    /// `movie` in v1.
    pub kind: String,
    pub title: String,
    pub stream: StreamChoice,
}

/// The playable streams of a title, best first.
pub fn streams(app: &AppPaths, kind: &str, id: &str) -> Result<Vec<StreamChoice>> {
    let http = UreqTransport::new();
    streams_with(&http, &Endpoints::default(), app, kind, id)
}

/// [`streams`] with an injected transport and endpoints, for tests.
pub fn streams_with(http: &dyn Transport, endpoints: &Endpoints, app: &AppPaths, kind: &str, id: &str) -> Result<Vec<StreamChoice>> {
    endpoints.validate()?;
    let key = store::read_auth(app)?.ok_or_else(|| anyhow!("Link your Stremio account in Settings → Watch to play."))?;
    let addons = addon_collection(http, &endpoints.api, &key).context("reading the addons of the linked account")?;

    let mut choices = Vec::new();
    let mut sources = 0;
    for addon in addons {
        let Some(manifest) = &addon.manifest else {
            log::warn!("watch addon: no manifest, skipped");
            continue;
        };
        if !addon.serves("stream") || !manifest.types.iter().any(|t| t == kind) {
            continue;
        }
        // A protected or half-configured addon has no transport to ask.
        if addon.transport_url.is_empty() {
            continue;
        }
        sources += 1;
        // The addon's transport URL may carry the person's configuration; the
        // error names the addon, never the URL.
        let client = match AddonClient::new(http, &addon.transport_url) {
            Ok(client) => client,
            Err(error) => {
                log::warn!("watch addon {}: {error:#}", manifest.name);
                continue;
            }
        };
        match client.stream(kind, id) {
            Ok(mut streams) => {
                for stream in &mut streams {
                    stream.addon.clone_from(&manifest.name);
                }
                choices.append(&mut streams);
            }
            Err(error) => log::warn!("watch addon {}: {error:#}", manifest.name),
        }
    }

    if sources == 0 {
        bail!("The linked Stremio account has no addon that provides streams. Install Torrentio in Stremio.");
    }
    choices.retain(StreamChoice::is_playable);
    choices.sort_by(|a, b| {
        b.seeders
            .unwrap_or(0)
            .cmp(&a.seeders.unwrap_or(0))
            .then_with(|| b.size_bytes.unwrap_or(0).cmp(&a.size_bytes.unwrap_or(0)))
            .then_with(|| a.addon.cmp(&b.addon))
    });
    Ok(choices)
}

/// Plays `request` to the end (or until `stop`), then records the position.
/// Blocking: the caller owns a thread.
pub fn play(app: &AppPaths, request: &PlaybackRequest, stop: &AtomicBool) -> Result<PlaybackOutcome> {
    let http = UreqTransport::new();
    let endpoints = Endpoints::default();
    endpoints.validate()?;
    let url = stream_url(&http, &endpoints, &request.stream)?;
    let start_ms = resume_position(app, &request.id);
    let outcome = play_url(request, url, start_ms, stop)?;
    record(app, &request.id, &outcome);
    Ok(outcome)
}

fn stream_url(http: &dyn Transport, endpoints: &Endpoints, stream: &StreamChoice) -> Result<Url> {
    if let Some(url) = &stream.url {
        return Url::parse(url).context("stream URL");
    }
    let hash = stream.info_hash.as_deref().ok_or_else(|| anyhow!("This stream has nothing to play."))?;
    let server = StreamingServer::new(http, endpoints.streaming_server.clone())?;
    let base = server.base_url().context("The Stremio service is not running")?;
    server.create(hash, &[]).context("The Stremio service refused the torrent")?;
    StreamingServer::stream_url(&base, hash, stream.file_idx)
}

#[cfg(unix)]
fn play_url(request: &PlaybackRequest, url: Url, start_ms: Option<u64>, stop: &AtomicBool) -> Result<PlaybackOutcome> {
    let mpv = heronite_watch_player::player::find_mpv().ok_or_else(|| anyhow!("mpv is not installed."))?;
    let gamepad = heronite_watch_player::player::gamepad_supported(&mpv);
    let play = PlayRequest { url, start_ms, title: Some(request.title.clone()) };
    let mut session = MpvSession::start(&mpv, &play, gamepad)?;
    session.wait_until(stop)
}

#[cfg(not(unix))]
fn play_url(_request: &PlaybackRequest, _url: Url, _start_ms: Option<u64>, _stop: &AtomicBool) -> Result<PlaybackOutcome> {
    bail!("Playback is only available on Linux.")
}

/// Resumes a title that was started and is not near its end.
fn resume_position(app: &AppPaths, id: &str) -> Option<u64> {
    let movies = store::read_movies(app);
    let progress = store::progress_for(&movies, id)?;
    if progress.finished || progress.position_ms == 0 {
        return None;
    }
    if progress.duration_ms > 0 && progress.position_ms * 100 >= progress.duration_ms * 95 {
        return None;
    }
    Some(progress.position_ms)
}

/// Stremio's finished threshold (`WATCHED_THRESHOLD_COEF`).
fn finished(position_ms: u64, duration_ms: u64) -> bool {
    duration_ms > 0 && position_ms * 100 >= duration_ms * 70
}

/// Local progress first, then Stremio. A failed remote write keeps the local
/// record for the next attempt; it never deletes anything.
fn record(app: &AppPaths, id: &str, outcome: &PlaybackOutcome) {
    // mpv exited before reporting anything: keep the position we had instead
    // of resetting it.
    if outcome.position_ms == 0 && outcome.duration_ms == 0 {
        return;
    }
    let progress = MovieProgress {
        id: id.to_owned(),
        position_ms: outcome.position_ms,
        duration_ms: outcome.duration_ms,
        updated_at: Utc::now(),
        finished: finished(outcome.position_ms, outcome.duration_ms),
        video_id: Some(id.to_owned()),
    };
    let mut movies = store::read_movies(app);
    store::upsert_progress(&mut movies, progress.clone());
    if let Err(error) = store::write_movies(app, &movies) {
        log::warn!("saving watch progress: {error:#}");
        return;
    }
    let Ok(Some(key)) = store::read_auth(app) else { return };
    let http = UreqTransport::new();
    let endpoints = Endpoints::default();
    let client = DatastoreClient::new(&http, &endpoints, key);
    if let Err(error) = write_progress(&client, &progress) {
        log::warn!("writing watch progress to Stremio: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(position_ms: u64, duration_ms: u64, finished: bool) -> MovieProgress {
        MovieProgress { id: "tt1".into(), position_ms, duration_ms, updated_at: Utc::now(), finished, video_id: None }
    }

    #[test]
    fn a_started_film_resumes_and_a_finished_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let mut movies = store::MoviesFile::default();
        store::upsert_progress(&mut movies, progress(60_000, 600_000, false));
        store::write_movies(&app, &movies).unwrap();
        assert_eq!(resume_position(&app, "tt1"), Some(60_000));

        store::upsert_progress(&mut movies, progress(0, 600_000, false));
        store::write_movies(&app, &movies).unwrap();
        assert_eq!(resume_position(&app, "tt1"), None);

        store::upsert_progress(&mut movies, progress(590_000, 600_000, false));
        store::write_movies(&app, &movies).unwrap();
        assert_eq!(resume_position(&app, "tt1"), None);
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
        let stream = StreamChoice::default();
        assert!(!stream.is_playable());
        let http = UreqTransport::new();
        let err = stream_url(&http, &Endpoints::default(), &stream).unwrap_err().to_string();
        assert!(err.contains("nothing to play"), "{err}");
    }

    #[test]
    fn an_unlinked_account_is_told_where_to_link() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let http = UreqTransport::new();
        let err = streams_with(&http, &Endpoints::default(), &app, "movie", "tt1").unwrap_err().to_string();
        assert!(err.contains("Settings → Watch"), "{err}");
    }
}
