//! The Watch space's console prefs: its own `console-watch.json`, and the
//! theme a fresh one starts from.
//!
//! The file holds the theme as the Watch space draws it: its own tabs and
//! rails, with the words this space uses. Loading it only *validates* (drops
//! what Watch cannot show), so a tab the person renamed or added is still
//! there on the next start — presentation happens once, when a theme is taken
//! (see [`crate::fork::watch::space_theme`]).

use std::path::{Path, PathBuf};

use heronite_data::console_theme::{presets, ConsolePrefs, ConsoleTheme};
use heronite_data::json_store;
use heronite_data::paths::AppPaths;

use super::space_theme::{present, validate};
use crate::collection::Space;

/// The upstream theme a fresh Watch lays out for itself: Classic's shelves and
/// cover grid, with the watch tabs and rails added.
const BASE: &str = "classic";

pub fn path(app: &AppPaths) -> PathBuf {
    app.config.join("console-watch.json")
}

/// The preset a Watch theme id names, including the base: upstream lists the
/// themes it offers (`presets::all`), and Classic is the base they start from
/// rather than one of them — so `presets::by_id("classic")` is `None` there.
pub fn preset(id: &str) -> Option<ConsoleTheme> {
    presets::by_id(id).or_else(|| (id == BASE).then(presets::classic))
}

/// The theme a fresh Watch starts from (and what "Reset Watch theme" goes
/// back to).
pub fn default_theme() -> ConsoleTheme {
    present(preset(BASE).unwrap_or_else(presets::classic), Space::Watch)
}

/// The preset id the Watch theme is based on, so the Theme row names it (and
/// does not read "Customised" for a theme nobody edited).
pub fn base_id() -> &'static str {
    BASE
}

pub fn load(app: &AppPaths) -> ConsolePrefs {
    let path = path(app);
    let saved = match json_store::read_value(&path) {
        Ok(value) if value.is_object() => serde_json::from_value::<ConsolePrefs>(value).ok(),
        _ => None,
    };
    match saved {
        Some(mut prefs) => {
            prefs.theme = validate(prefs.theme.normalized(), Space::Watch);
            prefs
        }
        None => {
            let prefs = defaults(app);
            write(&path, &prefs);
            prefs
        }
    }
}

/// A fresh Watch theme, starting from the Games prefs for the global console
/// options but with its own theme and rails.
pub fn defaults(app: &AppPaths) -> ConsolePrefs {
    let mut prefs = ConsolePrefs::load(app);
    prefs.theme = default_theme();
    prefs.based_on = base_id().into();
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

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::console_theme::{Layout, TabSpec};
    use heronite_data::paths::DataPaths;

    fn app_paths(dir: &Path) -> AppPaths {
        AppPaths::for_data(&DataPaths::at(dir.to_path_buf()))
    }

    #[test]
    fn a_watch_file_keeps_its_tabs_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        let mut prefs = defaults(&app);
        prefs.theme.tabs[0].name = "Films".into();
        prefs.theme.tabs.push(TabSpec::new("Games", Layout::Grid, "genre:Drama"));
        save(&app, &prefs).unwrap();

        let again = load(&app);
        assert_eq!(again.theme.tabs[0].name, "Films", "a renamed tab survives the next load");
        let drama = again.theme.tabs.iter().find(|tab| tab.source == "genre:Drama").expect("the added tab is kept");
        assert_eq!(drama.name, "Games", "a tab named like the games is not mistaken for a game file");
    }

    #[test]
    fn a_fresh_watch_starts_from_a_preset_it_can_name() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_paths(dir.path());
        let prefs = load(&app);
        assert_eq!(prefs.based_on, base_id(), "the Theme row can name the theme it started from");
        assert!(prefs.theme.tabs.iter().any(|tab| tab.source == "type:movie"));
        assert!(prefs.theme.tabs.iter().any(|tab| tab.source == "shelf"));
        assert!(!crate::console::theme_rows::is_edited_for(Space::Watch, &prefs, &[]), "a fresh Watch is not Customised");
    }
}
