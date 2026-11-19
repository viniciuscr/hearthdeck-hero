//! What a stored record means: the three states the screens, the recap and the
//! trackers agree on.
//!
//! The state is **derived**, never stored: `movies.json` keeps a position, a
//! length and a `finished` flag, and one rule turns those into "not started",
//! "started" or "watched". That is what lets a file written by an older build
//! read correctly here, and it keeps the two callers from drifting apart.

use crate::types::{EpisodeProgress, MovieProgress};

/// Reached this share of the length: watched. Stremio's own threshold
/// (`WATCHED_THRESHOLD_COEF`), so a title watched in Stremio is watched here.
pub const FINISHED_PERCENT: u64 = 70;

/// Under a minute is a mis-click or an mpv that never started, not watching.
pub const STARTED_MS: u64 = 60_000;

/// Whether `position_ms` of `duration_ms` is finished. A length the player never
/// reported (`0`) is not finished: nothing could have reached 70 % of it.
pub fn finished(position_ms: u64, duration_ms: u64) -> bool {
    duration_ms > 0 && u128::from(position_ms) * 100 >= u128::from(duration_ms) * u128::from(FINISHED_PERCENT)
}

/// How far a record got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchedState {
    /// Never played, or played too briefly to count.
    Unwatched,
    /// Played and left part-way: the mark the screens show differently from a
    /// tick, and the state a tracker with a "continue watching" place maps.
    InProgress {
        position_ms: u64,
        duration_ms: u64,
    },
    Watched,
}

impl WatchedState {
    /// The state of a position in a title (or an episode). `finished` wins over
    /// the position: it is what a provider told us.
    pub fn of(position_ms: u64, duration_ms: u64, finished: bool) -> Self {
        if finished {
            return Self::Watched;
        }
        if position_ms < STARTED_MS {
            return Self::Unwatched;
        }
        Self::InProgress { position_ms, duration_ms }
    }

    pub fn is_watched(self) -> bool {
        self == Self::Watched
    }

    pub fn is_started(self) -> bool {
        !matches!(self, Self::Unwatched)
    }

    /// How far in, from 0 to 100. `None` when there is no length to measure
    /// against — a record from a player that reported none.
    pub fn percent(self) -> Option<u8> {
        match self {
            Self::Unwatched => Some(0),
            Self::Watched => Some(100),
            Self::InProgress { position_ms, duration_ms } if duration_ms > 0 => {
                let percent = (u128::from(position_ms) * 100 / u128::from(duration_ms)).min(100) as u8;
                Some(percent.max(1))
            }
            Self::InProgress { .. } => None,
        }
    }

    /// How much is left, when the length is known.
    pub fn remaining_ms(self) -> Option<u64> {
        match self {
            Self::InProgress { position_ms, duration_ms } if duration_ms > position_ms => Some(duration_ms - position_ms),
            _ => None,
        }
    }
}

impl MovieProgress {
    /// The film's own state. A series keeps one per episode —
    /// [`MovieProgress::episodes`] — and its standing is [`SeriesStanding`].
    pub fn state(&self) -> WatchedState {
        WatchedState::of(self.position_ms, self.duration_ms, self.finished)
    }

    /// A series' standing from its per-episode records.
    pub fn series_standing(&self) -> SeriesStanding {
        let mut standing = SeriesStanding::default();
        for episode in self.episodes.values() {
            match episode.state() {
                WatchedState::Watched => standing.watched += 1,
                WatchedState::InProgress { .. } => standing.in_progress += 1,
                WatchedState::Unwatched => {}
            }
        }
        standing
    }
}

impl EpisodeProgress {
    pub fn state(&self) -> WatchedState {
        WatchedState::of(self.position_ms, self.duration_ms, self.finished)
    }
}

/// How many episodes of a series are watched, and how many are part-way.
/// A series' total is the source's (`TitleDetails::episodes`), not ours: we only
/// know the episodes something was played for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeriesStanding {
    pub watched: usize,
    pub in_progress: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn episode(position_ms: u64, duration_ms: u64, finished: bool) -> EpisodeProgress {
        EpisodeProgress { position_ms, duration_ms, updated_at: Utc::now(), finished, season: None, number: None, ..Default::default() }
    }

    fn record(position_ms: u64, duration_ms: u64, finished: bool) -> MovieProgress {
        MovieProgress {
            provider: "stremio".into(),
            id: "tt1".into(),
            position_ms,
            duration_ms,
            updated_at: Utc::now(),
            finished,
            video_id: None,
            imdb: Some("tt1".into()),
            episodes: Default::default(),
            ..Default::default()
        }
    }

    #[test]
    fn a_minute_is_the_line_between_never_played_and_started() {
        assert_eq!(WatchedState::of(0, 6_000_000, false), WatchedState::Unwatched);
        assert_eq!(WatchedState::of(59_999, 6_000_000, false), WatchedState::Unwatched);
        assert_eq!(WatchedState::of(60_000, 6_000_000, false), WatchedState::InProgress { position_ms: 60_000, duration_ms: 6_000_000 });
    }

    #[test]
    fn seventy_percent_is_watched_and_a_told_finish_wins() {
        assert!(!finished(0, 0), "a length nobody reported is not finished");
        assert!(!finished(419, 600));
        assert!(finished(420, 600));
        assert!(finished(600, 600));
        // What a provider said beats the arithmetic.
        assert_eq!(WatchedState::of(60_000, 6_000_000, true), WatchedState::Watched);
        assert_eq!(WatchedState::of(0, 0, true), WatchedState::Watched);
    }

    #[test]
    fn a_part_way_title_says_how_far_and_how_much_is_left() {
        let state = record(2_400_000, 6_000_000, false).state();
        assert_eq!(state.percent(), Some(40));
        assert_eq!(state.remaining_ms(), Some(3_600_000));
        assert_eq!(record(0, 6_000_000, false).state().percent(), Some(0));
        assert_eq!(record(6_000_000, 6_000_000, true).state().percent(), Some(100));
        // A record whose player reported no length has no percentage to show.
        assert_eq!(WatchedState::of(120_000, 0, false).percent(), None);
        assert_eq!(WatchedState::of(120_000, 60_000, false).percent(), Some(100), "past the end is the end");
    }

    #[test]
    fn a_series_counts_its_episodes_instead_of_guessing_from_the_pointer() {
        let mut progress = record(1_200_000, 3_000_000, false);
        progress.episodes.insert("ep1".into(), episode(3_000_000, 3_000_000, true));
        progress.episodes.insert("ep2".into(), episode(3_600_000, 3_000_000, true));
        progress.episodes.insert("ep3".into(), episode(900_000, 3_000_000, false));
        progress.episodes.insert("ep4".into(), episode(10_000, 3_000_000, false));
        assert_eq!(progress.series_standing(), SeriesStanding { watched: 2, in_progress: 1 });
        assert_eq!(progress.episodes["ep3"].state().percent(), Some(30));
        assert_eq!(progress.episodes["ep4"].state(), WatchedState::Unwatched);
        // A film's own record has no episodes.
        assert_eq!(record(0, 0, false).series_standing(), SeriesStanding::default());
    }
}
