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

            let home_layout = theme.tabs.first().map_or(Layout::Shelves, |tab| tab.layout);
            theme.tabs.retain(|tab| {
                matches!(tab.source.as_str(), "all" | "favourites")
                    || tab.source.starts_with("genre:")
                    || tab.source.starts_with("store:")
                    || tab.source.starts_with("type:")
            });
            if !theme.tabs.iter().any(|tab| tab.source == "all") {
                theme.tabs.insert(0, TabSpec::new(tr!("Watch"), home_layout, "all"));
            }
            for tab in &mut theme.tabs {
                if tab.source == "all" {
                    tab.name = tr!("Watch").to_string();
                } else if tab.source == "type:movie" {
                    tab.name = tr!("Movies").to_string();
                } else if tab.source == "type:series" {
                    tab.name = tr!("Series").to_string();
                }
            }
            for (source, name) in [("type:movie", tr!("Movies")), ("type:series", tr!("Series"))] {
                if !theme.tabs.iter().any(|tab| tab.source == source) {
                    theme.tabs.push(TabSpec::new(name, Layout::Grid, source));
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
            theme.interface.label = tr!("Watch").to_uppercase();
            theme
        }
    }
}
