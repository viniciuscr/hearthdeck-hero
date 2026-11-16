//! The Watch recap, as the app asks for it: the session log and a window in,
//! the numbers a screen draws out.
//!
//! The computation lives in `heronite-watch-stats` (pure, no files); this is the
//! one door the app opens: it reads `watch-sessions.json` and the clock.

use heronite_data::paths::AppPaths;

pub use heronite_watch_stats::{months, period, Recap, Span, TitleRow, Totals};

/// The recap of `span` as the clock reads it now. A session log that is missing
/// or broken is an empty recap, never an error.
pub fn recap(app: &AppPaths, span: Span) -> Recap {
    let sessions = heronite_watch_state::sessions::read(app).sessions;
    heronite_watch_stats::compute(&sessions, heronite_watch_stats::period(span, heronite_watch_stats::today()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use heronite_data::paths::DataPaths;
    use heronite_watch_state::sessions::{self, WatchSession};

    fn app(dir: &std::path::Path) -> AppPaths {
        AppPaths::for_data(&DataPaths::at(dir.to_path_buf()))
    }

    #[test]
    fn the_facade_reads_the_log_and_windows_it() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let now = Utc::now();
        sessions::append(
            &app,
            WatchSession {
                provider: "stremio".into(),
                kind: "movie".into(),
                id: "tt1".into(),
                video_id: None,
                title: "A Film".into(),
                started_at: now,
                ended_at: now,
                elapsed_seconds: 3_600.,
                position_from_ms: 0,
                position_to_ms: 3_600_000,
                duration_ms: 7_200_000,
                finished: false,
            },
        )
        .unwrap();
        let week = recap(&app, Span::Week);
        assert_eq!(week.totals.sessions, 1);
        assert!((week.totals.hours() - 1.).abs() < 1e-9);
        assert_eq!(week.titles[0].title, "A Film");
    }

    #[test]
    fn no_log_is_an_empty_recap() {
        let dir = tempfile::tempdir().unwrap();
        let recap = recap(&app(dir.path()), Span::Month);
        assert_eq!(recap.totals.sessions, 0);
        assert!(recap.titles.is_empty());
        // A quiet helper for the tests that need a timestamp.
        let _ = Utc.timestamp_opt(0, 0);
    }
}
