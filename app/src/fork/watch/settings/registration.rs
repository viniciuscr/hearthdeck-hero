//! How the Watch section registers itself in the host Settings page: its ID,
//! group, label, icon, blurb, search keywords, URL/debug name and availability.
//! The host keeps the enum variant and the group entry; the values live here.

use heronite_i18n::{n_, tr};

use crate::icons::Icon;
use crate::pages::settings::Section;

pub const SECTION: Section = Section::Watch;
pub const GROUP: (&str, &[Section]) = (n_!("Watch"), &[Section::Watch]);
pub const KEYWORDS: &str =
    "watch stremio movies series provider rails continue watching top movies top series account link qr filmes series provedor prateleiras continuar assistindo link conta";

pub fn available() -> bool {
    heronite_watch::available()
}

pub fn label() -> &'static str {
    tr!("Watch")
}

pub fn icon() -> Icon {
    Icon::Videocam
}

pub fn blurb() -> &'static str {
    tr!("The Stremio account, the provider and the Watch home rails: Continue watching, Top movies and Top series.")
}

pub fn parse(value: &str) -> Option<Section> {
    matches!(value, "watch" | "movies").then_some(Section::Watch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_registers_once_and_parses() {
        assert_eq!(Section::ALL.iter().filter(|section| **section == Section::Watch).count(), 1);
        assert!(Section::GROUPS.iter().any(|(_, sections)| sections.contains(&Section::Watch)));
        assert_eq!(parse("watch"), Some(Section::Watch));
        assert_eq!(parse("movies"), Some(Section::Watch));
        assert_eq!(parse("settings"), None);
    }
}
