//! The person's shelf in the view: their watchlist and their favourites.
//!
//! Read once and kept: the film page asks "is this in the watchlist?" for every
//! frame it draws, and re-reading a file per frame is the trap the TMDB key
//! already taught us to avoid. A toggle writes through the facade and re-reads the
//! small file once, so the file stays the truth.

use gpui_kit::prelude::*;
use heronite_data::CollectionGame;
use heronite_i18n::tr;
use heronite_watch::personal::{self, MovieItem, PersonalLibrary};

use super::CollectionView;

impl CollectionView {
    /// The shelf, read the first time something needs it.
    pub(super) fn watch_personal(&mut self) -> &PersonalLibrary {
        if !self.watch_personal_ready {
            self.watch_personal = personal::read(&self.app_paths);
            self.watch_personal_ready = true;
        }
        &self.watch_personal
    }

    /// Where one title stands: `(in the watchlist, a favourite)`.
    pub(super) fn watch_personal_flags(&mut self, item: &MovieItem) -> (bool, bool) {
        let shelf = self.watch_personal();
        (shelf.watchlisted(item), shelf.favourited(item))
    }

    /// Adds or removes the title from one of the two flags, then keeps the view in
    /// step. The row id says which: `watch-watchlist` or `watch-favourite`.
    pub(super) fn watch_toggle_personal(&mut self, game: &CollectionGame, row: &str, cx: &mut Context<Self>) {
        let item = crate::fork::watch::sources::personal_item(game);
        let result = match row {
            "watch-watchlist" => personal::toggle_watchlist(&self.app_paths, &item),
            "watch-favourite" => personal::toggle_favourite(&self.app_paths, &item),
            _ => return,
        };
        match result {
            Ok(_) => {
                // The file is the truth, and it is small: one read per press.
                self.watch_personal = personal::read(&self.app_paths);
                self.watch_personal_ready = true;
                cx.notify();
            }
            Err(error) => {
                log::warn!("watch shelf: {error:#}");
                self.show_toast(tr!("Could not save that: {error}", error = format!("{error:#}")), true, cx);
            }
        }
    }
}
