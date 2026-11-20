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
    // `timeWatched` stays 0 for a title that was only started: the position and
    // its length are what say whether anything was watched at all, and how far
    // it got. `flaggedWatched` is Stremio's own mark; the position reaching the
    // end is a watch that finished without one. `timesWatched` is a lifetime
    // counter, so it cannot tell a finished rewatch from one at minute five.
    let position_ms = item.state.time_offset;
    let duration_ms = item.state.duration;
    let finished = item.state.flagged_watched > 0 || (duration_ms > 0 && position_ms.saturating_add(60_000) >= duration_ms);
    if at.is_some() || minutes > 0.0 || position_ms > 0 || finished {
        title.watched = Some(Watched { at, minutes, position_ms, duration_ms, finished });
    }
    title
}
