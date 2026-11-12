//! The Watch space owns its menu, tabs and shelves; the editor only changes
//! the look. Game-only cards are hidden too, and these cards are locked so a
//! projection cannot silently undo edits.

use crate::collection::Space;
use crate::console::theme_rows::Card;

pub fn locked(card: Card) -> bool {
    locked_in(crate::console::themes::active_space(), card)
}

/// The space the editor is in, and whether `card` is structural or game-only.
pub fn locked_in(space: Space, card: Card) -> bool {
    space == Space::Watch && matches!(card, Card::Tabs | Card::Shelves | Card::Menu | Card::Hero | Card::GamePage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::console_theme::{presets, ConsolePrefs};

    #[test]
    fn selecting_a_preset_in_watch_is_not_edited() {
        let mut prefs = ConsolePrefs { theme: presets::classic(), based_on: "classic".into(), ..Default::default() };
        prefs.theme = crate::fork::watch::space_theme::project(prefs.theme, Space::Watch);
        assert!(!crate::console::theme_rows::is_edited_for(Space::Watch, &prefs, &[]));
    }

    #[test]
    fn watch_hides_game_only_cards_but_keeps_visuals() {
        for card in [Card::Tabs, Card::Shelves, Card::Menu, Card::Hero, Card::GamePage] {
            assert!(locked_in(Space::Watch, card), "{card:?} is not lockable in Watch");
            assert!(!locked_in(Space::Games, card), "{card:?} is hidden in Games");
        }
        for card in [Card::Theme, Card::Background, Card::Interface, Card::Colours, Card::Controls, Card::Covers] {
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
