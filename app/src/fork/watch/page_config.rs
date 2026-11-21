//! The Watch page's own configuration, kept beside the space's theme in
//! `console-watch.json` (`docs/WATCH-PAGE.md` §2.4, Phase 4).
//!
//! It rides in [`ConsolePrefs::extra`] — the map upstream already flattens and
//! round-trips — so the shared `ConsoleTheme` gains no fork field, and a theme
//! take (`present`) cannot wipe the choice. Only the Watch file ever holds it;
//! the Games file is never written from here.

use heronite_data::console_theme::ConsolePrefs;
use serde_json::Value;

/// The key this configuration lives under in `extra`.
const KEY: &str = "watchPage";

/// Which arrangement the film page draws. Written as its string, read leniently
/// (an unknown value is Film), so a file from a newer build still loads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageStyle {
    /// The page as built in Phases 1–3: one column of title / marks / list, one
    /// panel of sections.
    #[default]
    Film,
}

impl PageStyle {
    pub const ALL: &'static [PageStyle] = &[PageStyle::Film];

    pub fn as_str(self) -> &'static str {
        match self {
            PageStyle::Film => "film",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "film" => Some(PageStyle::Film),
            _ => None,
        }
    }
}

/// The space's page configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageConfig {
    pub style: PageStyle,
}

/// The configuration a space's prefs hold. A missing or unknown value is the
/// default, never an error: a theme file from another build still opens.
pub fn read(prefs: &ConsolePrefs) -> PageConfig {
    let style =
        prefs.extra.get(KEY).and_then(|entry| entry.get("style")).and_then(Value::as_str).and_then(PageStyle::parse).unwrap_or_default();
    PageConfig { style }
}

/// Writes the configuration into the prefs, keeping every other key `extra`
/// carries (another build's settings survive a round trip).
pub fn write(prefs: &mut ConsolePrefs, config: PageConfig) {
    let mut entry = match prefs.extra.get(KEY) {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    entry.insert("style".to_owned(), Value::String(config.style.as_str().to_owned()));
    prefs.extra.insert(KEY.to_owned(), Value::Object(entry));
}

/// Removes the choice, for a file built fresh from another space's prefs: the
/// Games prefs carry no page choice, and a new Watch file must not inherit one.
pub fn clear(prefs: &mut ConsolePrefs) {
    prefs.extra.remove(KEY);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> ConsolePrefs {
        ConsolePrefs::default()
    }

    #[test]
    fn a_space_without_a_choice_draws_film() {
        assert_eq!(read(&prefs()).style, PageStyle::Film, "no key means the page as it is");
    }

    #[test]
    fn a_choice_survives_the_round_trip() {
        let mut p = prefs();
        write(&mut p, PageConfig { style: PageStyle::Film });
        assert_eq!(read(&p).style, PageStyle::Film);
        assert!(p.extra.contains_key(KEY), "the choice is written where the next read finds it");
    }

    #[test]
    fn an_unknown_style_reads_as_film_and_keeps_the_file_loadable() {
        let mut p = prefs();
        p.extra.insert(KEY.to_owned(), serde_json::json!({ "style": "something-newer" }));
        assert_eq!(read(&p).style, PageStyle::Film, "a newer build's style must not break this one");
    }

    #[test]
    fn writing_keeps_the_other_settings_extra_carries() {
        let mut p = prefs();
        p.extra.insert("fromANewerBuild".to_owned(), serde_json::json!({ "a": 1 }));
        write(&mut p, PageConfig { style: PageStyle::Film });
        assert_eq!(p.extra["fromANewerBuild"]["a"], 1, "another build's settings survive a round trip");
    }

    #[test]
    fn every_style_has_a_name_and_reads_back() {
        for style in PageStyle::ALL {
            assert_eq!(PageStyle::parse(style.as_str()), Some(*style), "{} does not read back", style.as_str());
        }
    }

    /// The choice lives in `console-watch.json` and nowhere else: after the
    /// space saves it, the Games file still has no such key (`§2.4`, Phase 4).
    #[test]
    fn the_choice_never_reaches_the_games_file() {
        use heronite_data::paths::DataPaths;

        let dir = tempfile::tempdir().unwrap();
        let app = heronite_data::paths::AppPaths::for_data(&DataPaths::at(dir.path().to_path_buf()));

        // The Games file as it stands, byte for byte, before anything of ours.
        let games_path = app.console_prefs();
        let games_before = std::fs::read(&games_path).ok();

        let mut prefs = super::super::prefs::load(&app);
        write(&mut prefs, PageConfig { style: PageStyle::Film });
        super::super::prefs::save(&app, &prefs).unwrap();

        let again = super::super::prefs::load(&app);
        assert!(again.extra.contains_key(KEY), "the space's own file keeps the choice");
        assert_eq!(games_before, std::fs::read(&games_path).ok(), "saving the Watch choice rewrote the Games file");
    }
}
