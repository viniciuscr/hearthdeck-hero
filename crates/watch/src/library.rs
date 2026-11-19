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

use anyhow::Result;
use heronite_data::paths::AppPaths;
use heronite_data::{CollectionGame, DataPaths, Library};
use heronite_watch_core::catalog::{CatalogCtx, CatalogRequest, WatchCatalogSource};
use heronite_watch_core::collections::CollectionDescriptor;
use heronite_watch_core::http::{Transport, UreqTransport};
use heronite_watch_core::provider::{Ctx, Rail, RailDescriptor};
use heronite_watch_core::title::{RailMark, Title, TitleKey, IMPORTED_ADDON};
use heronite_watch_core::types::MovieItem;
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
/// than beside `Title`. It reads the keys the writer froze; the catalogue that
/// listed a title is not among them, so a reconstructed feed mark has no source
/// until the writer records one.
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
        genres: game.metadata.as_ref().map(|metadata| metadata.genres.clone()).unwrap_or_default(),
        top: field("stremioTop").and_then(serde_json::Value::as_bool).unwrap_or(false),
        catalog: field(collection::CATALOG_KEY).and_then(serde_json::Value::as_str).map(str::to_owned),
        last_watched: game.last_played.clone(),
        minutes_played: game.playtime_minutes as f64,
        provider: if imported { String::new() } else { game.runner.clone() },
        addon: if imported { IMPORTED_ADDON.to_owned() } else { String::new() },
        ..MovieItem::default()
    };
    Title::of_item(&item)
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
pub fn library(app: &AppPaths, include_hidden: bool) -> Result<Library> {
    let root = app.cache.join(ROOT);
    let http = UreqTransport::new();
    // The collection rails this build has and the person turned on. The refresh
    // keeps a rail's titles while its source cannot answer, and drops the ones a
    // rail no longer holds — or a rail that is now off — with the rail.
    let rails = crate::collections::enabled(app);
    let ids: Vec<String> = rails.iter().map(|(_, rail)| rail.id.clone()).collect();
    if let Err(error) = collection::refresh(&root, &ids, || items(&http, app, &rails)) {
        log::warn!("movies provider: {error:#}; keeping the last library");
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
    let titles = Titles::from_parts(Parts { rails, collections, shelf: &kept, placeholders: &imported.items });
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
    fn a_title_comes_back_from_the_projection_with_its_documented_losses() {
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
            RailMark::feed("", "movie:top:Popular"),
            "the catalogue that listed it is not in the projection, so a feed mark comes back without its source"
        );
        let stamp = chrono::DateTime::parse_from_rfc3339("2026-11-12T20:00:00Z").expect("an rfc3339 instant");
        assert_eq!(title.watched.as_ref().and_then(|seen| seen.at), Some(stamp.with_timezone(&chrono::Utc)));
        assert_eq!(title.watched.as_ref().map(|seen| seen.minutes), Some(42.0));
    }

    #[test]
    fn the_projections_two_readers_agree() {
        let dir = tempfile::tempdir().unwrap();
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-12T20:00:00Z").expect("an rfc3339 instant").with_timezone(&chrono::Utc);
        let mut listed = title("tt1", true);
        listed.rails = vec![RailMark::feed("tmdb", "movie:top:Popular")];
        listed.watched = Some(heronite_watch_core::title::Watched { at: Some(at), minutes: 42.0 });
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
                        "{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                        title.key.id.as_str(),
                        title.available(),
                        title.providers.len(),
                        title.rails.len(),
                        title.rails.first().map(RailMark::as_wire),
                        title.name,
                        title.release,
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
}
