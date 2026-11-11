//! The built-in Watch console theme: films and series, not a transformed
//! game theme. It is fork-owned; upstream presets stay untouched.

use heronite_data::console_theme::{presets, ConsoleTheme, Layout, MenuItemSpec, ShelfSpec, TabSpec};
use heronite_i18n::tr;

pub fn watch_theme() -> ConsoleTheme {
    let mut theme = presets::classic();
    theme.id = "watch".into();
    theme.name = tr!("Watch").to_string();
    theme.author = "Hearthdeck".into();
    theme.description = tr!("Films and series: Continue watching, Top movies and Top series.").to_string();
    theme.tabs = vec![
        TabSpec::new(tr!("Home"), Layout::Shelves, "all"),
        TabSpec::new(tr!("Movies"), Layout::Grid, "type:movie"),
        TabSpec::new(tr!("Series"), Layout::Grid, "type:series"),
    ];
    theme.shelves = vec![
        ShelfSpec { kind: "recent".into(), on: true, title: tr!("Continue watching").to_string() },
        ShelfSpec { kind: "provider:top-movies".into(), on: true, title: tr!("Top movies").to_string() },
        ShelfSpec { kind: "provider:top-series".into(), on: true, title: tr!("Top series").to_string() },
    ];
    theme.menu.items = ["watch", "customize", "settings", "separator", "desktop", "power"].into_iter().map(MenuItemSpec::new).collect();
    theme.interface.label = tr!("Watch").to_uppercase();
    theme
}
