//! The marks row: the watched mark of a film (or of the picked episode) and the
//! person's two shelf flags, built from the local records.
//!
//! Fork-owned (`docs/WATCH-PAGE.md`, Phase 5). Moved out of the page unchanged:
//! the render and the pad handler still call `CollectionView::watch_marks`, which
//! now lives here.

use heronite_data::CollectionGame;
use heronite_watch::metadata::Episode;

use crate::collection::watch_personal::Flag;
use crate::collection::CollectionView;

use super::super::{has_marks, watched_mark_label, watched_state, Mark, MarkState, Picked};

impl CollectionView {
    /// The marks the page draws, in the order it draws them: the watched mark
    /// first, then the two flags. A series whose episode list the page does not
    /// have yet has nothing to mark, and a title no source here has has no local
    /// record to mark at all: its row is the page, and the icon row stays empty.
    pub(in crate::collection::console::watch_page) fn watch_marks(
        &mut self,
        game: &CollectionGame,
        progress: Option<&heronite_watch::progress::MovieProgress>,
        episode: Option<&Episode>,
    ) -> Vec<MarkState> {
        if !has_marks(game) {
            return Vec::new();
        }
        // No record is a mark that is off, not one that is missing: a film nobody
        // has played is exactly what "Mark as watched" is for, and so is an
        // episode watched elsewhere.
        let picked = episode.map(|episode| Picked { id: episode.id.as_str(), season: episode.season, number: episode.number });
        let watched = watched_state(progress, picked, crate::fork::watch::sources::kind(game));
        let item = crate::fork::watch::sources::personal_item(game);
        let personal = self.watch_personal_states(&item);
        let mut marks = Vec::with_capacity(1 + Flag::ALL.len());
        // A film's watched mark, and the picked episode's: local first, the
        // trackers get it from the outbox afterwards.
        if let Some(on) = watched {
            let code = episode.map(Episode::code);
            marks.push(MarkState { mark: Mark::Watched, on, label: watched_mark_label(on, code.as_deref()) });
        }
        for (flag, on) in personal {
            let mark = match flag {
                Flag::Watchlist => Mark::Watchlist,
                Flag::Favourite => Mark::Favourite,
            };
            marks.push(MarkState { mark, on, label: flag.label(on).into() });
        }
        marks
    }
}
