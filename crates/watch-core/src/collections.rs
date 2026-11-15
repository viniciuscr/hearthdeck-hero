//! Rails a provider declares for the Shelf tab.
//!
//! A **collection** is a named rail of titles: a *rule* a catalogue source can
//! answer ("Top rated", "Popular"). The person turns them on in Settings; the tab
//! draws the ones that are on. This is not a catalogue (a browsable page) and not a
//! media provider (titles and streams): it is a saved query, and its source is the
//! source that declared it.
//!
//! A franchise's parts would be a fixed list rather than a rule; that kind arrives
//! with the TMDB account slice, and the descriptor below is where it will live.

use crate::catalog::{CatalogCtx, CatalogRequest};
use crate::Result;

/// What Settings lists a collection source by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollectionSourceDescriptor {
    /// Stable id (`tmdb`); the collection ids a settings file stores start with it.
    pub id: &'static str,
    /// How Settings names it.
    pub name: &'static str,
}

/// One rail a source declares.
#[derive(Clone, Debug, PartialEq)]
pub struct CollectionDescriptor {
    /// Stable id, unique across sources: `tmdb:top-rated:movie`.
    pub id: String,
    /// How the rail's header is drawn; the screens localize the ids they know.
    pub name: String,
    /// `movie` or `series`: a rule answers for one kind, so a source wanting both
    /// declares two.
    pub kind: String,
    /// The query, resolved by the declaring source's `catalog`.
    pub rule: CatalogRequest,
}

/// A source of collection rails. It plays nothing and pages nothing itself: it
/// declares queries that its own catalogue answers.
pub trait WatchCollectionSource: Send + Sync {
    fn descriptor(&self) -> &CollectionSourceDescriptor;

    /// The rails it declares, in the order Settings lists them.
    fn collections(&self, ctx: &CatalogCtx<'_>) -> Result<Vec<CollectionDescriptor>>;
}

/// The mark a collection rail leaves on the titles it brought in — the
/// `extra.watchCatalog` of their projection records: `collection:<id>`.
///
/// A title's mark says why it is in the library, so a rail the person turned on
/// is what the shelves read back; a catalogue page's feed key is a page that can
/// be replaced.
pub fn mark(id: &str) -> String {
    format!("collection:{id}")
}
