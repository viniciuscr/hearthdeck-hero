//! `console.json` at run time: read once when first needed (console mode
//! starting, its Settings section shown), changed in memory, and written off
//! the UI thread (the latest version wins; writes never overlap).
//!
//! Games and Watch keep separate prefs: `console.json` for games,
//! `console-watch.json` for the Watch space. The first Watch session starts
//! from the Games theme and turns its shelves into watch rails (Continue
//! watching, Top movies, Top series), so the two spaces can be customized
//! independently.
//!
//! [`update`] is the one way to change the theme in use: it also installs it
//! in [`super::look`] while console mode is on, so the screen follows at once.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

use heronite_data::console_theme::{ConsolePrefs, ConsoleTheme, KeptFilters};
use heronite_data::paths::AppPaths;

use crate::collection::Space;
use crate::fork::watch::space_theme::project;

const GAMES: u8 = 0;
const WATCH: u8 = 1;

static SPACE: AtomicU8 = AtomicU8::new(GAMES);
static PREFS: Mutex<[Option<ConsolePrefs>; 2]> = Mutex::new([None, None]);
static PENDING: Mutex<Option<(AppPaths, ConsolePrefs, Space)>> = Mutex::new(None);
static WRITING: AtomicBool = AtomicBool::new(false);

fn index(space: Space) -> usize {
    match space {
        Space::Games => GAMES as usize,
        Space::Watch => WATCH as usize,
    }
}

/// Tells the console which space is on screen; the next `prefs`/`update`
/// reads and writes that space's file.
pub fn set_space(space: Space) {
    SPACE.store(index(space) as u8, Ordering::Relaxed);
}

fn current_space() -> Space {
    if SPACE.load(Ordering::Relaxed) == WATCH {
        Space::Watch
    } else {
        Space::Games
    }
}

/// The space whose console prefs are active (Settings uses it).
pub fn active_space() -> Space {
    current_space()
}

fn load(app: &AppPaths, space: Space) -> ConsolePrefs {
    match space {
        Space::Games => {
            let mut prefs = ConsolePrefs::load(app);
            prefs.theme = project(prefs.theme, Space::Games);
            prefs
        }
        Space::Watch => crate::fork::watch::prefs::load(app),
    }
}

/// The prefs, read from disk the first time.
pub fn prefs(app: &AppPaths) -> ConsolePrefs {
    read(app, ConsolePrefs::clone)
}

/// The prefs of one space, without switching the active space.
pub fn prefs_for(app: &AppPaths, space: Space) -> ConsolePrefs {
    read_for(app, space, ConsolePrefs::clone)
}

/// `f` over one space's prefs without copying them (it must be quick: it holds the lock).
pub fn read_for<R>(app: &AppPaths, space: Space, f: impl FnOnce(&ConsolePrefs) -> R) -> R {
    let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
    let slot = &mut cur[index(space)];
    f(slot.get_or_insert_with(|| load(app, space)))
}

/// `f` over the prefs without copying them (it must be quick: it holds the lock).
pub fn read<R>(app: &AppPaths, f: impl FnOnce(&ConsolePrefs) -> R) -> R {
    read_for(app, current_space(), f)
}

/// Keeps `tab` as the one console mode reopens on. Unlike [`update`] it leaves
/// the theme in use alone (installing it again would restart the screen's animations).
pub fn remember_tab(app: &AppPaths, tab: usize) {
    let space = current_space();
    let next = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur[index(space)].get_or_insert_with(|| load(app, space));
        if p.last_tab == tab {
            return;
        }
        p.last_tab = tab;
        p.clone()
    };
    save(app, next, space);
}

/// Keeps the filters console mode starts with next time (like
/// [`remember_tab`], the theme in use is left alone).
pub fn remember_filters(app: &AppPaths, filters: KeptFilters) {
    let next = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur.get_or_insert_with(|| ConsolePrefs::load(app));
        if p.kept_filters == filters {
            return;
        }
        p.kept_filters = filters;
        p.clone()
    };
    save(app, next);
}

/// Changes the prefs, installs the theme while console mode is on and saves.
pub fn update(app: &AppPaths, change: impl FnOnce(&mut ConsolePrefs)) -> ConsolePrefs {
    update_for(app, current_space(), change)
}

/// Changes one space's prefs and saves them there. It installs the theme only
/// when that space is the one on screen.
pub fn update_for(app: &AppPaths, space: Space, change: impl FnOnce(&mut ConsolePrefs)) -> ConsolePrefs {
    let (before, next) = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur[index(space)].get_or_insert_with(|| load(app, space));
        let before = p.theme.clone();
        change(p);
        p.theme = project(std::mem::take(&mut p.theme).normalized(), space);
        (before, p.clone())
    };
    // Covers of a shape the new theme draws and the old one did not.
    crate::dialogs::metadata_update::families::console_theme_changed(&before, &next.theme);
    if space == current_space() && super::active() {
        super::look::set(next.theme.clone());
    }
    save(app, next.clone(), space);
    next
}

/// Uses `theme` (from a preset or a saved theme when `based_on` is given).
pub fn apply(app: &AppPaths, theme: ConsoleTheme, based_on: Option<&str>) -> ConsolePrefs {
    update(app, |p| {
        p.theme = theme;
        if let Some(b) = based_on {
            p.based_on = b.to_string();
        }
    })
}

/// Forgets the copies in memory (console mode ended and Settings is closed).
pub fn forget() {
    // Desktop Settings after console mode must edit the Games theme; the
    // Watch page selects its space explicitly with `prefs_for`.
    SPACE.store(GAMES, Ordering::Relaxed);
    *PREFS.lock().unwrap_or_else(|e| e.into_inner()) = [None, None];
}

fn save(app: &AppPaths, prefs: ConsolePrefs, space: Space) {
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some((app.clone(), prefs, space));
    if WRITING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| loop {
        let next = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some((app, prefs, space)) = next else {
            WRITING.store(false, Ordering::Release);
            // A change may have come in between the take and the store.
            if PENDING.lock().unwrap_or_else(|e| e.into_inner()).is_some() && !WRITING.swap(true, Ordering::AcqRel) {
                continue;
            }
            break;
        };
        if let Err(e) = write(&app, &prefs, space) {
            log::warn!("saving console settings: {e:#}");
        }
    });
}

fn write(app: &AppPaths, prefs: &ConsolePrefs, space: Space) -> anyhow::Result<()> {
    match space {
        Space::Games => prefs.save(app),
        Space::Watch => crate::fork::watch::prefs::save(app, prefs),
    }
}
