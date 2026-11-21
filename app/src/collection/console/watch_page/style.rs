//! Which arrangement the film page draws.
//!
//! Fork-owned (`docs/WATCH-PAGE.md`, Phase 3). Today there is one: [`PageStyle::Film`], the page as
//! it has always looked — the title, its marks and its list on the left, the
//! details panel on the right. Naming it is what lets a second arrangement land
//! without touching the engine, the sections or the pad routes.
//!
//! A style may carry one of upstream's detail-page names (Hero, Tabs, Sheet,
//! Showcase) **only** when it genuinely is that arrangement; otherwise it gets a
//! name of its own, so a person is never promised a thing the page does not do.
//! Every style after Film ships with a screenshot check against a committed
//! reference — `docs/WATCH-PAGE.md` §6.

/// The page's arrangement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::collection::console::watch_page) enum PageStyle {
    /// The page as built in Phases 1–2: one column of title / marks / list, one
    /// panel of sections.
    #[default]
    Film,
}

impl PageStyle {
    /// The arrangement drawn now. One place, so Phase 4 can read the space's
    /// choice (`console-watch.json`) without the page asking twice.
    pub(in crate::collection::console::watch_page) fn current() -> Self {
        Self::Film
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn film_is_the_arrangement_the_page_draws() {
        assert_eq!(PageStyle::current(), PageStyle::Film);
        assert_eq!(PageStyle::default(), PageStyle::Film, "a fresh page draws Film");
    }
}
