//! What the person owns: their watchlist, their favourites, and their lists.
//!
//! This is **user data**, not cache: it lives in `config/watch-personal.json`
//! (`heronite-watch-state::personal`), survives every library refresh, and is
//! hand-editable. A title is remembered as the card needs it — a `MovieItem`, the
//! same shape the screens already draw — so a rail can render while the palette
//! projection no longer holds that title.
//!
//! Identity is the pair the rest of Watch uses: `provider`, `kind`, `id`. A film and
//! a series may share an id, so kind is part of it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{MovieItem, Origin};

/// Whether a title is in a list, as the last decision said.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Membership {
    In,
    Out,
}

/// The decision recorded for one title in one list kind — the ledger that tells
/// "removed there" from "never was there". A sync reconciles against it; the
/// screens read it only through [`PersonalLibrary::membership`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDecision {
    pub state: Membership,
    /// When the decision was made (local clock): the tie-breaker in a conflict.
    pub changed_at: DateTime<Utc>,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
}

/// The list kinds this file records decisions for.
pub const WATCHLIST: &str = "watchlist";
pub const FAVOURITES: &str = "favourites";

/// The person's own shelf: two flags and their lists.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalLibrary {
    /// Titles meant to be watched, in the order they were added.
    #[serde(default)]
    pub watchlist: Vec<MovieItem>,
    /// Titles the person likes.
    #[serde(default)]
    pub favourites: Vec<MovieItem>,
    /// Named lists, reserved for the "Add to a list…" pass.
    #[serde(default)]
    pub lists: Vec<PersonalList>,
    /// The decision per list kind and title (`"watchlist:stremio:tt1"`): what a
    /// sync reconciles against. Absent means "never decided here".
    #[serde(default)]
    pub decisions: BTreeMap<String, ListDecision>,
    /// A newer build's fields survive a write here.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One named list. `remote` is reserved for TMDB's list id, so the account slice
/// does not have to migrate the file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalList {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<MovieItem>,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Whether two items are the same title: the pair every Lookup in Watch uses.
pub fn same_title(a: &MovieItem, b: &MovieItem) -> bool {
    a.provider == b.provider && a.kind == b.kind && a.id == b.id
}

impl PersonalLibrary {
    /// The ledger key for a list kind and a title.
    pub fn decision_key(kind: &str, provider: &str, id: &str) -> String {
        format!("{kind}:{provider}:{id}")
    }

    /// What was decided here, `None` for a title never decided.
    pub fn decision(&self, kind: &str, provider: &str, id: &str) -> Option<&ListDecision> {
        self.decisions.get(&Self::decision_key(kind, provider, id))
    }

    /// Records a decision, replacing any earlier one for the same title.
    pub fn decide(&mut self, kind: &str, provider: &str, id: &str, decision: ListDecision) {
        self.decisions.insert(Self::decision_key(kind, provider, id), decision);
    }

    /// The membership this file implies: the ledger when it has an entry, else the
    /// plain vecs (a file written before the ledger existed). `None` for a title
    /// that is in neither.
    pub fn membership(&self, kind: &str, provider: &str, id: &str) -> Option<Membership> {
        if let Some(decision) = self.decision(kind, provider, id) {
            return Some(decision.state);
        }
        let list = match kind {
            WATCHLIST => &self.watchlist,
            FAVOURITES => &self.favourites,
            _ => return None,
        };
        list.iter().any(|item| item.id == id && (item.provider.is_empty() || item.provider == provider)).then_some(Membership::In)
    }
    /// The membership this file implies for a title **regardless of provider**: a
    /// list entry from a sync carries no provider, and the ledger keeps it under
    /// an empty one.
    pub fn membership_any(&self, kind: &str, id: &str) -> Option<Membership> {
        if let Some(decision) = self.decision(kind, "", id) {
            return Some(decision.state);
        }
        let list = match kind {
            WATCHLIST => &self.watchlist,
            FAVOURITES => &self.favourites,
            _ => return None,
        };
        list.iter().any(|item| item.id == id).then_some(Membership::In)
    }

    /// Takes a title out of a list by its id, whatever kind or provider the entry
    /// was kept as. A sync removal knows the id, not the entry.
    pub fn remove_any(&mut self, kind: &str, id: &str) -> bool {
        let list = match kind {
            WATCHLIST => &mut self.watchlist,
            FAVOURITES => &mut self.favourites,
            _ => return false,
        };
        let before = list.len();
        list.retain(|item| item.id != id);
        list.len() != before
    }

    /// Adds a title to the watchlist, or refreshes the snapshot of one already
    /// there without moving it. `true` when it was not in the list before.
    pub fn add_watchlist(&mut self, item: &MovieItem) -> bool {
        keep(&mut self.watchlist, item)
    }

    pub fn remove_watchlist(&mut self, item: &MovieItem) -> bool {
        drop(&mut self.watchlist, item)
    }

    /// Adds or removes, and says where it stands afterwards: `true` = in.
    pub fn toggle_watchlist(&mut self, item: &MovieItem) -> bool {
        if self.remove_watchlist(item) {
            false
        } else {
            self.add_watchlist(item);
            true
        }
    }

    pub fn watchlisted(&self, item: &MovieItem) -> bool {
        self.watchlist.iter().any(|kept| same_title(kept, item))
    }

    pub fn add_favourite(&mut self, item: &MovieItem) -> bool {
        keep(&mut self.favourites, item)
    }

    pub fn remove_favourite(&mut self, item: &MovieItem) -> bool {
        drop(&mut self.favourites, item)
    }

    pub fn toggle_favourite(&mut self, item: &MovieItem) -> bool {
        if self.remove_favourite(item) {
            false
        } else {
            self.add_favourite(item);
            true
        }
    }

    pub fn favourited(&self, item: &MovieItem) -> bool {
        self.favourites.iter().any(|kept| same_title(kept, item))
    }

    pub fn list(&self, id: &str) -> Option<&PersonalList> {
        self.lists.iter().find(|list| list.id == id)
    }

    /// Adds to a named list; `false` when there is no such list.
    pub fn add_to_list(&mut self, id: &str, item: &MovieItem) -> bool {
        match self.lists.iter_mut().find(|list| list.id == id) {
            Some(list) => {
                keep(&mut list.items, item);
                true
            }
            None => false,
        }
    }

    pub fn remove_from_list(&mut self, id: &str, item: &MovieItem) -> bool {
        match self.lists.iter_mut().find(|list| list.id == id) {
            Some(list) => drop(&mut list.items, item),
            None => false,
        }
    }
}

/// Appends, or refreshes in place so the rail's order is the person's.
fn keep(items: &mut Vec<MovieItem>, item: &MovieItem) -> bool {
    match items.iter().position(|kept| same_title(kept, item)) {
        Some(index) => {
            items[index] = item.clone();
            false
        }
        None => {
            items.push(item.clone());
            true
        }
    }
}

/// Removes, and says whether it was there.
fn drop(items: &mut Vec<MovieItem>, item: &MovieItem) -> bool {
    match items.iter().position(|kept| same_title(kept, item)) {
        Some(index) => {
            items.remove(index);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(provider: &str, kind: &str, id: &str) -> MovieItem {
        MovieItem { provider: provider.into(), kind: kind.into(), id: id.into(), name: id.into(), ..MovieItem::default() }
    }

    #[test]
    fn a_toggle_adds_then_removes_and_says_where_it_stands() {
        let mut library = PersonalLibrary::default();
        let matrix = item("stremio", "movie", "tt0133093");
        assert!(library.toggle_watchlist(&matrix), "first press puts it in");
        assert!(library.watchlisted(&matrix));
        assert!(!library.toggle_watchlist(&matrix), "second press takes it out");
        assert!(!library.watchlisted(&matrix));
        assert!(library.watchlist.is_empty());
    }

    #[test]
    fn the_flags_are_separate() {
        let mut library = PersonalLibrary::default();
        let matrix = item("stremio", "movie", "tt0133093");
        library.toggle_favourite(&matrix);
        assert!(library.favourited(&matrix));
        assert!(!library.watchlisted(&matrix), "a favourite is not a watchlist entry");
        assert!(library.favourites.len() == 1 && library.watchlist.is_empty());
    }

    #[test]
    fn adding_again_keeps_the_place_and_refreshes_the_card() {
        let mut library = PersonalLibrary::default();
        let mut first = item("stremio", "movie", "tt1");
        first.name = "Old name".into();
        library.add_watchlist(&first);
        library.add_watchlist(&item("stremio", "movie", "tt2"));
        let mut again = item("stremio", "movie", "tt1");
        again.name = "The Matrix".into();
        assert!(!library.add_watchlist(&again), "it was already there");

        let ids: Vec<&str> = library.watchlist.iter().map(|kept| kept.id.as_str()).collect();
        assert_eq!(ids, ["tt1", "tt2"], "adding does not reorder the rail");
        assert_eq!(library.watchlist[0].name, "The Matrix", "but the card is refreshed");
    }

    #[test]
    fn a_film_and_a_series_may_share_an_id() {
        let mut library = PersonalLibrary::default();
        library.add_watchlist(&item("stremio", "movie", "tt1"));
        assert!(!library.watchlisted(&item("stremio", "series", "tt1")), "kind is part of the identity");
        assert!(!library.watchlisted(&item("other", "movie", "tt1")), "and so is the provider");
    }

    #[test]
    fn a_named_list_takes_and_drops_titles_and_says_when_it_does_not_exist() {
        let mut library = PersonalLibrary::default();
        library.lists.push(PersonalList { id: "sci-fi".into(), name: "Sci-fi to watch".into(), ..PersonalList::default() });
        let matrix = item("stremio", "movie", "tt1");
        assert!(library.add_to_list("sci-fi", &matrix));
        assert_eq!(library.list("sci-fi").map(|list| list.items.len()), Some(1));
        assert!(library.remove_from_list("sci-fi", &matrix));
        assert!(!library.remove_from_list("sci-fi", &matrix), "already gone");
        assert!(!library.add_to_list("nope", &matrix), "no such list");
    }

    #[test]
    fn decisions_round_trip_and_old_files_read() {
        // A file written before the ledger: no `decisions`, entries in the vecs.
        let legacy = r#"{"watchlist":[{"id":"tt1","kind":"movie","name":"A"}],"favourites":[]}"#;
        let mut library: PersonalLibrary = serde_json::from_str(legacy).expect("an old file still reads");
        assert!(library.decisions.is_empty());
        assert_eq!(library.membership(WATCHLIST, "stremio", "tt1"), Some(Membership::In), "the vec is still the answer");
        assert_eq!(library.membership(FAVOURITES, "stremio", "tt1"), None);

        library.decide(
            WATCHLIST,
            "stremio",
            "tt1",
            ListDecision { state: Membership::Out, changed_at: Utc::now(), origin: Origin::Local, imported_from: None },
        );
        let text = serde_json::to_string(&library).unwrap();
        let back: PersonalLibrary = serde_json::from_str(&text).unwrap();
        assert_eq!(back.membership(WATCHLIST, "stremio", "tt1"), Some(Membership::Out), "a decision outranks the vec");
        assert!(text.contains("\"decisions\""), "{text}");
    }
}
