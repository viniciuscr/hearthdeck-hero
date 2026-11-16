//! What a console tab shows: its games (`TabSpec::source`) and, for the
//! layouts that have them, the theme's shelves (`ConsoleTheme::shelves`).
//! Both are kept in a small cache ([`SourceCache`]) shared by the frame, the
//! controller presses and the hints, and rebuilt when anything they read
//! changes (the library, the theme, the categories, the day...), so a theme
//! edit shows at once without a filter and a sort over the library per call.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::*;
use heronite_data::categories::Categories;
use heronite_data::console_theme::ShelfSpec;
use heronite_data::hltb::WikiCache;
use heronite_data::organize::{GameExtras, Organize};
use heronite_data::smart_groups::SmartGroups;
use heronite_data::{CollectionGame, Library};

use super::super::CollectionView;
use crate::console::theme_rows;

/// One row of covers.
#[derive(Clone)]
pub(crate) struct Shelf {
    pub title: SharedString,
    pub keys: Vec<SharedString>,
    /// Its games before the theme's `shelfLimit` cut it (`keys` may hold fewer).
    pub total: usize,
    /// What it shows (`ShelfSpec::kind`, `statuses` already split into `status:<id>`).
    pub kind: SharedString,
}

impl Shelf {
    /// The theme's limit left games out of it.
    pub fn cut(&self) -> bool {
        self.total > self.keys.len()
    }
}

pub(super) fn by_last_played(mut v: Vec<&CollectionGame>) -> Vec<&CollectionGame> {
    v.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title_lower.cmp(&b.title_lower)));
    v
}

pub(super) fn by_title(mut v: Vec<&CollectionGame>) -> Vec<&CollectionGame> {
    v.sort_by(|a, b| a.title_lower.cmp(&b.title_lower));
    v
}

pub(super) fn keys(v: Vec<&CollectionGame>) -> Vec<SharedString> {
    v.into_iter().map(|g| SharedString::from(g.key.clone())).collect()
}

/// The status of `id` by its slug or id (`plan-to-play`, `beaten`...).
fn status_is(lib: &Library, g: &CollectionGame, slug: &str) -> bool {
    g.status_id == slug || lib.statuses.iter().any(|s| s.id == g.status_id && s.slug == slug)
}

/// The name of the status `value` (its slug or id) in the library.
fn status_name(lib: &Library, value: &str) -> String {
    lib.statuses
        .iter()
        .find(|s| s.id == value || s.slug == value)
        .map_or_else(|| value.to_string(), |s| heronite_data::status::status_label(&s.name).to_string())
}

/// A day number for "YYYY-MM-DD..." (enough to compare ages).
fn day_number(date: &str) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(date.get(..10)?, "%Y-%m-%d").ok()?;
    Some(i64::from(d.num_days_from_ce()))
}

use chrono::Datelike as _;
use heronite_i18n::tr;

/// Today, as a [`day_number`].
fn today() -> i64 {
    i64::from(chrono::Local::now().date_naive().num_days_from_ce())
}

/// A number per game that changes every day, for the daily pick: the lowest
/// is the pick.
pub(super) fn daily_rank(key: &str, day: u64) -> u64 {
    fxhash(key) ^ day.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// Everything the games and shelves of a source read; they are rebuilt when
/// any of it differs from what the cached ones were built from.
struct Inputs {
    lib: Arc<Library>,
    extras: Arc<GameExtras>,
    wiki: Arc<WikiCache>,
    categories: Categories,
    /// The smart categories' rules (their games follow from them).
    smart: SmartGroups,
    epoch: u64,
    day: i64,
}

/// The games and shelves of each source seen lately, for [`Inputs`].
#[derive(Default)]
pub(super) struct SourceCache {
    built_from: Option<Inputs>,
    games: HashMap<String, Arc<Vec<Arc<CollectionGame>>>>,
    shelves: HashMap<String, Arc<Vec<Shelf>>>,
}

impl SourceCache {
    /// Keeps the entries only while the inputs are what they were built from
    /// (the libraries and caches are compared by identity, held so that an
    /// address cannot be reused while it is remembered).
    fn check(&mut self, view: &CollectionView, lib: &Arc<Library>) {
        let (epoch, day) = (crate::console::look::epoch(), today());
        let same = self.built_from.as_ref().is_some_and(|b| {
            Arc::ptr_eq(&b.lib, lib)
                && Arc::ptr_eq(&b.extras, &view.game_extras)
                && Arc::ptr_eq(&b.wiki, &view.wiki)
                && b.epoch == epoch
                && b.day == day
                && b.categories == view.categories
                && b.smart == view.smart_groups
        });
        if !same {
            self.games.clear();
            self.shelves.clear();
            self.built_from = Some(Inputs {
                lib: lib.clone(),
                extras: view.game_extras.clone(),
                wiki: view.wiki.clone(),
                categories: view.categories.clone(),
                smart: view.smart_groups.clone(),
                epoch,
                day,
            });
        }
    }
}

impl CollectionView {
    pub(super) fn organize(&self) -> Organize<'_> {
        Organize { extras: Some(&self.game_extras), categories: Some(&self.categories), ..Default::default() }
    }

    /// `g` is in the smart category `id`.
    fn in_smart(&self, id: &str, g: &CollectionGame) -> bool {
        self.smart_groups.get(id).is_some_and(|s| self.group_members(s).contains(&g.key))
    }

    /// The name of a status key, or of a whole `smart:<id>` kind.
    fn shelf_name(&self, lib: &Library, value: &str) -> String {
        match value.strip_prefix("smart:") {
            Some(id) => self.smart_groups.get(id).map_or_else(|| id.to_owned(), |g| g.name.clone()),
            None => status_name(lib, value),
        }
    }

    /// Whether `g` belongs to a tab's `source` (see `TabSpec::source`).
    pub(super) fn console_in_source(&self, lib: &Library, g: &CollectionGame, source: &str) -> bool {
        let (kind, value) = source.split_once(':').unwrap_or((source, ""));
        match kind {
            "installed" => g.installed,
            "not-installed" => !g.installed,
            "favourites" => g.favourite,
            "pc" => !self.game_extras.systems.contains_key(&g.key),
            "emulated" => self.game_extras.systems.contains_key(&g.key),
            "status" => status_is(lib, g, value),
            "category" => self.categories.contains(value, &g.runner, &g.app_name),
            "type" => {
                crate::fork::watch::sources::kind(g) == value && self.watch_catalog.visible(value, crate::fork::watch::sources::id(g))
            }
            "genre" => crate::fork::watch::sources::has_genre(g, value),
            "smart" => self.in_smart(value, g),
            "store" => g.source_label == value,
            "platform" => self.organize().platform(g) == value,
            _ => true,
        }
    }

    /// The games of `source`, in library order.
    pub(super) fn console_games(&self, source: &str) -> Vec<Arc<CollectionGame>> {
        self.cached_games(source).to_vec()
    }

    fn cached_games(&self, source: &str) -> Arc<Vec<Arc<CollectionGame>>> {
        let Some(lib) = self.lib.as_ref() else { return Arc::default() };
        let mut cache = self.console.sources.borrow_mut();
        cache.check(self, lib);
        if let Some(games) = cache.games.get(source) {
            return games.clone();
        }
        let games: Arc<Vec<Arc<CollectionGame>>> =
            Arc::new(lib.games.iter().filter(|g| self.console_in_source(lib, g, source)).cloned().collect());
        cache.games.insert(source.to_string(), games.clone());
        games
    }

    /// The theme's shelves over `source`, empty ones left out.
    pub(super) fn console_shelves(&self, source: &str) -> Vec<Shelf> {
        self.cached_shelves(source).to_vec()
    }

    fn cached_shelves(&self, source: &str) -> Arc<Vec<Shelf>> {
        let Some(lib) = self.lib.as_ref() else { return Arc::default() };
        let cached = {
            let mut cache = self.console.sources.borrow_mut();
            cache.check(self, lib);
            cache.shelves.get(source).cloned()
        };
        if let Some(shelves) = cached {
            return shelves;
        }
        let shelves = Arc::new(self.build_shelves(lib, source));
        self.console.sources.borrow_mut().shelves.insert(source.to_string(), shelves.clone());
        shelves
    }

    fn build_shelves(&self, lib: &Library, source: &str) -> Vec<Shelf> {
        // The person's own shelf is not a theme shelf: it is Continue watching,
        // the watchlist and the favourites, and its rails hide when empty.
        // Only the Watch space has one; a Games theme cannot reach it.
        if source == "shelf" {
            return if self.space == crate::collection::Space::Watch { self.personal_shelves(lib) } else { Vec::new() };
        }
        let theme = crate::console::look::theme();
        let games = self.cached_games(source);
        let games: Vec<&CollectionGame> = games.iter().map(|g| g.as_ref()).collect();
        let limit = if theme.shelf_limit == 0 { usize::MAX } else { theme.shelf_limit as usize };
        // `statuses` stands for one shelf per status of the library, in its order.
        let specs: Vec<ShelfSpec> = theme
            .shelves
            .iter()
            .filter(|s| s.on)
            .flat_map(|s| {
                if s.kind == "statuses" {
                    lib.statuses.iter().map(|st| ShelfSpec::new(&format!("status:{}", st.id), true)).collect()
                } else {
                    vec![s.clone()]
                }
            })
            .collect();
        let mut shelves: Vec<Shelf> = specs
            .iter()
            .filter_map(|spec| {
                let mut keys = keys(self.shelf_games(lib, &games, &spec.kind));
                let total = keys.len();
                keys.truncate(limit);
                (!keys.is_empty()).then(|| Shelf {
                    title: theme_rows::shelf_title(self.space, spec, |v| self.shelf_name(lib, v)).into(),
                    keys,
                    total,
                    kind: spec.kind.clone().into(),
                })
            })
            .collect();
        // The provider collections the person turned on are drawn on the first
        // page, after the theme's own rails and in the settings' order: the
        // catalogues they name are what the tabs browse, these are their picks.
        if source == "all" && self.space == crate::collection::Space::Watch {
            let marked = games.iter().filter_map(|game| Some((crate::fork::watch::sources::rail_marker(game)?, game.key.as_str())));
            shelves.extend(collection_shelves(&self.watch_collections, marked));
        }
        shelves
    }

    /// The Shelf tab's rails, in order: where the person stopped (one rail per
    /// provider that has progress), what they mean to watch, what they like. The
    /// provider collections are the first page's. A rail with nothing in it is
    /// not drawn.
    fn personal_shelves(&self, lib: &Library) -> Vec<Shelf> {
        let all: Vec<&CollectionGame> = lib.games.iter().map(|game| game.as_ref()).collect();
        let shelf = |kind: &str, title: String, games: Vec<&CollectionGame>| Shelf {
            title: SharedString::from(title),
            total: games.len(),
            kind: SharedString::from(kind),
            keys: games.iter().map(|game| SharedString::from(game.key.clone())).collect(),
        };
        let mut shelves = Vec::new();

        // Continue watching, one rail per provider: several providers (Stremio
        // today, others later) each continue their own titles, so they are not
        // merged into one list.
        let resumed = all
            .iter()
            .copied()
            .filter(|game| game.last_played.is_some())
            .map(|game| (game.runner.as_str(), game.source_label, game.last_played.as_deref().unwrap_or_default(), game));
        for (kind, title, games) in continue_groups(resumed) {
            shelves.push(shelf(&kind, title, by_last_played(games)));
        }

        // The two flags: the shelf's snapshots, resolved against this library. A
        // title the projection no longer holds is not drawn (it still is in the
        // file; opening those is the next slice).
        for (kind, title, items) in [
            ("shelf:watchlist", tr!("Watchlist"), &self.watch_personal.watchlist),
            ("shelf:favourites", tr!("Favourites"), &self.watch_personal.favourites),
        ] {
            let games: Vec<&CollectionGame> = items
                .iter()
                .filter_map(|item| {
                    all.iter().copied().find(|game| {
                        crate::fork::watch::sources::id(game) == item.id
                            && crate::fork::watch::sources::kind(game) == item.kind
                            && game.runner == item.provider
                    })
                })
                .collect();
            shelves.push(shelf(kind, title.to_string(), games));
        }

        shelves.retain(|shelf| !shelf.keys.is_empty());
        shelves
    }

    /// Drops every cached rail, so a toggle or a new progress record shows at
    /// once: the Shelf's (Continue watching per provider, watchlist, favourites)
    /// and the first page's (the provider collections Settings lists).
    pub(crate) fn forget_watch_shelves(&mut self) {
        self.console.sources.borrow_mut().shelves.clear();
    }

    fn shelf_games<'a>(&self, lib: &Library, games: &[&'a CollectionGame], kind: &str) -> Vec<&'a CollectionGame> {
        let (k, value) = kind.split_once(':').unwrap_or((kind, ""));
        let pick = |f: &dyn Fn(&CollectionGame) -> bool| games.iter().copied().filter(|g| f(g)).collect::<Vec<_>>();
        match k {
            "recent" => {
                let mut v = by_last_played(pick(&|g| g.last_played.is_some()));
                v.truncate(20);
                v
            }
            "favourites" => by_last_played(pick(&|g| g.favourite)),
            "installed" => by_last_played(pick(&|g| g.installed)),
            "added" => {
                let org = self.organize();
                let mut v: Vec<(&CollectionGame, &str)> = games.iter().filter_map(|g| Some((*g, org.added(g)?))).collect();
                v.sort_by(|a, b| b.1.cmp(a.1));
                v.into_iter().take(20).map(|(g, _)| g).collect()
            }
            "backlog-short" => {
                let mut v: Vec<(&CollectionGame, f64)> = games
                    .iter()
                    .filter(|g| status_is(lib, g, "plan-to-play") || (g.playtime_minutes == 0 && g.last_played.is_none()))
                    .filter_map(|g| Some((*g, self.wiki.hltb(&g.title, &g.game.title)?.main_story)))
                    .filter(|(_, h)| *h > 0. && *h <= 15.)
                    .collect();
                v.sort_by(|a, b| a.1.total_cmp(&b.1));
                v.into_iter().map(|(g, _)| g).collect()
            }
            "random" => {
                // A different pick each day, the same all day long.
                let day = today() as u64;
                let mut v: Vec<(u64, &CollectionGame)> = games
                    .iter()
                    .filter(|g| status_is(lib, g, "plan-to-play") || g.playtime_minutes == 0)
                    .map(|g| (daily_rank(&g.key, day), *g))
                    .collect();
                v.sort_by_key(|(h, _)| *h);
                v.into_iter().take(6).map(|(_, g)| g).collect()
            }
            "forgotten" => {
                let today = today();
                by_last_played(pick(&|g| g.installed && g.last_played.as_deref().and_then(day_number).is_none_or(|d| today - d > 90)))
            }
            "status" => by_last_played(pick(&|g| status_is(lib, g, value))),
            "store" => by_last_played(pick(&|g| g.source_label == value)),
            "platform" => {
                let org = self.organize();
                by_last_played(pick(&|g| org.platform(g) == value))
            }
            "genre" => by_last_played(pick(&|g| g.metadata.as_ref().is_some_and(|m| m.genres.iter().any(|x| x == value)))),
            "category" => by_last_played(pick(&|g| self.categories.contains(value, &g.runner, &g.app_name))),
            "smart" => by_last_played(pick(&|g| self.in_smart(value, g))),
            _ => Vec::new(),
        }
    }
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// The Shelf tab's collection rails: one per enabled collection, in the person's
/// order, holding the library's titles that carry its mark. What an enabled rail
/// nothing carries is left out, like every other empty rail.
///
/// `marked` is each title's `extra.watchCatalog` and its key: the membership was
/// decided when the library was refreshed, so this is a grouping, not a request.
/// The Continue watching rails, one per provider: `(kind, title, the titles)`.
/// Each item is `(provider id, its label, when the title was last watched, the
/// title)`. A single provider's rail is just "Continue watching"; with several,
/// each says which provider's progress it is, and the provider watched most
/// recently comes first.
fn continue_groups<'a, G>(items: impl Iterator<Item = (&'a str, &'a str, &'a str, G)>) -> Vec<(String, String, Vec<G>)> {
    use std::collections::BTreeMap;
    let mut by_provider: BTreeMap<&str, (&str, Vec<(&str, G)>)> = BTreeMap::new();
    for (provider, label, when, game) in items {
        by_provider.entry(provider).or_insert((label, Vec::new())).1.push((when, game));
    }
    let several = by_provider.len() > 1;
    let mut rails: Vec<(&str, &str, String, Vec<G>)> = by_provider
        .into_iter()
        .map(|(provider, (label, games))| {
            let newest = games.iter().map(|(when, _)| *when).max().unwrap_or_default();
            let title = if several {
                tr!("Continue watching · {provider}", provider = label).to_string()
            } else {
                tr!("Continue watching").to_string()
            };
            (provider, newest, title, games.into_iter().map(|(_, game)| game).collect())
        })
        .collect();
    rails.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    rails.into_iter().map(|(provider, _, title, games)| (format!("shelf:continue:{provider}"), title, games)).collect()
}

fn collection_shelves<'a>(
    enabled: &[(String, heronite_watch::collections::CollectionDescriptor)],
    marked: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<Shelf> {
    let marked: Vec<(&str, &str)> = marked.collect();
    enabled
        .iter()
        .filter_map(|(_, descriptor)| {
            let mark = heronite_watch::collections::mark(&descriptor.id);
            let keys: Vec<SharedString> =
                marked.iter().filter(|(marker, _)| **marker == mark).map(|(_, key)| SharedString::from(*key)).collect();
            (!keys.is_empty()).then(|| Shelf {
                title: SharedString::from(crate::fork::watch::rails::title(descriptor)),
                total: keys.len(),
                kind: SharedString::from(mark),
                keys,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;
    use heronite_watch::catalog::CatalogRequest;

    fn rail(id: &str) -> (String, heronite_watch::collections::CollectionDescriptor) {
        let kind = id.rsplit(':').next().unwrap_or("movie");
        (
            "tmdb".to_owned(),
            heronite_watch::collections::CollectionDescriptor {
                id: id.to_owned(),
                name: format!("{kind} · rail"),
                kind: kind.to_owned(),
                rule: CatalogRequest::first(kind),
            },
        )
    }

    #[test]
    fn the_collection_rails_follow_the_settings_order_and_hide_what_is_empty() {
        let enabled = vec![rail("tmdb:popular:movie"), rail("tmdb:top-rated:movie"), rail("tmdb:popular:series")];
        let popular = heronite_watch::collections::mark("tmdb:popular:movie");
        let top = heronite_watch::collections::mark("tmdb:top-rated:movie");
        let marked = [(top.as_str(), "stremio:tt2"), (popular.as_str(), "stremio:tt1"), ("movie:tmdb:discover:Popular", "stremio:tt3")];

        let shelves = collection_shelves(&enabled, marked.into_iter());
        let titles: Vec<(&str, Vec<&str>)> =
            shelves.iter().map(|shelf| (shelf.title.as_ref(), shelf.keys.iter().map(AsRef::as_ref).collect())).collect();
        assert_eq!(titles.len(), 2, "the rail nothing carries is not drawn: {titles:?}");
        assert_eq!(titles[0].1, ["stremio:tt1"], "the person's order decides which rail comes first");
        assert_eq!(titles[0].0, crate::fork::watch::rails::title(&enabled[0].1));
        assert_eq!(titles[1].1, ["stremio:tt2"]);
        assert!(shelves.iter().all(|shelf| shelf.total == shelf.keys.len()), "a rail is not cut by the theme's shelf limit");
        assert!(shelves.iter().all(|shelf| shelf.kind.starts_with("collection:")), "a rail says what it is");
    }

    #[test]
    fn continue_watching_is_one_rail_per_provider_newest_first() {
        let stremio = [("stremio", "Stremio", "2026-11-14", "s1"), ("stremio", "Stremio", "2026-11-10", "s2")];
        let netflix = [("netflix", "Netflix", "2026-11-15", "n1")];
        let items = stremio.iter().chain(netflix.iter()).map(|(provider, label, when, game)| (*provider, *label, *when, *game));
        let rails = continue_groups(items);
        assert_eq!(rails.len(), 2, "one rail per provider, never merged");
        assert_eq!(rails[0].0, "shelf:continue:netflix", "the provider watched most recently first");
        assert_eq!(rails[0].1, format!("{} · Netflix", tr!("Continue watching")), "each rail says whose progress it is");
        assert_eq!(rails[0].2, ["n1"]);
        assert_eq!(rails[1].2.len(), 2);

        let saja = continue_groups(stremio.iter().map(|(provider, label, when, game)| (*provider, *label, *when, *game)));
        assert_eq!(saja[0].1, tr!("Continue watching"), "a single provider needs no name");
        assert_eq!(saja[0].0, "shelf:continue:stremio");
    }

    #[test]
    fn a_catalogue_pages_mark_is_not_a_rail() {
        let enabled = vec![rail("tmdb:popular:movie")];
        let marked = [("movie:tmdb:discover:Popular", "stremio:tt3")];
        assert!(collection_shelves(&enabled, marked.into_iter()).is_empty(), "only a rail's own mark is membership");
    }
}
