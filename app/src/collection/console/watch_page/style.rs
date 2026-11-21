//! Which arrangement the page draws, read from the space's own file.
//!
//! Fork-owned (`docs/WATCH-PAGE.md`, Phase 3; collapsed onto the stored choice
//! in Phase 5). There is one vocabulary, not two: the variants live in
//! [`crate::fork::watch::page_config`], which is also what Settings → Watch
//! writes, so a style cannot exist in one place and not the other.
//!
//! A style may carry one of upstream's detail-page names (Hero, Tabs, Sheet,
//! Showcase) **only** when it genuinely is that arrangement; otherwise it gets a
//! name of its own. Every style after Film ships with a screenshot check against
//! a committed reference — `docs/WATCH-PAGE.md` §10.

use heronite_data::paths::AppPaths;

use crate::collection::Space;
use crate::fork::watch::page_config::{self, PageStyle};

/// The arrangement drawn now: the space's stored choice, read from the Watch
/// prefs whatever space is on screen, or Film when the file says nothing this
/// build knows.
pub(in crate::collection::console::watch_page) fn current(app: &AppPaths) -> PageStyle {
    crate::console::themes::read_for(app, Space::Watch, |prefs| page_config::read(prefs).style)
}
