//! The local progress records: what was watched, read without a provider.
//!
//! `movies.json` is the source of truth between writes (`playback::play` keeps
//! it and hands a copy to the provider), so a page that only needs to know
//! where the person stopped reads it here, offline, without a `Ctx`.
//!
//! [`set_watched`] is the other writer: a mark a person made by hand. It writes
//! the same record a playback would and then queues the tracker event, so a tick
//! and an ending episode reach Simkl the same way.

use anyhow::Result;
use chrono::Utc;
use heronite_data::paths::AppPaths;
pub use heronite_watch_core::progress::WatchedState;
use heronite_watch_core::tracker::{TrackedTitle, TrackedView, TrackedWatch};
pub use heronite_watch_core::types::{EpisodeProgress, MovieProgress};
use heronite_watch_state::store;

/// The stored record of `provider`'s `id`, if there is one. A series keeps its
/// episode in `video_id`.
pub fn local(app: &AppPaths, provider: &str, id: &str) -> Option<MovieProgress> {
    let movies = store::read_movies(app);
    let mut base = record_for(&movies, provider, id).cloned()?;
    // Episodes can live in another writer's record for the same title — a file
    // written before the import learned to reuse the existing record, for instance.
    // The read is the title's, so it takes the union and never hides a watch.
    for other in movies.progress.iter().filter(|record| record.id == id || record.imdb.as_deref() == Some(id)) {
        for (key, episode) in &other.episodes {
            base.episodes.entry(key.clone()).or_insert_with(|| episode.clone());
        }
        base.finished |= other.finished;
    }
    Some(base)
}

/// The record for a title: the one its provider wrote, or the one another writer
/// made for the same title.
///
/// Progress is per title, not per provider — an import creates its own record when
/// nothing here has played the title yet, and a page asking for the Stremio record
/// must still see those episodes. Without this fallback an imported series reads as
/// never watched.
fn record_for<'a>(movies: &'a store::MoviesFile, provider: &str, id: &str) -> Option<&'a MovieProgress> {
    store::progress_for(movies, provider, id)
        .or_else(|| movies.progress.iter().find(|record| record.id == id || record.imdb.as_deref() == Some(id)))
}

/// What a mark is about: the film, or one episode of a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkTarget<'a> {
    Movie,
    Episode { video_id: &'a str, season: u32, number: u32 },
}

impl MarkTarget<'_> {
    fn view(&self) -> TrackedView {
        match self {
            Self::Movie => TrackedView::Movie,
            Self::Episode { season, number, .. } => TrackedView::Episode { season: *season, number: *number },
        }
    }
}

/// The title a mark belongs to: the keys the local record is written under, and
/// what a tracker records about it.
#[derive(Clone, Copy, Debug)]
pub struct Mark<'a> {
    pub provider: &'a str,
    pub id: &'a str,
    pub imdb: Option<&'a str>,
    pub kind: &'a str,
    pub name: &'a str,
    pub year: Option<&'a str>,
    pub target: MarkTarget<'a>,
}

/// Marks a title (or one episode) watched, or takes the mark away. The local
/// record is written first and the tracker outbox second; a tracker that is off,
/// or signed out, changes nothing here.
pub fn set_watched(app: &AppPaths, mark: &Mark<'_>, watched: bool) -> Result<()> {
    let at = Utc::now();
    // One per-path lock for every writer — this mark, a playback, and the import —
    // so a mark pressed while a session is being saved cannot be overwritten.
    store::update_movies(app, |movies| {
        // The same lookup the screens use: a mark updates the title's record even
        // when another writer made it, instead of keeping two.
        let mut progress = record_for(movies, mark.provider, mark.id).cloned().unwrap_or_else(|| MovieProgress {
            provider: mark.provider.to_owned(),
            id: mark.id.to_owned(),
            position_ms: 0,
            duration_ms: 0,
            updated_at: at,
            finished: false,
            video_id: None,
            imdb: None,
            episodes: Default::default(),
            ..Default::default()
        });

        match mark.target {
            MarkTarget::Movie => {
                // A length the player reported is kept; a film watched by hand
                // whose length nobody knows is still watched, because `finished`
                // says so.
                progress.finished = watched;
                progress.position_ms = if watched { progress.duration_ms.max(progress.position_ms) } else { 0 };
                progress.video_id = progress.video_id.clone().or_else(|| Some(mark.id.to_owned()));
            }
            MarkTarget::Episode { video_id, season, number } => {
                let duration_ms = progress.episodes.get(video_id).map_or(progress.duration_ms, |episode| episode.duration_ms);
                let entry = EpisodeProgress {
                    // Where to resume a rewatch: the beginning, unless it is watched.
                    position_ms: if watched { duration_ms } else { 0 },
                    duration_ms,
                    updated_at: at,
                    finished: watched,
                    season: Some(season),
                    number: Some(number),
                    ..Default::default()
                };
                progress.episodes.insert(video_id.to_owned(), entry);
                progress.video_id = Some(video_id.to_owned());
            }
        }
        progress.updated_at = at;
        // Same rule as playback: never leave a title without the id a tracker needs.
        progress.imdb = mark
            .imdb
            .map(str::to_owned)
            .or_else(|| progress.imdb.clone())
            .or_else(|| mark.id.starts_with("tt").then(|| mark.id.to_owned()));
        store::upsert_progress(movies, progress);
        Ok(())
    })?;

    // Said here, after the write: the log is the only place a person can see that
    // a mark landed, and the drain's lines say where it went afterwards.
    let what = match mark.target {
        MarkTarget::Movie => format!("{} {} · {}", mark.kind, mark.name, mark.provider),
        MarkTarget::Episode { season, number, .. } => format!("{} S{season}E{number}", mark.name),
    };
    log::info!("marked {what} as {} here", if watched { "watched" } else { "not watched" });

    crate::trackers::record_manual(
        app,
        TrackedTitle {
            imdb: mark.imdb.map(str::to_owned),
            kind: mark.kind.to_owned(),
            title: mark.name.to_owned(),
            year: mark.year.map(str::to_owned),
        },
        mark.target.view(),
        if watched { TrackedWatch::Watched } else { TrackedWatch::Unwatched },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::paths::DataPaths;

    fn app(dir: &std::path::Path) -> AppPaths {
        AppPaths::for_data(&DataPaths::at(dir.to_path_buf()))
    }

    /// A logger that keeps what our code says, so a test can prove the lines exist.
    struct Captured;
    static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    impl log::Log for Captured {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            LINES.lock().expect("a log line").push(record.args().to_string());
        }
        fn flush(&self) {}
    }

    fn capture() -> &'static std::sync::Mutex<Vec<String>> {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = log::set_logger(&Captured).map(|()| log::set_max_level(log::LevelFilter::Info));
        });
        &LINES
    }

    #[test]
    fn a_read_unions_episodes_from_the_other_record_for_the_title() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let at = Utc::now();
        store::update_movies(&app, |movies| {
            // The record playback made, with no episodes…
            movies.progress.push(MovieProgress {
                provider: "stremio".into(),
                id: "tt9".into(),
                updated_at: at,
                ..MovieProgress::default()
            });
            // …and the one the import made before it learned to reuse it.
            let mut imported = MovieProgress {
                provider: "imported".into(),
                id: "tt9".into(),
                imdb: Some("tt9".into()),
                updated_at: at,
                ..MovieProgress::default()
            };
            imported.episodes.insert(
                "tt9:1:2".into(),
                EpisodeProgress { season: Some(1), number: Some(2), finished: true, updated_at: at, ..EpisodeProgress::default() },
            );
            movies.progress.push(imported);
            Ok(())
        })
        .unwrap();

        let seen = local(&app, "stremio", "tt9").expect("the title's record");
        assert!(seen.episode("tt9:1:2", 1, 2).is_some_and(|episode| episode.finished), "the watch is not hidden because two records exist");
    }

    #[test]
    fn an_imported_record_is_what_the_page_reads() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let at = Utc::now();
        store::update_movies(&app, |movies| {
            let mut record = MovieProgress {
                provider: "imported".into(),
                id: "tt9".into(),
                imdb: Some("tt9".into()),
                updated_at: at,
                ..MovieProgress::default()
            };
            record.episodes.insert(
                "tt9:1:2".into(),
                EpisodeProgress { season: Some(1), number: Some(2), finished: true, updated_at: at, ..EpisodeProgress::default() },
            );
            movies.progress.push(record);
            Ok(())
        })
        .unwrap();

        // The page asks for the media provider's record; the import wrote another.
        let seen = local(&app, "stremio", "tt9").expect("the title's record, whoever wrote it");
        assert!(seen.episode("tt9:1:2", 1, 2).is_some_and(|episode| episode.finished), "and its episodes are visible");
    }

    #[test]
    fn a_mark_says_so_in_the_log() {
        let capture = capture();
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        capture.lock().unwrap().clear();

        set_watched(&app, &mark(MarkTarget::Movie), true).unwrap();
        let lines = capture.lock().unwrap().clone();
        assert!(
            lines.iter().any(|line| line.contains("marked") && line.contains("as watched here")),
            "a person must be able to see the mark landed: {lines:?}"
        );
    }

    fn mark<'a>(target: MarkTarget<'a>) -> Mark<'a> {
        Mark {
            provider: "stremio",
            id: "tt0944947",
            imdb: Some("tt0944947"),
            kind: "series",
            name: "Game of Thrones",
            year: Some("2011–2019"),
            target,
        }
    }

    #[test]
    fn a_film_marked_by_hand_is_watched_and_named_the_same_way_again() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let film = Mark { kind: "movie", id: "tt0133093", imdb: Some("tt0133093"), ..mark(MarkTarget::Movie) };
        set_watched(&app, &film, true).unwrap();
        let stored = local(&app, "stremio", "tt0133093").expect("the record");
        assert!(stored.finished);
        assert_eq!(stored.imdb.as_deref(), Some("tt0133093"), "the tracker's id is kept with it");

        set_watched(&app, &film, false).unwrap();
        let stored = local(&app, "stremio", "tt0133093").expect("the record stays");
        assert!(!stored.finished);
        assert_eq!(stored.position_ms, 0, "an unmarked film starts over");
    }

    #[test]
    fn an_episode_marked_by_hand_keeps_its_season_and_number() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        set_watched(&app, &mark(MarkTarget::Episode { video_id: "tt0944947:1:2", season: 1, number: 2 }), true).unwrap();
        let stored = local(&app, "stremio", "tt0944947").expect("the record");
        let episode = &stored.episodes["tt0944947:1:2"];
        assert!(episode.finished);
        assert_eq!((episode.season, episode.number), (Some(1), Some(2)));
        assert_eq!(stored.video_id.as_deref(), Some("tt0944947:1:2"), "the pointer follows the mark");

        set_watched(&app, &mark(MarkTarget::Episode { video_id: "tt0944947:1:2", season: 1, number: 2 }), false).unwrap();
        let stored = local(&app, "stremio", "tt0944947").expect("the record stays");
        assert!(!stored.episodes["tt0944947:1:2"].finished);
        assert_eq!(stored.episodes["tt0944947:1:2"].season, Some(1), "the numbering is not lost by unmarking");
    }

    #[test]
    fn a_series_mark_does_not_touch_the_other_episodes() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        for number in [1u32, 2] {
            let video = if number == 1 { "tt0944947:1:1" } else { "tt0944947:1:2" };
            set_watched(&app, &mark(MarkTarget::Episode { video_id: video, season: 1, number }), true).unwrap();
        }
        let stored = local(&app, "stremio", "tt0944947").expect("the record");
        assert!(stored.episodes.values().all(|episode| episode.finished));
        assert_eq!(stored.series_standing().watched, 2);
    }
}
