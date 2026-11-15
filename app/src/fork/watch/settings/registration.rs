//! How the Watch section registers itself in the host Settings page: its ID,
//! group, label, icon, blurb, search keywords, URL/debug name and availability.
//! The host keeps the enum variant and the group entry; the values live here.

use heronite_i18n::{n_, tr};

use crate::icons::Icon;
use crate::pages::settings::Section;

/// Stream providers: the Stremio account and the service that plays torrents.
pub const SECTION: Section = Section::Watch;
/// Catalog providers: what the Movies and Series tabs may browse.
pub const CATALOGS: Section = Section::WatchCatalogs;
/// Metadata providers: what describes a film or a series on its page.
pub const METADATA: Section = Section::WatchMetadata;
pub const GROUP: (&str, &[Section]) = (n_!("Watch"), &[Section::Watch, Section::WatchCatalogs, Section::WatchMetadata]);
pub const KEYWORDS: &str = "watch stremio movies series stream provider rails continue watching top movies top series account link qr service torrent playback filmes series provedor prateleiras continuar assistindo link conta servico reproducao";
pub const CATALOG_KEYWORDS: &str = "watch catalogs catalogue browse discover tmdb cinemeta api key genres sort search filmes series catalogos navegar descobrir chave generos ordenar buscar";
pub const METADATA_KEYWORDS: &str = "watch metadata details cast crew runtime rating genres tmdb cinemeta sources order filmes series detalhes elenco equipe duracao classificacao generos fontes ordem";

pub fn available() -> bool {
    heronite_watch::available()
}

pub fn label() -> &'static str {
    tr!("Stream providers")
}

pub fn catalogs_label() -> &'static str {
    tr!("Catalog providers")
}

pub fn metadata_label() -> &'static str {
    tr!("Metadata providers")
}

pub fn icon() -> Icon {
    Icon::Videocam
}

pub fn catalogs_icon() -> Icon {
    Icon::GridView
}

pub fn metadata_icon() -> Icon {
    Icon::Storefront
}

pub fn blurb() -> &'static str {
    tr!("The Stremio account and the service that fetches torrents: where titles come from and how they play.")
}

pub fn catalogs_blurb() -> &'static str {
    tr!("What the Movies and Series tabs may browse. A catalog decides what is listed; it never changes what plays.")
}

pub fn metadata_blurb() -> &'static str {
    tr!("What describes a film or a series on its page, asked in this order: the first answer for a field wins, the next fills what is still empty.")
}

pub fn parse(value: &str) -> Option<Section> {
    match value {
        "watch" | "movies" | "streams" | "providers" => Some(Section::Watch),
        "catalogs" | "catalog" | "catalog-providers" => Some(Section::WatchCatalogs),
        "metadata" | "details" | "metadata-providers" => Some(Section::WatchMetadata),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_registers_its_two_sections_and_parses_both() {
        for section in [Section::Watch, Section::WatchCatalogs, Section::WatchMetadata] {
            assert_eq!(Section::ALL.iter().filter(|candidate| **candidate == section).count(), 1);
            assert!(Section::GROUPS.iter().any(|(_, sections)| sections.contains(&section)), "{section:?} is in the Watch group");
        }
        assert_eq!(parse("watch"), Some(Section::Watch));
        assert_eq!(parse("movies"), Some(Section::Watch));
        assert_eq!(parse("catalogs"), Some(Section::WatchCatalogs));
        assert_eq!(parse("settings"), None);
    }
}
