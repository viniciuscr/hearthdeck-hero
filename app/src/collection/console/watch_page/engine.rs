//! The film page's geometry and focus routing, as plain numbers: how many marks
//! a title draws, which mark or episode a list row is, and the presses a click
//! turns into.
//!
//! Fork-owned (`docs/WATCH-PAGE.md`, Phase 2). Nothing here draws or reads a
//! record, so it is tested without a window, and the page's render and pad
//! handler take their routes from the same functions — a click and a press
//! cannot disagree.

use crate::collection::watch_personal::Flag;
use crate::collection::watch_playback::Pane;
use crate::console::list::Row;
use crate::console::Pad;

use super::Mark;

/// How many marks the row draws, for a title that has any (the caller asks
/// `has_marks` first): the watched one when the page draws it, then the two
/// flags. Reads no record, so the pad can ask it on every press.
pub(super) fn marks_len(watched: bool) -> usize {
    usize::from(watched) + Flag::ALL.len()
}

/// The mark at `index` of the row, for a title whose watched mark is (or is not)
/// drawn: read back from the same order `watch_marks` draws.
pub(super) fn mark_at(index: usize, watched: bool) -> Option<Mark> {
    let row: &[Mark] = if watched { &[Mark::Watched, Mark::Watchlist, Mark::Favourite] } else { &[Mark::Watchlist, Mark::Favourite] };
    row.get(index).copied()
}

/// The episode of the row the list is on, when that row is an episode: the
/// card follows the focus.
pub(super) fn focused_episode(rows: &[Row], focus: usize) -> Option<usize> {
    rows.get(focus)?.id.strip_prefix("episode:")?.parse().ok()
}

/// The presses a click on a rail's card `to` makes: into the card from the list
/// when it does not hold the pad, along the rail, then A — the same presses the
/// pad would make, so a click and a press cannot disagree.
pub(super) fn rail_route(pane: Pane, focus: usize, to: usize, columns: usize) -> Vec<Pad> {
    let columns = columns.max(1);
    let mut pads = Vec::new();
    if pane != Pane::Card {
        pads.push(Pad::Left);
    }
    // Rows first, then across the row the card is on.
    let (from_row, to_row) = (focus / columns, to / columns);
    if to_row > from_row {
        pads.extend(crate::console::pointer::repeat(Pad::Down, to_row - from_row));
    } else {
        pads.extend(crate::console::pointer::repeat(Pad::Up, from_row - to_row));
    }
    pads.extend(crate::console::pointer::along(focus % columns, to % columns, Pad::Left, Pad::Right));
    pads.push(Pad::A);
    pads
}

/// The presses a click on mark `to` makes, from wherever the pad is: back to
/// the list, up its rows to the first, then up into the marks row, across it and
/// A — one route per state of the page, so a click and a press cannot disagree.
pub(super) fn mark_route(pane: Pane, list_focus: usize, mark_focus: usize, to: usize) -> Vec<Pad> {
    let mut pads = Vec::new();
    if pane != Pane::Marks {
        // ◀ leaves the card for the list, at the row it was on.
        if pane == Pane::Card {
            pads.push(Pad::Left);
        }
        // Up the list to its first row, then up once more into the marks.
        pads.extend(crate::console::pointer::repeat(Pad::Up, list_focus + 1));
    }
    pads.extend(crate::console::pointer::along(mark_focus, to, Pad::Left, Pad::Right));
    pads.push(Pad::A);
    pads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_nobody_has_played_still_draws_the_two_flags() {
        assert_eq!(marks_len(false), Flag::ALL.len());
        assert_eq!(marks_len(true), Flag::ALL.len() + 1);
        assert_eq!(mark_at(0, false), Some(Mark::Watchlist));
        assert_eq!(mark_at(0, true), Some(Mark::Watched));
        assert_eq!(mark_at(usize::MAX, true), None);
    }

    #[test]
    fn only_an_episode_row_names_an_episode() {
        let rows = vec![Row::action("episode:3", "Episode 4"), Row::action("primary", "Play")];
        assert_eq!(focused_episode(&rows, 0), Some(3));
        assert_eq!(focused_episode(&rows, 1), None, "a row that is not an episode names none");
        assert_eq!(focused_episode(&rows, 9), None, "a focus past the list names none");
    }

    #[test]
    fn a_click_on_a_rail_card_walks_its_rows_then_its_columns() {
        // Two columns a row: card 4 sits under card 0, so the route is rows only.
        assert_eq!(rail_route(Pane::Card, 0, 4, 2), vec![Pad::Down, Pad::Down, Pad::A]);
        // Card 3 is one row down and one across.
        assert_eq!(rail_route(Pane::Card, 0, 3, 2), vec![Pad::Down, Pad::Right, Pad::A]);
        // From the list the card is entered first.
        assert_eq!(rail_route(Pane::List, 0, 0, 2), vec![Pad::Left, Pad::A]);
    }
}
