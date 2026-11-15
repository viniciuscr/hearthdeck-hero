//! TMDB as a metadata source: what a title *is*, from TMDB's own ids.
//!
//! The rest of Watch keys everything by IMDb id, so the source starts at
//! `/find/{imdb}` and works from the TMDB id it answers with. A film costs two
//! requests (find, then the record with its credits); a series costs one more per
//! season, because that is where its episodes live. The details cache keeps the
//! result for a week, and the season requests are what make the picker work for
//! titles only TMDB knows.
//!
//! TMDB's own "watch providers" are availability, not streams: they are not
//! fetched here yet, and they will be a fact when they are — never a stream source.

use anyhow::Result;
use serde::Deserialize;
use url::Url;

use heronite_watch_core::http::http_status;
use heronite_watch_core::metadata::{Episode, Fact, MetaCtx, MetaDescriptor, TitleDetails, TitleRef, WatchMetadataSource};

use crate::{api_key, API, IMAGE};

const DESCRIPTOR: MetaDescriptor = MetaDescriptor { id: "tmdb", name: "TMDB", applies_to: &[] };

/// How many cast names a card shows.
const CAST: usize = 10;

/// TMDB as a metadata source. Stateless: everything it keeps is the details
/// cache's.
#[derive(Clone, Copy, Debug, Default)]
pub struct TmdbMeta;

impl WatchMetadataSource for TmdbMeta {
    fn descriptor(&self) -> &MetaDescriptor {
        &DESCRIPTOR
    }

    fn details(&self, ctx: &MetaCtx<'_>, title: &TitleRef) -> Result<Option<TitleDetails>> {
        let Some(credentials) = ctx.credentials else {
            // Without a key it cannot answer; saying where the key goes is more
            // useful than a bare "not found".
            return Err(anyhow::anyhow!("TMDB needs an API key: Settings → Watch → Catalog providers."));
        };
        let key = api_key(credentials)?;
        let Some(found) = find(ctx, &key, &title.id)? else {
            // TMDB does not know the title: no answer, not a fault.
            return Ok(None);
        };
        match found {
            Found::Movie(id) => movie(ctx, &key, id, title).map(Some),
            Found::Series(id) => series(ctx, &key, id, title).map(Some),
        }
    }
}

/// What `/find` answered for an IMDb id.
enum Found {
    Movie(u64),
    Series(u64),
}

/// The TMDB id of the title, when TMDB knows it.
fn find(ctx: &MetaCtx<'_>, key: &str, imdb: &str) -> Result<Option<Found>> {
    let mut url = Url::parse(&format!("{API}/find/{imdb}"))?;
    url.query_pairs_mut().extend_pairs([("external_source", "imdb_id"), ("api_key", key)]);
    let body = match get(ctx, &url) {
        Ok(body) => body,
        Err(error) if http_status(&error) == Some(404) => return Ok(None),
        Err(error) => return Err(error),
    };
    let answer: FoundAnswer = serde_json::from_slice(&body).context("TMDB find")?;
    if let Some(result) = answer.movie_results.first() {
        return Ok(Some(Found::Movie(result.id)));
    }
    Ok(answer.tv_results.first().map(|result| Found::Series(result.id)))
}

fn movie(ctx: &MetaCtx<'_>, key: &str, id: u64, title: &TitleRef) -> Result<TitleDetails> {
    let body = get_record(ctx, &format!("{API}/movie/{id}"), key)?;
    let record: Movie = serde_json::from_slice(&body).context("TMDB movie")?;
    let mut details = TitleDetails {
        id: title.id.clone(),
        kind: "movie".to_owned(),
        name: record.title.clone().unwrap_or_else(|| title.title.clone()),
        year: record.release_date.as_deref().and_then(year_of),
        released: record.release_date.clone().filter(|date| !date.trim().is_empty()),
        runtime: record.runtime.filter(|minutes| *minutes > 0).map(|minutes| format!("{minutes} min")),
        rating: rating(record.vote_average),
        description: text(record.overview),
        genres: names(record.genres),
        directors: crew(&record.credits, &["Director"]),
        writers: crew(&record.credits, &["Writer", "Screenplay", "Story"]),
        cast: cast(&record.credits),
        countries: names(record.production_countries),
        poster: image(&record.poster_path, "w500"),
        background: image(&record.backdrop_path, "w1280"),
        ..TitleDetails::default()
    };
    details.extras = facts(DESCRIPTOR.id, record.tagline, record.budget, None, record.original_language);
    Ok(details)
}

fn series(ctx: &MetaCtx<'_>, key: &str, id: u64, title: &TitleRef) -> Result<TitleDetails> {
    let body = get_record(ctx, &format!("{API}/tv/{id}"), key)?;
    let record: Series = serde_json::from_slice(&body).context("TMDB series")?;
    let mut details = TitleDetails {
        id: title.id.clone(),
        kind: "series".to_owned(),
        name: record.name.clone().unwrap_or_else(|| title.title.clone()),
        year: record.first_air_date.as_deref().and_then(year_of),
        released: record.first_air_date.clone().filter(|date| !date.trim().is_empty()),
        runtime: record.episode_run_time.first().filter(|minutes| **minutes > 0).map(|minutes| format!("{minutes} min")),
        rating: rating(record.vote_average),
        description: text(record.overview),
        genres: names(record.genres),
        directors: crew(&record.credits, &["Director"]),
        writers: crew(&record.credits, &["Writer", "Screenplay", "Story"]),
        cast: cast(&record.credits),
        countries: names(record.origin_country.iter().map(|code| Country::named(code)).collect()),
        series_status: text(record.status),
        poster: image(&record.poster_path, "w500"),
        background: image(&record.backdrop_path, "w1280"),
        ..TitleDetails::default()
    };
    // The picker pages one source's episode list, so every season is fetched
    // here: a series costs one request per season, cached for a week.
    details.episodes = episodes(ctx, key, id, &title.id, &record.seasons)?;
    let network = record.networks.first().map(|network| network.name.clone());
    details.extras = facts(DESCRIPTOR.id, record.tagline, None, network, record.original_language);
    Ok(details)
}

/// Every episode of every season, season 0 (the specials) left out, each with the
/// id the stream resource is asked for (`tt0944947:1:1`).
fn episodes(ctx: &MetaCtx<'_>, key: &str, id: u64, imdb: &str, seasons: &[Season]) -> Result<Vec<Episode>> {
    let mut episodes = Vec::new();
    for season in seasons.iter().filter(|season| season.season_number > 0) {
        let body = get_record(ctx, &format!("{API}/tv/{id}/season/{}", season.season_number), key)?;
        let answer: SeasonAnswer = serde_json::from_slice(&body).context("TMDB season")?;
        let number = season.season_number;
        for found in answer.episodes {
            episodes.push(Episode {
                id: format!("{imdb}:{number}:{}", found.episode_number),
                season: number,
                number: found.episode_number,
                name: found.name.unwrap_or_default(),
                overview: text(found.overview),
                thumbnail: image(&found.still_path, "w300"),
                released: found.air_date.clone().filter(|date| !date.trim().is_empty()),
                rating: found.rating.and_then(rating),
            });
        }
    }
    episodes.sort_by_key(|episode| (episode.season, episode.number));
    Ok(episodes)
}

/// The open facts TMDB knows and we do not model.
fn facts(source: &str, tagline: Option<String>, budget: Option<u64>, network: Option<String>, language: Option<String>) -> Vec<Fact> {
    let budget = budget.filter(|budget| *budget > 0).map(|budget| format!("${budget}"));
    [
        ("tagline", "Tagline", tagline),
        ("budget", "Budget", budget),
        ("network", "Network", network),
        ("original_language", "Original language", language),
    ]
    .into_iter()
    .filter_map(|(id, label, value)| {
        let value = value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())?;
        Some(Fact { id: id.to_owned(), label: label.to_owned(), value, source: source.to_owned() })
    })
    .collect()
}

/// One record with its credits and external ids, and a 404 as "no answer".
fn get_record(ctx: &MetaCtx<'_>, url: &str, key: &str) -> Result<Vec<u8>> {
    let mut url = Url::parse(url)?;
    url.query_pairs_mut().extend_pairs([("append_to_response", "credits,external_ids"), ("language", "en-US"), ("api_key", key)]);
    get(ctx, &url)
}

fn get(ctx: &MetaCtx<'_>, url: &Url) -> Result<Vec<u8>> {
    ctx.http.get(url)
}

fn rating(value: f64) -> Option<String> {
    (value > 0.0).then(|| format!("{value:.1}"))
}

fn year_of(date: &str) -> Option<String> {
    date.get(..4).filter(|year| year.chars().all(|c| c.is_ascii_digit())).map(str::to_owned)
}

fn text(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

fn names<T: Named>(rows: Vec<T>) -> Vec<String> {
    rows.into_iter().map(|row| row.name()).filter(|name| !name.trim().is_empty()).collect()
}

fn cast(credits: &Option<Credits>) -> Vec<String> {
    credits
        .as_ref()
        .map(|credits| credits.cast.iter().take(CAST).filter_map(|member| text(Some(member.name.clone()))).collect())
        .unwrap_or_default()
}

/// The crew's names for a set of jobs, each person once.
fn crew(credits: &Option<Credits>, jobs: &[&str]) -> Vec<String> {
    let Some(credits) = credits else { return Vec::new() };
    let mut names: Vec<String> = Vec::new();
    for member in &credits.crew {
        let matching = member.job.as_deref().is_some_and(|job| jobs.contains(&job));
        if matching && !names.contains(&member.name) {
            names.push(member.name.clone());
        }
    }
    names
}

fn image(path: &Option<String>, width: &str) -> Option<String> {
    path.as_ref().map(|path| format!("{IMAGE}/{width}{path}"))
}

trait Named {
    fn name(self) -> String;
}

#[derive(Deserialize)]
struct FoundAnswer {
    #[serde(default)]
    movie_results: Vec<FoundRow>,
    #[serde(default)]
    tv_results: Vec<FoundRow>,
}

#[derive(Deserialize)]
struct FoundRow {
    id: u64,
}

#[derive(Deserialize)]
struct Movie {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    runtime: Option<u64>,
    #[serde(default)]
    vote_average: f64,
    #[serde(default)]
    tagline: Option<String>,
    #[serde(default)]
    budget: Option<u64>,
    #[serde(default)]
    original_language: Option<String>,
    #[serde(default)]
    genres: Vec<Genre>,
    #[serde(default)]
    production_countries: Vec<Country>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    credits: Option<Credits>,
}

#[derive(Deserialize)]
struct Series {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    first_air_date: Option<String>,
    #[serde(default)]
    episode_run_time: Vec<u64>,
    #[serde(default)]
    vote_average: f64,
    #[serde(default)]
    tagline: Option<String>,
    #[serde(default)]
    original_language: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    genres: Vec<Genre>,
    #[serde(default)]
    origin_country: Vec<String>,
    #[serde(default)]
    networks: Vec<Network>,
    #[serde(default)]
    seasons: Vec<Season>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    credits: Option<Credits>,
}

#[derive(Deserialize)]
struct Season {
    #[serde(default)]
    season_number: u32,
}

#[derive(Deserialize)]
struct SeasonAnswer {
    #[serde(default)]
    episodes: Vec<SeasonEpisode>,
}

#[derive(Deserialize)]
struct SeasonEpisode {
    #[serde(default)]
    episode_number: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    air_date: Option<String>,
    #[serde(default)]
    still_path: Option<String>,
    #[serde(default, rename = "vote_average")]
    rating: Option<f64>,
}

#[derive(Deserialize)]
struct Genre {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Country {
    #[serde(default)]
    name: String,
}

impl Named for Country {
    fn name(self) -> String {
        self.name
    }
}

impl Named for Genre {
    fn name(self) -> String {
        self.name
    }
}

impl Country {
    /// The origin country arrives as an ISO code on a series; it has no name.
    fn named(code: &str) -> Country {
        Country { name: code.to_uppercase() }
    }
}

#[derive(Deserialize)]
struct Network {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Credits {
    #[serde(default)]
    cast: Vec<CastMember>,
    #[serde(default)]
    crew: Vec<CrewMember>,
}

#[derive(Deserialize)]
struct CastMember {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct CrewMember {
    #[serde(default)]
    name: String,
    #[serde(default)]
    job: Option<String>,
}

/// `serde` needs `context` from anyhow's trait.
use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::{Credentials, Secret};
    use std::collections::HashMap;
    use std::sync::Mutex;

    const MOVIE: &str = r#"{"id":603,"imdb_id":"tt0133093","title":"The Matrix","overview":"A hacker learns the truth.",
        "release_date":"1999-03-31","runtime":136,"vote_average":8.7,"tagline":"Free your mind.","budget":63000000,
        "original_language":"en","genres":[{"id":28,"name":"Action"},{"id":878,"name":"Science Fiction"}],
        "production_countries":[{"iso_3166_1":"US","name":"United States of America"}],
        "poster_path":"/matrix.jpg","backdrop_path":"/matrix-bg.jpg",
        "credits":{"cast":[{"name":"Keanu Reeves"},{"name":"Carrie-Anne Moss"}],
                   "crew":[{"name":"Lana Wachowski","job":"Director"},{"name":"Lilly Wachowski","job":"Director"},
                           {"name":"Lana Wachowski","job":"Writer"}]}}"#;
    const SERIES: &str = r#"{"id":1396,"name":"Breaking Bad","overview":"A chemistry teacher.","first_air_date":"2008-01-20",
        "episode_run_time":[47],"vote_average":8.9,"tagline":"All hail the king.","original_language":"en","status":"Ended",
        "genres":[{"id":18,"name":"Drama"}],"origin_country":["US"],"networks":[{"name":"AMC"}],
        "seasons":[{"season_number":0},{"season_number":1},{"season_number":2}],
        "credits":{"cast":[{"name":"Bryan Cranston"}],"crew":[]}}"#;
    const SEASON: &str = r#"{"season_number":1,"episodes":[
        {"episode_number":1,"name":"Pilot","overview":"The start.","air_date":"2008-01-20","still_path":"/pilot.jpg","vote_average":8.3},
        {"episode_number":2,"name":"Cat's in the Bag...","air_date":"2008-01-27","vote_average":8.1}]}"#;

    struct Fake {
        answers: HashMap<&'static str, &'static str>,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            let mut answers = HashMap::new();
            answers.insert("/3/find/tt0133093", r#"{"movie_results":[{"id":603}],"tv_results":[]}"#);
            answers.insert("/3/movie/603", MOVIE);
            answers.insert("/3/find/tt0944947", r#"{"movie_results":[],"tv_results":[{"id":1396}]}"#);
            answers.insert("/3/tv/1396", SERIES);
            answers.insert("/3/tv/1396/season/1", SEASON);
            answers.insert("/3/tv/1396/season/2", r#"{"season_number":2,"episodes":[]}"#);
            Self { answers, seen: Mutex::new(Vec::new()) }
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            let body = self.answers.get(url.path()).copied().unwrap_or("{}");
            Ok(body.as_bytes().to_vec())
        }
        fn post_json(&self, _: &Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(b"{}".to_vec())
        }
    }

    struct Keyed(Option<&'static str>);

    impl Credentials for Keyed {
        fn get(&self, provider: &str) -> Result<Option<Secret>> {
            Ok(self.0.filter(|_| provider == "tmdb").map(crate::secret))
        }
        fn set(&self, _: &str, _: &Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    fn ctx<'a>(http: &'a Fake, credentials: &'a Keyed) -> MetaCtx<'a> {
        MetaCtx::with(http, credentials)
    }

    #[test]
    fn a_film_is_described_from_tmdb_and_keeps_our_imdb_key() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let title = TitleRef::new("stremio", "tt0133093", "movie");
        let details = TmdbMeta.details(&ctx(&http, &credentials), &title).unwrap().expect("TMDB knows the film");
        assert!(details.source.is_empty(), "the registry stamps the source, as it does for Cinemeta");
        assert_eq!((details.id.as_str(), details.kind.as_str()), ("tt0133093", "movie"), "our own key, not TMDB's id");
        assert_eq!(details.name, "The Matrix");
        assert_eq!(details.year.as_deref(), Some("1999"));
        assert_eq!(details.released.as_deref(), Some("1999-03-31"));
        assert_eq!(details.runtime.as_deref(), Some("136 min"));
        assert_eq!(details.rating.as_deref(), Some("8.7"));
        assert_eq!(details.genres, ["Action", "Science Fiction"]);
        assert_eq!(details.directors, ["Lana Wachowski", "Lilly Wachowski"]);
        assert_eq!(details.writers, ["Lana Wachowski"], "the same person once");
        assert_eq!(details.cast, ["Keanu Reeves", "Carrie-Anne Moss"]);
        assert_eq!(details.countries, ["United States of America"]);
        assert_eq!(details.poster.as_deref(), Some("https://image.tmdb.org/t/p/w500/matrix.jpg"));
        let facts: Vec<(&str, &str)> = details.extras.iter().map(|fact| (fact.id.as_str(), fact.value.as_str())).collect();
        assert_eq!(
            facts,
            [("tagline", "Free your mind."), ("budget", "$63000000"), ("original_language", "en")],
            "the open facts TMDB knows"
        );
    }

    #[test]
    fn a_series_gets_its_episodes_with_the_ids_streams_are_asked_for() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let title = TitleRef::new("stremio", "tt0944947", "series");
        let details = TmdbMeta.details(&ctx(&http, &credentials), &title).unwrap().expect("TMDB knows the series");
        assert_eq!(details.kind, "series");
        assert_eq!(details.runtime.as_deref(), Some("47 min"), "an episode's length stands for the series");
        assert_eq!(details.series_status.as_deref(), Some("Ended"));
        assert_eq!(details.episodes.len(), 2, "the specials season is left out");
        assert_eq!(details.episodes[0].id, "tt0944947:1:1");
        assert_eq!(details.episodes[0].name, "Pilot");
        assert_eq!(details.episodes[0].rating.as_deref(), Some("8.3"));
        assert_eq!(details.episodes[1].id, "tt0944947:1:2");
        let asked = http.seen.lock().unwrap().clone();
        assert!(asked.iter().any(|url| url.contains("/tv/1396/season/1")), "{asked:?}");
        assert!(asked.iter().any(|url| url.contains("/tv/1396/season/2")), "{asked:?}");
        assert!(!asked.iter().any(|url| url.contains("/season/0")), "specials are not asked: {asked:?}");
    }

    #[test]
    fn a_title_tmdb_does_not_know_is_no_answer_not_a_fault() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let title = TitleRef::new("stremio", "tt0000001", "movie");
        assert_eq!(TmdbMeta.details(&ctx(&http, &credentials), &title).unwrap(), None);
    }

    #[test]
    fn without_a_key_tmdb_says_where_it_goes() {
        let http = Fake::new();
        let credentials = Keyed(None);
        let title = TitleRef::new("stremio", "tt0133093", "movie");
        let error = TmdbMeta.details(&ctx(&http, &credentials), &title).unwrap_err();
        assert!(format!("{error:#}").contains("Settings → Watch"), "{error:#}");
    }

    #[test]
    fn a_movie_without_those_fields_answers_without_them() {
        let http = Fake::new();
        let credentials = Keyed(Some("test-key"));
        let _ = credentials;
        let bare = Fake {
            answers: HashMap::from([("/3/find/tt1", r#"{"movie_results":[{"id":9}]}"#), ("/3/movie/9", r#"{"title":"Bare"}"#)]),
            seen: Mutex::new(Vec::new()),
        };
        let details = TmdbMeta.details(&ctx(&bare, &credentials), &TitleRef::new("stremio", "tt1", "movie")).unwrap().expect("answers");
        assert_eq!(details.name, "Bare");
        assert!(details.runtime.is_none() && details.rating.is_none() && details.genres.is_empty());
        assert!(details.extras.is_empty(), "nothing to show, nothing invented");
        let _ = http;
    }
}
