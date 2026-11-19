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
use heronite_watch_core::progress::finished;
use heronite_watch_core::provider::{Ctx, ResolvedStream, WatchProvider};
use heronite_watch_core::types::{MovieProgress, StreamChoice};
use heronite_watch_player::player::PlaybackOutcome;
#[cfg(unix)]
use heronite_watch_player::player::{MpvSession, PlayRequest};
use heronite_watch_state::store;
use heronite_watch_state::store::StoreCredentials;
use url::Url;

mod session;

use crate::registry;

/// What to play: one normalized stream of a title.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackRequest {
    /// The provider that owns the title, as `GameInfo.runner` names it.
    pub provider: String,
    /// The IMDb id, when the title has one: kept with the record so it can be
    /// matched after a provider renames its own ids.
    pub imdb: Option<String>,
    /// The provider's id for the title, also the local progress key.
    pub id: String,
    /// `movie` or `series`.
    pub kind: String,
    /// The episode being played, when the title is a series. The progress
    /// record stays keyed by `id`; this is what resumes, and what the provider
    /// stores as the record's `video_id`.
    pub video_id: Option<String>,
    /// The episode's season and number, when the page knew them: kept with the
    /// record and the session, so a tracker asks for what Simkl wants instead of
    /// parsing the provider's id.
    pub season: Option<u32>,
    pub number: Option<u32>,
    /// What mpv's window and the desktop panels call it: the episode, when the
    /// page picked one.
    pub title: String,
    /// The title as the library names it — the film, or the show — and its year:
    /// what a tracker records, where `title` may name the episode.
    pub name: String,
    pub year: Option<String>,
    pub stream: StreamChoice,
}

/// The tallest picture a trailer is resolved at: full HD, which is what a
/// thirty-second clip deserves. Its source offers more; nothing here needs it.
#[cfg(unix)]
const TRAILER_MAX_HEIGHT: u32 = 1080;

/// Plays a trailer in mpv, and keeps nothing: no progress record, no session,
/// no tracker event — a trailer is not something a person is watching.
///
/// The page goes to mpv as it is (YouTube today), and mpv resolves it through
/// yt-dlp, which the bootstrap installs. The player's own policy still applies:
/// a trailer must be an https page and may not look like an option.
#[cfg(unix)]
pub fn play_trailer(url: &str, title: Option<&str>, stop: &AtomicBool) -> Result<()> {
    let url = Url::parse(url.trim()).map_err(|error| anyhow!("the trailer's address: {error}"))?;
    let mpv = heronite_watch_player::player::find_mpv().ok_or_else(|| anyhow!("mpv is not installed."))?;
    let gamepad = heronite_watch_player::player::gamepad_supported(&mpv);
    // Full HD is plenty for a trailer: the page (YouTube today) would otherwise
    // hand over its 4K, and decoding it costs more than the thirty seconds are worth.
    let play = PlayRequest {
        url,
        origin: heronite_watch_core::provider::StreamOrigin::Remote,
        start_ms: None,
        title: title.map(str::to_owned),
        quality: heronite_watch_player::player::Quality::AtMost(TRAILER_MAX_HEIGHT),
    };
    let mut session = MpvSession::start(&mpv, &play, gamepad)?;
    session.wait_until(stop)?;
    Ok(())
}

/// Playing is Linux-only, like every session here.
#[cfg(not(unix))]
pub fn play_trailer(_url: &str, _title: Option<&str>, _stop: &AtomicBool) -> Result<()> {
    anyhow::bail!("Playback is only available on Linux.")
}

/// The playable streams of a title, best first.
pub fn streams(app: &AppPaths, provider: &str, kind: &str, id: &str) -> Result<Vec<StreamChoice>> {
    enabled(app, provider)?;
    let http = UreqTransport::new();
    streams_with(&http, app, provider, kind, id)
}

/// Whether Settings → Watch → Sources lets `provider` be asked for streams. Its
/// account and rails stay either way: only playback is gated.
pub fn enabled(app: &AppPaths, provider: &str) -> Result<()> {
    let settings = heronite_watch_state::settings::read(app);
    if settings.stream_providers.iter().any(|id| id == provider) {
        return Ok(());
    }
    anyhow::bail!("{provider} is turned off in Settings → Watch → Sources (Streams).")
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
    enabled(app, &request.provider)?;
    let http = UreqTransport::new();
    let credentials = StoreCredentials::new(app);
    let ctx = Ctx { http: &http, credentials: &credentials };
    let provider = registry::find(&request.provider).ok_or_else(|| anyhow!("Unknown Watch provider: {}", request.provider))?;
    let resolved = provider.resolve(&ctx, &request.stream)?;
    let previous = progress_record(app, provider, &ctx, &request.id);
    let start_ms = previous.as_ref().filter(|progress| resumes_this(progress, request)).and_then(resume_at);
    let outcome = play_url(request, resolved, start_ms, stop)?;
    record(app, provider, &ctx, request, previous.as_ref(), &outcome);
    // What the recap is built from: this playback, its start and its end.
    session::record(app, request, previous.as_ref(), &outcome);
    // And what a tracker is told, later and elsewhere: the local files above are
    // already the truth, so this only appends to their outbox.
    crate::trackers::record_playback(app, request, previous.as_ref(), &outcome);
    Ok(outcome)
}

#[cfg(unix)]
fn play_url(request: &PlaybackRequest, resolved: ResolvedStream, start_ms: Option<u64>, stop: &AtomicBool) -> Result<PlaybackOutcome> {
    let mpv = heronite_watch_player::player::find_mpv().ok_or_else(|| anyhow!("mpv is not installed."))?;
    let gamepad = heronite_watch_player::player::gamepad_supported(&mpv);
    let play = PlayRequest {
        url: resolved.url,
        origin: resolved.origin,
        start_ms,
        title: Some(request.title.clone()),
        quality: heronite_watch_player::player::Quality::Best,
    };
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

/// Whether the stored position belongs to what is being played: a series only
/// resumes the episode its pointer names, never another one.
fn resumes_this(progress: &MovieProgress, request: &PlaybackRequest) -> bool {
    match request.video_id.as_deref() {
        Some(video) => progress.video_id.as_deref() == Some(video),
        // A film stores its own id as the pointer; a record without one is a film's too.
        None => progress.video_id.as_deref().is_none_or(|pointer| pointer == request.id),
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

/// Local progress first, then the provider's. A failed remote write keeps the
/// local record for the next attempt; it never deletes anything.
fn record(
    app: &AppPaths,
    provider: &dyn WatchProvider,
    ctx: &Ctx<'_>,
    request: &PlaybackRequest,
    previous: Option<&MovieProgress>,
    outcome: &PlaybackOutcome,
) {
    let (id, video_id, imdb) = (request.id.as_str(), request.video_id.as_deref(), request.imdb.as_deref());
    // mpv exited before reporting anything: keep the position we had instead
    // of resetting it.
    if outcome.position_ms == 0 && outcome.duration_ms == 0 {
        return;
    }
    let finished = finished(outcome.position_ms, outcome.duration_ms);
    let at = Utc::now();
    let mut episodes = previous.map(|progress| progress.episodes.clone()).unwrap_or_default();
    // A series keeps each episode's state: finishing one does not erase the one
    // before it, and moving on does not un-watch what ended.
    if let Some(episode) = video_id {
        episodes.insert(
            episode.to_owned(),
            heronite_watch_core::types::EpisodeProgress {
                position_ms: outcome.position_ms,
                duration_ms: outcome.duration_ms,
                updated_at: at,
                finished,
                season: request.season,
                number: request.number,
                ..Default::default()
            },
        );
    }
    let progress = MovieProgress {
        provider: provider.id().to_owned(),
        id: id.to_owned(),
        position_ms: outcome.position_ms,
        duration_ms: outcome.duration_ms,
        updated_at: at,
        // A film's own state; a series is finished episode by episode, so the
        // title-level flag says nothing about it.
        finished: video_id.is_none() && finished,
        // The episode just played; without one, the pointer the record already
        // had (a series), else the film's own id.
        video_id: video_id
            .map(str::to_owned)
            .or_else(|| previous.and_then(|progress| progress.video_id.clone()))
            .or_else(|| Some(id.to_owned())),
        imdb: imdb.map(str::to_owned).or_else(|| previous.and_then(|progress| progress.imdb.clone())),
        episodes,
        ..Default::default()
    };
    if let Err(error) = store::update_movies(app, |movies| {
        store::upsert_progress(movies, progress.clone());
        Ok(())
    }) {
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
            imdb: None,
            episodes: Default::default(),
            ..Default::default()
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
            imdb: Some("tt1".into()),
            episodes: Default::default(),
            ..Default::default()
        };
        record(&app, &NoopProvider, &ctx, &request("tt1", None), Some(&previous), &PlaybackOutcome { position_ms: 500, duration_ms: 1000 });

        let stored = store::read_movies(&app);
        assert_eq!(store::progress_for(&stored, "stremio", "tt1").unwrap().video_id.as_deref(), Some("tt1:1:2"));
    }

    #[test]
    fn a_series_records_the_episode_that_played_and_a_film_its_own_id() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let credentials = StoreCredentials::new(&app);
        let http = UreqTransport::new();
        let ctx = Ctx { http: &http, credentials: &credentials };
        record(&app, &NoopProvider, &ctx, &request("tt1", Some("tt1:1:3")), None, &PlaybackOutcome { position_ms: 500, duration_ms: 1000 });
        assert_eq!(store::progress_for(&store::read_movies(&app), "stremio", "tt1").unwrap().video_id.as_deref(), Some("tt1:1:3"));

        record(&app, &NoopProvider, &ctx, &request("tt2", None), None, &PlaybackOutcome { position_ms: 500, duration_ms: 1000 });
        assert_eq!(store::progress_for(&store::read_movies(&app), "stremio", "tt2").unwrap().video_id.as_deref(), Some("tt2"));
    }

    #[test]
    fn finishing_an_episode_keeps_it_watched_when_the_next_one_plays() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let credentials = StoreCredentials::new(&app);
        let http = UreqTransport::new();
        let ctx = Ctx { http: &http, credentials: &credentials };
        // Episode 1 played to the end (70 % is the threshold), then episode 2 began.
        record(
            &app,
            &NoopProvider,
            &ctx,
            &request("tt1", Some("tt1:1:1")),
            None,
            &PlaybackOutcome { position_ms: 1_800_000, duration_ms: 2_400_000 },
        );
        record(
            &app,
            &NoopProvider,
            &ctx,
            &request("tt1", Some("tt1:1:2")),
            Some(&store::read_movies(&app).progress[0].clone()),
            &PlaybackOutcome { position_ms: 60_000, duration_ms: 2_400_000 },
        );
        let stored = store::read_movies(&app);
        let title = store::progress_for(&stored, "stremio", "tt1").expect("the title's record");
        assert!(title.episodes.get("tt1:1:1").is_some_and(|episode| episode.finished), "the finished episode stays watched");
        assert!(title.episodes.get("tt1:1:2").is_some_and(|episode| !episode.finished), "the new one is not");
        assert_eq!(title.video_id.as_deref(), Some("tt1:1:2"), "the pointer moved to where playback resumes");
        assert!(!title.finished, "a series is not finished because one episode ended");
        assert_eq!(title.imdb.as_deref(), Some("tt1"), "the IMDb id is kept with the record");
    }

    fn request(id: &str, video_id: Option<&str>) -> PlaybackRequest {
        PlaybackRequest {
            provider: "stremio".into(),
            imdb: Some(id.into()),
            id: id.into(),
            kind: if video_id.is_some() { "series".into() } else { "movie".into() },
            video_id: video_id.map(str::to_owned),
            season: video_id.map(|_| 1),
            number: video_id.map(|_| 1),
            title: id.into(),
            name: "A Title".into(),
            year: Some("2011".into()),
            stream: StreamChoice::default(),
        }
    }

    #[test]
    fn only_the_episode_that_played_resumes() {
        let earlier = MovieProgress { video_id: Some("tt1:1:2".into()), ..progress(60_000, 600_000, false) };
        assert!(resumes_this(&earlier, &request("tt1", Some("tt1:1:2"))), "the same episode resumes");
        assert!(!resumes_this(&earlier, &request("tt1", Some("tt1:1:3"))), "another episode starts at the beginning");
        assert!(!resumes_this(&earlier, &request("tt1", None)), "a film's record is not this series' pointer");

        let film = progress(60_000, 600_000, false);
        assert!(resumes_this(&film, &request("tt1", None)));
        let legacy = MovieProgress { video_id: Some("tt1".into()), ..progress(60_000, 600_000, false) };
        assert!(resumes_this(&legacy, &request("tt1", None)), "a film stores its own id as the pointer");
    }

    #[test]
    fn a_provider_turned_off_in_sources_is_not_asked_for_a_stream() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        assert!(enabled(&app, "stremio").is_ok(), "the streaming service is on by default");

        let mut settings = heronite_watch_state::settings::read(&app);
        settings.stream_providers.clear();
        heronite_watch_state::settings::write(&app, &settings).unwrap();
        let error = enabled(&app, "stremio").unwrap_err();
        assert!(format!("{error:#}").contains("turned off"), "{error:#}");
        // Its account and rails are untouched: only asking for a stream is refused.
        assert!(heronite_watch_state::settings::read(&app).stream_providers.is_empty());

        let mut settings = heronite_watch_state::settings::read(&app);
        settings.stream_providers.push("stremio".into());
        heronite_watch_state::settings::write(&app, &settings).unwrap();
        assert!(enabled(&app, "stremio").is_ok());
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
            imdb: None,
            episodes: Default::default(),
            ..Default::default()
        };
        assert_eq!(resume_at(&huge), None);
        assert!(finished(u64::MAX, u64::MAX));
        assert!(!finished(1, u64::MAX));
    }
}
