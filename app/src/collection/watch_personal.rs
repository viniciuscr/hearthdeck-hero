//! The person's shelf in the view: their watchlist and their favourites.
//!
//! Read once and kept: the film page asks "is this in the watchlist?" for every
//! frame it draws, and re-reading a file per frame is the trap the TMDB key
//! already taught us to avoid. A toggle writes through the facade and re-reads the
//! small file once, so the file stays the truth.

use gpui_kit::prelude::*;
use heronite_data::{AppPaths, CollectionGame};
use heronite_i18n::tr;
use heronite_watch::personal::{self, PersonalLibrary, ShelfTitle};

use super::CollectionView;
use crate::icons::Icon;

/// One of the person's two flags on a title: what the film page's marks write.
///
/// The icon and the label live here and nowhere else: the page draws a mark from
/// a `Flag` and the pad handler reads a press back as a `Flag`, so a mark cannot
/// be drawn without its press being understood — which is exactly how "Add to
/// watch" once repainted and wrote nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flag {
    Watchlist,
    Favourite,
}

impl Flag {
    /// Both flags, in the order the film page's marks show them.
    pub(crate) const ALL: [Self; 2] = [Self::Watchlist, Self::Favourite];

    /// The icon the film page's mark wears: filled while the flag is on.
    pub(crate) fn icon(self, on: bool) -> Icon {
        match (self, on) {
            (Self::Watchlist, true) => Icon::Bookmark,
            (Self::Watchlist, false) => Icon::BookmarkBorder,
            (Self::Favourite, true) => Icon::Favorite,
            (Self::Favourite, false) => Icon::FavoriteBorder,
        }
    }

    /// The label the mark reads: the state itself (§7), so the swap is the answer.
    pub(crate) fn label(self, on: bool) -> &'static str {
        match (self, on) {
            (Self::Watchlist, true) => tr!("In watchlist"),
            (Self::Watchlist, false) => tr!("Add to watch"),
            (Self::Favourite, true) => tr!("In favourites"),
            (Self::Favourite, false) => tr!("Add to favourites"),
        }
    }

    /// Where this flag stands on the shelf.
    fn is_on(self, shelf: &PersonalLibrary, item: &ShelfTitle) -> bool {
        match self {
            Self::Watchlist => shelf.watchlisted(item),
            Self::Favourite => shelf.favourited(item),
        }
    }

    /// Flips the flag on disk and says where it stands afterwards.
    fn flip(self, app: &AppPaths, item: &ShelfTitle) -> anyhow::Result<bool> {
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
    ///
    /// Identity is the title (kind and id), so a snapshot kept from one provider
    /// reads as kept when another provider's card asks — the same rule the
    /// write path uses, which is why a toggle cannot add a second entry.
    pub(super) fn watch_personal_states(&mut self, item: &ShelfTitle) -> [(Flag, bool); 2] {
        let shelf = self.watch_personal();
        Flag::ALL.map(|flag| (flag, flag.is_on(shelf, item)))
    }

    /// Adds or removes the title from one of the two flags, then keeps the view in
    /// step. The file is the truth, so the marks redraw from it.
    ///
    /// Local first, then the providers (`docs/WATCH-SYNC-RULES.md`): the press
    /// writes the list and the intent in the same file, and the pull a moment
    /// later derives what the configured trackers are owed from that ledger and
    /// delivers it. Nothing waits on the network — the shelf is already right.
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
                self.watch_push_shelf(flag, cx);
            }
            Err(error) => {
                log::warn!("watch shelf: {error:#}");
                self.watch_notify(tr!("Could not save that: {error}", error = format!("{error:#}")), true, cx);
            }
        }
    }

    /// Sends the change the press just wrote to every enabled tracker, off the UI
    /// thread.
    ///
    /// The same pull the timer runs: it derives what the ledger owes and delivers
    /// it, so a provider that cannot answer now simply reads the same ledger on
    /// the next tick. What it brings back can add titles to the lists (a phone's
    /// add, a removal of ours it confirms), so the rails are dropped when it ends.
    fn watch_push_shelf(&self, flag: Flag, cx: &mut Context<Self>) {
        let app = self.app_paths.clone();
        cx.spawn(async move |this, cx| {
            let ran = cx
                .background_spawn(async move {
                    if heronite_watch::import::enabled(&app).is_empty() {
                        return Vec::new();
                    }
                    heronite_watch::import::tick_now(&app)
                })
                .await;
            this.update(cx, |this, cx| {
                for (provider, result) in ran {
                    match result {
                        Ok(report) if report.lists_delivered > 0 => {
                            log::info!("{provider}: the {flag:?} press reached the provider");
                        }
                        Ok(report) => log::debug!(
                            "{provider}: the {flag:?} press stays owed ({} added, {} removed, {} delivered)",
                            report.lists_added,
                            report.lists_removed,
                            report.lists_delivered
                        ),
                        Err(error) => log::warn!("{provider}: the shelf change was not sent: {error:#}"),
                    }
                }
                this.forget_watch_shelves();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn app(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    fn matrix() -> ShelfTitle {
        ShelfTitle {
            provider: "stremio".into(),
            kind: "movie".into(),
            id: "tt0133093".into(),
            name: "The Matrix".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_flag_draws_the_icon_its_state_asks_for() {
        for flag in Flag::ALL {
            assert_ne!(flag.icon(false), flag.icon(true), "{flag:?} wears a filled icon only while it is on");
        }
        assert_ne!(Flag::ALL[0].icon(true), Flag::ALL[1].icon(true), "the two flags never look alike");
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

    /// The write the page's marks end in: each flag in the row's order flips its
    /// own list in the file, and leaves the other alone. The page's own path
    /// (mark → `watch_press_mark` → `Flag::flip`) is `watch_page.rs`'s to test;
    /// this is the half that touches the disk.
    #[test]
    fn pressing_a_flag_mark_writes_that_flag() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let item = matrix();

        assert_eq!(Flag::ALL, [Flag::Watchlist, Flag::Favourite], "the row draws the watchlist before the heart");
        for flag in Flag::ALL {
            assert!(flag.flip(&app, &item).unwrap());
        }

        let shelf = personal::read(&app);
        assert!(shelf.watchlisted(&item) && shelf.favourited(&item), "both presses landed in the file");
    }
}
