//! The marks row: the watched mark of a film (or of the picked episode) and the
//! person's two shelf flags, built from the local records.
//!
//! Fork-owned (`docs/WATCH-PAGE.md`, Phase 5). Moved out of the page unchanged:
//! the render still calls `CollectionView::watch_marks`, which now lives here.
//! The page's own types and the record readers stay in `../mod.rs`, because the
//! engine and the pad handler use them too — this file holds the section, not a
//! second owner of them.

use heronite_data::CollectionGame;
use heronite_watch::metadata::Episode;

use crate::collection::watch_personal::Flag;
use crate::collection::CollectionView;

use super::super::{watched_mark_label, Mark, MarkState};

impl CollectionView {
    /// The marks the page draws, in the order it draws them: the watched mark
    /// first (when `watched` says the row has one), then the two flags.
    ///
    /// `watched` is [`watched_mark`]'s answer, computed by the render and by the
    /// pad from one predicate — this function does not decide the row's shape, so
    /// an icon cannot be drawn where a press expects another one. A series the
    /// page knows no episode of, and a title no source here has, have no watched
    /// mark; the two shelf flags are drawn for every title.
    pub(in crate::collection::console::watch_page) fn watch_marks(
        &mut self,
        game: &CollectionGame,
        watched: Option<bool>,
        episode: Option<&Episode>,
    ) -> Vec<MarkState> {
        // No record is a mark that is off, not one that is missing: a film nobody
        // has played is exactly what "Mark as watched" is for, and so is an
        // episode watched elsewhere.
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
