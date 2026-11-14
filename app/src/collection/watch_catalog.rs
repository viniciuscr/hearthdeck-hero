//! The Movies and Series tabs as provider catalogs.
//!
//! Those tabs are not a filter over the rails: they ask the provider for the
//! next page of one of its catalogs (with the order and filters it declares) and
//! merge it into the Watch library, which is then reloaded, so cards, covers,
//! details and streams keep working exactly as they do for the rails.
//!
//! What a tab shows is the current feed's own ids — not every title the library
//! holds — so the rails never leak into a catalog and a search shows the
//! search's titles alone. A feed carries a generation: a page asked for before
//! the feed was replaced is dropped when it lands, so a slow answer cannot
//! repaint a tab the person has already changed.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use heronite_i18n::tr;
use heronite_watch::catalog::{CatalogDescriptor, CatalogRequest, CatalogSort};

use super::CollectionView;

/// The provider the tabs browse.
const PROVIDER: &str = "stremio";
/// How many rows from the end the next page is asked for.
const PREFETCH_ROWS: usize = 2;
/// How long to wait after a failed page (a broken request must not spin the
/// grid: a fetch per frame is what made the screen blink).
const RETRY: Duration = Duration::from_secs(20);
/// Pages that brought nothing new before the feed gives up (a provider that
/// ignores `skip` would otherwise be asked for ever).
const EMPTY_PAGES: u8 = 2;
/// The catalog a tab shows before the provider's list has arrived.
const DEFAULT_CATALOG: &str = "top";

/// The feed a tab source browses, when the tab is one of the catalog ones.
pub(super) fn feed_kind(source: &str) -> Option<&'static str> {
    match source {
        "type:movie" => Some("movie"),
        "type:series" => Some("series"),
        _ => None,
    }
}

/// One tab's catalog feed.
#[derive(Default)]
struct Feed {
    /// One per feed the tab has asked for: a page from an older generation is
    /// stale and dropped.
    generation: u64,
    /// The ids this feed shows: the tab lists these and nothing else.
    ids: BTreeSet<String>,
    /// A page has been asked for.
    requested: bool,
    /// Items already fetched (the provider's `skip`).
    skip: u32,
    /// A page is in flight.
    loading: bool,
    /// The provider has no more pages.
    done: bool,
    /// The catalog the tab shows.
    catalog: String,
    sort: CatalogSort,
    /// The chosen filters as `filter id → value`.
    filters: Vec<(String, String)>,
    /// The search the tab shows, once one is in.
    shown_query: Option<String>,
    /// The query being looked up, while it is in flight.
    looking_up: Option<String>,
    /// When a failed page may be tried again.
    retry_at: Option<Instant>,
    /// Pages in a row that brought nothing new.
    empty_pages: u8,
}

impl Feed {
    /// Whether a page asked for in `generation` still belongs to this feed.
    fn accepts(&self, generation: u64) -> bool {
        self.generation == generation
    }

    /// A new feed of the same tab: what was fetched belongs to the previous
    /// catalog, order, filter or search.
    fn reset(&mut self, generation: u64) {
        self.generation = generation;
        self.ids.clear();
        self.requested = false;
        self.skip = 0;
        self.done = false;
        self.empty_pages = 0;
        self.retry_at = None;
    }
}

/// The two feeds, and what the provider declared.
#[derive(Default)]
pub(super) struct WatchCatalog {
    feeds: HashMap<&'static str, Feed>,
    /// The provider's catalogs, fetched once when a tab needs them.
    declared: Option<Vec<CatalogDescriptor>>,
    loading_declared: bool,
}

impl WatchCatalog {
    /// Whether a title belongs in the tab: the current feed's own items once it
    /// has asked for a page, and everything before that.
    pub(super) fn visible(&self, kind: &str, id: &str) -> bool {
        match self.feeds.get(kind) {
            Some(feed) if feed.requested => feed.ids.contains(id),
            _ => true,
        }
    }

    /// Whether a page is in flight (the grid says so at its end).
    pub(super) fn loading(&self, kind: &str) -> bool {
        self.feeds.get(kind).is_some_and(|feed| feed.loading)
    }
}

impl CollectionView {
    /// The catalogs the provider declares for a tab's kind; empty until the list
    /// has been fetched.
    pub(super) fn watch_catalogs_of(&self, kind: &str) -> Vec<CatalogDescriptor> {
        self.watch_catalog.declared.clone().unwrap_or_default().into_iter().filter(|catalog| catalog.kind == kind).collect()
    }

    /// The catalog the tab shows now (the provider's first one by default).
    pub(super) fn watch_feed_catalog(&self, kind: &str) -> String {
        match self.watch_catalog.feeds.get(kind).map(|feed| feed.catalog.clone()).filter(|catalog| !catalog.is_empty()) {
            Some(catalog) => catalog,
            None => self.watch_catalogs_of(kind).first().map(|catalog| catalog.id.clone()).unwrap_or_else(|| DEFAULT_CATALOG.to_owned()),
        }
    }

    /// The descriptor the tab is browsing now, with its sorts and filters.
    pub(super) fn watch_descriptor(&self, kind: &str) -> Option<CatalogDescriptor> {
        let chosen = self.watch_feed_catalog(kind);
        self.watch_catalogs_of(kind).into_iter().find(|catalog| catalog.id == chosen)
    }

    /// The order the tab shows now.
    pub(super) fn watch_feed_sort(&self, kind: &str) -> CatalogSort {
        self.watch_catalog.feeds.get(kind).map_or(CatalogSort::Popular, |feed| feed.sort)
    }

    /// The filters the tab shows now.
    pub(super) fn watch_feed_filters(&self, kind: &str) -> Vec<(String, String)> {
        self.watch_catalog.feeds.get(kind).map(|feed| feed.filters.clone()).unwrap_or_default()
    }

    /// Fetches what the provider declares, once.
    pub(super) fn watch_catalogs_ensure(&mut self, cx: &mut Context<Self>) {
        if self.watch_catalog.declared.is_some() || self.watch_catalog.loading_declared {
            return;
        }
        self.watch_catalog.loading_declared = true;
        let app = self.app_paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { heronite_watch::catalog::catalogs(&app, PROVIDER) }).await;
            this.update(cx, |this, cx| {
                this.watch_catalog.loading_declared = false;
                match result {
                    Ok(declared) => {
                        log::info!("watch catalogs: {} declared", declared.len());
                        this.watch_catalog.declared = Some(declared);
                        cx.notify();
                    }
                    Err(error) => log::warn!("watch catalogs: {error:#}"),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Shows another catalog, order or set of filters: the tab starts again at
    /// the first page and the previous feed's pages leave the library.
    pub(super) fn watch_feed_select(
        &mut self,
        kind: &'static str,
        catalog: String,
        sort: CatalogSort,
        filters: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        let feed = self.watch_catalog.feeds.entry(kind).or_default();
        if feed.requested && feed.catalog == catalog && feed.sort == sort && feed.filters == filters {
            return;
        }
        feed.catalog = catalog;
        feed.sort = sort;
        feed.filters = filters;
        feed.shown_query = None;
        feed.looking_up = None;
        let generation = feed.generation + 1;
        feed.reset(generation);
        self.watch_feed_fetch(kind, cx);
    }

    /// Keeps the tab's catalogue coming: the first page when there is none, and
    /// the next one when the grid's last visible row is near the end of what is
    /// loaded. Called from the grid's render, so it must be a no-op unless a
    /// page is genuinely due.
    pub(super) fn watch_feed_advance(&mut self, kind: &'static str, visible_end: usize, rows: usize, cx: &mut Context<Self>) {
        let feed = self.watch_catalog.feeds.entry(kind).or_default();
        if feed.done || feed.loading || feed.looking_up.is_some() {
            return;
        }
        if feed.retry_at.is_some_and(|at| Instant::now() < at) {
            return;
        }
        if !feed.requested || visible_end + PREFETCH_ROWS >= rows.saturating_sub(1) {
            self.watch_feed_fetch(kind, cx);
        }
    }

    /// The search the tab shows, or the catalog again when the box is empty.
    pub(super) fn watch_feed_search(&mut self, kind: &'static str, query: String, cx: &mut Context<Self>) {
        let feed = self.watch_catalog.feeds.entry(kind).or_default();
        if feed.loading
            || feed.looking_up.as_deref() == Some(query.as_str())
            || (feed.shown_query.as_deref() == Some(query.as_str()) && feed.requested)
        {
            return;
        }
        feed.shown_query = None;
        feed.looking_up = Some(query);
        let generation = feed.generation + 1;
        feed.reset(generation);
        self.watch_feed_fetch(kind, cx);
    }

    /// The tab's search box was cleared: the catalog shows again.
    pub(super) fn watch_feed_clear_search(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let Some(feed) = self.watch_catalog.feeds.get_mut(kind) else { return };
        if feed.loading || (feed.shown_query.is_none() && feed.looking_up.is_none()) {
            return;
        }
        feed.shown_query = None;
        feed.looking_up = None;
        let generation = feed.generation + 1;
        feed.reset(generation);
        self.watch_feed_fetch(kind, cx);
    }

    /// Whether the tab is waiting for a page (the grid says so under it).
    pub(super) fn watch_feed_loading(&self, kind: &str) -> bool {
        self.watch_catalog.loading(kind)
    }

    /// Fetches the next page of the tab's feed. The first page of a feed
    /// replaces that kind's pages; later ones are merged, so scrolling deeper
    /// keeps what is already there.
    fn watch_feed_fetch(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let feed = self.watch_catalog.feeds.entry(kind).or_default();
        if feed.loading {
            return;
        }
        feed.requested = true;
        feed.loading = true;
        let generation = feed.generation;
        let first = feed.skip == 0;
        let request = CatalogRequest {
            kind: kind.to_owned(),
            catalog: feed.catalog.clone(),
            skip: feed.skip,
            search: feed.looking_up.clone(),
            sort: feed.sort,
            filters: feed.filters.clone(),
        };
        let app = self.app_paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let http = heronite_watch::UreqTransport::new();
                    if first {
                        heronite_watch::catalog::replace_with(&http, &app, PROVIDER, &request)
                    } else {
                        heronite_watch::catalog::more_with(&http, &app, PROVIDER, &request)
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                let feed = this.watch_catalog.feeds.entry(kind).or_default();
                if !feed.accepts(generation) {
                    // The person changed catalog, order, filter or search while
                    // this page was in flight: it belongs to a feed that is gone.
                    log::debug!("watch catalog {kind}: a stale page was dropped");
                    return;
                }
                feed.loading = false;
                feed.retry_at = None;
                match result {
                    Ok((page, added)) => {
                        let items = page.items;
                        let mut done = !page.more || items.is_empty();
                        feed.ids.extend(items.iter().map(|item| item.id.clone()));
                        if let Some(query) = feed.looking_up.take() {
                            // A search is one page: Cinemeta does not paginate it.
                            done = true;
                            feed.skip = items.len() as u32;
                            feed.shown_query = Some(query);
                            feed.empty_pages = 0;
                        } else {
                            feed.skip += items.len() as u32;
                            feed.empty_pages = if added == 0 && !items.is_empty() { feed.empty_pages.saturating_add(1) } else { 0 };
                        }
                        // A page that brings nothing new twice stops the feed: it
                        // would otherwise be asked for again as soon as it lands.
                        feed.done = done || feed.empty_pages >= EMPTY_PAGES;
                        log::info!("watch catalog {kind}: {} items, {added} new", items.len());
                        if added > 0 {
                            // The library the screens read now holds the page.
                            this.reload(cx);
                        }
                    }
                    Err(error) => {
                        feed.looking_up = None;
                        feed.retry_at = Some(Instant::now() + RETRY);
                        log::warn!("watch catalog {kind}: {error:#}");
                        this.show_toast(tr!("Could not load more titles: {error}", error = format!("{error:#}")), true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_movies_and_series_tabs_are_catalogs() {
        assert_eq!(feed_kind("type:movie"), Some("movie"));
        assert_eq!(feed_kind("type:series"), Some("series"));
        assert_eq!(feed_kind("all"), None);
        assert_eq!(feed_kind("genre:Action"), None);
    }

    #[test]
    fn a_tab_shows_the_feed_ids_and_the_library_until_it_asks() {
        let mut catalog = WatchCatalog::default();
        assert!(catalog.visible("movie", "tt1"), "before any page, the library shows");
        let feed = catalog.feeds.entry("movie").or_default();
        feed.requested = true;
        feed.ids.insert("tt0133093".to_owned());
        assert!(catalog.visible("movie", "tt0133093"));
        assert!(!catalog.visible("movie", "tt0234215"), "a title the feed did not send is not in the tab");
        assert!(catalog.visible("series", "tt0234215"), "the other tab is untouched");
    }

    #[test]
    fn resetting_a_feed_forgets_its_pages_and_its_ids() {
        let mut feed = Feed { catalog: "top".into(), sort: CatalogSort::Popular, ..Feed::default() };
        feed.ids.insert("tt1".to_owned());
        feed.skip = 100;
        feed.requested = true;
        feed.done = true;
        feed.reset(7);
        assert_eq!(feed.generation, 7);
        assert!(feed.ids.is_empty() && !feed.requested && feed.skip == 0 && !feed.done);
    }

    #[test]
    fn a_page_from_a_feed_the_person_left_is_not_accepted() {
        let mut feed = Feed::default();
        feed.reset(3);
        assert!(feed.accepts(3), "the current feed's page lands");
        assert!(!feed.accepts(2), "a page from before the change does not");
    }

    #[test]
    fn a_feed_reports_its_page_in_flight() {
        let mut catalog = WatchCatalog::default();
        assert!(!catalog.loading("movie"));
        catalog.feeds.entry("movie").or_default().loading = true;
        assert!(catalog.loading("movie"));
        assert!(!catalog.loading("series"));
    }
}
