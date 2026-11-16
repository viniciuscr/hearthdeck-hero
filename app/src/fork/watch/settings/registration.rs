//! How the Watch sections register themselves in the host Settings page: their
//! IDs, group, labels, icons, blurbs, search keywords, URL/debug names and
//! availability. The host keeps the enum variants and the group entry; the
//! values live here.

use heronite_i18n::{n_, tr};

use crate::icons::Icon;
use crate::pages::settings::Section;

/// The Watch space itself: its theme and anything that is nobody's provider.
pub const SECTION: Section = Section::Watch;
/// Providers: what the person configures, one card each, and nothing else.
pub const PROVIDERS: Section = Section::WatchProviders;
/// Home: the rails the first page draws, each with the provider it comes from.
pub const HOME: Section = Section::WatchHome;
pub const GROUP: (&str, &[Section]) = (n_!("Watch"), &[Section::Watch, Section::WatchProviders, Section::WatchHome]);
pub const KEYWORDS: &str = "watch stremio movies series theme reset filmes series tema reiniciar";
pub const PROVIDER_KEYWORDS: &str = "watch providers stremio tmdb cinemeta account link qr api key sign in connect service torrent metadata catalogs filmes series provedores conta chave entrar conectar servico metadados catalogos";
pub const HOME_KEYWORDS: &str = "watch home rails continue watching recommended top rated popular shelf filmes series inicio prateleiras continuar assistindo recomendados destaques";

pub fn available() -> bool {
    heronite_watch::available()
}

pub fn label() -> &'static str {
    tr!("Watch")
}

pub fn providers_label() -> &'static str {
    tr!("Providers")
}

pub fn home_label() -> &'static str {
    tr!("Home")
}

pub fn icon() -> Icon {
    Icon::Videocam
}

pub fn providers_icon() -> Icon {
    Icon::Storefront
}

pub fn home_icon() -> Icon {
    Icon::Home
}

pub fn blurb() -> &'static str {
    tr!("The Watch space itself: its theme, and the rails that belong to nobody's provider.")
}

pub fn providers_blurb() -> &'static str {
    tr!("Every provider Watch can use, one card each: its account or key, whether it is set up, and the one switch that turns it on.")
}

pub fn home_blurb() -> &'static str {
    tr!("What the first page shows: each rail says which provider it comes from, and they are drawn in the order they were turned on.")
}

pub fn parse(value: &str) -> Option<Section> {
    match value {
        "watch" | "movies" | "theme" => Some(Section::Watch),
        "providers" | "catalogs" | "catalog" | "metadata" | "details" | "accounts" => Some(Section::WatchProviders),
        "home" | "rails" | "shelves" => Some(Section::WatchHome),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_registers_its_three_sections_and_parses_each() {
        for section in [Section::Watch, Section::WatchProviders, Section::WatchHome] {
            assert_eq!(Section::ALL.iter().filter(|candidate| **candidate == section).count(), 1);
            assert!(Section::GROUPS.iter().any(|(_, sections)| sections.contains(&section)), "{section:?} is in the Watch group");
        }
        assert_eq!(parse("watch"), Some(Section::Watch));
        assert_eq!(parse("providers"), Some(Section::WatchProviders));
        assert_eq!(parse("catalogs"), Some(Section::WatchProviders), "the old name still opens the page");
        assert_eq!(parse("home"), Some(Section::WatchHome));
        assert_eq!(parse("settings"), None);
    }
}
