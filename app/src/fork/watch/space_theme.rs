//! A console theme through a space. The theme keeps its look — colours,
//! background, tiles, fonts, layout — while the content structure (menu, tabs,
//! shelves) is restricted to what that space can show. Upstream presets stay
//! untouched.
//!
//! Two steps, and the difference between them is what keeps the Watch editor
//! honest:
//!
//! - [`validate`] runs on every load and after every change. It only
//!   **removes** what the space cannot show — game tab sources in Watch, the
//!   Watch tab and rails in Games, the shell-level menu entries — so it is
//!   idempotent and can never undo an edit.
//! - [`present`] runs when a theme is **taken** (selected, imported, reset, or
//!   the space's first run). It validates, then adds the tabs and rails the
//!   space needs to be usable (Movies, Series, Shelf, Continue watching),
//!   fitted to `MAX_TABS`. It never renames or reorders what the theme already
//!   brought.

use heronite_data::console_theme::{ConsoleTheme, Layout, MenuItemSpec, TabSpec, MAX_TABS};
use heronite_i18n::tr;

use crate::collection::Space;

/// The words the upstream presets use for a home of games. A space of films
/// keeps the tab and its layout and reads `Watch` instead.
const GAME_WORDS: [&str; 2] = ["All games", "Games"];

/// The theme with what `space` cannot show removed. Idempotent: applying it to
/// its own result changes nothing, so it may run on every load.
pub fn validate(mut theme: ConsoleTheme, space: Space) -> ConsoleTheme {
    match space {
        Space::Games => {
            // The space switch is shell-level; the theme keeps pages only.
            theme.menu.items.retain(|item| !matches!(item.item.as_str(), "collection" | "watch" | "games"));
            // A Watch theme's tabs and rails are not games'.
            theme.tabs.retain(|tab| !tab.source.starts_with("type:") && tab.source != "shelf");
            theme.shelves.retain(|shelf| !shelf.kind.starts_with("provider:"));
            theme
        }
        Space::Watch => {
            // Statistics is the Watch recap here, so the menu keeps its entry.
            theme
                .menu
                .items
                .retain(|item| matches!(item.item.as_str(), "statistics" | "customize" | "settings" | "desktop" | "power" | "separator"));
            // Inserted at the top in reverse, so the menu reads statistics, customize.
            for needed in ["customize", "statistics"] {
                if !theme.menu.items.iter().any(|item| item.item == needed) {
                    theme.menu.items.insert(0, MenuItemSpec::new(needed));
                }
            }
            theme.tabs = theme.tabs.iter().filter_map(watch_tab).collect();
            // Two `favourites` tabs become one Shelf.
            let mut shelf = false;
            theme.tabs.retain(|tab| {
                if tab.source != "shelf" {
                    return true;
                }
                let first = !shelf;
                shelf = true;
                first
            });
            if theme.tabs.is_empty() {
                theme.tabs.push(watch_home());
            }
            for tab in &mut theme.tabs {
                if tab.source == "all" && GAME_WORDS.contains(&tab.name.as_str()) {
                    tab.name = tr!("Watch").to_string();
                }
            }
            // The first page is the rails Settings lists: a theme's shelves would
            // be a second, invisible owner of the same screen.
            theme.shelves.clear();
            theme
        }
    }
}

/// The theme ready for `space`: [`validate`], then the structure the space
/// needs to be usable. Applied when a theme is taken, never on load.
pub fn present(theme: ConsoleTheme, space: Space) -> ConsoleTheme {
    let mut theme = validate(theme, space);
    if space != Space::Watch {
        return theme;
    }
    // The catalogues are how a person finds something to watch; a theme that
    // brings neither gets them, if the tabs it brought leave room.
    for (source, name, layout) in [("type:movie", tr!("Movies"), Layout::Grid), ("type:series", tr!("Series"), Layout::Grid)] {
        if theme.tabs.len() >= MAX_TABS {
            break;
        }
        if !theme.tabs.iter().any(|tab| tab.source == source) {
            theme.tabs.push(TabSpec::new(name, layout, source));
        }
    }
    if theme.tabs.len() < MAX_TABS && !theme.tabs.iter().any(|tab| tab.source == "shelf") {
        theme.tabs.push(TabSpec::new(tr!("Shelf"), Layout::Shelves, "shelf"));
    }
    // No shelves: Settings → Watch → Home owns the first page's rails, and a
    // theme brought here has none of them.
    theme
}

/// The tab the space starts from when a theme has none it can draw.
fn watch_home() -> TabSpec {
    TabSpec::new(tr!("Watch"), Layout::Shelves, "all")
}

/// A tab as the Watch space can show it (`None`: it cannot).
///
/// The theme's words and layout are kept: only the sources Watch cannot
/// resolve are dropped, and a `favourites` tab becomes the Shelf, which is
/// drawn as rows of covers whatever the theme's layout for it was.
fn watch_tab(tab: &TabSpec) -> Option<TabSpec> {
    let source = tab.source.as_str();
    if source == "favourites" {
        return Some(TabSpec::new(tr!("Shelf"), Layout::Shelves, "shelf"));
    }
    let keep =
        matches!(source, "all" | "type:movie" | "type:series" | "shelf") || source.starts_with("genre:") || source.starts_with("store:");
    keep.then(|| tab.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::console_theme::{presets, ShelfSpec};

    #[test]
    fn a_preset_gets_one_tab_per_watch_source() {
        let theme = present(presets::cinema(), Space::Watch);
        assert_eq!(theme.tabs.iter().filter(|tab| tab.source == "all").count(), 2, "the preset's own two tabs survive");
        assert_eq!(theme.tabs.iter().filter(|tab| tab.source == "type:movie").count(), 1);
        assert_eq!(theme.tabs.iter().filter(|tab| tab.source == "type:series").count(), 1);
        assert_eq!(theme.tabs.iter().filter(|tab| tab.source == "shelf").count(), 1);
        assert!(theme.tabs.len() <= MAX_TABS);
        assert!(theme.shelves.is_empty(), "the first page's rails are Settings', not a theme's");
    }

    #[test]
    fn a_presets_layouts_reach_watch() {
        let theme = present(presets::living_room(), Space::Watch);
        assert_eq!(theme.tabs[0].layout, Layout::Xbox, "the Series X home is the theme's");
        let cinema = present(presets::cinema(), Space::Watch);
        assert_eq!(cinema.tabs[0].layout, Layout::Cinema);
        assert!(cinema.tabs.iter().any(|tab| tab.layout == Layout::Grid), "the preset's Library grid survives");
    }

    #[test]
    fn validate_is_idempotent_and_keeps_edits() {
        let mut theme = present(presets::classic(), Space::Watch);
        theme.tabs[0].name = "Films".into();
        theme.tabs[1].source = "genre:Drama".into();
        let once = validate(theme.clone(), Space::Watch);
        let twice = validate(once.clone(), Space::Watch);
        assert_eq!(once, twice, "validating a validated theme changes nothing");
        assert_eq!(once.tabs[0].name, "Films", "a name the person chose is not rewritten");
        assert_eq!(once.tabs[1].source, "genre:Drama");
    }

    #[test]
    fn watch_projection_keeps_the_theme_look() {
        let mut theme = presets::cinema();
        theme.colors.background = "#112233".into();
        theme.interface.label = "My channel".into();
        let projected = present(theme, Space::Watch);
        assert_eq!(projected.colors.background, "#112233");
        assert_eq!(projected.interface.label, "My channel");
    }

    #[test]
    fn a_shelf_tab_does_not_reach_the_games() {
        let mut theme = presets::classic();
        theme.tabs.push(TabSpec::new("Shelf", Layout::Shelves, "shelf"));
        theme.shelves.push(ShelfSpec { kind: "provider:top-movies".into(), on: true, title: "Top movies".into() });
        let games = validate(theme, Space::Games);
        assert!(games.tabs.iter().all(|tab| tab.source != "shelf"));
        assert!(games.shelves.iter().all(|shelf| !shelf.kind.starts_with("provider:")));
    }

    #[test]
    fn the_upstream_home_word_is_read_as_watch() {
        let mut theme = presets::cinema();
        let named = theme.tabs.iter().any(|tab| tab.name == "All games");
        assert!(named, "Cinema's home tab is the case this covers");
        let watch = validate(theme.clone(), Space::Watch);
        assert_eq!(watch.tabs[0].name, "Watch");
        // A name the person chose stays.
        theme.tabs[0].name = "Films".into();
        assert_eq!(validate(theme, Space::Watch).tabs[0].name, "Films");
    }

    #[test]
    fn the_watch_menu_carries_statistics_because_it_is_the_recap_there() {
        for theme in [presets::minimal(), presets::classic(), presets::cinema()] {
            let watch = present(theme, Space::Watch);
            assert!(watch.menu.items.iter().any(|item| item.item == "statistics"), "☰ → Statistics opens the recap in Watch");
            assert!(watch.menu.items.iter().any(|item| item.item == "customize"));
        }
        // Whatever order a theme had, statistics comes before customize when added.
        let mut theme = presets::classic();
        theme.menu.items.retain(|item| item.item == "settings");
        let watch = present(theme, Space::Watch);
        let at = |item: &str| watch.menu.items.iter().position(|entry| entry.item == item).unwrap_or(usize::MAX);
        assert!(at("statistics") < at("customize"), "the menu reads item by item: {:?}", watch.menu.items);
    }

    #[test]
    fn a_theme_brings_no_rails_to_watch() {
        let mut theme = presets::classic();
        theme.shelves.push(ShelfSpec { kind: "provider:top-movies".into(), on: true, title: "Top movies".into() });
        let watch = present(theme, Space::Watch);
        assert!(watch.shelves.is_empty(), "Settings → Watch → Home owns the first page");
    }

    #[test]
    fn two_favourites_tabs_become_one_shelf() {
        let mut theme = presets::cinema();
        theme.tabs.push(TabSpec::new("Favourites", Layout::Grid, "favourites"));
        let watch = validate(theme, Space::Watch);
        assert_eq!(watch.tabs.iter().filter(|tab| tab.source == "shelf").count(), 1);
    }

    #[test]
    fn a_theme_with_no_usable_tab_gets_a_home() {
        let mut theme = presets::classic();
        theme.tabs = vec![TabSpec::new("Installed games", Layout::Grid, "installed")];
        let watch = validate(theme, Space::Watch);
        assert_eq!(watch.tabs.len(), 1);
        assert_eq!(watch.tabs[0].source, "all");
    }
}
