//! The Watch space edits its own tabs, shelves and hero; the cards that belong
//! to games are hidden.
//!
//! - **Tabs and shelves** are editable: the sources and kinds the pickers offer
//!   are the space's (`theme_rows::source_options`, `addable_kinds`), and the
//!   theme file keeps what the person chose (`space_theme::validate` only
//!   removes what Watch cannot draw).
//! - **Hero** is editable: its two game-worded lines say watch facts instead
//!   (`CollectionView::console_facts` / `console_stats`).
//! - **Shelves** stay locked: in Watch the first page's rails are the ones
//!   Settings → Watch → Home lists, so a theme's shelves would be a second,
//!   invisible owner of the same screen.
//! - **Menu** stays locked: its entries are the shell's and Watch has no
//!   stores, downloads or emulators to put in them.
//! - **Game page** stays locked: a film page is fork-owned (`watch_page.rs`) and
//!   reads none of `theme.game_page`.

use crate::collection::Space;
use crate::console::theme_rows::Card;

/// The space the editor is in, and whether `card` is game-only.
pub fn locked_in(space: Space, card: Card) -> bool {
    space == Space::Watch && matches!(card, Card::Menu | Card::GamePage | Card::Shelves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::watch::space_theme::present;
    use heronite_data::console_theme::{presets, ConsolePrefs};

    #[test]
    fn selecting_a_preset_in_watch_is_not_edited() {
        let mut prefs = ConsolePrefs { theme: presets::classic(), based_on: "classic".into(), ..Default::default() };
        prefs.theme = present(presets::classic(), Space::Watch);
        assert!(!crate::console::theme_rows::is_edited_for(Space::Watch, &prefs, &[]));
    }

    #[test]
    fn a_tab_edited_in_watch_counts_as_edited() {
        use heronite_data::console_theme::{Layout, TabSpec};
        let mut prefs = ConsolePrefs { theme: present(presets::classic(), Space::Watch), based_on: "classic".into(), ..Default::default() };
        prefs.theme.tabs.push(TabSpec::new("Drama", Layout::Grid, "genre:Drama"));
        assert!(prefs.theme.tabs.len() > 2);
        assert!(crate::console::theme_rows::is_edited_for(Space::Watch, &prefs, &[]), "a tab the theme did not bring is an edit");
    }

    #[test]
    fn watch_hides_only_the_game_cards() {
        for card in [Card::Menu, Card::GamePage, Card::Shelves] {
            assert!(locked_in(Space::Watch, card), "{card:?} is not lockable in Watch");
            assert!(!locked_in(Space::Games, card), "{card:?} is hidden in Games");
        }
        for card in [Card::Theme, Card::Tabs, Card::Hero, Card::Background, Card::Interface, Card::Colours, Card::Controls, Card::Covers] {
            assert!(!locked_in(Space::Watch, card), "{card:?} should stay editable in Watch");
        }
    }

    #[test]
    fn upstream_presets_are_still_part_of_the_theme_list() {
        let all = presets::all();
        assert!(all.len() >= 7, "the built-in theme registry lost themes");
        for id in ["minimal", "classic", "cinema", "pulse", "arcade", "hub", "living-room"] {
            assert!(all.iter().any(|theme| theme.id == id), "{id} is missing from the presets");
        }
        let list = crate::console::theme_rows::theme_list(&[]);
        assert!(list.len() >= 7, "the Theme picker lost the built-in themes");
    }
}
