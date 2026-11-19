//! TMDB as a catalog source: discover for films and series.
//!
//! TMDB is a catalogue here and nothing else: no streams, no account of ours, no
//! playback. Its discoveries are keyed by IMDb id — the id the rest of Watch
//! uses — resolved per result (`/external_ids`), because discover answers TMDB
//! ids. A title with no IMDb id could not be played, so it is left out.
//!
//! The key is a TMDB **v3 API key**, stored under `tmdb` in the credential store
//! and sent as `api_key` (the transport carries no headers). It is never logged:
//! an error names the host only.
//!
//! TMDB's own "watch providers" (Netflix, …) are *availability*, not streams, and
//! belong to the metadata layer — this source never reports them.

pub mod auth;
pub mod meta;

pub use auth::{Session, Step};
pub use meta::TmdbMeta;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use chrono::Datelike;
use serde::Deserialize;
use url::Url;

use heronite_watch_core::catalog::{
    CatalogCtx, CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor, WatchCatalogSource,
};
use heronite_watch_core::provider::{Credentials, Secret};
use heronite_watch_core::title::{Art, ProviderId, Title, TitleKey};

/// TMDB's API root.
pub const API: &str = "https://api.themoviedb.org/3";
/// The v4 root: TMDB documents some endpoints only under its v4 reference, and
/// answers them under either root when the key allows it.
pub const API_V4: &str = "https://api.themoviedb.org/4";
/// Where TMDB keeps its images; a width follows.
pub const IMAGE: &str = "https://image.tmdb.org/t/p";

/// The media provider that can play what TMDB finds (TMDB itself plays nothing).
const PLAYS_WITH: &str = "stremio";
/// How many results a discover page holds.
const PAGE: u32 = 20;
/// The last page TMDB answers; past it the API errors instead of ending.
const MAX_PAGE: u32 = 500;
/// Years offered as a filter: this year and the 39 before it.
const YEARS: i32 = 40;
/// A rating order over the whole catalogue is noise without a floor.
const MIN_VOTES: &str = "200";

const DESCRIPTOR: CatalogSourceDescriptor = CatalogSourceDescriptor { id: "tmdb", name: "TMDB" };

/// TMDB, with its genre ids remembered for the process' life (they change
/// rarely, and a page should not pay for the list every time).
pub struct Tmdb {
    genres: Mutex<Option<Arc<Genres>>>,
}

impl Tmdb {
    pub const fn new() -> Self {
        Self { genres: Mutex::new(None) }
    }
}

impl Default for Tmdb {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchCatalogSource for Tmdb {
    fn descriptor(&self) -> &CatalogSourceDescriptor {
        &DESCRIPTOR
    }

    /// One `discover` catalog per kind: the sorts TMDB can order by on the
    /// server, its genres and a year.
    fn catalogs(&self, ctx: &CatalogCtx<'_>) -> Result<Vec<CatalogDescriptor>> {
        let key = api_key(ctx.credentials)?;
        let genres = self.genres(ctx, &key)?;
        Ok(KINDS
            .iter()
            .map(|kind| CatalogDescriptor {
                id: "discover".into(),
                // The chip is the catalogue's name, not the endpoint's: `discover`
                // is where the page comes from.
                name: "TMDB".into(),
                kind: (*kind).to_owned(),
                sorts: vec![CatalogSort::Popular, CatalogSort::Rating, CatalogSort::Newest, CatalogSort::Title],
                filters: vec![
                    CatalogFilter {
                        id: "genre".into(),
                        label: "genre".into(),
                        values: genres.of(tmdb_kind(kind).unwrap_or("movie")).iter().map(|(_, name)| name.clone()).collect(),
                        required: false,
                    },
                    CatalogFilter { id: "year".into(), label: "year".into(), values: years(), required: false },
                ],
                search: true,
                paging: true,
            })
            .collect())
    }

    fn catalog(&self, ctx: &CatalogCtx<'_>, request: &CatalogRequest) -> Result<CatalogPage> {
        let key = api_key(ctx.credentials)?;
        let Some(kind) = tmdb_kind(&request.kind) else {
            return Ok(CatalogPage::default());
        };
        // The account's own suggestions: v4 is the only place they exist.
        if request.catalog == "recommended" {
            return self.recommendations(ctx, request, &key, kind);
        }
        let page = request.skip / PAGE + 1;
        let search = request.search.as_deref().map(str::trim).filter(|search| !search.is_empty());
        let mut url = Url::parse(&format!("{API}/{}/{kind}", if search.is_some() { "search" } else { "discover" }))?;
        let mut query: Vec<(String, String)> =
            vec![("api_key".to_owned(), key.clone()), ("page".to_owned(), page.to_string()), ("language".to_owned(), "en-US".to_owned())];
        if let Some(search) = search {
            query.push(("query".to_owned(), search.to_owned()));
        } else {
            query.push(("sort_by".to_owned(), sort_of(request.sort, kind).to_owned()));
            if request.sort == CatalogSort::Rating {
                query.push(("vote_count.gte".to_owned(), MIN_VOTES.to_owned()));
            }
            if let Some(year) = chosen(&request.filters, "year") {
                let field = if kind == "movie" { "primary_release_year" } else { "first_air_date_year" };
                query.push((field.to_owned(), year.to_owned()));
            }
            if let Some(genre) = chosen(&request.filters, "genre") {
                let genres = self.genres(ctx, &key)?;
                if let Some((id, _)) = genres.of(kind).iter().find(|(_, name)| name == genre) {
                    query.push(("with_genres".to_owned(), id.to_string()));
                }
            }
        }
        url.query_pairs_mut().extend_pairs(query.iter().map(|(key, value)| (key.as_str(), value.as_str())));
        let body = ctx.http.get(&url)?;
        let found: Discover = serde_json::from_slice(&body).context("TMDB discover")?;
        self.page_of(ctx, &key, kind, found, page)
    }
}

impl Tmdb {
    /// A `discover`-shaped answer as a page of Watch items. TMDB ids mean nothing
    /// to our library, players or progress: an item without an IMDb id could not
    /// be played, so it is not shown. The ids are kept between runs, and the
    /// misses are fetched a few at a time.
    fn page_of(&self, ctx: &CatalogCtx<'_>, key: &str, kind: &str, found: Discover, page: u32) -> Result<CatalogPage> {
        let ids = self.imdb_ids(ctx, key, kind, &found.results);
        let mut items = Vec::new();
        for result in &found.results {
            if let Some(Some(imdb)) = ids.get(&result.id) {
                items.push(item(result, imdb, watch_kind(kind)));
            }
        }
        // TMDB stops at page 500: beyond it the answer is an error, not an empty
        // page, so the feed must end here.
        let more = page < found.total_pages.clamp(1, MAX_PAGE);
        let consumed = found.results.len() as u32;
        Ok(CatalogPage { items, more, consumed })
    }

    /// `/4/account/{object}/{kind}/recommendations`: what the person's own TMDB
    /// account suggests. The id in the path is the **object id** inside the read
    /// token (`sub`), not the numeric v3 id — `/4/account/{numeric-id}/…` is a 404,
    /// and `?session_id=` is not wanted here at all: the token is the user.
    fn recommendations(&self, ctx: &CatalogCtx<'_>, request: &CatalogRequest, key: &str, kind: &str) -> Result<CatalogPage> {
        let secret = ctx.credentials.get("tmdb")?.context("TMDB needs its read access token")?;
        let token = auth::access_token_of(&secret)
            .context("TMDB's v4 read access token is not set: it is what recommendations need (Settings → Watch → Sources).")?;
        let object = auth::account_object_id(&token).context("TMDB's read token names no account")?;
        let page = request.skip / PAGE + 1;
        let mut url = Url::parse(&format!("{API_V4}/account/{object}/{kind}/recommendations"))?;
        let number = page.to_string();
        url.query_pairs_mut().extend_pairs([("page", number.as_str()), ("language", "en-US")]);
        // The path names the account; the token travels in a header and must not
        // reach an error message.
        let body = ctx.http.get_bearer(&url, &token).with_context(|| format!("TMDB recommendations for account {object}"))?;
        let found: Discover = serde_json::from_slice(&body).context("TMDB recommendations")?;
        self.page_of(ctx, key, kind, found, page)
    }

    /// The IMDb id of every title on a page: the cache answers most of them, and
    /// the misses go out four at a time instead of twenty in a row.
    fn imdb_ids(&self, ctx: &CatalogCtx<'_>, key: &str, kind: &str, results: &[Found]) -> HashMap<u64, Option<String>> {
        let mut known: HashMap<u64, Option<String>> = HashMap::new();
        let mut missing: Vec<u64> = Vec::new();
        for result in results {
            match ctx.cache.and_then(|cache| cache.imdb_id(DESCRIPTOR.id, kind, result.id)) {
                Some(imdb) => {
                    known.insert(result.id, imdb);
                }
                None => missing.push(result.id),
            }
        }
        // One request per title is the only way TMDB gives an IMDb id (there is no
        // batch endpoint), and the page itself is the bound: twenty at most here,
        // so they go out together and a cold page costs one round trip. The cache
        // above is what keeps this from happening twice for the same title.
        let answers: Vec<(u64, Option<String>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = missing.iter().map(|id| scope.spawn(move || fetched(ctx, key, kind, *id))).collect();
            handles.into_iter().filter_map(|handle| handle.join().ok()).collect()
        });
        known.extend(answers);
        known
    }

    /// TMDB's genre ids for both kinds, fetched once and remembered.
    fn genres(&self, ctx: &CatalogCtx<'_>, key: &str) -> Result<Arc<Genres>> {
        if let Some(cached) = self.cached() {
            return Ok(cached);
        }
        let mut genres = Genres::default();
        for kind in ["movie", "tv"] {
            let mut url = Url::parse(&format!("{API}/genre/{kind}/list"))?;
            url.query_pairs_mut().extend_pairs([("api_key", key), ("language", "en-US")]);
            let body = ctx.http.get(&url)?;
            let list: GenreList = serde_json::from_slice(&body).context("TMDB genres")?;
            let names: Vec<(u32, String)> = list.genres.into_iter().map(|genre| (genre.id, genre.name)).collect();
            match kind {
                "movie" => genres.movie = names,
                _ => genres.tv = names,
            }
        }
        let genres = Arc::new(genres);
        self.remember(&genres);
        Ok(genres)
    }

    /// The genre lists already fetched, cloned as a pointer: a page must not
    /// copy two vectors to read one name.
    fn cached(&self) -> Option<Arc<Genres>> {
        self.lock().as_ref().cloned()
    }

    fn remember(&self, genres: &Arc<Genres>) {
        *self.lock() = Some(genres.clone());
    }

    /// The lock, recovering from a panic elsewhere: a poisoned mutex must not
    /// silence the genre list for the rest of the run.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Arc<Genres>>> {
        self.genres.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The kinds Watch browses, in chip order.
const KINDS: [&str; 2] = ["movie", "series"];

/// TMDB's kind for one of ours (`series` is `tv` there).
fn tmdb_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "movie" => Some("movie"),
        "series" => Some("tv"),
        _ => None,
    }
}

/// Watch's kind for a TMDB kind.
fn watch_kind(kind: &str) -> &'static str {
    if kind == "tv" {
        "series"
    } else {
        "movie"
    }
}

/// TMDB's `sort_by` for a sort and a kind.
fn sort_of(sort: CatalogSort, kind: &str) -> &'static str {
    let series = kind == "tv";
    match sort {
        CatalogSort::Popular => "popularity.desc",
        CatalogSort::Rating => "vote_average.desc",
        CatalogSort::Newest if series => "first_air_date.desc",
        CatalogSort::Newest => "primary_release_date.desc",
        CatalogSort::Title if series => "original_name.asc",
        CatalogSort::Title => "original_title.asc",
    }
}

/// The value chosen for a filter, when one is.
fn chosen<'a>(filters: &'a [(String, String)], id: &str) -> Option<&'a str> {
    filters.iter().find(|(filter, _)| filter == id).map(|(_, value)| value.as_str()).filter(|value| !value.is_empty())
}

/// The years the year filter offers, newest first.
fn years() -> Vec<String> {
    let this = chrono::Utc::now().year();
    (0..YEARS).map(|back| (this - back).to_string()).collect()
}

/// The TMDB API key this build carries (`HEARTHDECK_TMDB_API_KEY` when it was
/// compiled). TMDB needs a key for **every** call, the login included — there is
/// no anonymous way in — so the application is what holds one, and the person
/// only ever approves a session.
pub fn built_in_key() -> Option<&'static str> {
    option_env!("HEARTHDECK_TMDB_API_KEY").map(str::trim).filter(|key| !key.is_empty())
}

/// The file holding the v4 **read access token** (the long one on TMDB's API
/// page), which is what v4 endpoints take as a `Bearer` header.
pub const TOKEN_FILE: &str = "tmdb-access-token";

/// The v4 read token in `dir/` ([`TOKEN_FILE`]), when the file is there.
pub fn token_file(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;
    let token = text.lines().next().unwrap_or_default().trim().trim_matches(|c: char| c == '"' || c == '\'').to_owned();
    (!token.is_empty()).then_some(token)
}

/// The file a maintainer build reads when it was compiled without the variable:
/// `tmdb-api-key` in the config folder, the key on the first line. It is the
/// same application key, kept out of the source tree.
pub const KEY_FILE: &str = "tmdb-api-key";

/// The key in `dir/` ([`KEY_FILE`]), when the file is there and holds something.
pub fn key_file(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(KEY_FILE)).ok()?;
    let line = text.lines().next().unwrap_or_default().trim();
    // What a person pastes carries whatever the source had around it.
    let key = line.trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace()).to_owned();
    if key.is_empty() {
        return None;
    }
    // A v3 key is 32 hex characters; a v4 read token is a long JWT and TMDB
    // refuses it with 401, which is worth a log line before that happens.
    if key.len() != 32 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        log::warn!("the TMDB key in {KEY_FILE} is not 32 hexadecimal characters; TMDB will probably refuse it");
    }
    Some(key)
}

/// The API key to use: the build's own, else the `[]KEY_FILE` a self-built copy
/// may carry, else one kept in the credential store (a build made without the
/// variable still works when a key was stored).
pub(crate) fn api_key(credentials: &dyn Credentials) -> Result<String> {
    // The key this machine carries wins: the bootstrap writes one for the console
    // box, and someone running their own key keeps it. The build's own key is the
    // default a release ships with.
    let stored = credentials
        .get("tmdb")
        .ok()
        .flatten()
        .map(|secret| match secret.decode::<Key>() {
            Ok(Key { api_key }) => api_key,
            // A bare string is accepted too: it is what the key file holds.
            Err(_) => secret.value().as_str().unwrap_or_default().to_owned(),
        })
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    if let Some(key) = stored {
        return Ok(key);
    }
    if let Some(key) = built_in_key() {
        return Ok(key.to_owned());
    }
    bail!("TMDB needs this app's key: the bootstrap sets it, or set HEARTHDECK_TMDB_API_KEY when building.");
}

/// The IMDb id TMDB holds for one of its titles, when it holds one, kept in the
/// caller's cache so a page does not pay for it twice.
fn imdb_id(ctx: &CatalogCtx<'_>, key: &str, kind: &str, id: u64) -> Result<Option<String>> {
    if let Some(cache) = ctx.cache {
        if let Some(known) = cache.imdb_id(DESCRIPTOR.id, kind, id) {
            return Ok(known);
        }
    }
    let mut url = Url::parse(&format!("{API}/{kind}/{id}/external_ids"))?;
    url.query_pairs_mut().extend_pairs([("api_key", key)]);
    let body = ctx.http.get(&url)?;
    let ids: ExternalIds = serde_json::from_slice(&body).context("TMDB external ids")?;
    let imdb = ids.imdb_id.map(|imdb| imdb.trim().to_owned()).filter(|imdb| !imdb.is_empty());
    if let Some(cache) = ctx.cache {
        cache.keep_imdb_id(DESCRIPTOR.id, kind, id, imdb.as_deref());
    }
    Ok(imdb)
}

/// One title's IMDb id, fetched: a failure costs that card, not the page.
fn fetched(ctx: &CatalogCtx<'_>, key: &str, kind: &str, id: u64) -> (u64, Option<String>) {
    match imdb_id(ctx, key, kind, id) {
        Ok(imdb) => (id, imdb),
        Err(error) => {
            log::warn!("TMDB external ids for {kind} {id}: {error:#}");
            (id, None)
        }
    }
}

/// One discover result as a title, keyed by its IMDb id.
fn item(found: &Found, imdb: &str, kind: &str) -> Title {
    let date = found.release_date.as_deref().or(found.first_air_date.as_deref()).unwrap_or_default();
    let year = date.get(..4).filter(|year| year.chars().all(|c| c.is_ascii_digit())).map(str::to_owned);
    let image = |path: &Option<String>, width: &str| path.as_ref().map(|path| format!("{IMAGE}/{width}{path}"));
    let name = found.title.clone().or(found.name.clone()).unwrap_or_default();
    let provider = ProviderId::declared(PLAYS_WITH);
    let mut title = Title::new(TitleKey::of_source(kind, imdb, &provider), &name).plays(provider);
    title.release = year;
    title.art = Art { poster: image(&found.poster_path, "w500"), background: image(&found.backdrop_path, "w1280"), logo: None };
    title.description = found.overview.clone();
    title
}

/// The key's shape in the credential store.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Key {
    api_key: String,
}

#[derive(Clone, Debug, Default)]
struct Genres {
    movie: Vec<(u32, String)>,
    tv: Vec<(u32, String)>,
}

impl Genres {
    fn of(&self, kind: &str) -> &[(u32, String)] {
        if kind == "tv" {
            &self.tv
        } else {
            &self.movie
        }
    }
}

#[derive(Deserialize)]
struct Discover {
    #[serde(default)]
    results: Vec<Found>,
    #[serde(default, rename = "total_pages")]
    total_pages: u32,
}

#[derive(Deserialize)]
struct Found {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "release_date")]
    release_date: Option<String>,
    #[serde(default, rename = "first_air_date")]
    first_air_date: Option<String>,
    #[serde(default, rename = "poster_path")]
    poster_path: Option<String>,
    #[serde(default, rename = "backdrop_path")]
    backdrop_path: Option<String>,
    #[serde(default)]
    overview: Option<String>,
}

#[derive(Deserialize)]
struct GenreList {
    #[serde(default)]
    genres: Vec<Genre>,
}

#[derive(Deserialize)]
struct Genre {
    id: u32,
    name: String,
}

#[derive(Deserialize)]
struct ExternalIds {
    #[serde(default, rename = "imdb_id")]
    imdb_id: Option<String>,
}

/// Kept so a caller can name the secret it must write.
pub fn secret(api_key: &str) -> Secret {
    Secret::new(serde_json::json!({ "apiKey": api_key }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::catalog::CatalogCache;
    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::Credentials;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use url::Url;

    /// Answers by path, and remembers what was asked.
    struct Fake {
        answers: HashMap<&'static str, &'static str>,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            let mut answers = HashMap::new();
            answers.insert("genre/movie/list", r#"{"genres":[{"id":28,"name":"Action"},{"id":878,"name":"Science Fiction"}]}"#);
            answers.insert("genre/tv/list", r#"{"genres":[{"id":18,"name":"Drama"}]}"#);
            answers.insert(
                "discover/movie",
                r#"{"page":1,"total_pages":3,"results":[
                    {"id":603,"title":"The Matrix","release_date":"1999-03-31","poster_path":"/matrix.jpg","backdrop_path":"/matrix-bg.jpg","overview":"A hacker learns the truth."},
                    {"id":604,"title":"No IMDb id","release_date":"2003-05-15"}
                ]}"#,
            );
            answers.insert(
                "search/movie",
                r#"{"page":1,"total_pages":1,"results":[{"id":603,"title":"The Matrix","release_date":"1999-03-31"}]}"#,
            );
            answers.insert("movie/603/external_ids", r#"{"id":603,"imdb_id":"tt0133093"}"#);
            answers.insert("movie/604/external_ids", r#"{"id":604,"imdb_id":null}"#);
            answers.insert("movie/605/external_ids", r#"{"id":605,"imdb_id":"tt0068646"}"#);
            // `Fake` keys by the path without the `/3` root.
            answers.insert("account", r#"{"id":42,"username":"cinephile"}"#);
            answers.insert(
                "account/42/movie/recommendations",
                r#"{"page":1,"total_pages":1,"results":[{"id":605,"title":"The Godfather","release_date":"1972-03-14"}]}"#,
            );
            Self { answers, seen: Mutex::new(Vec::new()) }
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            let path = url.path().trim_start_matches("/3/").to_owned();
            let body = self.answers.get(path.as_str()).copied().unwrap_or("{}");
            Ok(body.as_bytes().to_vec())
        }
        fn post_json(&self, _: &Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(b"{}".to_vec())
        }
    }

    struct Keyed(Option<&'static str>);

    impl Credentials for Keyed {
        fn get(&self, provider: &str) -> Result<Option<Secret>> {
            Ok(self.0.filter(|_| provider == "tmdb").map(secret))
        }
        fn set(&self, _: &str, _: &Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    fn ctx_with<'a>(http: &'a Fake, credentials: &'a Keyed) -> CatalogCtx<'a> {
        CatalogCtx { http, credentials, cache: None }
    }

    #[test]
    fn one_discover_catalog_a_kind_with_the_sorts_and_filters_tmdb_has() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let catalogs = Tmdb::new().catalogs(&ctx_with(&http, &credentials)).unwrap();
        assert_eq!(catalogs.len(), 2);
        let film = &catalogs[0];
        assert_eq!((film.kind.as_str(), film.id.as_str()), ("movie", "discover"));
        assert_eq!(film.sorts, [CatalogSort::Popular, CatalogSort::Rating, CatalogSort::Newest, CatalogSort::Title]);
        assert!(film.search && film.paging);
        let genre = film.filters.iter().find(|filter| filter.id == "genre").unwrap();
        assert_eq!(genre.values, ["Action", "Science Fiction"], "TMDB's own genres");
        let year = film.filters.iter().find(|filter| filter.id == "year").unwrap();
        assert_eq!(year.values.len(), YEARS as usize);
        assert_eq!(catalogs[1].kind, "series", "Watch's vocabulary, not TMDB's");
        let tv_genre = catalogs[1].filters.iter().find(|filter| filter.id == "genre").unwrap();
        assert_eq!(tv_genre.values, ["Drama"], "series have their own genre list");
    }

    #[test]
    fn a_page_is_keyed_by_imdb_id_and_drops_what_has_none() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let page = Tmdb::new().catalog(&ctx_with(&http, &credentials), &CatalogRequest::first("movie")).unwrap();
        assert_eq!(page.items.len(), 1, "the title without an IMDb id cannot be played, so it is left out");
        let matrix = &page.items[0];
        assert_eq!(matrix.key.id.as_str(), "tt0133093");
        assert_eq!(matrix.name, "The Matrix");
        assert_eq!(matrix.release.as_deref(), Some("1999"));
        assert_eq!(matrix.key.kind, heronite_watch_core::title::TitleKind::Movie);
        assert!(matrix.played_by(&ProviderId::declared(PLAYS_WITH)), "TMDB describes; Stremio plays");
        assert_eq!(matrix.art.poster.as_deref(), Some("https://image.tmdb.org/t/p/w500/matrix.jpg"));
        assert_eq!(matrix.art.background.as_deref(), Some("https://image.tmdb.org/t/p/w1280/matrix-bg.jpg"));
        assert!(page.more, "page 1 of 3 has more");
    }

    #[test]
    fn the_order_the_genre_and_the_year_go_into_the_query() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let mut request = CatalogRequest::first("movie");
        request.sort = CatalogSort::Rating;
        request.filters = vec![("genre".into(), "Science Fiction".into()), ("year".into(), "1999".into())];
        request.skip = 40;
        Tmdb::new().catalog(&ctx_with(&http, &credentials), &request).unwrap();
        let asked = http.seen.lock().unwrap().clone();
        let discover = asked.iter().find(|url| url.contains("/discover/movie")).expect("a discover request");
        assert!(discover.contains("sort_by=vote_average.desc"), "{discover}");
        assert!(discover.contains("vote_count.gte=200"), "a rating order needs a floor: {discover}");
        assert!(discover.contains("with_genres=878"), "the genre name became TMDB's id: {discover}");
        assert!(discover.contains("primary_release_year=1999"), "{discover}");
        assert!(discover.contains("page=3"), "skip 40 is page 3 of twenty: {discover}");
    }

    #[test]
    fn a_search_asks_search_and_a_series_years_use_the_air_date() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let mut request = CatalogRequest::first("series");
        request.search = Some("matrix".into());
        Tmdb::new().catalog(&ctx_with(&http, &credentials), &request).unwrap();
        let asked = http.seen.lock().unwrap().clone();
        assert!(asked.iter().any(|url| url.contains("/search/tv") && url.contains("query=matrix")), "{asked:?}");

        let mut by_year = CatalogRequest::first("series");
        by_year.filters = vec![("year".into(), "2008".into())];
        Tmdb::new().catalog(&ctx_with(&http, &credentials), &by_year).unwrap();
        let asked = http.seen.lock().unwrap().clone();
        assert!(asked.iter().any(|url| url.contains("discover/tv") && url.contains("first_air_date_year=2008")), "{asked:?}");
    }

    /// A cache in memory, so a test can see what was asked and what was kept.
    #[derive(Default)]
    struct Cache {
        ids: Mutex<HashMap<(String, String, u64), Option<String>>>,
    }

    impl CatalogCache for Cache {
        fn imdb_id(&self, source: &str, kind: &str, id: u64) -> Option<Option<String>> {
            self.ids.lock().unwrap().get(&(source.to_owned(), kind.to_owned(), id)).cloned()
        }
        fn keep_imdb_id(&self, source: &str, kind: &str, id: u64, imdb: Option<&str>) {
            self.ids.lock().unwrap().insert((source.to_owned(), kind.to_owned(), id), imdb.map(str::to_owned));
        }
    }

    #[test]
    fn a_page_pays_for_its_imdb_ids_once() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let cache = Cache::default();
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: Some(&cache) };
        let tmdb = Tmdb::new();

        let first = tmdb.catalog(&ctx, &CatalogRequest::first("movie")).unwrap();
        assert_eq!(first.items.len(), 1);
        let asked = |seen: &Mutex<Vec<String>>| seen.lock().unwrap().iter().filter(|url| url.contains("external_ids")).count();
        assert_eq!(asked(&http.seen), 2, "the two titles of the page, once each");
        assert_eq!(cache.imdb_id("tmdb", "movie", 603), Some(Some("tt0133093".to_owned())));
        assert_eq!(cache.imdb_id("tmdb", "movie", 604), Some(None), "and the one with no id is remembered as such");

        let second = tmdb.catalog(&ctx, &CatalogRequest::first("movie")).unwrap();
        assert_eq!(second.items.len(), 1, "the same page, from the cache");
        assert_eq!(asked(&http.seen), 2, "no id asked twice");
    }

    #[test]
    fn the_catalogue_is_named_after_the_site() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let catalogs = Tmdb::new().catalogs(&ctx_with(&http, &credentials)).unwrap();
        assert_eq!(catalogs[0].name, "TMDB", "`discover` is the endpoint, not the catalogue");
    }

    #[test]
    fn without_a_key_tmdb_says_where_to_put_it() {
        let http = Fake::new();
        let credentials = Keyed(None);
        let error = Tmdb::new().catalogs(&ctx_with(&http, &credentials)).unwrap_err();
        assert!(format!("{error:#}").contains("HEARTHDECK_TMDB_API_KEY"), "the build holds the key: {error:#}");
    }
}

/// TMDB's collection rails: two queries per kind, answered by its own catalogue.
#[derive(Clone, Copy, Debug, Default)]
pub struct TmdbCollections;

const COLLECTIONS: heronite_watch_core::collections::CollectionSourceDescriptor =
    heronite_watch_core::collections::CollectionSourceDescriptor { id: "tmdb", name: "TMDB" };

impl heronite_watch_core::collections::WatchCollectionSource for TmdbCollections {
    fn descriptor(&self) -> &heronite_watch_core::collections::CollectionSourceDescriptor {
        &COLLECTIONS
    }

    fn collections(
        &self,
        _ctx: &heronite_watch_core::catalog::CatalogCtx<'_>,
    ) -> Result<Vec<heronite_watch_core::collections::CollectionDescriptor>> {
        use heronite_watch_core::catalog::{CatalogRequest, CatalogSort};
        use heronite_watch_core::collections::CollectionDescriptor;
        let mut rails = Vec::new();
        // The account's own suggestions need a session *and* the v4 read token:
        // without either, a rail nobody can answer would be a dead switch.
        // Declared unconditionally: the rail is a switch the person turns on, and
        // whether it can answer (the v4 read token) is the resolver's business —
        // a switch that disappears is worse than one that explains itself.
        for (kind, label) in [("movie", "Films"), ("series", "Series")] {
            rails.push(CollectionDescriptor {
                id: format!("tmdb:recommended:{kind}"),
                name: format!("{label} · Recommended for you"),
                kind: kind.to_owned(),
                rule: CatalogRequest { catalog: "recommended".to_owned(), ..CatalogRequest::first(kind) },
            });
        }
        for (kind, label) in [("movie", "Films"), ("series", "Series")] {
            for (id, name, sort) in [("top-rated", "Top rated", CatalogSort::Rating), ("popular", "Popular", CatalogSort::Popular)] {
                rails.push(CollectionDescriptor {
                    id: format!("tmdb:{id}:{kind}"),
                    name: format!("{label} · {name}"),
                    kind: kind.to_owned(),
                    rule: CatalogRequest { sort, ..CatalogRequest::first(kind) },
                });
            }
        }
        Ok(rails)
    }
}

#[cfg(test)]
mod collection_tests {
    use super::*;
    use heronite_watch_core::catalog::{CatalogCtx, CatalogSort};
    use heronite_watch_core::collections::WatchCollectionSource;
    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::{Credentials, Secret};

    struct Nothing;

    impl Transport for Nothing {
        fn get(&self, _: &url::Url) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn post_json(&self, _: &url::Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    struct NoKey;

    impl Credentials for NoKey {
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

    #[test]
    fn tmdb_declares_two_rails_a_kind_without_needing_a_key() {
        let (http, credentials) = (Nothing, NoKey);
        let ctx = CatalogCtx { http: &http, credentials: &credentials, cache: None };
        let rails = TmdbCollections.collections(&ctx).unwrap();
        let ids: Vec<&str> = rails.iter().map(|rail| rail.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                // Declared without a key or a token: the switches are always there,
                // and the resolver says what it needs.
                "tmdb:recommended:movie",
                "tmdb:recommended:series",
                "tmdb:top-rated:movie",
                "tmdb:popular:movie",
                "tmdb:top-rated:series",
                "tmdb:popular:series",
            ]
        );
        assert_eq!(rails[2].rule.sort, CatalogSort::Rating);
        assert_eq!(rails[3].rule.sort, CatalogSort::Popular);
        assert_eq!(rails[2].kind, "movie");
        assert_eq!(rails[4].kind, "series");
        assert!(rails[2].name.contains("Top rated"));
    }
}
