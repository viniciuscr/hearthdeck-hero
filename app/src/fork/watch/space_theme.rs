//! Projects a console theme through a space. The theme keeps its look — colours,
//! background, tiles, fonts, layout — while the content structure (menu, tabs,
//! shelves) is restricted to what that space can show. Upstream presets stay
//! untouched.

use heronite_data::console_theme::{ConsoleTheme, Layout, MenuItemSpec, ShelfSpec, TabSpec};
use heronite_i18n::tr;

use crate::collection::Space;

pub fn project(mut theme: ConsoleTheme, space: Space) -> ConsoleTheme {
    match space {
        Space::Games => {
            // The space switch is shell-level; the theme keeps pages only.
            theme.menu.items.retain(|item| !matches!(item.item.as_str(), "collection" | "watch" | "games"));
            theme.tabs.retain(|tab| !tab.source.starts_with("type:"));
            theme.shelves.retain(|shelf| !shelf.kind.starts_with("provider:"));
            theme
        }
        Space::Watch => {
            theme.menu.items.retain(|item| matches!(item.item.as_str(), "customize" | "settings" | "desktop" | "power" | "separator"));
            if !theme.menu.items.iter().any(|item| item.item == "customize") {
                theme.menu.items.push(MenuItemSpec::new("customize"));
            }

            let current = std::mem::take(&mut theme.tabs);
            let home = current
                .iter()
                .find(|tab| tab.source == "all" && tab.layout == Layout::Shelves)
                .or_else(|| current.iter().find(|tab| tab.source == "all"))
                .cloned()
                .unwrap_or_else(|| TabSpec::new(tr!("Watch"), Layout::Shelves, "all"));
            let mut tabs = vec![home];
            for (source, name) in [("type:movie", tr!("Movies")), ("type:series", tr!("Series"))] {
                if !tabs.iter().any(|tab| tab.source == source) {
                    tabs.push(TabSpec::new(name, Layout::Grid, source));
                }
            }
            for tab in &current {
                if matches!(tab.source.as_str(), "all" | "type:movie" | "type:series") {
                    continue;
                }
                if (matches!(tab.source.as_str(), "favourites") || tab.source.starts_with("genre:") || tab.source.starts_with("store:"))
                    && !tabs.iter().any(|existing| existing.source == tab.source)
                {
                    tabs.push(tab.clone());
                }
            }
            theme.tabs = tabs;
            for tab in &mut theme.tabs {
                if tab.source == "all" {
                    tab.name = tr!("Watch").to_string();
                } else if tab.source == "type:movie" {
                    tab.name = tr!("Movies").to_string();
                } else if tab.source == "type:series" {
                    tab.name = tr!("Series").to_string();
                }
            }

            theme.shelves.retain(|shelf| {
                matches!(shelf.kind.as_str(), "recent" | "favourites")
                    || shelf.kind.starts_with("genre:")
                    || shelf.kind.starts_with("store:")
                    || shelf.kind.starts_with("provider:")
            });
            for shelf in &mut theme.shelves {
                if shelf.kind == "recent" && shelf.title.trim().is_empty() {
                    shelf.title = tr!("Continue watching").to_string();
                }
            }
            if !theme.shelves.iter().any(|shelf| shelf.kind == "recent") {
                theme.shelves.insert(0, ShelfSpec { kind: "recent".into(), on: true, title: tr!("Continue watching").to_string() });
            }
            for (kind, title) in [("provider:top-movies", tr!("Top movies")), ("provider:top-series", tr!("Top series"))] {
                if !theme.shelves.iter().any(|shelf| shelf.kind == kind) {
                    theme.shelves.push(ShelfSpec { kind: kind.into(), on: true, title: title.to_string() });
                }
            }
            theme
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::console_theme::presets;

    #[test]
    fn a_preset_gets_one_tab_per_watch_source() {
        let theme = project(presets::cinema(), Space::Watch);
        assert_eq!(theme.tabs.iter().filter(|tab| tab.source == "all").count(), 1);
        assert_eq!(theme.tabs.iter().filter(|tab| tab.source == "type:movie").count(), 1);
        assert_eq!(theme.tabs.iter().filter(|tab| tab.source == "type:series").count(), 1);
        assert!(theme.shelves.iter().all(|shelf| !matches!(shelf.kind.as_str(), "installed" | "statuses" | "backlog-short" | "forgotten")));
    }

    #[test]
    fn watch_projection_keeps_the_theme_look() {
        let mut theme = presets::cinema();
        theme.colors.background = "#112233".into();
        theme.interface.label = "My channel".into();
        let projected = project(theme, Space::Watch);
        assert_eq!(projected.colors.background, "#112233");
        assert_eq!(projected.interface.label, "My channel");
    }
}
