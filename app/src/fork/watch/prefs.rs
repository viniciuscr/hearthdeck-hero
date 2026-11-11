//! The Watch space's console prefs: its own `console-watch.json`, the first
//! Watch theme, and the one-time migration of a game-shaped file.

use std::path::{Path, PathBuf};

use heronite_data::console_theme::{ConsolePrefs, ConsoleTheme};
use heronite_data::json_store;
use heronite_data::paths::AppPaths;

use super::space_theme::project;
use super::watch_theme::watch_theme;
use crate::collection::Space;

pub fn path(app: &AppPaths) -> PathBuf {
    app.config.join("console-watch.json")
}

pub fn load(app: &AppPaths) -> ConsolePrefs {
    let path = path(app);
    let saved = match json_store::read_value(&path) {
        Ok(value) if value.is_object() => serde_json::from_value::<ConsolePrefs>(value).ok(),
        _ => None,
    };
    let mut prefs = match saved {
        Some(prefs) if !is_game_theme(&prefs.theme) => prefs,
        Some(_) => {
            backup(&path);
            let prefs = defaults(app);
            write(&path, &prefs);
            prefs
        }
        None => {
            let prefs = defaults(app);
            write(&path, &prefs);
            prefs
        }
    };
    prefs.theme = project(prefs.theme.normalized(), Space::Watch);
    prefs
}

/// A fresh Watch theme, starting from the Games prefs for the global console
/// options but with its own theme and rails.
pub fn defaults(app: &AppPaths) -> ConsolePrefs {
    let mut prefs = ConsolePrefs::load(app);
    prefs.theme = watch_theme();
    prefs.based_on = "watch".into();
    prefs.last_tab = 0;
    prefs
}

pub fn save(app: &AppPaths, prefs: &ConsolePrefs) -> anyhow::Result<()> {
    json_store::write_indented(&path(app), prefs, b"  ")
}

fn write(path: &Path, prefs: &ConsolePrefs) {
    if let Err(error) = json_store::write_indented(path, prefs, b"  ") {
        log::warn!("writing {}: {error:#}", path.display());
    }
}

/// A Watch file written before the Watch theme existed is a copy of a game
/// theme: it has game shelves or game tab sources. It is migrated once.
fn is_game_theme(theme: &ConsoleTheme) -> bool {
    theme.shelves.iter().any(|shelf| matches!(shelf.kind.as_str(), "installed" | "statuses" | "backlog-short" | "forgotten"))
        || theme
            .tabs
            .iter()
            .any(|tab| matches!(tab.source.as_str(), "installed" | "pc" | "emulated") || tab.name.to_lowercase().contains("games"))
}

fn backup(path: &Path) {
    let backup = path.with_extension("json.bak");
    if backup.exists() {
        return;
    }
    if let Err(error) = std::fs::copy(path, &backup) {
        log::warn!("backing up {}: {error}", path.display());
    }
}
