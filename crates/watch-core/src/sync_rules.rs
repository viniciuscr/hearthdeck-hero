//! The list rules as code: `docs/WATCH-SYNC-RULES.md`, TABLE 1 and TABLE 2.
//!
//! Pure and total — every combination of what we hold, what the last applied tick
//! saw remotely, and what the provider reports now has exactly one answer. No I/O,
//! so the spec, the table and the tests cannot drift apart.

use chrono::{DateTime, Utc};

/// What the local list says about a title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Local {
    /// Never decided here (a fresh profile, or a title never seen).
    Unset,
    In,
    Out,
}

/// What the last fully applied tick observed in the provider's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Absent,
    In,
}

/// What the provider reports in this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    Absent,
    In,
}

/// What the reconciliation asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do on either side.
    None,
    /// The provider has it and we do not: the local list gains it.
    AddLocal,
    /// It was there and is gone: the local list loses it, with a notice.
    RemoveLocal,
    /// Added here, never seen there: the provider must be told.
    PushAdd,
    /// Removed here: the provider must be told.
    PushRemove,
    /// Both sides moved: [`resolve_conflict`] decides.
    Conflict,
}

/// TABLE 1: 3 × 2 × 2 = 12 rows, total.
///
/// The row that matters most is `(In, Absent, Absent)` → [`Action::PushAdd`]:
/// absence in the provider is **never** a removal, only "it was there and is
/// gone" is.
pub fn reconcile(local: Local, seen: Seen, remote: Remote) -> Action {
    use Action::{AddLocal, None as Nothing, PushAdd, PushRemove, RemoveLocal};
    use Local::{In, Out, Unset};
    use Remote::{Absent as RemoteAbsent, In as RemoteIn};
    use Seen::{Absent, In as SeenIn};

    match (local, seen, remote) {
        (Unset, Absent, RemoteIn) => AddLocal,
        (Unset, Absent, RemoteAbsent) => Nothing,
        (Unset, SeenIn, RemoteIn) => AddLocal,
        (Unset, SeenIn, RemoteAbsent) => RemoveLocal,
        (In, Absent, RemoteAbsent) => PushAdd,
        (In, Absent, RemoteIn) => Nothing,
        (In, SeenIn, RemoteIn) => Nothing,
        (In, SeenIn, RemoteAbsent) => RemoveLocal,
        (Out, Absent, RemoteAbsent) => Nothing,
        (Out, Absent, RemoteIn) => Action::Conflict,
        (Out, SeenIn, RemoteIn) => PushRemove,
        (Out, SeenIn, RemoteAbsent) => Nothing,
    }
}

/// Which side wins a [`Action::Conflict`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Local,
    Remote,
}

/// TABLE 2: newest wins, local on a tie, and a missing stamp loses.
pub fn resolve_conflict(local_changed: Option<DateTime<Utc>>, remote_changed: Option<DateTime<Utc>>) -> Side {
    match (local_changed, remote_changed) {
        (_, None) => Side::Local,
        (None, Some(_)) => Side::Remote,
        (Some(local), Some(remote)) if remote > local => Side::Remote,
        (Some(_), Some(_)) => Side::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use Action::{AddLocal, Conflict, PushAdd, PushRemove, RemoveLocal};
    use Local::{In, Out, Unset};
    use Remote::{Absent as R0, In as R1};
    use Seen::{Absent as S0, In as S1};

    /// TABLE 1, row for row — the acceptance gate for the import slice.
    #[test]
    fn every_list_combination_has_its_row() {
        let table = [
            ((Unset, S0, R1), AddLocal),
            ((Unset, S0, R0), Action::None),
            ((Unset, S1, R1), AddLocal),
            ((Unset, S1, R0), RemoveLocal),
            ((In, S0, R0), PushAdd),
            ((In, S0, R1), Action::None),
            ((In, S1, R1), Action::None),
            ((In, S1, R0), RemoveLocal),
            ((Out, S0, R0), Action::None),
            ((Out, S0, R1), Conflict),
            ((Out, S1, R1), PushRemove),
            ((Out, S1, R0), Action::None),
        ];
        for ((local, seen, remote), expected) in table {
            assert_eq!(reconcile(local, seen, remote), expected, "{local:?} {seen:?} {remote:?}");
        }
    }

    /// The rule the owner insisted on: adding here while absent there pushes, it
    /// never removes the local entry.
    #[test]
    fn absence_is_not_removal() {
        assert_eq!(reconcile(In, S0, R0), PushAdd, "added here, never there");
        assert_eq!(reconcile(In, S1, R0), RemoveLocal, "was there, gone now");
    }

    /// TABLE 1 is total: every combination answers something.
    #[test]
    fn every_combination_is_covered() {
        for local in [Unset, In, Out] {
            for seen in [S0, S1] {
                for remote in [R0, R1] {
                    let _ = reconcile(local, seen, remote);
                }
            }
        }
    }

    #[test]
    fn a_conflict_is_won_by_the_newest_and_local_wins_a_tie() {
        let early = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let late = Utc.with_ymd_and_hms(2026, 1, 1, 10, 5, 0).unwrap();
        assert_eq!(resolve_conflict(Some(early), Some(late)), Side::Remote);
        assert_eq!(resolve_conflict(Some(late), Some(early)), Side::Local);
        assert_eq!(resolve_conflict(Some(early), Some(early)), Side::Local, "a tie is ours");
        assert_eq!(resolve_conflict(Some(early), None), Side::Local, "a missing remote stamp loses");
        assert_eq!(resolve_conflict(None, Some(late)), Side::Remote);
    }
}
