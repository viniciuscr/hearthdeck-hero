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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

use heronite_data::console_theme::{ConsolePrefs, ConsoleTheme, ShelfSpec};
use heronite_data::json_store;
use heronite_data::paths::AppPaths;
use heronite_i18n::tr;

use crate::collection::Space;

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

fn watch_path(app: &AppPaths) -> PathBuf {
    app.config.join("console-watch.json")
}

fn load(app: &AppPaths, space: Space) -> ConsolePrefs {
    match space {
        Space::Games => ConsolePrefs::load(app),
        Space::Watch => {
            let path = watch_path(app);
            let saved = match json_store::read_value(&path) {
                Ok(value) if value.is_object() => serde_json::from_value::<ConsolePrefs>(value).ok(),
                _ => None,
            };
            let mut prefs = saved.unwrap_or_else(|| watch_from(ConsolePrefs::load(app)));
            prefs.theme = prefs.theme.normalized();
            prefs
        }
    }
}

/// The first Watch session starts from the Games theme, with watch rails and
/// labels. It is only a starting point: the user's saved theme wins next time.
fn watch_from(mut prefs: ConsolePrefs) -> ConsolePrefs {
    let theme = &mut prefs.theme;
    theme.tabs.retain(|tab| {
        matches!(tab.source.as_str(), "all" | "installed" | "favourites")
            || tab.source.starts_with("genre:")
            || tab.source.starts_with("store:")
    });
    for tab in &mut theme.tabs {
        if matches!(tab.source.as_str(), "all" | "installed") {
            tab.name = tr!("Watch").to_string();
            tab.source = "all".into();
        }
    }
    theme.shelves.retain(|shelf| {
        matches!(shelf.kind.as_str(), "recent" | "favourites") || shelf.kind.starts_with("genre:") || shelf.kind.starts_with("store:")
    });
    for shelf in &mut theme.shelves {
        if shelf.kind == "recent" {
            shelf.title = tr!("Continue watching").to_string();
        }
    }
    if !theme.shelves.iter().any(|shelf| shelf.kind == "recent") {
        theme.shelves.insert(0, ShelfSpec { kind: "recent".into(), on: true, title: tr!("Continue watching").to_string() });
    }
    if !theme.shelves.iter().any(|shelf| shelf.kind == "provider:top-movies") {
        theme.shelves.push(ShelfSpec { kind: "provider:top-movies".into(), on: true, title: tr!("Top movies").to_string() });
    }
    if !theme.shelves.iter().any(|shelf| shelf.kind == "provider:top-series") {
        theme.shelves.push(ShelfSpec { kind: "provider:top-series".into(), on: true, title: tr!("Top series").to_string() });
    }
    prefs
}

/// The prefs, read from disk the first time.
pub fn prefs(app: &AppPaths) -> ConsolePrefs {
    read(app, ConsolePrefs::clone)
}

/// `f` over the prefs without copying them (it must be quick: it holds the lock).
pub fn read<R>(app: &AppPaths, f: impl FnOnce(&ConsolePrefs) -> R) -> R {
    let space = current_space();
    let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
    let slot = &mut cur[index(space)];
    f(slot.get_or_insert_with(|| load(app, space)))
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

/// Changes the prefs, installs the theme while console mode is on and saves.
pub fn update(app: &AppPaths, change: impl FnOnce(&mut ConsolePrefs)) -> ConsolePrefs {
    let space = current_space();
    let next = {
        let mut cur = PREFS.lock().unwrap_or_else(|e| e.into_inner());
        let p = cur[index(space)].get_or_insert_with(|| load(app, space));
        change(p);
        p.theme = std::mem::take(&mut p.theme).normalized();
        p.clone()
    };
    if super::active() {
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
        Space::Watch => json_store::write_indented(&watch_path(app), prefs, b"  "),
    }
}
