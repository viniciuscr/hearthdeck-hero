//! The Watch space's library: the **titles** the sources answered with, and the
//! Collection entry point that projects them into the synthetic `Library` the
//! existing screens read.
//!
//! `Titles` is the model (`docs/WATCH-TITLES.md` §3): one entry per title,
//! identified by kind and id, with the providers that play it and the rails that
//! brought it. `from_parts` is pure — the rails, the shelf and the placeholders
//! in, titles out — so the screen's contents can be tested without a network,
//! and `from_projection` reads a written library back (§5.1).
//!
//! Providers and catalogue sources emit titles; the shelf stores its own
//! snapshot, which `Title::of_shelf` adapts (§3.1). The projection in
//! `heronite-watch-state::collection` takes the titles and writes the records.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use heronite_data::paths::AppPaths;
use heronite_data::{CollectionGame, DataPaths, Library};
use heronite_watch_core::catalog::{CatalogCtx, CatalogRequest, WatchCatalogSource};
use heronite_watch_core::collections::CollectionDescriptor;
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::{Ctx, Rail, RailDescriptor};
use heronite_watch_core::title::{RailMark, Title, TitleKey, Watched, IMPORTED_ADDON};
use heronite_watch_core::types::{MovieItem, MovieProgress};
use heronite_watch_state::catalog_cache::CatalogIds;
use heronite_watch_state::collection::{self, Fetched};
use heronite_watch_state::placeholders::Placeholder;
use heronite_watch_state::store::StoreCredentials;

use crate::personal::ShelfTitle;
use crate::registry;

/// Where the proxy keeps its synthetic library.
pub use collection::ROOT;

/// The titles a library holds, in the order the rails gave them.
///
/// Insertion-ordered on purpose: the rails' order is the grid's order, so a map
/// keyed by id would reorder the screen. The index only says where a title is.
#[derive(Debug, Default)]
pub struct Titles {
    items: Vec<Title>,
    index: HashMap<TitleKey, usize>,
}

/// What a library is built from.
pub struct Parts<'a> {
    /// The provider rails and the catalogue pages, in display order.
    pub rails: Vec<Rail>,
    /// The collection rails the person turned on: their mark, and the titles no
    /// other rail carried.
    pub collections: Vec<(String, Vec<Title>)>,
    /// The person's own shelf: what they kept, whatever the rails said.
    pub shelf: &'a [ShelfTitle],
    /// Titles a provider knows and nothing here plays.
    pub placeholders: &'a [Placeholder],
}

impl Titles {
    /// Builds the library from what the sources answered.
    ///
    /// The order is the one the screens draw: the provider rails, then the
    /// catalogue pages, then the shelf, then the placeholders, then whatever
    /// only a collection rail brought. Two observations of one title merge into
    /// one entry.
    pub fn from_parts(parts: Parts<'_>) -> Self {
        let mut titles = Self::default();
        for rail in parts.rails {
            for title in rail.items {
                titles.insert(title);
            }
        }
        for item in parts.shelf {
            titles.insert(Title::of_shelf(item));
        }
        for placeholder in parts.placeholders {
            titles.insert(Title::of_item(&placeholder_item(placeholder)));
        }
        for (id, items) in parts.collections {
            let rail = RailMark::collection(&id);
            for title in items {
                titles.mark(&rail, title);
            }
        }
        titles
    }

    /// The library a projection root holds now — the read-back half of the
    /// round trip: one record gives one provider and one rail, and what no
    /// record carries is gone.
    pub fn from_projection(paths: &DataPaths) -> Result<Self> {
        let library = Library::load_with(paths, true)?;
        let mut titles = Self::default();
        for game in &library.games {
            titles.insert(title_of_game(game));
        }
        Ok(titles)
    }

    /// The titles, in the order they arrived.
    pub fn titles(&self) -> &[Title] {
        &self.items
    }

    /// Folds this machine's own records into the titles.
    ///
    /// The projection carries what the provider answered; a mark made on the
    /// page, or a playback the provider has not been told about yet, lives in
    /// `movies.json` only. Without this fold a cover says "Not watched" under a
    /// page that says "Watched".
    pub fn fold_local_progress(&mut self, local: &crate::progress::LocalProgress) {
        if local.is_empty() {
            return;
        }
        for title in &mut self.items {
            let record = title.providers.iter().find_map(|provider| local.for_title(provider.as_str(), title.key.id.as_str()));
            if let Some(record) = record {
                fold_watch(title, record);
            }
        }
    }

    /// The titles, consuming the container.
    pub fn into_titles(self) -> Vec<Title> {
        self.items
    }

    pub fn get(&self, key: &TitleKey) -> Option<&Title> {
        self.index.get(key).map(|at| &self.items[*at])
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Adds a title, or folds it into the one already there.
    fn insert(&mut self, title: Title) {
        match self.index.get(&title.key) {
            Some(&at) => {
                // The index is keyed by this title's key, so the merge cannot
                // refuse it; a refusal would mean the index and the titles
                // disagree.
                if self.items[at].merge(title).is_some() {
                    log::error!("watch library: a title changed key under the index");
                }
            }
            None => {
                self.index.insert(title.key.clone(), self.items.len());
                self.items.push(title);
            }
        }
    }

    /// Marks a collection rail's title, adding it when no other rail has one.
    ///
    /// A collection rail outranks a catalogue page's own feed key — the first
    /// mark is the one the projection persists — but the feed stays behind it:
    /// the wire is the same, and the catalogue that listed the title is not
    /// lost. The first rail of the settings order names the title, so a second
    /// collection does not displace it.
    fn mark(&mut self, rail: &RailMark, title: Title) {
        match self.index.get(&title.key).copied() {
            Some(at) => {
                let held = &mut self.items[at];
                if !matches!(held.rails.first(), Some(RailMark::Collection(_))) {
                    held.rails.insert(0, rail.clone());
                }
            }
            None => {
                let mut title = title;
                title.rails.insert(0, rail.clone());
                self.index.insert(title.key.clone(), self.items.len());
                self.items.push(title);
            }
        }
    }
}

/// The title a projection record is: the read-back adapter.
///
/// `CollectionGame` is upstream's view shape, so this adapter lives here rather
/// than beside `Title`. It reads the keys the writer froze, including the
/// catalogue that listed the title (`extra.watchSource`), so a feed mark comes
/// back with its source.
pub fn title_of_game(game: &CollectionGame) -> Title {
    let extra = game.game.extra.as_ref();
    let field = |key: &str| extra.and_then(|extra| extra.get(key));
    // The same rule the writer's own decode uses: an imported record names no
    // provider, and a record from before the marker existed is imported iff its
    // runner is the owner's pseudo-provider.
    let imported = field("imported").and_then(serde_json::Value::as_bool).unwrap_or(game.runner == IMPORTED_ADDON);
    let item = MovieItem {
        id: field("watchId").and_then(serde_json::Value::as_str).unwrap_or(&game.app_name).to_owned(),
        kind: field("stremioType").and_then(serde_json::Value::as_str).unwrap_or("movie").to_owned(),
        name: game.title.clone(),
        year: game.details.release_date.clone(),
        poster: game.game.art_cover.clone(),
        background: game.game.art_background.clone(),
        logo: game.game.art_logo.clone(),
        description: game.metadata.as_ref().and_then(|metadata| metadata.description.clone()),
        runtime: field(collection::RUNTIME_KEY).and_then(serde_json::Value::as_str).map(str::to_owned),
        released: field(collection::RELEASED_KEY).and_then(serde_json::Value::as_str).map(str::to_owned),
        genres: game.metadata.as_ref().map(|metadata| metadata.genres.clone()).unwrap_or_default(),
        top: field("stremioTop").and_then(serde_json::Value::as_bool).unwrap_or(false),
        catalog: field(collection::CATALOG_KEY).and_then(serde_json::Value::as_str).map(str::to_owned),
        catalogs: field(collection::CATALOGS_KEY)
            .and_then(serde_json::Value::as_array)
            .map(|marks| {
                marks
                    .iter()
                    .filter_map(|mark| {
                        Some(heronite_watch_core::types::WireMark {
                            source: mark.get("source").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(),
                            catalog: mark.get("catalog").and_then(serde_json::Value::as_str)?.to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        last_watched: game.last_played.clone(),
        minutes_played: game.playtime_minutes as f64,
        position_ms: field(collection::POSITION_KEY).and_then(serde_json::Value::as_u64).unwrap_or(0),
        duration_ms: field(collection::DURATION_KEY).and_then(serde_json::Value::as_u64).unwrap_or(0),
        finished: field(collection::FINISHED_KEY).and_then(serde_json::Value::as_bool).unwrap_or(false),
        provider: if imported { String::new() } else { game.runner.clone() },
        addon: if imported {
            IMPORTED_ADDON.to_owned()
        } else {
            field(collection::SOURCE_KEY).and_then(serde_json::Value::as_str).map(str::to_owned).unwrap_or_default()
        },
    };
    Title::of_item(&item)
}

/// Folds one local record into a title: the later observation wins, as
/// [`Title::merge`] decides between two providers' states. The minutes stay the
/// provider's (a mark made by hand says nothing about time spent); the position,
/// the length and the flag come from the record this machine wrote.
fn fold_watch(title: &mut Title, local: &MovieProgress) {
    let provider = title.watched.take();
    if provider.as_ref().and_then(|seen| seen.at).is_some_and(|at| at > local.updated_at) {
        title.watched = provider;
        return;
    }
    let minutes = provider.as_ref().map_or(0.0, |seen| seen.minutes);
    title.watched = Some(Watched {
        at: Some(local.updated_at),
        minutes,
        position_ms: local.position_ms,
        duration_ms: local.duration_ms,
        finished: local.finished,
    });
}

/// The item a placeholder is: a title a provider knows and no source here plays.
fn placeholder_item(placeholder: &Placeholder) -> MovieItem {
    MovieItem {
        id: placeholder.imdb.clone(),
        kind: placeholder.kind.clone(),
        name: placeholder.title.clone(),
        year: placeholder.year.clone(),
        provider: String::new(),
        addon: IMPORTED_ADDON.to_owned(),
        ..MovieItem::default()
    }
}

/// The movies Collection, refreshed from the providers when possible. A
/// provider failure keeps the last good library instead of emptying the screen.
/// How long a rails refresh stays good for.
///
/// A page landing reloads the library so the grid can show what was just
/// appended, and re-fetching every rail on each page is what made the grid
/// rebuild under the person scrolling (a live rail answers in another order
/// seconds later) — besides costing a request per page. Within this window a
/// reload only re-reads the file; a rail turned on or off refreshes at once.
const REFRESH_AFTER: Duration = Duration::from_secs(120);

/// What the last refresh saw: when it ran, the rails it asked for, and the
/// person's own record at that moment (`movies.json`'s modified time).
struct Refreshed {
    at: Instant,
    rails: Vec<String>,
    record: Option<std::time::SystemTime>,
}

/// The modified time of the person's record, which is what the fold into the
/// projection reads. `None` when the file is missing or its time cannot be read:
/// that is not "unchanged", so the first read after it lands refreshes.
fn record_stamp(app: &AppPaths) -> Option<std::time::SystemTime> {
    std::fs::metadata(heronite_watch_state::store::movies_path(app)).and_then(|meta| meta.modified()).ok()
}

/// Whether the rails are due a refresh: never fetched, fetched long enough ago,
/// the set the person turned on changed, or **their own record changed** (a mark
/// or a finished session folds into the projection, and waiting the whole window
/// for it would show a stale mark on the grid).
fn refresh_due(last: Option<&Refreshed>, rails: &[String], record: Option<std::time::SystemTime>) -> bool {
    let Some(last) = last else { return true };
    last.at.elapsed() >= REFRESH_AFTER || last.rails.as_slice() != rails || last.record != record
}

/// The movies Collection, refreshed from the providers when possible. A
/// provider failure keeps the last good library instead of emptying the screen.
pub fn library(app: &AppPaths, include_hidden: bool) -> Result<Library> {
    let root = app.cache.join(ROOT);
    let http = UreqTransport::new();
    // The collection rails this build has and the person turned on. The refresh
    // keeps a rail's titles while its source cannot answer, and drops the ones a
    // rail no longer holds — or a rail that is now off — with the rail.
    let rails = crate::collections::enabled(app);
    let ids: Vec<String> = rails.iter().map(|(_, rail)| rail.id.clone()).collect();
    static LAST_REFRESH: Mutex<Option<Refreshed>> = Mutex::new(None);
    let record = record_stamp(app);
    let due = {
        let last = LAST_REFRESH.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        refresh_due(last.as_ref(), &ids, record)
    };
    // One refresh at a time: a page landing, a mark and a rail's keep can each
    // start a load, and each would otherwise fetch every rail again (Simkl's
    // calendar is 2.5 MB). A load that finds a refresh running reads the file and
    // lets the refresh it waited for stand.
    static REFRESHING: Mutex<()> = Mutex::new(());
    let running = if due {
        match REFRESHING.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    } else {
        None
    };
    if due && running.is_none() {
        log::debug!("movies provider: a refresh is already running; reading the file");
    }
    if let Some(_refreshing) = running {
        match collection::refresh(&root, &ids, || items(&http, app, &rails)) {
            Ok(()) => {
                let mut last = LAST_REFRESH.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                // The stamp is read **before** the fold: a write that lands while
                // it runs is one the next load must see.
                *last = Some(Refreshed { at: Instant::now(), rails: ids, record });
            }
            Err(error) => log::warn!("movies provider: {error:#}; keeping the last library"),
        }
    }
    Library::load_with(&DataPaths::at(root), include_hidden)
}

/// Every provider's rails, in registry order. A provider that fails aborts the
/// refresh, so the last good library stays on screen.
///
/// The library is built by the one model the tests exercise (`Titles`), then
/// projected into the records the screens still read. The shelf and the
/// placeholders travel beside the rails, not as rails of their own.
fn items(http: &dyn Transport, app: &AppPaths, collections: &[(String, CollectionDescriptor)]) -> Result<Fetched> {
    let credentials = StoreCredentials::new(app);
    let ctx = Ctx { http, credentials: &credentials };
    let mut rails = Vec::new();
    for provider in registry::providers() {
        rails.extend(provider.browse(&ctx)?);
    }
    // Browsing rails come from the catalogue sources the person turned on, not
    // from a media provider: a source that is off contributes nothing, to the
    // home or to the tabs.
    let ids = CatalogIds::at(app);
    let catalog_ctx = CatalogCtx { http, credentials: &credentials, cache: Some(&ids) };
    let enabled = heronite_watch_state::settings::read(app).catalog_sources;
    rails.extend(catalog_rails(&heronite_watch_catalogs::enabled(&enabled), &catalog_ctx));
    // The person's own shelf: a title they kept opens like any other, even on a
    // refresh where no rail carried it.
    let shelf = crate::personal::read(app);
    let mut kept = shelf.watchlist;
    kept.extend(shelf.favourites);
    // Titles a provider knows and no source here has: they appear where titles
    // appear (greyed — the app greys what is not installed) instead of being
    // invisible until the library gains them.
    let imported = heronite_watch_state::placeholders::read(app);
    // The collection rails the person turned on: their mark, and the titles they
    // brought that nothing else did. What they did not bring leaves with them.
    let (collections, answered) = collection_rails(http, app, collections);
    let mut titles = Titles::from_parts(Parts { rails, collections, shelf: &kept, placeholders: &imported.items });
    titles.fold_local_progress(&crate::progress::LocalProgress::read(app));
    Ok(Fetched { titles: titles.into_titles(), answered })
}

/// One rail per enabled collection, in the person's order, and the ids of the
/// rails that answered — an empty page is an answer, a source that could not be
/// asked (no key, an outage) is not.
///
/// A rule is a catalogue page asked of the source that declared it, so one rail
/// costs what a tab page costs; a source that cannot answer costs that rail and
/// nothing else.
fn collection_rails(
    http: &dyn Transport,
    app: &AppPaths,
    collections: &[(String, CollectionDescriptor)],
) -> (Vec<(String, Vec<Title>)>, Vec<String>) {
    let mut rails = Vec::new();
    let mut answered = Vec::new();
    for (source, descriptor) in collections {
        match crate::collections::rail_with(http, app, source, descriptor) {
            Ok(titles) => {
                answered.push(descriptor.id.clone());
                if !titles.is_empty() {
                    rails.push((descriptor.id.clone(), titles));
                }
            }
            // The reason goes to the log only: a transport URL can carry a
            // debrid key.
            Err(error) => log::warn!("watch collection {}: {error:#}", descriptor.id),
        }
    }
    (rails, answered)
}

/// One rail per enabled source and kind, from that source's first catalogue of
/// the kind, every title marked as a Top item so the home shelves can pick them.
fn catalog_rails(sources: &[&dyn WatchCatalogSource], ctx: &CatalogCtx<'_>) -> Vec<Rail> {
    let mut rails = Vec::new();
    for source in sources {
        let catalogs = match source.catalogs(ctx) {
            Ok(catalogs) => catalogs,
            // One source failing (no key, an outage) must not empty the home.
            Err(error) => {
                log::warn!("watch catalogs {}: {error:#}", source.descriptor().id);
                continue;
            }
        };
        for kind in ["movie", "series"] {
            let Some(descriptor) = catalogs.iter().find(|catalog| catalog.kind == kind) else { continue };
            let request = CatalogRequest { catalog: descriptor.id.clone(), ..CatalogRequest::first(kind) };
            match source.catalog(ctx, &request) {
                Ok(page) if !page.items.is_empty() => {
                    let items = page.items.into_iter().map(|mut title| {
                        title.top = true;
                        title
                    });
                    rails.push(Rail { catalog: RailDescriptor { id: "top", kind }, items: items.collect() });
                }
                Ok(_) => {}
                Err(error) => log::warn!("watch catalogs {}: {error:#}", source.descriptor().id),
            }
        }
    }
    rails
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::catalog::{CatalogDescriptor, CatalogPage, CatalogSourceDescriptor};
    use heronite_watch_core::title::ProviderId;

    fn provider() -> ProviderId {
        ProviderId::declared("stremio")
    }

    /// A title a source answers with: one provider, no rail.
    fn title(id: &str, top: bool) -> Title {
        let mut title = Title::new(TitleKey::of_source("movie", id, &provider()), id).plays(provider());
        title.top = top;
        title
    }

    fn placeholder(imdb: &str) -> Placeholder {
        Placeholder {
            imdb: imdb.into(),
            kind: "movie".into(),
            title: "Imported Film".into(),
            year: Some("2020".into()),
            provider: "simkl".into(),
            first_seen: None,
        }
    }

    struct NoNet;

    impl Transport for NoNet {
        fn get(&self, _: &url::Url) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn post_json(&self, _: &url::Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    struct NoCredentials;

    impl heronite_watch_core::provider::Credentials for NoCredentials {
        fn get(&self, _: &str) -> Result<Option<heronite_watch_core::provider::Secret>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &heronite_watch_core::provider::Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    /// A catalogue source of the test's own, offering one film.
    struct Stub;

    const STUB: CatalogSourceDescriptor = CatalogSourceDescriptor { id: "stub", name: "Stub" };

    impl WatchCatalogSource for Stub {
        fn descriptor(&self) -> &CatalogSourceDescriptor {
            &STUB
        }
        fn catalogs(&self, _: &CatalogCtx<'_>) -> Result<Vec<CatalogDescriptor>> {
            Ok(vec![CatalogDescriptor {
                id: "shelf".into(),
                name: "Shelf".into(),
                kind: "movie".into(),
                sorts: Vec::new(),
                filters: Vec::new(),
                search: false,
                paging: true,
            }])
        }
        fn catalog(&self, _: &CatalogCtx<'_>, _: &CatalogRequest) -> Result<CatalogPage> {
            Ok(CatalogPage { items: vec![title("tt1", false)], more: false, consumed: 1 })
        }
    }

    #[test]
    fn an_enabled_source_supplies_the_browsing_rail() {
        let http = NoNet;
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };
        let rails = catalog_rails(&[&Stub], &ctx);
        assert_eq!(rails.len(), 1, "one rail, for the kind it offers");
        assert_eq!((rails[0].catalog.id, rails[0].catalog.kind), ("top", "movie"));
        assert!(rails[0].items.iter().all(|title| title.top), "a browsing rail's items are Top items");
    }

    #[test]
    fn a_source_that_is_off_contributes_no_rail() {
        let http = NoNet;
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };
        assert!(catalog_rails(&[], &ctx).is_empty(), "nothing enabled, nothing to browse");
    }

    fn rail(id: &'static str, items: Vec<Title>) -> Rail {
        Rail { catalog: RailDescriptor { id, kind: "movie" }, items }
    }

    #[test]
    fn a_mark_made_here_reaches_the_title_the_screens_read() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths::for_data(&DataPaths::at(dir.path().to_path_buf()));
        // A title the provider never counted, marked watched on its page: the
        // record lives in `movies.json`, and without the fold the cover would
        // still say it was never watched.
        let mut titles = Titles::from_parts(Parts {
            rails: vec![rail("top", vec![title("tt1", false)])],
            collections: Vec::new(),
            shelf: &[],
            placeholders: &[],
        });
        use crate::progress::{Mark, MarkTarget};
        let mark =
            Mark { provider: "stremio", id: "tt1", imdb: Some("tt1"), kind: "movie", name: "tt1", year: None, target: MarkTarget::Movie };
        assert!(crate::progress::set_watched(&app, &mark, true).is_ok());

        titles.fold_local_progress(&crate::progress::LocalProgress::read(&app));

        let seen = titles.get(&TitleKey::of_source("movie", "tt1", &provider())).and_then(|title| title.watched.as_ref());
        let seen = seen.expect("the mark reaches the title");
        assert_eq!(seen.state(), heronite_watch_core::progress::WatchedState::Watched);
        assert_eq!(seen.minutes, 0.0, "a mark by hand says nothing about time spent");
    }

    fn shelf_item(id: &str) -> ShelfTitle {
        ShelfTitle { id: id.into(), kind: "movie".into(), name: id.into(), provider: "stremio".into(), ..ShelfTitle::default() }
    }

    #[test]
    fn a_kept_title_rides_after_the_rails_and_before_the_placeholders() {
        let shelf = vec![shelf_item("tt3")];
        let placeholders = vec![placeholder("tt9")];
        let titles = Titles::from_parts(Parts {
            rails: vec![rail("top", vec![title("tt1", true)])],
            collections: Vec::new(),
            shelf: &shelf,
            placeholders: &placeholders,
        });
        let ids: Vec<&str> = titles.titles().iter().map(|title| title.key.id.as_str()).collect();
        assert_eq!(ids, ["tt1", "tt3", "tt9"], "the rails, then the shelf, then what nothing here plays");
    }

    #[test]
    fn a_collection_mark_keeps_the_feed_behind_it() {
        let mut listed = title("tt1", true);
        listed.rails = vec![RailMark::feed("tmdb", "movie:tmdb:top:Popular")];
        let titles = Titles::from_parts(Parts {
            rails: vec![rail("top", vec![listed])],
            collections: vec![("tmdb:top-rated:movie".to_owned(), vec![title("tt1", false)])],
            shelf: &[],
            placeholders: &[],
        });
        let held = &titles.titles()[0];
        assert_eq!(
            held.rails.first(),
            Some(&RailMark::collection("tmdb:top-rated:movie")),
            "the collection outranks the feed key: it is what the wire carries"
        );
        assert_eq!(
            held.rails.get(1),
            Some(&RailMark::feed("tmdb", "movie:tmdb:top:Popular")),
            "and the catalogue that listed it is not destroyed in memory"
        );
        assert_eq!(held.rails.first().map(RailMark::as_wire).as_deref(), Some("collection:tmdb:top-rated:movie"));
    }

    #[test]
    fn a_collection_rail_adds_a_title_no_other_rail_brought() {
        let title = title("tt4", false);
        let titles = Titles::from_parts(Parts {
            rails: Vec::new(),
            collections: vec![("tmdb:top-rated:movie".to_owned(), vec![title])],
            shelf: &[],
            placeholders: &[],
        });
        assert_eq!(titles.len(), 1);
        assert_eq!(titles.titles()[0].rails.first(), Some(&RailMark::collection("tmdb:top-rated:movie")));
    }

    #[test]
    fn the_first_collection_of_the_settings_order_names_a_title() {
        let mut series = title("tt1", false);
        series.key.kind = heronite_watch_core::title::TitleKind::Series;
        let titles = Titles::from_parts(Parts {
            rails: Vec::new(),
            collections: vec![
                ("tmdb:popular:movie".to_owned(), vec![title("tt1", false)]),
                ("tmdb:top-rated:movie".to_owned(), vec![series]),
            ],
            shelf: &[],
            placeholders: &[],
        });
        let marks: Vec<(&str, &str, Option<String>)> = titles
            .titles()
            .iter()
            .map(|title| (title.key.id.as_str(), title.key.kind.as_str(), title.rails.first().map(RailMark::as_wire)))
            .collect();
        assert_eq!(
            marks,
            [
                ("tt1", "movie", Some("collection:tmdb:popular:movie".to_owned())),
                ("tt1", "series", Some("collection:tmdb:top-rated:movie".to_owned())),
            ],
            "a title in two rails keeps the first, and the same id in another kind is another title"
        );
    }

    #[test]
    fn a_collection_whose_source_cannot_answer_costs_that_rail_only() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        // Turned on, but the credential store holds no TMDB key: the rule cannot
        // be resolved, and that is a log line rather than a broken library.
        let settings =
            heronite_watch_state::settings::Settings { home_rails: vec!["tmdb:top-rated:movie".to_owned()], ..Default::default() };
        heronite_watch_state::settings::write(&app, &settings).unwrap();
        let (rails, answered) = collection_rails(&NoNet, &app, &crate::collections::enabled(&app));
        assert!(rails.is_empty() && answered.is_empty(), "a keyless source is skipped, not fatal");
    }

    #[test]
    fn two_rails_carrying_one_title_keep_both_the_providers_and_the_placement() {
        let mut first = title("tt1", false);
        first.rails = vec![RailMark::feed("tmdb", "movie:top:Popular")];
        let mut second = title("tt1", true);
        second.providers.insert(ProviderId::declared("youtube"));
        let titles = Titles::from_parts(Parts {
            rails: vec![rail("continue-watching", vec![first]), rail("top", vec![second])],
            collections: Vec::new(),
            shelf: &[],
            placeholders: &[],
        });
        let held = &titles.titles()[0];
        assert_eq!(titles.len(), 1, "one title, not two");
        assert_eq!(held.providers.len(), 2, "the second observation's provider is not lost");
        assert!(held.top, "a Top placement is kept from any observation");
    }

    #[test]
    fn the_library_keeps_the_order_the_availability_and_the_first_mark() {
        let mut listed = title("tt1", true);
        listed.rails = vec![RailMark::feed("tmdb", "movie:top:Popular")];
        let placeholders = vec![placeholder("tt9")];
        let rails = vec![rail("top", vec![listed, title("tt2", true)]), rail("personal", vec![title("tt3", false)])];
        let collections = vec![("tmdb:top-rated:movie".to_owned(), vec![title("tt1", false), title("tt4", false)])];

        let titles = Titles::from_parts(Parts { rails, collections, shelf: &[], placeholders: &placeholders });

        let rows: Vec<(&str, bool, Option<String>)> = titles
            .titles()
            .iter()
            .map(|title| (title.key.id.as_str(), title.available(), title.rails.first().map(RailMark::as_wire)))
            .collect();
        assert_eq!(
            rows,
            [
                ("tt1", true, Some("collection:tmdb:top-rated:movie".to_owned())),
                ("tt2", true, None),
                ("tt3", true, None),
                ("tt9", false, None),
                ("tt4", true, Some("collection:tmdb:top-rated:movie".to_owned())),
            ],
            "the rails' order, then the placeholders, then what only a collection brought"
        );
    }

    #[test]
    fn two_rails_carrying_one_title_give_one_title() {
        let mut first = title("tt1", true);
        first.rails = vec![RailMark::feed("tmdb", "movie:top:Popular")];
        let mut second = title("tt1", true);
        second.providers.insert(ProviderId::declared("youtube"));
        second.rails = vec![RailMark::feed("cinemeta", "movie:new")];

        let titles = Titles::from_parts(Parts {
            rails: vec![rail("top", vec![first]), rail("new", vec![second])],
            collections: Vec::new(),
            shelf: &[],
            placeholders: &[],
        });

        assert_eq!(titles.len(), 1, "one title, not two");
        let title = &titles.titles()[0];
        assert_eq!(title.providers.len(), 2, "both providers play it");
        assert_eq!(title.rails.len(), 2, "both rails are remembered");
        assert_eq!(title.rails[0], RailMark::feed("tmdb", "movie:top:Popular"), "the first rail still names it");
    }

    #[test]
    fn a_kept_title_the_rails_did_not_carry_is_still_a_title() {
        let shelf = vec![shelf_item("tt3")];
        let titles = Titles::from_parts(Parts { rails: Vec::new(), collections: Vec::new(), shelf: &shelf, placeholders: &[] });
        assert_eq!(titles.len(), 1);
        assert!(titles.titles()[0].available(), "the snapshot names the provider that played it");
    }

    #[test]
    fn a_placeholder_is_a_title_nothing_here_plays() {
        let placeholders = vec![placeholder("tt9")];
        let titles = Titles::from_parts(Parts { rails: Vec::new(), collections: Vec::new(), shelf: &[], placeholders: &placeholders });
        assert_eq!(titles.len(), 1);
        assert!(!titles.titles()[0].available(), "a title a provider knows and no source here plays");
        assert_eq!(titles.titles()[0].key.id.as_str(), "tt9");
        assert_eq!(titles.titles()[0].release.as_deref(), Some("2020"));
    }

    #[test]
    fn a_page_reload_does_not_refetch_the_rails_but_a_new_rail_does() {
        let rails = vec!["tmdb:top-rated:movie".to_owned()];
        let record = None;
        assert!(refresh_due(None, &rails, record), "the first load fetches");
        let just_now = Refreshed { at: Instant::now(), rails: rails.clone(), record };
        assert!(!refresh_due(Some(&just_now), &rails, record), "a page landing reloads the file, not the rails");
        let stale = Refreshed { at: Instant::now() - REFRESH_AFTER - Duration::from_secs(1), rails: rails.clone(), record };
        assert!(refresh_due(Some(&stale), &rails, record), "the window passes");
        assert!(refresh_due(Some(&just_now), &["other:rail".to_owned()], record), "turning a rail on refreshes at once");
    }

    #[test]
    fn the_production_path_keeps_a_placeholder_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        // Catalogue sources are off by default and Stremio has no account here,
        // so the only title is the one a provider knows and nothing can play.
        heronite_watch_state::placeholders::upsert(&app, placeholder("tt9")).unwrap();

        let fetched = items(&NoNet, &app, &[]).unwrap();
        assert_eq!(fetched.titles.len(), 1, "the placeholder is the library");
        assert!(!fetched.titles[0].available(), "and no provider plays it");
    }

    #[test]
    fn a_placeholder_projects_as_unavailable_and_the_writer_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let placeholders = vec![placeholder("tt9")];
        let titles = Titles::from_parts(Parts { rails: Vec::new(), collections: Vec::new(), shelf: &[], placeholders: &placeholders });
        heronite_watch_state::collection::write_root(dir.path(), titles.titles()).unwrap();
        let library = Library::load_with(&DataPaths::at(dir.path().to_path_buf()), true).unwrap();
        let game = &library.games[0];
        assert!(!game.installed, "nothing here can play it, so the tiles grey it");
        assert_eq!(game.runner, "imported", "not a provider's record");
        assert_eq!(game.game.extra.as_ref().and_then(|extra| extra.get("imported")).and_then(serde_json::Value::as_bool), Some(true));
    }

    #[test]
    fn a_title_comes_back_from_the_projection_with_its_catalogue_and_its_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut item = MovieItem {
            id: "tt1".into(),
            kind: "movie".into(),
            name: "A Film".into(),
            provider: "stremio".into(),
            addon: "tmdb".into(),
            top: true,
            catalog: Some("movie:top:Popular".into()),
            description: Some("A plot".into()),
            genres: vec!["Drama".into()],
            last_watched: Some("2026-11-12T20:00:00Z".into()),
            minutes_played: 42.0,
            ..MovieItem::default()
        };
        item.year = Some("2026".into());
        heronite_watch_state::collection::write_root(dir.path(), &[Title::of_item(&item)]).unwrap();

        let titles = Titles::from_projection(&DataPaths::at(dir.path().to_path_buf())).unwrap();
        assert_eq!(titles.len(), 1);
        let title = &titles.titles()[0];
        assert_eq!(title.key, TitleKey::of_item(&item));
        assert_eq!(title.name, item.name);
        assert_eq!(title.description.as_deref(), Some("A plot"));
        assert_eq!(title.genres, ["Drama"]);
        assert!(title.available());
        assert_eq!(title.providers.len(), 1, "one record carries one provider");
        assert_eq!(title.rails.len(), 1, "one record carries one mark");
        assert_eq!(
            title.rails[0],
            RailMark::feed("tmdb", "movie:top:Popular"),
            "the catalogue that listed it travels in the record's watchSource"
        );
        let stamp = chrono::DateTime::parse_from_rfc3339("2026-11-12T20:00:00Z").expect("an rfc3339 instant");
        assert_eq!(title.watched.as_ref().and_then(|seen| seen.at), Some(stamp.with_timezone(&chrono::Utc)));
        assert_eq!(title.watched.as_ref().map(|seen| seen.minutes), Some(42.0));
    }

    #[test]
    fn a_stream_probe_marks_the_record_and_the_writer_reads_it_back() {
        use heronite_watch_state::streams;
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths::for_data(&DataPaths::at(dir.path().to_path_buf()));
        // The projection root lives under the cache; a probe is kept beside it.
        let root = app.cache.join(heronite_watch_state::collection::ROOT);
        std::fs::create_dir_all(&root).unwrap();
        streams::note(&app, "stremio", "movie", "tt1", 0).unwrap();
        let item = MovieItem {
            id: "tt1".into(),
            kind: "movie".into(),
            name: "A Film".into(),
            provider: "stremio".into(),
            addon: "cinemeta".into(),
            catalog: Some("movie:cinemeta:top:Popular".into()),
            ..MovieItem::default()
        };
        heronite_watch_state::collection::write_root(&root, &[Title::of_item(&item)]).unwrap();

        let library = Library::load_with(&DataPaths::at(root.clone()), true).unwrap();
        let extra = library.games[0].game.extra.as_ref().expect("the record's extra");
        assert_eq!(extra[heronite_watch_state::collection::SOURCE_KEY], "cinemeta", "where it was listed");
        assert_eq!(extra[heronite_watch_state::collection::STREAMS_KEY], false, "asked, and no stream answered");
        assert!(library.games[0].installed, "the record is still playable in principle; the probe is a separate fact");
    }

    #[test]
    fn the_projections_two_readers_agree() {
        let dir = tempfile::tempdir().unwrap();
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-12T20:00:00Z").expect("an rfc3339 instant").with_timezone(&chrono::Utc);
        let mut listed = title("tt1", true);
        listed.rails = vec![RailMark::feed("tmdb", "movie:top:Popular")];
        listed.released = Some("2026-11-14".into());
        listed.runtime = Some("173 min".into());
        listed.watched = Some(heronite_watch_core::title::Watched {
            at: Some(at),
            minutes: 42.0,
            position_ms: 2_271_644,
            duration_ms: 6_628_768,
            finished: false,
        });
        let placeholders = vec![placeholder("tt9")];
        let titles = Titles::from_parts(Parts {
            rails: vec![rail("top", vec![listed])],
            collections: Vec::new(),
            shelf: &[],
            placeholders: &placeholders,
        });
        heronite_watch_state::collection::write_root(dir.path(), titles.titles()).unwrap();

        let raw = heronite_watch_state::collection::titles(dir.path());
        let loaded = Titles::from_projection(&DataPaths::at(dir.path().to_path_buf())).unwrap();

        let view = |titles: &[Title]| -> Vec<String> {
            titles
                .iter()
                .map(|title| {
                    format!(
                        "{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                        title.key.id.as_str(),
                        title.available(),
                        title.providers.len(),
                        title.rails.len(),
                        title.rails.first().map(RailMark::as_wire),
                        title.name,
                        title.release,
                        title.released,
                        title.runtime,
                        title.description,
                        title.genres,
                        title.art,
                        title.watched,
                    )
                })
                .collect()
        };
        assert_eq!(view(&raw), view(loaded.titles()), "the raw decode and the loaded-library decode must agree");
        assert!(!raw[1].available(), "and the placeholder is one, whichever reader");
    }

    #[test]
    fn the_rails_refresh_when_the_persons_record_moved_inside_the_window() {
        use std::time::SystemTime;

        let rails = vec!["cinemeta".to_owned()];
        let seen = SystemTime::UNIX_EPOCH;
        let now = || Refreshed { at: Instant::now(), rails: rails.clone(), record: Some(seen) };

        assert!(refresh_due(None, &rails, Some(seen)), "never fetched is due");
        assert!(!refresh_due(Some(&now()), &rails, Some(seen)), "same rails, same record, inside the window: not due");
        assert!(refresh_due(Some(&now()), &rails, Some(seen + std::time::Duration::from_secs(1))), "a mark landed: due now");
        assert!(refresh_due(Some(&now()), &rails, None), "the record went away: due");
        assert!(!refresh_due(Some(&now()), &rails, Some(seen)), "and a record that did not move is not");
        assert!(refresh_due(Some(&now()), &["tmdb".to_owned()], Some(seen)), "another set of rails is due");

        let old = Refreshed { at: Instant::now() - REFRESH_AFTER - std::time::Duration::from_secs(1), ..now() };
        assert!(refresh_due(Some(&old), &rails, Some(seen)), "long enough ago is due");
    }
}
