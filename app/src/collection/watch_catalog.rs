//! The Movies and Series tabs as provider catalogs.
//!
//! Those tabs are not a filter over the rails: they ask the provider for the
//! next page of its catalog (the first one, the genre, or a search) and merge it
//! into the Watch library, which is then reloaded, so cards, covers, details
//! and streams keep working exactly as they do for the rails.
//!
//! Fork-owned. Pages arrive as the grid comes near the end of what is loaded
//! (two rows before the end), and a search replaces what the tab shows until
//! it is cleared.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use heronite_i18n::tr;
use heronite_watch::catalog::{CatalogRequest, PAGE};

use super::CollectionView;

/// The provider the tabs browse.
const PROVIDER: &str = "stremio";
/// The catalog the tabs show.
const CATALOG: &str = "top";
/// How many rows from the end the next page is asked for.
const PREFETCH_ROWS: usize = 2;
/// How long to wait after a failed page (a broken request must not spin the
/// grid: a fetch per frame is what made the screen blink).
const RETRY: Duration = Duration::from_secs(20);
/// Pages that brought nothing new before the feed gives up (a provider that
/// ignores `skip` would otherwise be asked for ever).
const EMPTY_PAGES: u8 = 2;

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
    /// A page has been asked for (the feed is a catalog tab, not empty).
    requested: bool,
    /// Items already fetched (the provider's `skip`).
    skip: u32,
    /// A page is in flight.
    loading: bool,
    /// The provider has no more pages.
    done: bool,
    /// The search the tab shows: its query and the ids it answered with.
    search: Option<(String, BTreeSet<String>)>,
    /// The query being looked up, while it is in flight.
    looking_up: Option<String>,
    /// When a failed page may be tried again.
    retry_at: Option<Instant>,
    /// Pages in a row that brought nothing new.
    empty_pages: u8,
}

/// The two feeds, by kind.
#[derive(Default)]
pub(super) struct WatchCatalog {
    feeds: HashMap<&'static str, Feed>,
}

impl WatchCatalog {
    /// Whether a title belongs in the tab: always, unless a search is showing.
    pub(super) fn visible(&self, kind: &str, id: &str) -> bool {
        match self.feeds.get(kind).and_then(|feed| feed.search.as_ref()) {
            Some((_, ids)) => ids.contains(id),
            None => true,
        }
    }

    /// Whether a page is in flight (the grid says so at its end).
    pub(super) fn loading(&self, kind: &str) -> bool {
        self.feeds.get(kind).is_some_and(|feed| feed.loading)
    }
}

impl CollectionView {
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
            || feed.search.as_ref().is_some_and(|(shown, _)| *shown == query)
        {
            return;
        }
        feed.looking_up = Some(query.clone());
        feed.skip = 0;
        feed.done = false;
        self.watch_feed_fetch(kind, cx);
    }

    /// Whether the tab is waiting for a page (the grid says so under it).
    pub(super) fn watch_feed_loading(&self, kind: &str) -> bool {
        self.watch_catalog.loading(kind)
    }

    /// The tab's search box was cleared: the catalog shows again.
    pub(super) fn watch_feed_clear_search(&mut self, kind: &'static str) {
        if let Some(feed) = self.watch_catalog.feeds.get_mut(kind) {
            if feed.search.is_some() {
                feed.search = None;
            }
        }
    }

    /// Fetches the next page of the tab's feed (a catalog page or a search).
    fn watch_feed_fetch(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let feed = self.watch_catalog.feeds.entry(kind).or_default();
        feed.requested = true;
        feed.loading = true;
        let request = CatalogRequest {
            kind: kind.to_owned(),
            catalog: CATALOG.to_owned(),
            skip: feed.skip,
            search: feed.looking_up.clone(),
            genre: None,
        };
        let app = self.app_paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let http = heronite_watch::UreqTransport::new();
                    heronite_watch::catalog::more_with(&http, &app, PROVIDER, &request)
                })
                .await;
            this.update(cx, |this, cx| {
                let feed = this.watch_catalog.feeds.entry(kind).or_default();
                feed.loading = false;
                feed.retry_at = None;
                match result {
                    Ok((items, added)) => {
                        let mut done = items.len() < PAGE as usize;
                        if let Some(query) = feed.looking_up.take() {
                            // Cinemeta answers a search with one page of at most 20.
                            done = true;
                            feed.skip = items.len() as u32;
                            feed.search = Some((query, items.iter().map(|item| item.id.clone()).collect()));
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
    fn a_search_shows_only_its_own_titles_and_clearing_shows_all_again() {
        let mut catalog = WatchCatalog::default();
        assert!(catalog.visible("movie", "tt1"), "an empty feed shows everything");
        let feed = catalog.feeds.entry("movie").or_default();
        feed.search = Some(("matrix".into(), BTreeSet::from(["tt0133093".to_string()])));
        assert!(catalog.visible("movie", "tt0133093"));
        assert!(!catalog.visible("movie", "tt0234215"));
        assert!(catalog.visible("series", "tt0234215"), "the other tab is untouched");
        catalog.feeds.get_mut("movie").unwrap().search = None;
        assert!(catalog.visible("movie", "tt0234215"));
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
