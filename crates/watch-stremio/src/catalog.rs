//! The account's records as Watch items.

use heronite_watch_core::title::{ProviderId, Title, TitleKey, Watched};

use crate::datastore::LibraryItem;
use crate::ID;

/// A datastore record as a title: what the account knows about it.
pub fn from_library(item: &LibraryItem) -> Title {
    let provider = ProviderId::declared(ID);
    let mut title = Title::new(TitleKey::of_source(&item.kind, &item.id, &provider), &item.name).plays(provider);
    title.art.poster = item.poster.clone();
    let at = item.state.last_watched;
    let minutes = item.state.time_watched as f64 / 60_000.0;
    if at.is_some() || minutes > 0.0 {
        title.watched = Some(Watched { at, minutes });
    }
    title
}
