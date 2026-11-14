//! The local progress records: what was watched, read without a provider.
//!
//! `movies.json` is the source of truth between writes (`playback::play` keeps
//! it and hands a copy to the provider), so a page that only needs to know
//! where the person stopped reads it here, offline, without a `Ctx`.

use heronite_data::paths::AppPaths;
use heronite_watch_core::types::MovieProgress;
use heronite_watch_state::store;

/// The stored record of `provider`'s `id`, if there is one. A series keeps its
/// episode in `video_id`.
pub fn local(app: &AppPaths, provider: &str, id: &str) -> Option<MovieProgress> {
    store::progress_for(&store::read_movies(app), provider, id).cloned()
}
