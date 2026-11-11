//! The Watch space owns its menu, tabs and shelves; the editor only changes
//! the look. Locking these cards prevents edits that projection would undo.

use crate::collection::Space;
use crate::console::theme_rows::Card;

pub fn locked(card: Card) -> bool {
    crate::console::themes::active_space() == Space::Watch && matches!(card, Card::Tabs | Card::Shelves | Card::Menu)
}
