//! What the person owns: their watchlist, their favourites, and their lists.
//!
//! This is **user data**, not cache: it lives in `config/watch-personal.json`
//! (`heronite-watch-state::personal`), survives every library refresh, and is
//! hand-editable. A title is remembered as the card needs it — a [`ShelfTitle`],
//! the fields the person saw — so a rail can render while the palette projection
//! no longer holds that title.
//!
//! Identity is the pair the rest of Watch uses: `kind`, `id`. A film and a series
//! may share an id, so kind is part of it; the provider is provenance.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::title::Art;
use crate::types::Origin;

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

/// A title the person kept: the card's fields, as they were when it was kept.
///
/// This is the one persisted item shape (`watch-personal.json`), frozen so a
/// downgrade still reads it: the wire names are the old ones (`year`, and the art
/// flat at the top level), and the provider is *provenance* — who played it when
/// it was kept — not part of the identity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShelfTitle {
    /// The provider that played it when it was kept. Provenance, not identity.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    /// `movie` or `series`, the word the library stores.
    #[serde(default)]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// The library's `year` on the wire; a series' `2011–2019`.
    #[serde(default, rename = "year", skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    /// Flat on the wire (`poster`, `background`, `logo`), where the old item
    /// wrote them.
    #[serde(flatten)]
    pub art: Art,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
}

impl ShelfTitle {
    /// Refreshes this snapshot from a newer press on the same title: the newer
    /// value wins where it has one, and what it does not know is kept. Pressing a
    /// flag on an imported placeholder must not wipe the poster a real card gave
    /// the shelf, and pressing it on a real card must update what the person saw.
    pub fn merge(&mut self, other: &ShelfTitle) {
        if !other.provider.is_empty() {
            self.provider.clone_from(&other.provider);
        }
        if !other.name.is_empty() {
            self.name.clone_from(&other.name);
        }
        if other.release.is_some() {
            self.release.clone_from(&other.release);
        }
        if other.description.is_some() {
            self.description.clone_from(&other.description);
        }
        if !other.genres.is_empty() {
            self.genres.clone_from(&other.genres);
        }
        if other.art.poster.is_some() {
            self.art.poster.clone_from(&other.art.poster);
        }
        if other.art.background.is_some() {
            self.art.background.clone_from(&other.art.background);
        }
        if other.art.logo.is_some() {
            self.art.logo.clone_from(&other.art.logo);
        }
    }
}

/// The person's own shelf: two flags and their lists.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalLibrary {
    /// Titles meant to be watched, in the order they were added.
    #[serde(default)]
    pub watchlist: Vec<ShelfTitle>,
    /// Titles the person likes.
    #[serde(default)]
    pub favourites: Vec<ShelfTitle>,
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
    pub items: Vec<ShelfTitle>,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Whether two items are the same title: the pair every Lookup in Watch uses.
/// Whether two snapshots are the same title: kind and id. The provider is
/// *provenance* — who played it when it was kept — and refreshes in place, so
/// keeping a title from a second source does not add a second entry.
pub fn same_title(a: &ShelfTitle, b: &ShelfTitle) -> bool {
    a.kind == b.kind && a.id == b.id
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
        // The ledger first: this provider's decision, then the one a sync wrote
        // (under no provider). Then the lists, which are canonical — a snapshot's
        // provider is provenance.
        if let Some(decision) = self.decision(kind, provider, id).or_else(|| self.decision(kind, "", id)) {
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

    /// The membership this file implies for a title **regardless of provider**: a
    /// list entry from a sync carries no provider, and the ledger keeps it under
    /// an empty one.
    ///
    /// The person's own intent, which is what a sync reconciles against: their
    /// press when the newest decision for the title is theirs (`origin: Local`),
    /// else the list entry itself. A decision an import wrote says what a provider
    /// reported, not what the person wants, so it never speaks for them — a pull
    /// that wrote one by mistake cannot make a title the person kept look removed.
    pub fn local_membership(&self, kind: &str, id: &str) -> Option<Membership> {
        let newest = self
            .decisions
            .iter()
            .filter_map(|(key, decision)| {
                let mut parts = key.splitn(3, ':');
                (parts.next() == Some(kind) && parts.next().is_some() && parts.next() == Some(id)).then_some(decision)
            })
            // Two decisions can share an instant (a press and a pull in the same
            // second): the person's wins, so the tie-break cannot hand their
            // intent to a row an import wrote.
            .max_by_key(|decision| (decision.changed_at, decision.origin == Origin::Local));
        match newest {
            Some(decision) if decision.origin == Origin::Local => Some(decision.state),
            // Nothing of the person's to go by: the list itself, which is what a
            // file written before the ledger has, and what an import's add edits.
            _ => {
                let list = match kind {
                    WATCHLIST => &self.watchlist,
                    FAVOURITES => &self.favourites,
                    _ => return None,
                };
                list.iter().any(|item| item.id == id).then_some(Membership::In)
            }
        }
    }

    /// Records where a list stands after the person's own press: the decision a
    /// sync reads back as their intent, and the one the send side pushes.
    ///
    /// Provider-less on purpose: a press is about the title, not about which
    /// source played it (the same rule [`same_title`] follows).
    fn decide_local(&mut self, kind: &str, id: &str, state: Membership) {
        self.decide(kind, "", id, ListDecision { state, changed_at: Utc::now(), origin: Origin::Local, imported_from: None });
    }

    /// Adds a title to the watchlist, or refreshes the snapshot of one already
    /// there without moving it. `true` when it was not in the list before.
    pub fn add_watchlist(&mut self, item: &ShelfTitle) -> bool {
        keep(&mut self.watchlist, item)
    }

    pub fn remove_watchlist(&mut self, item: &ShelfTitle) -> bool {
        drop(&mut self.watchlist, item)
    }

    /// Adds or removes, and says where it stands afterwards: `true` = in.
    ///
    /// A press records where the list stands (`decide_local`) as well as editing
    /// it: the ledger is the intent a sync pushes, so pressing again after a pull
    /// said "removed" is a decision, not a silent edit the next tick undoes.
    pub fn toggle_watchlist(&mut self, item: &ShelfTitle) -> bool {
        let member = if self.remove_watchlist(item) {
            false
        } else {
            self.add_watchlist(item);
            true
        };
        self.decide_local(WATCHLIST, &item.id, membership_of(member));
        member
    }

    pub fn watchlisted(&self, item: &ShelfTitle) -> bool {
        self.watchlist.iter().any(|kept| same_title(kept, item))
    }

    pub fn add_favourite(&mut self, item: &ShelfTitle) -> bool {
        keep(&mut self.favourites, item)
    }

    pub fn remove_favourite(&mut self, item: &ShelfTitle) -> bool {
        drop(&mut self.favourites, item)
    }

    pub fn toggle_favourite(&mut self, item: &ShelfTitle) -> bool {
        let member = if self.remove_favourite(item) {
            false
        } else {
            self.add_favourite(item);
            true
        };
        self.decide_local(FAVOURITES, &item.id, membership_of(member));
        member
    }

    pub fn favourited(&self, item: &ShelfTitle) -> bool {
        self.favourites.iter().any(|kept| same_title(kept, item))
    }

    pub fn list(&self, id: &str) -> Option<&PersonalList> {
        self.lists.iter().find(|list| list.id == id)
    }

    /// Adds to a named list; `false` when there is no such list.
    pub fn add_to_list(&mut self, id: &str, item: &ShelfTitle) -> bool {
        match self.lists.iter_mut().find(|list| list.id == id) {
            Some(list) => {
                keep(&mut list.items, item);
                true
            }
            None => false,
        }
    }

    pub fn remove_from_list(&mut self, id: &str, item: &ShelfTitle) -> bool {
        match self.lists.iter_mut().find(|list| list.id == id) {
            Some(list) => drop(&mut list.items, item),
            None => false,
        }
    }
}

/// The membership a list edit left behind: `true` is in the list.
fn membership_of(member: bool) -> Membership {
    if member {
        Membership::In
    } else {
        Membership::Out
    }
}

/// Appends, or refreshes in place so the rail's order is the person's.
///
/// Refreshing fills what the new snapshot is missing from the kept one, never the
/// reverse: the same title may be pressed on a card that knows less than the
/// shelf does.
fn keep(items: &mut Vec<ShelfTitle>, item: &ShelfTitle) -> bool {
    match items.iter().position(|kept| same_title(kept, item)) {
        Some(index) => {
            items[index].merge(item);
            false
        }
        None => {
            items.push(item.clone());
            true
        }
    }
}

/// Removes, and says whether it was there.
fn drop(items: &mut Vec<ShelfTitle>, item: &ShelfTitle) -> bool {
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

    fn item(provider: &str, kind: &str, id: &str) -> ShelfTitle {
        ShelfTitle { provider: provider.into(), kind: kind.into(), id: id.into(), name: id.into(), ..ShelfTitle::default() }
    }

    #[test]
    fn an_old_shelf_entry_loads_and_keeps_its_wire_names() {
        // What the previous build wrote: the flat item, with the keys the
        // screens have always read.
        let old = r#"{"provider":"stremio","kind":"movie","id":"tt1","name":"A Film",
                      "year":"1999","poster":"https://img/p.jpg","background":null,
                      "description":"A plot","genres":["Drama"],"top":true,"catalog":null,
                      "last_watched":"2026-11-12T20:00:00Z","minutes_played":42.0,"addon":"cinemeta"}"#;
        let shelf: ShelfTitle = serde_json::from_str(old).expect("an old entry still loads");
        assert_eq!(shelf.release.as_deref(), Some("1999"));
        assert_eq!(shelf.art.poster.as_deref(), Some("https://img/p.jpg"));
        assert_eq!(shelf.genres, ["Drama"]);

        let written = serde_json::to_string(&shelf).expect("a snapshot writes");
        assert!(written.contains("\"year\":\"1999\""), "the old build reads its own key: {written}");
        assert!(written.contains("\"poster\":\"https://img/p.jpg\""), "art stays flat: {written}");
        assert!(!written.contains("\"top\""), "what the snapshot does not keep is dropped: {written}");
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
    fn a_title_kept_from_one_provider_is_one_entry_whichever_one_asks() {
        let mut library = PersonalLibrary::default();
        library.add_watchlist(&item("stremio", "movie", "tt1"));
        library.add_favourite(&item("stremio", "series", "tt2"));

        assert!(library.watchlisted(&item("youtube", "movie", "tt1")), "the same title, another provider");
        assert!(library.favourited(&item("youtube", "series", "tt2")));
        assert!(!library.watchlisted(&item("stremio", "series", "tt1")), "kind is part of the identity");

        // Keeping it from the second source refreshes the snapshot in place: the
        // rail draws one card, and the provenance follows the press.
        assert!(!library.add_watchlist(&item("youtube", "movie", "tt1")), "already kept");
        assert_eq!(library.watchlist.len(), 1, "no duplicate");
        assert_eq!(library.watchlist[0].provider, "youtube", "the snapshot says who played it last");
        assert!(library.remove_watchlist(&item("youtube", "movie", "tt1")), "and it can be removed");
        assert!(library.watchlist.is_empty());
    }

    #[test]
    fn a_film_and_a_series_may_share_an_id() {
        let mut library = PersonalLibrary::default();
        library.add_watchlist(&item("stremio", "movie", "tt1"));
        assert!(!library.watchlisted(&item("stremio", "series", "tt1")), "kind is part of the identity");
        assert!(library.watchlisted(&item("other", "movie", "tt1")), "the provider is provenance, not identity");
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
    fn a_press_records_the_intent_a_sync_pushes() {
        let mut library = PersonalLibrary::default();
        let matrix = item("stremio", "movie", "tt0133093");

        assert!(library.toggle_watchlist(&matrix));
        assert_eq!(library.local_membership(WATCHLIST, "tt0133093"), Some(Membership::In), "the press is the decision");
        let decision = library.decision(WATCHLIST, "", "tt0133093").expect("recorded under no provider");
        assert_eq!(decision.origin, Origin::Local, "the person's, not a pull's");

        assert!(!library.toggle_watchlist(&matrix));
        assert_eq!(library.local_membership(WATCHLIST, "tt0133093"), Some(Membership::Out), "and so is the second press");
        assert_eq!(library.local_membership(FAVOURITES, "tt0133093"), None, "the other flag was never touched");
    }

    #[test]
    fn a_decision_a_pull_wrote_never_speaks_for_the_person() {
        // What the delta-pull bug left behind: an `out` an import derived from a
        // partial answer, with the title back in the list the person kept.
        let mut library = PersonalLibrary::default();
        let matrix = item("stremio", "movie", "tt0149460");
        library.add_watchlist(&matrix);
        library.decide(
            WATCHLIST,
            "",
            "tt0149460",
            ListDecision { state: Membership::Out, changed_at: Utc::now(), origin: Origin::Imported, imported_from: Some("simkl".into()) },
        );

        assert_eq!(
            library.local_membership(WATCHLIST, "tt0149460"),
            Some(Membership::In),
            "the import's row is a report, not an intent: the list is the answer"
        );

        // A press after it is the person's, and outranks it.
        assert!(!library.toggle_watchlist(&matrix));
        assert_eq!(library.local_membership(WATCHLIST, "tt0149460"), Some(Membership::Out));
    }

    #[test]
    fn a_tie_between_the_person_and_a_pull_goes_to_the_person() {
        // Both wrote in the same instant: only the origin can break the tie, and
        // the person's intent must win it (a pull's row is a report).
        let mut library = PersonalLibrary::default();
        let matrix = item("stremio", "movie", "tt1");
        let at = Utc::now();
        library.decide(
            WATCHLIST,
            "",
            "tt1",
            ListDecision { state: Membership::Out, changed_at: at, origin: Origin::Local, imported_from: None },
        );
        // A second writer's row for the same title, in the same instant: the
        // ledger keeps one row per `kind:provider:id`, so it lives under a key of
        // its own and the scan over the title has to choose.
        library.decide(
            WATCHLIST,
            "simkl",
            "tt1",
            ListDecision { state: Membership::In, changed_at: at, origin: Origin::Imported, imported_from: Some("simkl".into()) },
        );

        assert!(!library.watchlisted(&matrix), "the list is empty: the pull only reported");
        assert_eq!(library.local_membership(WATCHLIST, "tt1"), Some(Membership::Out), "the person's press stands");
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
