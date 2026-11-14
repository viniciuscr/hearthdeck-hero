//! What a title is: the metadata sources' answer, with their cache.
//!
//! A media provider (Stremio, YouTube) owns titles and plays them; a metadata
//! source describes them. This facade is the app's only door to that layer:
//! it asks the sources in order, keeps their answer for a week in the app
//! cache, and serves the stale answer when none can be reached. It is
//! blocking: the app calls it from a background task.

use anyhow::Result;
use chrono::Utc;
use heronite_data::paths::AppPaths;

use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::metadata::MetaCtx;
use heronite_watch_state::details as cache;

pub use heronite_watch_core::metadata::{Episode, MetaDescriptor, TitleDetails, TitleRef, WatchMetadataSource};

/// The metadata sources Watch ships with, in the order they are asked.
pub fn sources() -> &'static [&'static dyn WatchMetadataSource] {
    heronite_watch_metadata::sources()
}

/// The details of `title`, from the cache or the sources.
pub fn details(app: &AppPaths, title: &TitleRef) -> Result<Option<TitleDetails>> {
    let http = UreqTransport::new();
    details_with(&http, app, title)
}

/// [`details`] with an injected transport, for tests.
pub fn details_with(http: &dyn Transport, app: &AppPaths, title: &TitleRef) -> Result<Option<TitleDetails>> {
    let root = app.cache.join(cache::ROOT);
    let cached = cache::read(&root, title);
    if cached.as_ref().is_some_and(|cached| cached.is_fresh(Utc::now())) {
        return Ok(cached.map(|cached| cached.details));
    }
    match heronite_watch_metadata::details(&MetaCtx::new(http), title) {
        Ok(Some(answer)) => {
            if let Err(error) = cache::write(&root, title, &answer, Utc::now()) {
                log::warn!("watch details: {error:#}");
            }
            Ok(Some(answer))
        }
        // A title no source knows is not a fault, and not something to cache.
        Ok(None) => Ok(None),
        Err(error) => match cached {
            Some(cached) => {
                log::warn!("watch details: {error:#}; keeping the cached answer");
                Ok(Some(cached.details))
            }
            None => Err(error),
        },
    }
}
