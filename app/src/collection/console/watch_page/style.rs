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
    /// The arrangement drawn now: the space's stored choice (`console-watch.json`,
    /// read from the Watch prefs whatever space is on screen), or Film when the
    /// file says nothing this build knows.
    pub(in crate::collection::console::watch_page) fn current(app: &heronite_data::paths::AppPaths) -> Self {
        let prefs = crate::console::themes::prefs_for(app, crate::collection::Space::Watch);
        Self::from_config(crate::fork::watch::page_config::read(&prefs).style)
    }

    /// The view's reading of the stored choice.
    fn from_config(style: crate::fork::watch::page_config::PageStyle) -> Self {
        match style {
            crate::fork::watch::page_config::PageStyle::Film => PageStyle::Film,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn film_is_the_arrangement_every_stored_choice_reads_as() {
        use crate::fork::watch::page_config::PageStyle as Stored;
        for choice in Stored::ALL {
            assert_eq!(PageStyle::from_config(*choice), PageStyle::Film, "{} does not draw Film", choice.as_str());
        }
        assert_eq!(PageStyle::default(), PageStyle::Film, "a fresh page draws Film");
    }
}
