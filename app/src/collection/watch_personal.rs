//! The person's shelf in the view: their watchlist and their favourites.
//!
//! Read once and kept: the film page asks "is this in the watchlist?" for every
//! frame it draws, and re-reading a file per frame is the trap the TMDB key
//! already taught us to avoid. A toggle writes through the facade and re-reads the
//! small file once, so the file stays the truth.

use gpui_kit::prelude::*;
use heronite_data::{AppPaths, CollectionGame};
use heronite_i18n::tr;
use heronite_watch::personal::{self, MovieItem, PersonalLibrary};

use super::CollectionView;

/// One of the person's two flags on a title: what the film page's rows write.
///
/// The row id lives here and nowhere else: the page draws a row from a `Flag` and
/// the pad handler reads a press back through [`Flag::of`], so a row cannot be
/// drawn without its press being understood — which is exactly how "Add to watch"
/// once repainted and wrote nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flag {
    Watchlist,
    Favourite,
}

impl Flag {
    /// Both flags, in the order the film page's rows show them.
    pub(crate) const ALL: [Self; 2] = [Self::Watchlist, Self::Favourite];

    /// The row id the film page gives this flag.
    pub(crate) const fn row(self) -> &'static str {
        match self {
            Self::Watchlist => "watch-watchlist",
            Self::Favourite => "watch-favourite",
        }
    }

    /// The flag a row id stands for, when it is one of the two.
    pub(crate) fn of(row: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|flag| flag.row() == row)
    }

    /// The label the row shows: the state itself (§7), so the swap is the answer.
    pub(crate) fn label(self, on: bool) -> &'static str {
        match (self, on) {
            (Self::Watchlist, true) => tr!("In watchlist"),
            (Self::Watchlist, false) => tr!("Add to watch"),
            (Self::Favourite, true) => tr!("In favourites"),
            (Self::Favourite, false) => tr!("Add to favourites"),
        }
    }

    /// Flips the flag on disk and says where it stands afterwards.
    fn flip(self, app: &AppPaths, item: &MovieItem) -> anyhow::Result<bool> {
        match self {
            Self::Watchlist => personal::toggle_watchlist(app, item),
            Self::Favourite => personal::toggle_favourite(app, item),
        }
    }
}

impl CollectionView {
    /// The shelf, read the first time something needs it.
    pub(super) fn watch_personal(&mut self) -> &PersonalLibrary {
        if !self.watch_personal_ready {
            self.watch_personal = personal::read(&self.app_paths);
            self.watch_personal_ready = true;
        }
        &self.watch_personal
    }

    /// Where one title stands on both flags, in the film page's row order.
    pub(super) fn watch_personal_states(&mut self, item: &MovieItem) -> [(Flag, bool); 2] {
        let shelf = self.watch_personal();
        [(Flag::Watchlist, shelf.watchlisted(item)), (Flag::Favourite, shelf.favourited(item))]
    }

    /// Adds or removes the title from one of the two flags, then keeps the view in
    /// step. The file is the truth, so the rows redraw from it.
    pub(super) fn watch_toggle_personal(&mut self, game: &CollectionGame, flag: Flag, cx: &mut Context<Self>) {
        let item = crate::fork::watch::sources::personal_item(game);
        match flag.flip(&self.app_paths, &item) {
            Ok(_) => {
                // The file is the truth, and it is small: one read per press, and
                // the Shelf tab's rails are rebuilt from it.
                self.watch_personal = personal::read(&self.app_paths);
                self.watch_personal_ready = true;
                self.forget_watch_shelves();
                cx.notify();
            }
            Err(error) => {
                log::warn!("watch shelf: {error:#}");
                self.show_toast(tr!("Could not save that: {error}", error = format!("{error:#}")), true, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::list::{ListNav, Outcome, Row, Value};
    use crate::console::Pad;
    use core::prelude::v1::test;

    fn app(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    fn matrix() -> MovieItem {
        MovieItem {
            provider: "stremio".into(),
            kind: "movie".into(),
            id: "tt0133093".into(),
            name: "The Matrix".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_flag_reads_back_the_row_it_was_drawn_as() {
        assert_eq!(Flag::ALL.map(Flag::row), ["watch-watchlist", "watch-favourite"], "the page's two rows");
        for flag in Flag::ALL {
            assert_eq!(Flag::of(flag.row()), Some(flag), "{:?} understands its own press", flag);
        }
        assert_eq!(Flag::of("watch-catalog:tmdb"), None, "another row's id is another row's");
        assert_eq!(Flag::of("primary"), None);
    }

    #[test]
    fn a_flags_label_is_the_state() {
        assert_ne!(Flag::Watchlist.label(false), Flag::Watchlist.label(true));
        assert_ne!(Flag::Favourite.label(false), Flag::Favourite.label(true));
    }

    #[test]
    fn each_flag_writes_its_own_list() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let item = matrix();
        assert!(Flag::Watchlist.flip(&app, &item).unwrap(), "the first press adds it");
        assert!(personal::read(&app).watchlisted(&item));
        assert!(Flag::Favourite.flip(&app, &item).unwrap());
        assert!(personal::read(&app).favourited(&item));

        assert!(!Flag::Watchlist.flip(&app, &item).unwrap(), "pressing again takes it off");
        let shelf = personal::read(&app);
        assert!(!shelf.watchlisted(&item) && shelf.favourited(&item), "and leaves the other flag alone");
    }

    /// The press the film page receives, end to end: the rows it draws, A on one,
    /// the flag that row names, and the write. This is the path that once
    /// repainted and wrote nothing, because the page's handler swallowed the
    /// change and the `watch-` arm it was believed to reach belonged to the games'
    /// page.
    #[test]
    fn pressing_a_flag_row_writes_that_flag() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let item = matrix();
        let rows: Vec<Row> = Flag::ALL.into_iter().map(|flag| Row::toggle(flag.row(), flag.label(false), false)).collect();
        let mut list = ListNav::default();

        for (index, flag) in Flag::ALL.into_iter().enumerate() {
            list.focus = index;
            let Outcome::Changed(id, Value::Toggle(true)) = list.on_pad(Pad::A, &rows) else {
                panic!("A on a switch row flips it");
            };
            assert_eq!(Flag::of(&id), Some(flag), "the page understands the row it drew");
            assert!(flag.flip(&app, &item).unwrap());
        }

        let shelf = personal::read(&app);
        assert!(shelf.watchlisted(&item) && shelf.favourited(&item), "both presses landed in the file");
    }
}
