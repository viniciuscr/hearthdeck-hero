//! Simkl as a **catalog** source: what airs over the next month, browsed
//! without an account.
//!
//! Simkl publishes that schedule as pre-built JSON on
//! `data.simkl.in/calendar/v2/tv.json` — the same per-day calendar that powers
//! `simkl.com/tv/airing/` — and **the file costs nothing**: it is exempt from
//! both of Simkl's limits (the daily allowance and the per-second rate), needs no
//! user token and never reaches Simkl's origin. The live alternatives do not fit
//! this app:
//!
//! - `/tv/airing` is the endpoint behind that page, and Simkl's own reference
//!   says not to poll it: uncached, and under AUTH V2 the person's token is
//!   required (500 requests a day on a free account, shared with the tracker's
//!   writes). High-traffic clients are told to read this calendar file instead.
//! - `/tv/genres/…`, `/movies/genres/…`, `/best` and `/search` are counted the
//!   same way, and the genre ones answer a thin row, so every card would cost
//!   another lookup — its reference warns they are "not for TV / console /
//!   10-foot apps".
//!
//! The calendar keeps the schedule apart from the shows (`calendar` lists every
//! airing, `metadata` describes each show once), and that metadata carries what
//! the airing page filters by: network, country, genres, premiere date and
//! ratings. Those filters are applied here, locally, over one download.
//!
//! Two things the file cannot do: it holds a ~month window, so this is "what is
//! on now" rather than every show a network ever made; and a show with no IMDb id
//! cannot be played here, so it is dropped.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Context, Result};
use serde::Deserialize;
use url::Url;

use heronite_watch_core::catalog::{
    CatalogCtx, CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor, WatchCatalogSource,
};
use heronite_watch_core::title::{Art, ProviderId, Title, TitleKey};

use crate::sync::{APP_NAME, APP_VERSION};

/// Where the airing calendars live: the rolling window Simkl regenerates every
/// six hours. No user token, no quota.
pub const CALENDAR: &str = "https://data.simkl.in/calendar/v2";
/// The media provider that can play what Simkl describes (Simkl plays nothing).
const PLAYS_WITH: &str = "stremio";
/// Simkl's image paths are composed through the proxy its own guide recommends.
const PROXY: &str = "https://wsrv.nl/";
/// The image host the proxy is asked to fetch from.
const IMAGES: &str = "https://simkl.in";
/// A poster is a **portrait**: Simkl's `_w` is a 600 × 338 landscape crop meant
/// for banners, so a cover takes the medium portrait instead. The metadata source
/// draws its card from the same two sizes.
pub(crate) const POSTER: &str = "_m.webp";
/// The full-HD fanart size, for the background a detail screen draws.
pub(crate) const FANART: &str = "_medium.webp";
/// How many of the schedule's shows make a page.
const PAGE: usize = 100;
/// How many votes a rating needs before this source may sort by it.
const RATING_VOTES: u32 = 50;
/// How many values a filter that counts airing shows offers. The calendar names
/// hundreds of networks; a row the panel cycles cannot hold them, so the busiest
/// are offered and the rest are reached by search.
const AIRING_VALUES: usize = 30;

const DESCRIPTOR: CatalogSourceDescriptor = CatalogSourceDescriptor { id: "simkl", name: "Simkl" };

/// The one catalog this source declares. It is also the id the screens know the
/// chip by.
const AIRING: &str = "airing";

/// The genres a general-audience app leaves out: the adult ones, and recorded
/// gameplay, which Simkl's own SDK hides by default. A record names them as words
/// (`Video Game Play`); the comparison folds `-`/`_` so a slug would match too.
const HIDDEN_GENRES: [&str; 3] = ["erotica", "hentai", "video game play"];

/// Simkl as a catalog source.
///
/// Stateless but for the calendar, kept per run: every page of the catalog asks
/// the same file, and this is what keeps that to one download (~2.5 MB).
pub struct SimklCatalog {
    airing: Mutex<Option<(String, Arc<Vec<AirShow>>)>>,
}

impl SimklCatalog {
    pub const fn new() -> Self {
        Self { airing: Mutex::new(None) }
    }
}

impl Default for SimklCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchCatalogSource for SimklCatalog {
    fn descriptor(&self) -> &CatalogSourceDescriptor {
        &DESCRIPTOR
    }

    /// One catalog, on the Series tab: what airs over the next month, filterable
    /// by the choices the airing page offers. A calendar that cannot be read
    /// costs its chip and nothing else.
    fn catalogs(&self, ctx: &CatalogCtx<'_>) -> Result<Vec<CatalogDescriptor>> {
        match self.airing_shows(ctx) {
            Ok(shows) if !shows.is_empty() => Ok(vec![airing(&shows)]),
            Ok(_) => Ok(Vec::new()),
            Err(error) => {
                log::warn!("Simkl airing calendar: {error:#}");
                Ok(Vec::new())
            }
        }
    }

    /// One page of the schedule: narrowed by network, country, genre and premiere
    /// year (the choices `simkl.com/tv/airing/` offers), ordered by when a show
    /// next airs, by rating or by name, and searched by name, network, country or
    /// genre.
    fn catalog(&self, ctx: &CatalogCtx<'_>, request: &CatalogRequest) -> Result<CatalogPage> {
        if request.catalog != AIRING || request.kind != "series" {
            // A catalog id from another build's settings, or the wrong tab: no
            // titles rather than someone else's list.
            return Ok(CatalogPage::default());
        }
        let shows = self.airing_shows(ctx)?;
        let query = request.search.as_deref().map(str::trim).filter(|query| !query.is_empty()).map(str::to_lowercase);
        let mut rows: Vec<&AirShow> = shows
            .iter()
            .filter(|show| airing_matches(show, &request.filters))
            .filter(|show| query.as_deref().is_none_or(|query| airing_found(show, query)))
            .collect();
        airing_order(&mut rows, request.sort);
        let start = (request.skip as usize).min(rows.len());
        let end = (start + PAGE).min(rows.len());
        let page = &rows[start..end];
        Ok(CatalogPage {
            items: page.iter().filter_map(|show| show.listed.as_title("series")).collect(),
            // A search is one page, like every other search here: the grid stops
            // at what it was given.
            more: query.is_none() && end < rows.len(),
            consumed: page.len() as u32,
        })
    }
}

impl SimklCatalog {
    /// The rolling TV calendar, read at most once per run.
    ///
    /// The lock is only ever held to read the cache or to replace it, never
    /// across the request: two tabs asking at once fetch twice, which is fine,
    /// and neither waits on the other's download.
    fn airing_shows(&self, ctx: &CatalogCtx<'_>) -> Result<Arc<Vec<AirShow>>> {
        let url = params(Url::parse(&format!("{CALENDAR}/tv.json"))?, ctx);
        let key = url.as_str();
        if let Some((cached, shows)) = lock(&self.airing).as_ref() {
            if cached == key {
                return Ok(Arc::clone(shows));
            }
        }
        let body = ctx.http.get(&url).context("Simkl airing calendar")?;
        // A calendar Simkl has nothing for answers `null`.
        let calendar: Option<Calendar> = serde_json::from_slice(&body).context("Simkl airing calendar")?;
        let shows = Arc::new(airing_rows(calendar.unwrap_or_default()));
        *lock(&self.airing) = Some((key.to_owned(), Arc::clone(&shows)));
        Ok(shows)
    }
}

/// The shows of a calendar as an airing list: one row per show, in the order the
/// schedule airs them, with the shows that have no date left at the end.
fn airing_rows(calendar: Calendar) -> Vec<AirShow> {
    let mut next: HashMap<u64, String> = HashMap::new();
    for entry in calendar.calendar.unwrap_or_default() {
        let (Some(id), Some(date)) = (entry.simkl_id, entry.date) else {
            continue;
        };
        // The schedule lists every airing; the row wants the nearest one.
        let known = next.entry(id).or_insert_with(|| date.clone());
        if date < *known {
            *known = date;
        }
    }
    let mut shows: Vec<AirShow> = calendar
        .metadata
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(key, listed)| {
            let id = key.trim().parse::<u64>().ok()?;
            Some(AirShow { airs: next.remove(&id), listed })
        })
        .collect();
    shows.sort_by(|a, b| match (&a.airs, &b.airs) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.listed.name().to_lowercase().cmp(&b.listed.name().to_lowercase()),
    });
    shows
}

/// What the airing catalog offers: the filters the airing page has, with the
/// values its own schedule carries.
fn airing(shows: &[AirShow]) -> CatalogDescriptor {
    CatalogDescriptor {
        id: AIRING.to_owned(),
        name: "Simkl Airing".to_owned(),
        kind: "series".to_owned(),
        // Popular is the file's own order here: what airs next.
        sorts: vec![CatalogSort::Popular, CatalogSort::Rating, CatalogSort::Newest, CatalogSort::Title],
        filters: vec![
            CatalogFilter {
                id: "network".to_owned(),
                label: "network".to_owned(),
                values: busiest(shows.iter().filter_map(|show| show.listed.network())),
                required: false,
            },
            CatalogFilter {
                id: "country".to_owned(),
                label: "country".to_owned(),
                values: busiest(shows.iter().filter_map(|show| show.listed.country())),
                required: false,
            },
            CatalogFilter { id: "genre".to_owned(), label: "genre".to_owned(), values: airing_genres(shows), required: false },
            CatalogFilter { id: "year".to_owned(), label: "year".to_owned(), values: airing_years(shows), required: false },
        ],
        search: true,
        paging: true,
    }
}

/// The most frequent values, busiest first and then alphabetical. The calendar
/// names hundreds of networks and a row the panel cycles cannot hold them, so the
/// busiest are offered and the rest are reached by search.
fn busiest<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() {
            *counts.entry(value).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked.into_iter().take(AIRING_VALUES).map(|(value, _)| value.to_owned()).collect()
}

/// Every genre the schedule's shows carry, A–Z, without the ones a
/// general-audience app leaves out.
fn airing_genres(shows: &[AirShow]) -> Vec<String> {
    let mut genres: Vec<String> =
        shows.iter().flat_map(|show| show.listed.genres()).filter(|genre| !hidden_genre(genre)).cloned().collect();
    genres.sort_by_key(|genre| genre.to_lowercase());
    genres.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    genres
}

/// The premiere years the schedule covers, newest first.
fn airing_years(shows: &[AirShow]) -> Vec<String> {
    let mut years: Vec<String> = shows.iter().filter_map(|show| show.listed.year()).collect();
    years.sort_by(|a, b| b.cmp(a));
    years.dedup();
    years.truncate(AIRING_VALUES);
    years
}

/// Whether a genre is one a general-audience app leaves out. A genre list names
/// it as a slug (`video-game-play`) and a record as words (`Video Game Play`), so
/// the separators are folded before the comparison.
fn hidden_genre(name: &str) -> bool {
    let normalised = name.trim().to_ascii_lowercase().replace(['-', '_'], " ");
    HIDDEN_GENRES.iter().any(|hidden| normalised == *hidden)
}

/// Whether a show is what the filters ask for: the same value, ignoring case, and
/// a genre the show carries rather than a single one.
fn airing_matches(show: &AirShow, filters: &[(String, String)]) -> bool {
    let same = |id: &str, value: Option<&str>| match chosen(filters, id) {
        None => true,
        Some(wanted) => value.is_some_and(|value| value.eq_ignore_ascii_case(wanted)),
    };
    same("network", show.listed.network())
        && same("country", show.listed.country())
        && chosen(filters, "genre").is_none_or(|wanted| show.listed.genres().iter().any(|genre| genre.eq_ignore_ascii_case(wanted)))
        && chosen(filters, "year").is_none_or(|wanted| show.listed.year().as_deref() == Some(wanted))
}

/// Whether a show answers `query`: its name, its network, its country or a genre.
/// The filter row offers only the busiest networks and countries, so this is how
/// the rest are reached.
fn airing_found(show: &AirShow, query: &str) -> bool {
    let listed = &show.listed;
    listed.name().to_lowercase().contains(query)
        || listed.network().is_some_and(|network| network.to_lowercase().contains(query))
        || listed.country().is_some_and(|country| country.eq_ignore_ascii_case(query))
        || listed.genres().iter().any(|genre| genre.to_lowercase().contains(query))
}

/// The airing list in the order the person asked for, with a name as the tie
/// breaker so two pages of one feed cannot interleave differently.
fn airing_order(rows: &mut [&AirShow], sort: CatalogSort) {
    let by_name = |a: &&AirShow, b: &&AirShow| a.listed.name().to_lowercase().cmp(&b.listed.name().to_lowercase());
    match sort {
        // The file's own order: what airs next.
        CatalogSort::Popular => {}
        CatalogSort::Rating => rows.sort_by(|a, b| b.listed.rating().total_cmp(&a.listed.rating()).then_with(|| by_name(a, b))),
        CatalogSort::Newest => rows.sort_by(|a, b| b.listed.year().cmp(&a.listed.year()).then_with(|| by_name(a, b))),
        CatalogSort::Title => rows.sort_by(by_name),
    }
}

/// The value chosen for a filter, when one is.
fn chosen<'a>(filters: &'a [(String, String)], id: &str) -> Option<&'a str> {
    filters.iter().find(|(filter, _)| filter == id).map(|(_, value)| value.as_str()).map(str::trim).filter(|value| !value.is_empty())
}

fn lock<T>(cell: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A CDN URL with the parameters Simkl asks every request to carry.
fn params(mut url: Url, ctx: &CatalogCtx<'_>) -> Url {
    if let Some(client_id) = client_id(ctx) {
        url.query_pairs_mut().append_pair("client_id", &client_id);
    }
    url.query_pairs_mut().extend_pairs([("app-name", APP_NAME), ("app-version", APP_VERSION)]);
    url
}

/// The app's client id: the one stored for the account, else the build's own.
/// A machine with neither still browses — the files answer anyway — it just does
/// not report the id Simkl's rules ask for.
fn client_id(ctx: &CatalogCtx<'_>) -> Option<String> {
    crate::stored_client_id(ctx.credentials).or_else(|| crate::built_in_client_id().map(str::to_owned))
}

/// One entry of a calendar's `metadata`: what Simkl knows about a show. The files
/// are permissive — fields come and go by type, and Simkl writes `null` where it
/// knows nothing — so every field is an `Option`: `#[serde(default)]` covers a
/// missing one, not a null.
#[derive(Debug, Default, Deserialize)]
struct Listed {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    poster: Option<String>,
    #[serde(default)]
    fanart: Option<String>,
    /// `2026-03-26T15:30:00Z` in the calendar.
    #[serde(default)]
    release_date: Option<String>,
    /// The length in Simkl's own words (`1h 45m`, `30m`).
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    genres: Option<Vec<String>>,
    #[serde(default)]
    ids: Option<Ids>,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    ratings: Option<Ratings>,
}

#[derive(Debug, Default, Deserialize)]
struct Ids {
    #[serde(default)]
    imdb: Option<String>,
}

/// The rating blocks a calendar record carries, one per catalogue.
#[derive(Debug, Default, Deserialize)]
struct Ratings {
    #[serde(default)]
    simkl: Option<Rating>,
    #[serde(default)]
    imdb: Option<Rating>,
}

#[derive(Debug, Default, Deserialize)]
struct Rating {
    #[serde(default)]
    rating: Option<f32>,
    #[serde(default)]
    votes: Option<u32>,
}

impl Listed {
    /// Its name, trimmed; a row without one is no title at all.
    fn name(&self) -> &str {
        self.title.as_deref().unwrap_or_default().trim()
    }

    /// The genres it carries, empty when the file names none.
    fn genres(&self) -> &[String] {
        self.genres.as_deref().unwrap_or_default()
    }

    fn network(&self) -> Option<&str> {
        self.network.as_deref().map(str::trim).filter(|network| !network.is_empty())
    }

    fn country(&self) -> Option<&str> {
        self.country.as_deref().map(str::trim).filter(|country| !country.is_empty())
    }

    /// The year it premiered, as the card prints it.
    fn year(&self) -> Option<String> {
        date_of(self.release_date.as_deref()).1
    }

    /// Its IMDb rating, else Simkl's, for ordering — and only when enough people
    /// voted for it to mean anything. A title with three votes can sit at ten and
    /// is not what a "top rated" row should open with.
    fn rating(&self) -> f32 {
        let of = |rating: Option<&Rating>| {
            let rating = rating?;
            let votes = rating.votes.unwrap_or_default();
            (votes >= RATING_VOTES).then_some(rating.rating).flatten()
        };
        let ratings = self.ratings.as_ref();
        of(ratings.and_then(|ratings| ratings.imdb.as_ref()))
            .or_else(|| of(ratings.and_then(|ratings| ratings.simkl.as_ref())))
            .unwrap_or_default()
    }

    /// The card this row is, keyed by the IMDb id Watch resolves everything by.
    fn as_title(&self, kind: &str) -> Option<Title> {
        let name = self.name();
        if name.is_empty() {
            return None;
        }
        let imdb = self.ids.as_ref()?.imdb.as_deref().map(str::trim).filter(|id| id.starts_with("tt"))?;
        let provider = ProviderId::declared(PLAYS_WITH);
        let (released, year) = date_of(self.release_date.as_deref());
        let mut title = Title::new(TitleKey::of_source(kind, imdb, &provider), name).plays(provider);
        title.release = year;
        title.released = released;
        title.runtime = text(self.runtime.as_deref());
        title.art = Art {
            poster: image(self.poster.as_deref(), "posters", POSTER),
            background: image(self.fanart.as_deref(), "fanart", FANART),
            logo: None,
        };
        title.genres = self.genres().iter().map(|genre| genre.trim().to_owned()).filter(|genre| !genre.is_empty()).collect();
        Some(title)
    }
}

/// One `calendar/v2/{type}.json`: the schedule, and one record per title it
/// schedules (a show airing twelve times in the window appears twelve times in
/// `calendar` and once in `metadata`).
#[derive(Debug, Default, Deserialize)]
struct Calendar {
    // `null` where Simkl has nothing, like every other field it writes: an
    // `Option` is what survives it.
    #[serde(default)]
    calendar: Option<Vec<Scheduled>>,
    #[serde(default)]
    metadata: Option<BTreeMap<String, Listed>>,
}

/// One airing: which show, and when.
#[derive(Debug, Default, Deserialize)]
struct Scheduled {
    #[serde(default)]
    simkl_id: Option<u64>,
    #[serde(default)]
    date: Option<String>,
}

/// One show of the airing calendar, with when it next airs.
struct AirShow {
    listed: Listed,
    airs: Option<String>,
}

/// A title's release day (`2026-03-26`) and year (`2026`), from either shape
/// Simkl writes dates in. Anything this build cannot read is no date at all.
fn date_of(value: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, None);
    };
    // `2026-03-26T…` (the calendar) or `03/26/2026` (a watched list).
    let (year, month, day) = match value.get(..4).filter(|year| year.chars().all(|c| c.is_ascii_digit())) {
        Some(year) => (Some(year), value.get(5..7), value.get(8..10)),
        None => {
            let mut parts = value.split('/');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(month), Some(day), Some(year)) if year.len() == 4 => (Some(year), Some(month), Some(day)),
                _ => (None, None, None),
            }
        }
    };
    let Some(year) = year else {
        return (None, None);
    };
    let (Some(month), Some(day)) = (number(month), number(day)) else {
        return (None, Some(year.to_owned()));
    };
    // A day this build cannot make sense of is no day; a year alone still is.
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return (None, Some(year.to_owned()));
    }
    (Some(format!("{year}-{month:02}-{day:02}")), Some(year.to_owned()))
}

/// A day or a month, when what Simkl wrote is two digits and nothing else.
fn number(part: Option<&str>) -> Option<u32> {
    let part = part.filter(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_digit()))?;
    part.parse().ok()
}

/// Simkl's image path as the proxy URL its guide recommends; a card without one
/// is drawn without it.
///
/// The path is the proxy's **data**, not part of our URL: it goes in the query
/// encoded, so a fragment carrying `&` or `#` cannot break the request.
fn image(path: Option<&str>, category: &str, size: &str) -> Option<String> {
    let path = path?.trim();
    if path.is_empty() {
        return None;
    }
    let mut url = Url::parse(PROXY).ok()?;
    url.query_pairs_mut().append_pair("url", &format!("{IMAGES}/{category}/{path}{size}")).append_pair("q", "90");
    Some(url.into())
}

fn text(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::{Credentials, Secret};

    /// The rolling calendar: four records, one without an IMDb id, one without a
    /// date, one airing twice, and one whose 9.9 rests on three votes.
    const CALENDAR: &str = r#"{
        "calendar": [
            {"simkl_id":1,"date":"2026-11-22T05:00:00Z","episode":{"season":3,"episode":4}},
            {"simkl_id":2,"date":"2026-11-20T05:00:00Z","episode":{"season":1,"episode":1}},
            {"simkl_id":2,"date":"2026-11-27T05:00:00Z","episode":{"season":1,"episode":2}}
        ],
        "metadata": {
            "1": {"title":"Alpha","poster":"74/74415673dcdc9cdd","fanart":"71/710408ec0a1bd207",
                  "release_date":"2019-05-01T00:00:00Z","runtime":"45m","genres":["Drama","Erotica"],
                  "network":"HBO","country":"us","ratings":{"imdb":{"rating":8.4,"votes":900}},"ids":{"imdb":"tt1"}},
            "2": {"title":"Beta","release_date":"2021-02-02T00:00:00Z","genres":["Comedy"],
                  "network":"Netflix","country":"gb","ratings":{"simkl":{"rating":7.1,"votes":120}},"ids":{"imdb":"tt2"}},
            "3": {"title":"No IMDb","network":"HBO","country":"us","genres":["Drama"],"ids":{"simkl_id":3}},
            "4": {"title":"Hyped","release_date":"2025-01-01T00:00:00Z","network":"HBO","country":"us",
                  "genres":["Drama","Video Game Play"],"ratings":{"imdb":{"rating":9.9,"votes":3}},"ids":{"imdb":"tt4"}}
        }
    }"#;

    /// Answers the calendar and counts every request.
    struct Fake {
        seen: Mutex<Vec<String>>,
        body: &'static str,
    }

    impl Fake {
        fn new() -> Self {
            Self { seen: Mutex::new(Vec::new()), body: CALENDAR }
        }

        fn answering(body: &'static str) -> Self {
            Self { seen: Mutex::new(Vec::new()), body }
        }

        fn asked(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            Ok(self.body.as_bytes().to_vec())
        }
        fn post_json(&self, _: &Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(b"{}".to_vec())
        }
    }

    struct NoCredentials;

    impl Credentials for NoCredentials {
        fn get(&self, _: &str) -> Result<Option<Secret>> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    fn with<F: FnOnce(&CatalogCtx<'_>)>(http: &Fake, body: F) {
        let credentials = NoCredentials;
        let ctx = CatalogCtx { http, credentials: &credentials, cache: None };
        body(&ctx);
    }

    fn request(filters: Vec<(String, String)>) -> CatalogRequest {
        CatalogRequest { catalog: AIRING.into(), kind: "series".into(), filters, ..CatalogRequest::first("series") }
    }

    #[test]
    fn the_catalog_is_the_airing_calendar_and_its_own_filters() {
        let http = Fake::new();
        with(&http, |ctx| {
            let catalogs = SimklCatalog::new().catalogs(ctx).unwrap();
            let declared: Vec<(&str, &str)> = catalogs.iter().map(|catalog| (catalog.kind.as_str(), catalog.id.as_str())).collect();
            assert_eq!(declared, [("series", "airing")], "one catalog, on the Series tab");

            let airing = &catalogs[0];
            assert_eq!(airing.sorts, [CatalogSort::Popular, CatalogSort::Rating, CatalogSort::Newest, CatalogSort::Title]);
            assert!(airing.search && airing.paging);
            let ids: Vec<&str> = airing.filters.iter().map(|filter| filter.id.as_str()).collect();
            assert_eq!(ids, ["network", "country", "genre", "year"], "the choices the airing page makes");
            assert_eq!(airing.filters[0].values, ["HBO", "Netflix"], "busiest first");
            assert_eq!(airing.filters[1].values, ["us", "gb"]);
            assert_eq!(airing.filters[2].values, ["Comedy", "Drama"], "adult and gameplay genres left out");
            assert_eq!(airing.filters[3].values, ["2025", "2021", "2019"], "premiere years, newest first");
            assert!(http.asked().iter().all(|url| url.starts_with("https://data.simkl.in/calendar/v2/tv.json?")), "{:?}", http.asked());
        });
    }

    #[test]
    fn the_calendar_is_one_download_however_many_pages_are_asked() {
        let http = Fake::new();
        with(&http, |ctx| {
            let source = SimklCatalog::new();
            source.catalogs(ctx).unwrap();
            source.catalog(ctx, &request(Vec::new())).unwrap();
            source.catalog(ctx, &CatalogRequest { skip: 100, ..request(Vec::new()) }).unwrap();
            assert_eq!(http.asked().len(), 1, "declaring and paging are the same file");
        });
    }

    #[test]
    fn the_schedule_pages_in_the_order_it_airs() {
        let http = Fake::new();
        with(&http, |ctx| {
            let page = SimklCatalog::new().catalog(ctx, &request(Vec::new())).unwrap();
            let names: Vec<&str> = page.items.iter().map(|title| title.name.as_str()).collect();
            assert_eq!(names, ["Beta", "Alpha", "Hyped"], "what airs next first, then the shows without a date by name");
            assert_eq!(page.consumed, 4, "the unplayable row still counts as moved on");
            assert!(!page.more);
            assert!(page.items[0].played_by(&ProviderId::declared(PLAYS_WITH)));
            assert_eq!(page.items[0].released.as_deref(), Some("2021-02-02"), "the premiere day, from the calendar's timestamp");
            assert_eq!(page.items[1].genres, ["Drama", "Erotica"], "the card keeps the genres the file names");

            let alpha = &page.items[1];
            assert_eq!(
                alpha.art.poster.as_deref(),
                Some("https://wsrv.nl/?url=https%3A%2F%2Fsimkl.in%2Fposters%2F74%2F74415673dcdc9cdd_m.webp&q=90"),
                "a portrait cover: `_w` is Simkl's landscape banner"
            );
            assert_eq!(
                alpha.art.background.as_deref(),
                Some("https://wsrv.nl/?url=https%3A%2F%2Fsimkl.in%2Ffanart%2F71%2F710408ec0a1bd207_medium.webp&q=90"),
                "the full-HD fanart, not the original-resolution JPG"
            );
        });
    }

    #[test]
    fn the_filters_are_the_schedule_own_fields() {
        let http = Fake::new();
        with(&http, |ctx| {
            let source = SimklCatalog::new();
            let page = |filters: Vec<(String, String)>| source.catalog(ctx, &request(filters)).unwrap();

            let hbo = page(vec![("network".into(), "HBO".into())]);
            let names: Vec<&str> = hbo.items.iter().map(|title| title.name.as_str()).collect();
            assert_eq!(names, ["Alpha", "Hyped"], "HBO's shows airing in the window, and the one without a date");
            assert_eq!(hbo.consumed, 3, "the HBO row without an IMDb id counts too");

            assert_eq!(page(vec![("country".into(), "gb".into())]).items[0].name, "Beta");
            assert_eq!(page(vec![("genre".into(), "comedy".into())]).items[0].name, "Beta", "a genre matches ignoring case");
            assert_eq!(page(vec![("year".into(), "2019".into())]).items[0].name, "Alpha");
            assert!(
                page(vec![("network".into(), "HBO".into()), ("genre".into(), "Comedy".into())]).items.is_empty(),
                "the filters narrow together"
            );
        });
    }

    #[test]
    fn the_list_can_be_ordered_and_searched() {
        let http = Fake::new();
        with(&http, |ctx| {
            let source = SimklCatalog::new();
            let with_sort = |sort: CatalogSort| source.catalog(ctx, &CatalogRequest { sort, ..request(Vec::new()) }).unwrap();

            let rated: Vec<String> = with_sort(CatalogSort::Rating).items.into_iter().map(|title| title.name).collect();
            assert_eq!(rated, ["Alpha", "Beta", "Hyped"], "IMDb's rating first, Simkl's when there is none, and a three-vote 9.9 last");

            let newest: Vec<String> = with_sort(CatalogSort::Newest).items.into_iter().map(|title| title.name).collect();
            assert_eq!(newest, ["Hyped", "Beta", "Alpha"], "what premiered last first");

            let search =
                |query: &str| source.catalog(ctx, &CatalogRequest { search: Some(query.to_owned()), ..request(Vec::new()) }).unwrap();
            let found = search("alp");
            assert_eq!(found.items.len(), 1);
            assert_eq!(found.items[0].name, "Alpha");
            assert!(!found.more, "a search is one page");
            assert_eq!(search("netflix").items[0].name, "Beta", "a network the filter row does not list is reachable by search");
            assert_eq!(search("video game").items[0].name, "Hyped", "and so is a genre");
        });
    }

    #[test]
    fn a_calendar_simkl_has_nothing_for_is_an_empty_catalog() {
        for body in ["null", "{}", r#"{"calendar":[],"metadata":null}"#] {
            let http = Fake::answering(body);
            with(&http, |ctx| {
                let source = SimklCatalog::new();
                assert!(source.catalogs(ctx).unwrap().is_empty(), "{body} names no shows, so there is no chip");
                let page = source.catalog(ctx, &request(Vec::new())).unwrap();
                assert!(page.items.is_empty() && !page.more);
            });
        }
    }

    #[test]
    fn another_catalogs_id_or_a_film_gets_nothing() {
        let http = Fake::new();
        with(&http, |ctx| {
            let source = SimklCatalog::new();
            let stale = CatalogRequest { catalog: "trending".into(), ..request(Vec::new()) };
            assert!(source.catalog(ctx, &stale).unwrap().items.is_empty(), "a chip this build no longer has");
            let film = CatalogRequest { kind: "movie".into(), ..request(Vec::new()) };
            assert!(source.catalog(ctx, &film).unwrap().items.is_empty(), "the airing calendar is series only");
        });
    }

    #[test]
    fn a_date_this_build_cannot_read_is_no_date_at_all() {
        assert_eq!(date_of(Some("03/31/1999")), (Some("1999-03-31".into()), Some("1999".into())));
        assert_eq!(date_of(Some("2026-03-26T15:30:00Z")), (Some("2026-03-26".into()), Some("2026".into())));
        assert_eq!(date_of(Some("idk")), (None, None));
        assert_eq!(date_of(Some("   ")), (None, None));
        assert_eq!(date_of(None), (None, None));
        assert_eq!(date_of(Some("1999")), (None, Some("1999".into())), "a year alone is still the year");
        assert_eq!(date_of(Some("13/40/1999")), (None, Some("1999".into())), "a day or month out of range is no day");
    }
}
