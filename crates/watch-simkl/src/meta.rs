//! Simkl as a **metadata** source: what a title is, as its catalog knows it.
//!
//! Simkl is not a catalog source here — it never fills a tab — and not a media
//! provider: this asks one title, by the IMDb id the rest of Watch resolves
//! titles by. Catalog lookups are the one thing Simkl exempts from the user
//! token, so this works even signed out: it needs only the app's client id, the
//! same one the sign-in uses.
//!
//! One title costs three requests at most: `/search/id` names Simkl's own id and
//! which of its catalogs the title lives in (`movie`, `tv`, `anime`), the record
//! endpoint answers what the card shows, and a series adds its episode list for
//! the picker. A title Simkl does not know is "no answer", not a fault — another
//! source fills the card.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use url::Url;

use heronite_watch_core::metadata::{Episode, Fact, MetaCtx, MetaDescriptor, TitleDetails, TitleRef, Trailer, WatchMetadataSource};

use crate::auth::client_id_of;
use crate::sync::{API, APP_NAME, APP_VERSION};

const DESCRIPTOR: MetaDescriptor = MetaDescriptor { id: "simkl", name: "Simkl", applies_to: &[] };

/// Simkl's image paths are composed through the proxy its own guide recommends.
const IMAGE: &str = "https://wsrv.nl/?url=https://simkl.in";

/// Simkl as a metadata source. Stateless: the details cache keeps what it answers.
#[derive(Clone, Copy, Debug, Default)]
pub struct SimklMeta;

impl WatchMetadataSource for SimklMeta {
    fn descriptor(&self) -> &MetaDescriptor {
        &DESCRIPTOR
    }

    fn details(&self, ctx: &MetaCtx<'_>, title: &TitleRef) -> Result<Option<TitleDetails>> {
        let client_id = client_id(ctx)?;
        let imdb = title.id.trim();
        if !imdb.starts_with("tt") {
            // Watch resolves titles by IMDb id; Simkl's search takes one of
            // those (or a TMDB/Simkl id this caller does not have).
            return Ok(None);
        }
        let Some(found) = resolve(ctx, &client_id, imdb)? else {
            // Simkl does not know the title: no answer, not a fault.
            return Ok(None);
        };
        match found.kind.as_str() {
            "movie" => movie(ctx, &client_id, found, title),
            _ => series(ctx, &client_id, found, title),
        }
    }
}

/// The client id this source signs its catalog lookups with: the one stored for
/// the account, else the build's own.
fn client_id(ctx: &MetaCtx<'_>) -> Result<String> {
    if let Some(credentials) = ctx.credentials {
        if let Ok(Some(secret)) = credentials.get("simkl") {
            if let Some(client_id) = client_id_of(&secret) {
                return Ok(client_id);
            }
        }
    }
    crate::built_in_client_id().map(str::to_owned).ok_or_else(|| {
        anyhow!("Simkl needs this app's client id: the bootstrap sets it (or a file in ~/.config/hearthdeck/simkl-client-id).")
    })
}

/// What `/redirect` answered: Simkl's id, and which of its catalogs it is in.
struct Found {
    id: u64,
    kind: String,
}

/// Simkl's own id and catalog for a title, resolved through `/redirect`: the
/// endpoint answers a `301` whose `Location` is the Simkl page
/// (`https://simkl.com/tv/17465/game-of-thrones`). That is what its docs
/// recommend — headers only, no JSON — and it needs **no user token**, where
/// `search` does: AUTH V2 puts search behind the account, which is why a
/// `/search/id` lookup answers 401 on a signed-out machine.
fn resolve(ctx: &MetaCtx<'_>, client_id: &str, imdb: &str) -> Result<Option<Found>> {
    let mut url = Url::parse(&format!("{API}/redirect"))?;
    url.query_pairs_mut().extend_pairs([("to", "simkl"), ("imdb", imdb)]).extend_pairs(params(client_id));
    let Some(location) = ctx.http.get_location(&url).context("Simkl redirect")? else { return Ok(None) };
    Ok(found_in(&location))
}

/// The id and catalog a Simkl page URL names, `None` for a page that is not a
/// title (an authorize page, a search) or a URL this build cannot read.
fn found_in(location: &str) -> Option<Found> {
    let path = Url::parse(location).ok()?.path().to_owned();
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let kind = match segments.next()? {
        "movies" => "movie",
        "tv" => "tv",
        "anime" => "anime",
        _ => return None,
    };
    let id = segments.next()?.parse().ok()?;
    Some(Found { id, kind: kind.to_owned() })
}

/// A film's page.
fn movie(ctx: &MetaCtx<'_>, client_id: &str, found: Found, title: &TitleRef) -> Result<Option<TitleDetails>> {
    let Some(mut record) = record(ctx, client_id, "movies", found.id)? else { return Ok(None) };
    // Everything the record is borrowed for is read before the fields move into
    // the details.
    let name = name_of(&record, title);
    let released = date_of(record.released.as_deref());
    let runtime = runtime_of(record.runtime);
    let rating = record.rating();
    let description = text(record.overview.take());
    let poster = image(record.poster.as_deref(), "posters", "_w.webp");
    let background = image(record.fanart.as_deref(), "fanart", "_d.jpg");
    let trailers = trailers(&record);
    let extras = facts(DESCRIPTOR.id, &record);
    Ok(Some(TitleDetails {
        source: DESCRIPTOR.id.to_owned(),
        id: title.id.clone(),
        kind: "movie".to_owned(),
        name,
        year: record.year.map(|year| year.to_string()),
        released,
        runtime,
        rating,
        description,
        genres: record.genres,
        directors: record.director.into_iter().collect(),
        writers: Vec::new(),
        cast: Vec::new(),
        countries: record.country.into_iter().collect(),
        awards: None,
        series_status: None,
        episodes: Vec::new(),
        poster,
        background,
        logo: None,
        trailers,
        extras,
    }))
}

/// A series' page, with the episode list the picker needs.
fn series(ctx: &MetaCtx<'_>, client_id: &str, found: Found, title: &TitleRef) -> Result<Option<TitleDetails>> {
    let path = if found.kind == "anime" { "anime" } else { "tv" };
    let Some(mut record) = record(ctx, client_id, path, found.id)? else { return Ok(None) };
    let mut episodes = episodes(ctx, client_id, path, found.id, &title.id)?;
    episodes.sort_by_key(|episode| (episode.season, episode.number));
    let name = name_of(&record, title);
    let year = record.year_start_end.take().or_else(|| record.year.map(|year| year.to_string()));
    let released = date_of(record.first_aired.as_deref());
    let runtime = runtime_of(record.runtime);
    let rating = record.rating();
    let description = text(record.overview.take());
    let poster = image(record.poster.as_deref(), "posters", "_w.webp");
    let background = image(record.fanart.as_deref(), "fanart", "_d.jpg");
    let series_status = status_of(record.status.as_deref());
    let trailers = trailers(&record);
    let extras = facts(DESCRIPTOR.id, &record);
    Ok(Some(TitleDetails {
        source: DESCRIPTOR.id.to_owned(),
        id: title.id.clone(),
        kind: "series".to_owned(),
        name,
        year,
        released,
        runtime,
        rating,
        description,
        genres: record.genres,
        directors: Vec::new(),
        writers: Vec::new(),
        cast: Vec::new(),
        countries: record.country.into_iter().collect(),
        awards: None,
        series_status,
        episodes,
        poster,
        background,
        logo: None,
        trailers,
        extras,
    }))
}

/// "148 min", or nothing when Simkl has no length.
fn runtime_of(minutes: Option<u32>) -> Option<String> {
    minutes.filter(|minutes| *minutes > 0).map(|minutes| format!("{minutes} min"))
}

/// One record (`/movies/{id}`, `/tv/{id}`, `/anime/{id}`). Simkl answers an
/// empty body for an id it does not know; that is no answer, not a fault.
fn record(ctx: &MetaCtx<'_>, client_id: &str, path: &str, id: u64) -> Result<Option<Record>> {
    let mut url = Url::parse(&format!("{API}/{path}/{id}"))?;
    url.query_pairs_mut().extend_pairs(params(client_id));
    let body = ctx.http.get(&url).with_context(|| format!("Simkl {path}"))?;
    if body.iter().all(u8::is_ascii_whitespace) || body.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice(&body).with_context(|| format!("Simkl {path}"))
}

/// The episodes of a series, each with the id the stream resource is asked for
/// (`tt0944947:1:1`), as every other source here builds them.
fn episodes(ctx: &MetaCtx<'_>, client_id: &str, path: &str, id: u64, imdb: &str) -> Result<Vec<Episode>> {
    let mut url = Url::parse(&format!("{API}/{path}/episodes/{id}"))?;
    url.query_pairs_mut().extend_pairs(params(client_id));
    let body = ctx.http.get(&url).with_context(|| format!("Simkl {path} episodes"))?;
    let found: Vec<EpisodeRecord> = serde_json::from_slice(&body).with_context(|| format!("Simkl {path} episodes"))?;
    Ok(found
        .into_iter()
        // Season 0 is the specials: the picker lists numbered seasons, like the
        // other sources do.
        .filter(|episode| episode.season > 0)
        .map(|episode| Episode {
            id: format!("{imdb}:{}:{}", episode.season, episode.episode),
            season: episode.season,
            number: episode.episode,
            name: episode.title.unwrap_or_default(),
            overview: text(episode.description),
            thumbnail: image(episode.img.as_deref(), "episodes", "_w.webp"),
            released: date_of(episode.date.as_deref()),
            rating: None,
        })
        .collect())
}

/// The API parameters every catalog call carries.
fn params(client_id: &str) -> [(&'static str, String); 3] {
    [("client_id", client_id.trim().to_owned()), ("app-name", APP_NAME.to_owned()), ("app-version", APP_VERSION.to_owned())]
}

/// The trailers Simkl knows, in its own order (English first, trailers before
/// teasers), each as a YouTube page with its still. The page does not care that
/// they are YouTube's: a `Trailer` is a URL a person opens.
fn trailers(record: &Record) -> Vec<Trailer> {
    let videos = record.trailers.as_deref().unwrap_or_default();
    videos
        .iter()
        .filter_map(|video| {
            let id = video.youtube.trim();
            (!id.is_empty()).then(|| Trailer {
                name: video.name.as_deref().map(str::trim).unwrap_or_default().to_owned(),
                kind: video.kind.as_deref().map(str::trim).unwrap_or_default().to_owned(),
                url: format!("https://www.youtube.com/watch?v={id}"),
                // The wide sizes only: `hqdefault` is 4:3 with black bars baked
                // in, and cropping it is what made the banner look cut. Not every
                // video has a `maxres`, so the small one that always exists backs
                // it up.
                stills: ["maxresdefault", "hq720", "mqdefault"]
                    .iter()
                    .map(|size| format!("https://i.ytimg.com/vi/{id}/{size}.jpg"))
                    .collect(),
            })
        })
        .collect()
}

/// The open facts Simkl knows: what the card shows under its own rows. Only the
/// ones a person would want now — trailers, rankings and the rest are a later
/// slice.
fn facts(source: &str, record: &Record) -> Vec<Fact> {
    let budget = record.budget.filter(|budget| *budget > 0).map(|budget| format!("${budget}"));
    [
        ("network", "Network", record.network.clone()),
        ("certification", "Certification", record.certification.clone()),
        ("budget", "Budget", budget),
        ("original_language", "Original language", record.original_language.clone()),
    ]
    .into_iter()
    .filter_map(|(id, label, value)| {
        let value = value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())?;
        Some(Fact { id: id.to_owned(), label: label.to_owned(), value, source: source.to_owned() })
    })
    .collect()
}

fn name_of(record: &Record, title: &TitleRef) -> String {
    let name = record.title.trim();
    if name.is_empty() {
        title.title.clone()
    } else {
        name.to_owned()
    }
}

/// Simkl's dates can carry a time and a zone (`2011-04-18T01:00:00-04:00`); the
/// card shows a day.
fn date_of(value: Option<&str>) -> Option<String> {
    let date = value?.trim();
    if date.is_empty() {
        return None;
    }
    Some(date.get(..10).unwrap_or(date).to_owned())
}

/// A series' standing, in the words the film page prints.
fn status_of(status: Option<&str>) -> Option<String> {
    match status?.trim() {
        "" => None,
        "ended" => Some("Ended".to_owned()),
        "continuing" => Some("Continuing".to_owned()),
        other => Some({
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }),
    }
}

/// `poster` and `fanart` are paths, not URLs: composed here the way Simkl's own
/// guide does it.
fn image(path: Option<&str>, kind: &str, size: &str) -> Option<String> {
    let path = path?.trim();
    (!path.is_empty()).then(|| format!("{IMAGE}/{kind}/{path}{size}&q=90"))
}

fn text(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

/// The record the card is built from (`MovieDetail` / `ShowDetail`).
#[derive(Deserialize, Default)]
struct Record {
    #[serde(default)]
    title: String,
    #[serde(default)]
    year: Option<u32>,
    #[serde(default)]
    year_start_end: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    released: Option<String>,
    #[serde(default)]
    first_aired: Option<String>,
    #[serde(default)]
    runtime: Option<u32>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    poster: Option<String>,
    #[serde(default)]
    fanart: Option<String>,
    #[serde(default)]
    certification: Option<String>,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    original_language: Option<String>,
    #[serde(default)]
    budget: Option<u64>,
    #[serde(default)]
    director: Option<String>,
    #[serde(default)]
    ratings: Option<Ratings>,
    /// YouTube videos: trailers, teasers, clips. Simkl orders them itself.
    #[serde(default)]
    trailers: Option<Vec<TrailerRecord>>,
}

#[derive(Deserialize)]
struct TrailerRecord {
    #[serde(default)]
    name: Option<String>,
    /// The YouTube video id, not a URL.
    #[serde(default)]
    youtube: String,
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

impl Record {
    /// The IMDb score when Simkl carries one, else its own, to one decimal.
    fn rating(&self) -> Option<String> {
        let ratings = self.ratings.as_ref()?;
        let score = ratings.imdb.as_ref().or(ratings.simkl.as_ref())?;
        let rating = score.rating?;
        (rating > 0.0).then(|| format!("{rating:.1}"))
    }
}

#[derive(Deserialize, Default)]
struct Ratings {
    #[serde(default)]
    imdb: Option<Score>,
    #[serde(default)]
    simkl: Option<Score>,
}

#[derive(Deserialize)]
struct Score {
    #[serde(default)]
    rating: Option<f64>,
}

/// One entry of `/tv/episodes/{id}`.
#[derive(Deserialize)]
struct EpisodeRecord {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    season: u32,
    #[serde(default)]
    episode: u32,
    #[serde(default)]
    img: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::{Credentials, Secret};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Answers by path, a `Location` for the redirect, and remembers what it was
    /// asked.
    struct Fake {
        location: &'static str,
        answers: HashMap<&'static str, &'static str>,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            let mut answers = HashMap::new();
            answers.insert(
                "/movies/472214",
                r#"{
                    "title":"Inception","year":2010,"type":"movie","ids":{"simkl":472214},
                    "released":"2010-07-16","runtime":148,"director":"Christopher Nolan",
                    "certification":"PG-13","budget":160000000,
                    "overview":"A thief who steals corporate secrets through dream-sharing.",
                    "genres":["Action","Science Fiction"],"country":"US","original_language":"EN",
                    "poster":"74/74415673dcdc9cdd","fanart":"a0/a0bc38fad2b1ded3",
                    "ratings":{"simkl":{"rating":8.6,"votes":100},"imdb":{"rating":8.8,"votes":200}},
                    "trailers":[{"name":"Official Trailer","youtube":"YoHD9XEInc0","size":1080,"type":"trailer"},
                                {"name":null,"youtube":"","type":"teaser"}]
                }"#,
            );
            Self { location: "https://simkl.com/movies/472214/inception", answers, seen: Mutex::new(Vec::new()) }
        }

        fn series() -> Self {
            let mut fake = Self::new();
            fake.location = "https://simkl.com/tv/17465/game-of-thrones";
            fake.answers.insert(
                "/tv/17465",
                r#"{
                    "title":"Game of Thrones","year":2011,"type":"show","ids":{"simkl":17465},
                    "year_start_end":"2011–2019","first_aired":"2011-04-17T21:00:00-04:00",
                    "runtime":60,"network":"HBO","status":"ended","country":"US",
                    "genres":["Drama","Fantasy"],"overview":"Noble families vie for the Iron Throne.",
                    "poster":"57/5742576cd8f59fcb0","ratings":{"imdb":{"rating":9.2}}
                }"#,
            );
            fake.answers.insert(
                "/tv/episodes/17465",
                r#"[
                    {"title":"Winter Is Coming","description":"The Starks host the king.","season":1,"episode":1,
                     "img":"54/546b1b0d0dbd0d6a","date":"2011-04-17T21:00:00-04:00","type":"episode"},
                    {"title":"A special","season":0,"episode":1,"type":"episode"},
                    {"title":"The Kingsroad","season":1,"episode":2,"type":"episode"}
                ]"#,
            );
            fake
        }

        fn asked(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            self.answers
                .get(url.path())
                .map(|body| body.as_bytes().to_vec())
                .ok_or_else(|| anyhow!("the fake has no answer for {}", url.path()))
        }
        fn get_location(&self, url: &Url) -> Result<Option<String>> {
            self.seen.lock().unwrap().push(format!("LOCATION {url}"));
            Ok((!self.location.is_empty()).then(|| self.location.to_owned()))
        }
        fn post_json(&self, _: &Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            unreachable!("a metadata source reads")
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

    struct WithCredentials(Secret);

    impl Credentials for WithCredentials {
        fn get(&self, provider: &str) -> Result<Option<Secret>> {
            Ok((provider == "simkl").then(|| self.0.clone()))
        }
        fn set(&self, _: &str, _: &Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    fn credentials() -> WithCredentials {
        WithCredentials(Secret::new(serde_json::json!({"clientId": "client-1"})))
    }

    fn title(kind: &str) -> TitleRef {
        TitleRef {
            provider: "stremio".into(),
            id: "tt1375666".into(),
            kind: kind.into(),
            title: "Inception".into(),
            year: Some("2010".into()),
        }
    }

    #[test]
    fn a_film_is_resolved_through_the_redirect_then_read() {
        let fake = Fake::new();
        let credentials = credentials();
        let ctx = MetaCtx { http: &fake, credentials: Some(&credentials) };
        let details = SimklMeta.details(&ctx, &title("movie")).unwrap().expect("the fake answers");
        assert_eq!(details.name, "Inception");
        assert_eq!(details.year.as_deref(), Some("2010"));
        assert_eq!(details.released.as_deref(), Some("2010-07-16"), "a day, not a timestamp");
        assert_eq!(details.runtime.as_deref(), Some("148 min"));
        assert_eq!(details.rating.as_deref(), Some("8.8"), "the IMDb score when Simkl carries one");
        assert_eq!(details.genres, ["Action", "Science Fiction"]);
        assert_eq!(details.directors, ["Christopher Nolan"]);
        assert!(details.description.is_some());
        assert_eq!(details.trailers.len(), 1, "the video without an id is not a page to open");
        let trailer = &details.trailers[0];
        assert_eq!(trailer.name, "Official Trailer");
        assert_eq!(trailer.kind, "trailer");
        assert_eq!(trailer.url, "https://www.youtube.com/watch?v=YoHD9XEInc0");
        assert_eq!(
            trailer.stills,
            [
                "https://i.ytimg.com/vi/YoHD9XEInc0/maxresdefault.jpg",
                "https://i.ytimg.com/vi/YoHD9XEInc0/hq720.jpg",
                "https://i.ytimg.com/vi/YoHD9XEInc0/mqdefault.jpg",
            ],
            "wide sizes only, best first"
        );
        assert_eq!(details.poster.as_deref(), Some("https://wsrv.nl/?url=https://simkl.in/posters/74/74415673dcdc9cdd_w.webp&q=90"));
        assert!(details.background.is_some());
        let facts: Vec<(&str, &str)> = details.extras.iter().map(|fact| (fact.id.as_str(), fact.value.as_str())).collect();
        assert_eq!(
            facts,
            [("network", "HBO"), ("certification", "PG-13"), ("budget", "$160000000"), ("original_language", "EN")]
                .into_iter()
                .filter(|(id, _)| *id != "network")
                .collect::<Vec<_>>(),
            "a film has no network: {facts:?}"
        );
        let asked = fake.asked();
        assert!(asked[0].starts_with("LOCATION https://api.simkl.com/redirect?"), "{}", asked[0]);
        assert!(asked[0].contains("to=simkl") && asked[0].contains("imdb=tt1375666"), "{}", asked[0]);
        assert!(asked[0].contains("client_id=client-1") && asked[0].contains("app-name=hearthdeck-hero"), "{}", asked[0]);
        assert_eq!(asked.len(), 2, "the record is the second call — no search: {asked:?}");
        assert!(asked[1].contains("/movies/472214?"), "{}", asked[1]);
    }

    #[test]
    fn a_series_brings_its_seasons_and_skips_the_specials() {
        let fake = Fake::series();
        let credentials = credentials();
        let ctx = MetaCtx { http: &fake, credentials: Some(&credentials) };
        let mut title = title("series");
        title.id = "tt0944947".into();
        title.title = "Game of Thrones".into();
        let details = SimklMeta.details(&ctx, &title).unwrap().expect("the fake answers");
        assert_eq!(details.year.as_deref(), Some("2011–2019"), "the span the card shows");
        assert_eq!(details.released.as_deref(), Some("2011-04-17"));
        assert_eq!(details.series_status.as_deref(), Some("Ended"));
        assert_eq!(details.episodes.len(), 2, "season 0 is not the picker's");
        assert_eq!(details.episodes[0].id, "tt0944947:1:1");
        assert_eq!(details.episodes[0].season, 1);
        assert_eq!(details.episodes[1].number, 2);
        assert!(details.episodes[0].thumbnail.is_some());
        assert_eq!(details.episodes[0].released.as_deref(), Some("2011-04-17"));
        assert_eq!(details.extras.iter().find(|fact| fact.id == "network").map(|fact| fact.value.as_str()), Some("HBO"));
        assert!(fake.asked().iter().any(|url| url.contains("/tv/episodes/17465?")), "{:?}", fake.asked());
    }

    #[test]
    fn a_title_simkl_does_not_know_is_no_answer() {
        let mut fake = Fake::new();
        fake.location = "";
        let ctx = MetaCtx { http: &fake, credentials: Some(&credentials()) };
        assert_eq!(SimklMeta.details(&ctx, &title("movie")).unwrap(), None);
        assert_eq!(fake.asked().len(), 1, "the record is never asked without an id");
    }

    #[test]
    fn the_redirect_names_the_id_and_the_catalog_it_lives_in() {
        assert_eq!(
            found_in("https://simkl.com/movies/472214/inception").map(|found| (found.id, found.kind)),
            Some((472214, "movie".into()))
        );
        assert_eq!(found_in("https://simkl.com/tv/17465/game-of-thrones").map(|found| found.kind), Some("tv".into()));
        assert_eq!(found_in("https://simkl.com/anime/831411/demon-slayer").map(|found| found.kind), Some("anime".into()));
        assert!(found_in("https://simkl.com/oauth/authorize?x=1").is_none(), "not a title page");
        assert!(found_in("https://simkl.com/tv/not-a-number").is_none());
        assert!(found_in("").is_none());
    }

    #[test]
    fn a_title_without_an_imdb_id_is_not_asked() {
        let fake = Fake::new();
        let ctx = MetaCtx { http: &fake, credentials: Some(&credentials()) };
        let mut title = title("movie");
        title.id = "simkl:42".into();
        assert_eq!(SimklMeta.details(&ctx, &title).unwrap(), None);
        assert!(fake.asked().is_empty(), "no request for an id the redirect cannot take");
    }

    #[test]
    fn without_a_client_id_it_says_where_one_comes_from() {
        let fake = Fake::new();
        let credentials = NoCredentials;
        let ctx = MetaCtx { http: &fake, credentials: Some(&credentials) };
        let error = SimklMeta.details(&ctx, &title("movie")).unwrap_err();
        // With none stored, the build's own is the fallback; a test binary has
        // none, so this is the message a self-built copy shows.
        if crate::built_in_client_id().is_none() {
            assert!(format!("{error:#}").contains("client id"), "{error:#}");
        }
    }
}
