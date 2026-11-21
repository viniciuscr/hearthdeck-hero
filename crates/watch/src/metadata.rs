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

pub use heronite_watch_core::metadata::{
    Episode, Fact, MetaDescriptor, Rail, Related, TitleDetails, TitleRef, Trailer, WatchMetadataSource,
};

/// The library title a rail's card is, ready to be kept and opened.
///
/// The card already names the IMDb id, the kind, the year and the art, which is
/// everything a title needs; the provider that plays it is the one a catalogue
/// stamps on its titles (Stremio), because a rail is a list of things to open
/// here and nothing else here can play them.
pub fn related_title(related: &Related) -> heronite_watch_core::title::Title {
    use heronite_watch_core::title::{Art, ProviderId, Title, TitleKey};

    let provider = ProviderId::declared("stremio");
    // The key the projection reads back (`TitleKey::of_item` → `of_parts`): an
    // IMDb id stays an IMDb id, and anything else is that provider's own id — an
    // `imdb` key built from a non-IMDb id could never be merged or adopted.
    let mut title = Title::new(TitleKey::of_source(&related.kind, &related.id, &provider), &related.name).plays(provider);
    title.release = related.year.clone();
    title.art = Art { poster: related.poster.clone(), background: related.background.clone(), logo: None };
    title
}

/// The metadata sources Watch ships with, in registry order.
pub fn sources() -> &'static [&'static dyn WatchMetadataSource] {
    heronite_watch_metadata::sources()
}

/// The sources the settings turn on, in their priority order: the first to fill
/// a field keeps it, the next fills what is still empty.
pub fn enabled(ids: &[String]) -> Vec<&'static dyn WatchMetadataSource> {
    heronite_watch_metadata::enabled(ids)
}

/// The details of `title`, from the cache or the sources.
pub fn details(app: &AppPaths, title: &TitleRef) -> Result<Option<TitleDetails>> {
    let http = UreqTransport::new();
    details_with(&http, app, title)
}

/// [`details`] with an injected transport, for tests.
pub fn details_with(http: &dyn Transport, app: &AppPaths, title: &TitleRef) -> Result<Option<TitleDetails>> {
    // The sources the person turned on, in their order: TMDB first by default,
    // Cinemeta as the fallback. The cache remembers which set answered.
    let enabled = heronite_watch_metadata::enabled(&heronite_watch_state::settings::read(app).metadata_sources);
    let sources = enabled.iter().map(|source| source.descriptor().id).collect::<Vec<_>>().join("+");
    let root = app.cache.join(cache::ROOT);
    let cached = cache::read(&root, title, &sources);
    // Fresh **and** of the shape this build draws: an answer from before a
    // screen gained a field (the trailers) must not hide it for a week.
    if cached.as_ref().is_some_and(|cached| cached.is_fresh(Utc::now()) && cached.is_current()) {
        log::debug!("watch details cache: {}:{} answered ({sources})", title.provider, title.id);
        return Ok(cached.map(|cached| cached.details));
    }
    log::debug!("watch details cache: {}:{} asked ({sources})", title.provider, title.id);
    // The machine's own credentials are adopted here too, not only when Settings
    // opens: a page must not go without a source because nobody visited it — the
    // bootstrap's key and client id are files waiting for a reader.
    // Both, always: `||` would skip Simkl's file whenever TMDB's was taken.
    let tmdb_adopted = crate::tmdb_sign_in::adopt_key_file(app);
    let simkl_adopted = crate::simkl_sign_in::adopt_client_id_file(app);
    if tmdb_adopted || simkl_adopted {
        log::debug!("watch details: adopted the machine's own credentials");
    }
    // A source that needs a key (TMDB) reads its own entry; Cinemeta ignores it.
    let credentials = heronite_watch_state::store::StoreCredentials::new(app);
    match heronite_watch_metadata::details(&enabled, &MetaCtx::with(http, &credentials), title) {
        Ok(Some(answer)) => {
            if let Err(error) = cache::write(&root, title, &sources, &answer, Utc::now()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::metadata::Related;
    use heronite_watch_core::title::{ProviderId, TitleKey};

    /// The key a rail's card is built with must be the key the projection reads
    /// back, or the title it keeps can never be found, merged or adopted.
    #[test]
    fn a_related_title_is_keyed_the_way_the_projection_reads_it_back() {
        let related = |id: &str| Related { id: id.into(), kind: "series".into(), name: id.into(), ..Related::default() };
        let stremio = ProviderId::declared("stremio");

        assert_eq!(related_title(&related("tt1234567")).key, TitleKey::of_source("series", "tt1234567", &stremio));
        let other = related_title(&related("kitsu:42"));
        assert_eq!(other.key, TitleKey::of_source("series", "kitsu:42", &stremio));
        assert_ne!(
            other.key.id,
            heronite_watch_core::title::TitleId::imdb("kitsu:42"),
            "a non-IMDb id stays in its provider's space, which is the shape the projection reads back"
        );
    }
}
