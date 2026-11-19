//! Pulling Simkl's history and paused playback into our words.
//!
//! The change query is `/sync/activities`; the deltas are `/sync/all-items`
//! (with `date_from` on every pull after the first) and `/sync/playback`. What
//! comes back is normalised here and nowhere else: `heronite-watch-core` only
//! ever sees [`Pulled`].

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use heronite_watch_core::http::HttpStatus;
use heronite_watch_core::provider::Ctx;
use heronite_watch_core::pull::{Pulled, RemoteMembership, RemotePosition, RemoteWatch, WatchSync};
use heronite_watch_core::tracker::{TrackedTitle, TrackedView};

use crate::auth::{client_id_of, tokens_of};

use super::sync::{API, APP_NAME, APP_VERSION};

/// The pull side of one signed-in account.
pub struct SimklPull;

impl WatchSync for SimklPull {
    fn id(&self) -> &'static str {
        crate::PROVIDER
    }

    fn name(&self) -> &'static str {
        "Simkl"
    }

    fn snapshot(&self, ctx: &Ctx<'_>) -> Result<Value> {
        let (client_id, token) = credentials(ctx)?;
        let url = api_url(&client_id, "/sync/activities")?;
        let body = ctx.http.get_bearer(&url, &token).context("Simkl activities")?;
        serde_json::from_slice(&body).context("Simkl activities")
    }

    fn unchanged(&self, since: &Value, now: &Value) -> bool {
        match (since.get("all").and_then(Value::as_str), now.get("all").and_then(Value::as_str)) {
            (Some(since), Some(now)) => since == now,
            // A snapshot without `all` cannot say "nothing moved".
            _ => false,
        }
    }

    fn pull(&self, ctx: &Ctx<'_>, since: Option<&Value>) -> Result<Pulled> {
        let (client_id, token) = credentials(ctx)?;
        let mut pulled = Pulled::default();

        let since = since.and_then(|since| since.get("all")).and_then(Value::as_str);
        let (answers, unreachable) = self.all_items(ctx, &client_id, &token, since)?;
        pulled.skipped += unreachable;
        for answer in answers {
            for movie in answer.movies {
                match movie.status.as_str() {
                    // A plan-to-watch row is list membership, not a watch.
                    "plantowatch" => match movie.membership() {
                        Some(membership) => pulled.memberships.push(membership),
                        None => pulled.skipped += 1,
                    },
                    _ => match movie.watched() {
                        Some(watch) => pulled.watches.push(watch),
                        None => pulled.skipped += 1,
                    },
                }
            }
            for show in answer.shows.into_iter().chain(answer.anime) {
                let (membership, episodes, missing) = show.into_parts();
                pulled.memberships.extend(membership);
                pulled.watches.extend(episodes);
                pulled.skipped += missing;
            }
        }

        let url = api_url(&client_id, "/sync/playback")?;
        let body = ctx.http.get_bearer(&url, &token).context("Simkl playback")?;
        let sessions: Vec<Playback> = serde_json::from_slice(&body).context("Simkl playback")?;
        for session in sessions {
            match session.position() {
                Some(position) => pulled.positions.push(position),
                None => pulled.skipped += 1,
            }
        }
        Ok(pulled)
    }
}

/// What one listing answered: the rows, or Simkl saying the request is too big.
enum Size {
    Fits(AllItems),
    TooBig,
}

impl SimklPull {
    /// Every row of the library, splitting the request when Simkl says it is too
    /// big — by media type first, then by status. That is the guide's own ladder,
    /// and the reason a pull can finish on a library of any size.
    fn all_items(&self, ctx: &Ctx<'_>, client_id: &str, token: &str, since: Option<&str>) -> Result<(Vec<AllItems>, usize)> {
        match self.items_at(ctx, client_id, token, "/sync/all-items", since)? {
            Size::Fits(answer) => return Ok((vec![answer], 0)),
            Size::TooBig => {}
        }
        let mut answers = Vec::new();
        let mut missed = 0;
        for kind in ["movies", "shows", "anime"] {
            match self.items_at(ctx, client_id, token, &format!("/sync/all-items/{kind}"), since)? {
                Size::Fits(answer) => answers.push(answer),
                Size::TooBig => {
                    for status in ["watching", "plantowatch", "hold", "completed", "dropped"] {
                        // The last rung: what is still too big is *reported*, never
                        // silently treated as empty — a partial library that says
                        // nothing is worse than a number a screen can show.
                        match self.items_at(ctx, client_id, token, &format!("/sync/all-items/{kind}/{status}"), since)? {
                            Size::Fits(answer) => answers.push(answer),
                            Size::TooBig => {
                                log::warn!(
                                    "Simkl still refuses /sync/all-items/{kind}/{status} as too big: that bucket is missing from this pull"
                                );
                                missed += 1;
                            }
                        }
                    }
                }
            }
        }
        Ok((answers, missed))
    }

    fn items_at(&self, ctx: &Ctx<'_>, client_id: &str, token: &str, path: &str, since: Option<&str>) -> Result<Size> {
        let url = items_url(client_id, path, since)?;
        let (status, body) = ctx.http.get_bearer_answer(&url, token).context("Simkl all-items")?;
        if status == 400 && error_code(&body).as_deref() == Some("max_items") {
            return Ok(Size::TooBig);
        }
        if !(200..300).contains(&status) {
            return Err(HttpStatus::new(&url, status).into());
        }
        Ok(Size::Fits(serde_json::from_slice(&body).context("Simkl all-items")?))
    }
}

/// One listing's URL: full rows with the episode stamps, and the window.
fn items_url(client_id: &str, path: &str, since: Option<&str>) -> Result<Url> {
    let mut url = api_url(client_id, path)?;
    url.query_pairs_mut().append_pair("extended", "full");
    url.query_pairs_mut().append_pair("episode_watched_at", "yes");
    match since {
        // The exact stamp the snapshot gave, never one of ours.
        Some(all) => {
            url.query_pairs_mut().append_pair("date_from", all);
        }
        // The first pull wants every episode, so a show finished elsewhere
        // arrives whole.
        None => {
            url.query_pairs_mut().append_pair("include_all_episodes", "yes");
        }
    }
    Ok(url)
}

/// The `error` code an answer carries, when it carries one.
fn error_code(body: &[u8]) -> Option<String> {
    let answer: Value = serde_json::from_slice(body).ok()?;
    let code = answer.get("error")?.as_str()?.trim();
    (!code.is_empty()).then(|| code.to_owned())
}

/// The client id and token a pull runs with.
fn credentials(ctx: &Ctx<'_>) -> Result<(String, String)> {
    let secret = ctx.credentials.get(crate::PROVIDER)?.ok_or_else(|| anyhow!("Simkl is not connected: sign in first"))?;
    let client_id = client_id_of(&secret)
        .or_else(|| crate::built_in_client_id().map(str::to_owned))
        .ok_or_else(|| anyhow!("Simkl needs this app's client id"))?;
    let tokens = tokens_of(&secret).ok_or_else(|| anyhow!("Simkl needs a fresh sign-in"))?;
    Ok((client_id, tokens.access_token))
}

/// The API URL with the parameters every call carries.
fn api_url(client_id: &str, path: &str) -> Result<Url> {
    let mut url = Url::parse(&format!("{API}{path}")).with_context(|| format!("building the Simkl URL for {path}"))?;
    url.query_pairs_mut().extend_pairs([("client_id", client_id.trim()), ("app-name", APP_NAME), ("app-version", APP_VERSION)]);
    Ok(url)
}

/// A date Simkl prints, if it prints one.
fn at(raw: Option<&str>) -> Option<DateTime<Utc>> {
    let raw = raw?.trim();
    (!raw.is_empty()).then(|| DateTime::parse_from_rfc3339(raw).ok().map(|at| at.with_timezone(&Utc)))?
}

#[derive(Deserialize, Default)]
struct AllItems {
    #[serde(default)]
    movies: Vec<MovieRow>,
    #[serde(default)]
    shows: Vec<ShowRow>,
    #[serde(default)]
    anime: Vec<ShowRow>,
}

/// One show in the pull, with the episodes Simkl lists under it.
#[derive(Deserialize)]
struct ShowRow {
    #[serde(default)]
    status: String,
    show: Media,
    #[serde(default)]
    seasons: Vec<SeasonRow>,
}

#[derive(Deserialize, Default)]
struct SeasonRow {
    /// Absent when Simkl did not say: an episode with no season cannot be placed.
    #[serde(default)]
    number: Option<u32>,
    #[serde(default)]
    episodes: Vec<EpisodeRow>,
}

#[derive(Deserialize, Default)]
struct EpisodeRow {
    #[serde(default)]
    number: Option<u32>,
    /// Present when `episode_watched_at` was asked for, and only on an episode the
    /// person watched.
    #[serde(default)]
    watched_at: Option<String>,
}

impl ShowRow {
    /// What one show contributes: its watchlist membership when it is on the list,
    /// one fact per watched episode, and the rows that could not be placed.
    ///
    /// With `episode_watched_at=yes` Simkl shares the show's last-watched stamp
    /// across its episodes: honest enough for the one-time baseline, and the reason
    /// a delta asks for it too — an episode that moved carries its own date.
    ///
    /// An episode nobody watched is **not** a skipped row: it is simply not a fact.
    fn into_parts(self) -> (Option<RemoteMembership>, Vec<RemoteWatch>, usize) {
        let ShowRow { status, show, seasons } = self;
        let Some(title) = series_title(show) else {
            // Without a title anybody can be shown, its episodes are not facts we
            // can place: counted, not guessed.
            return (None, Vec::new(), seasons.iter().map(|season| season.episodes.len()).sum());
        };
        let membership =
            (status == "plantowatch").then(|| RemoteMembership { title: title.clone(), list: "watchlist".into(), member: true });
        let mut watches = Vec::new();
        let mut missing = 0;
        for season in seasons {
            let Some(season_number) = season.number else {
                missing += season.episodes.len();
                continue;
            };
            for episode in season.episodes {
                match (episode.number, at(episode.watched_at.as_deref())) {
                    (Some(number), Some(stamp)) => watches.push(RemoteWatch {
                        title: title.clone(),
                        view: TrackedView::Episode { season: season_number, number },
                        at: Some(stamp),
                    }),
                    // A stamp without a number cannot be placed.
                    (None, Some(_)) => missing += 1,
                    // No stamp: never watched, or the provider did not say, so there
                    // is nothing for a history to carry.
                    (_, None) => {}
                }
            }
        }
        (membership, watches, missing)
    }
}

/// A show's title, with the kind a tracker needs.
fn series_title(media: Media) -> Option<TrackedTitle> {
    let imdb = media.ids.imdb?.trim().to_owned();
    (!imdb.is_empty()).then(|| TrackedTitle { imdb: Some(imdb), kind: "series".into(), title: media.title, year: None })
}

#[derive(Deserialize)]
struct MovieRow {
    #[serde(default)]
    status: String,
    #[serde(default)]
    last_watched_at: Option<String>,
    movie: Media,
}

impl MovieRow {
    /// A finished film is a watched fact; anything else is list membership,
    /// which a later slice handles.
    fn watched(self) -> Option<RemoteWatch> {
        if self.status != "completed" {
            return None;
        }
        let (title, view) = self.movie.into_title_and_view()?;
        Some(RemoteWatch { title, view, at: at(self.last_watched_at.as_deref()) })
    }

    /// A plan-to-watch film as a watchlist membership.
    fn membership(self) -> Option<RemoteMembership> {
        let (title, _) = self.movie.into_title_and_view()?;
        Some(RemoteMembership { title, list: "watchlist".into(), member: true })
    }
}

#[derive(Deserialize)]
struct Media {
    #[serde(default)]
    title: String,
    #[serde(default)]
    ids: Ids,
}

#[derive(Deserialize, Default)]
struct Ids {
    #[serde(default)]
    imdb: Option<String>,
}

impl Media {
    fn into_title_and_view(self) -> Option<(TrackedTitle, TrackedView)> {
        let imdb = self.ids.imdb?.trim().to_owned();
        if imdb.is_empty() {
            return None;
        }
        Some((TrackedTitle { imdb: Some(imdb), kind: "movie".into(), title: self.title, year: None }, TrackedView::Movie))
    }
}

#[derive(Deserialize)]
struct Playback {
    #[serde(default)]
    progress: f64,
    #[serde(default)]
    paused_at: Option<String>,
    #[serde(default)]
    movie: Option<Media>,
    #[serde(default)]
    show: Option<Media>,
    #[serde(default)]
    episode: Option<Episode>,
}

#[derive(Deserialize)]
struct Episode {
    #[serde(default)]
    season: Option<u32>,
    #[serde(default)]
    number: Option<u32>,
}

impl Playback {
    fn position(self) -> Option<RemotePosition> {
        let percent = self.progress.clamp(0., 100.);
        let paused_at = at(self.paused_at.as_deref());
        match (self.episode.and_then(|episode| Some((episode.season?, episode.number?))), self.show) {
            (Some((season, number)), Some(show)) => {
                let (mut title, _) = show.into_title_and_view()?;
                title.kind = "series".into();
                Some(RemotePosition { title, view: TrackedView::Episode { season, number }, percent, at: paused_at })
            }
            _ => {
                let (title, view) = self.movie?.into_title_and_view()?;
                Some(RemotePosition { title, view, percent, at: paused_at })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::http::Transport;
    use heronite_watch_core::provider::{Credentials, Secret};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct Fake {
        answers: HashMap<&'static str, &'static str>,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            let mut answers = HashMap::new();
            answers.insert(
                "/sync/activities",
                r#"{"all":"2026-01-05T00:00:00Z","movies":{"all":"2026-01-05T00:00:00Z","playback":"2026-01-04T00:00:00Z"}}"#,
            );
            answers.insert(
                "/sync/all-items",
                r#"{
                    "movies":[
                      {"status":"completed","last_watched_at":"2026-01-02T20:00:00Z","movie":{"title":"Inception","year":2010,"ids":{"imdb":"tt1375666"}}},
                      {"status":"plantowatch","movie":{"title":"Dune","ids":{"imdb":"tt1160419"}}},
                      {"status":"completed","movie":{"title":"No ids"}}
                    ],
                    "shows":[{"status":"watching","last_watched_at":"2026-01-03T20:00:00Z","show":{"title":"A Show","ids":{"imdb":"tt0944947"}},
                      "seasons":[{"number":1,"episodes":[{"number":1,"watched_at":"2026-01-03T20:00:00Z"},{"number":2},{"number":3,"watched_at":"2026-01-03T20:00:00Z"}]}]},
                     {"status":"plantowatch","show":{"title":"Another Show","ids":{"imdb":"tt2468"}},"seasons":[]}],
                    "anime":[]
                }"#,
            );
            answers.insert(
                "/sync/playback",
                r#"[
                    {"progress":40.0,"paused_at":"2026-01-04T20:00:00Z","type":"movie","movie":{"title":"Inception","ids":{"imdb":"tt1375666"}}},
                    {"progress":12.5,"paused_at":"2026-01-04T21:00:00Z","type":"episode","show":{"title":"A Show","ids":{"imdb":"tt0944947"}},"episode":{"season":2,"number":3}}
                ]"#,
            );
            Self { answers, seen: Mutex::new(Vec::new()) }
        }

        fn asked(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn get(&self, _: &Url) -> Result<Vec<u8>> {
            unreachable!("a pull reads with the bearer")
        }
        fn post_json(&self, _: &Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            unreachable!("a pull reads")
        }
        fn get_bearer_answer(&self, url: &Url, token: &str) -> Result<(u16, Vec<u8>)> {
            Ok((200, self.get_bearer(url, token)?))
        }
        fn get_bearer(&self, url: &Url, token: &str) -> Result<Vec<u8>> {
            assert_eq!(token, "simkl_at_x");
            self.seen.lock().unwrap().push(url.to_string());
            self.answers
                .get(url.path())
                .map(|body| body.as_bytes().to_vec())
                .ok_or_else(|| anyhow!("the fake has no answer for {}", url.path()))
        }
    }

    /// A fake that answers each path with a status: the ladder needs refusals.
    struct Sized {
        answers: HashMap<&'static str, (u16, &'static str)>,
        seen: Mutex<Vec<String>>,
    }

    impl Transport for Sized {
        fn get(&self, _: &Url) -> Result<Vec<u8>> {
            unreachable!("a pull reads with the bearer")
        }
        fn post_json(&self, _: &Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            unreachable!("a pull reads")
        }
        fn get_bearer(&self, url: &Url, token: &str) -> Result<Vec<u8>> {
            Ok(self.get_bearer_answer(url, token)?.1)
        }
        fn get_bearer_answer(&self, url: &Url, _: &str) -> Result<(u16, Vec<u8>)> {
            self.seen.lock().unwrap().push(url.to_string());
            let (status, body) = self.answers.get(url.path()).copied().ok_or_else(|| anyhow!("no answer for {}", url.path()))?;
            Ok((status, body.as_bytes().to_vec()))
        }
    }

    #[test]
    fn a_library_that_is_too_big_is_split_by_type() {
        let mut answers = HashMap::new();
        answers.insert("/sync/activities", (200, r#"{"all":"2026-01-05T00:00:00Z"}"#));
        answers.insert("/sync/all-items", (400, r#"{"error":"max_items"}"#));
        answers.insert(
            "/sync/all-items/movies",
            (200, r#"{"movies":[{"status":"completed","last_watched_at":"2026-01-02T20:00:00Z","movie":{"title":"Inception","ids":{"imdb":"tt1375666"}}}]}"#),
        );
        answers.insert("/sync/all-items/shows", (200, r#"{"shows":[]}"#));
        answers.insert("/sync/all-items/anime", (200, r#"{"anime":[]}"#));
        answers.insert("/sync/playback", (200, "[]"));
        let fake = Sized { answers, seen: Mutex::new(Vec::new()) };
        let credentials = credentials();
        let ctx = Ctx { http: &fake, credentials: &credentials };

        let pulled = SimklPull.pull(&ctx, None).unwrap();
        assert_eq!(pulled.watches.len(), 1, "the film came from its own request");
        let seen = fake.seen.lock().unwrap().clone();
        assert!(seen.iter().any(|url| url.contains("/sync/all-items/movies")), "{seen:?}");
        assert!(seen.iter().any(|url| url.contains("/sync/all-items/shows")), "{seen:?}");
    }

    #[test]
    fn a_refusal_that_is_not_a_size_is_still_an_error() {
        let mut answers = HashMap::new();
        answers.insert("/sync/activities", (200, r#"{"all":"2026-01-05T00:00:00Z"}"#));
        answers.insert("/sync/all-items", (400, r#"{"error":"invalid_date_from"}"#));
        let fake = Sized { answers, seen: Mutex::new(Vec::new()) };
        let credentials = credentials();
        let ctx = Ctx { http: &fake, credentials: &credentials };

        let error = SimklPull.pull(&ctx, None).unwrap_err();
        assert!(format!("{error:#}").contains("HTTP 400"), "{error:#}");
        assert_eq!(fake.seen.lock().unwrap().len(), 1, "no ladder on a different refusal");
    }

    struct WithCredentials(Secret);

    impl Credentials for WithCredentials {
        fn get(&self, _: &str) -> Result<Option<Secret>> {
            Ok(Some(self.0.clone()))
        }
        fn set(&self, _: &str, _: &Secret) -> Result<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    fn credentials() -> WithCredentials {
        WithCredentials(Secret::new(serde_json::json!({
            "clientId": "client-1",
            "accessToken": "simkl_at_x",
            "refreshToken": "simkl_rt_y",
            "scope": "media:read",
            "expiresAt": "2099-01-01T00:00:00Z",
        })))
    }

    #[test]
    fn a_snapshot_is_the_change_query_and_its_all_decides() {
        let fake = Fake::new();
        let credentials = credentials();
        let ctx = Ctx { http: &fake, credentials: &credentials };
        let snapshot = SimklPull.snapshot(&ctx).unwrap();
        assert_eq!(snapshot["all"], "2026-01-05T00:00:00Z");
        assert!(SimklPull.unchanged(&snapshot, &snapshot));
        assert!(!SimklPull.unchanged(&snapshot, &serde_json::json!({"all": "2026-01-06T00:00:00Z"})));
        assert!(!SimklPull.unchanged(&serde_json::json!({}), &snapshot), "no all is not 'unchanged'");
    }

    #[test]
    fn the_baseline_asks_for_every_episode_and_a_delta_does_not() {
        let fake = Fake::new();
        let credentials = credentials();
        let ctx = Ctx { http: &fake, credentials: &credentials };
        let _ = SimklPull.pull(&ctx, None).unwrap();
        let seen = fake.asked();
        assert!(seen.iter().any(|url| url.contains("include_all_episodes=yes")), "{seen:?}");
        assert!(
            seen.iter()
                .filter(|url| url.contains("/sync/all-items"))
                .all(|url| url.contains("episode_watched_at=yes") && url.contains("extended=full")),
            "the episodes and their stamps, on every pull: {seen:?}"
        );

        let since = serde_json::json!({"all": "2026-01-01T00:00:00Z"});
        let _ = SimklPull.pull(&ctx, Some(&since)).unwrap();
        let seen = fake.asked();
        let delta: Vec<&String> = seen.iter().filter(|url| url.contains("date_from=")).collect();
        assert!(!delta.is_empty(), "{seen:?}");
        assert!(delta.iter().all(|url| !url.contains("include_all_episodes")), "a delta asks for what moved: {seen:?}");
    }

    #[test]
    fn the_first_pull_has_no_date_and_later_ones_do() {
        let fake = Fake::new();
        let credentials = credentials();
        let ctx = Ctx { http: &fake, credentials: &credentials };
        let _ = SimklPull.pull(&ctx, None).unwrap();
        let seen = fake.asked();
        assert!(seen.iter().any(|url| url.contains("/sync/all-items?") && !url.contains("date_from")), "{seen:?}");
        assert!(seen.iter().all(|url| url.contains("client_id=client-1") && url.contains("app-name=hearthdeck-hero")), "{seen:?}");

        let fake = Fake::new();
        let ctx = Ctx { http: &fake, credentials: &credentials };
        let since = serde_json::json!({"all": "2026-01-01T00:00:00Z"});
        let _ = SimklPull.pull(&ctx, Some(&since)).unwrap();
        let seen = fake.asked();
        assert!(
            seen.iter().any(|url| url.contains("date_from=2026-01-01T00%3A00%3A00Z") || url.contains("date_from=2026-01-01T00:00:00Z")),
            "the exact stamp comes back: {seen:?}"
        );
    }

    #[test]
    fn a_completed_film_is_a_watch_and_the_rest_is_counted() {
        let fake = Fake::new();
        let credentials = credentials();
        let ctx = Ctx { http: &fake, credentials: &credentials };
        let pulled = SimklPull.pull(&ctx, None).unwrap();
        assert_eq!(pulled.watches.len(), 3, "{:?}", pulled.watches);
        let watch = &pulled.watches[0];
        assert_eq!(watch.title.imdb.as_deref(), Some("tt1375666"));
        assert_eq!(watch.view, TrackedView::Movie);
        assert!(watch.at.is_some(), "the provider's own stamp");

        // The show arrives as its episodes, each its own fact.
        assert_eq!(pulled.watches[1].view, TrackedView::Episode { season: 1, number: 1 });
        assert_eq!(pulled.watches[2].view, TrackedView::Episode { season: 1, number: 3 });
        assert_eq!(pulled.watches[1].title.kind, "series");
        assert_eq!(pulled.watches[1].title.imdb.as_deref(), Some("tt0944947"));
        // the id-less film; the unwatched episode is not a skipped row
        assert_eq!(pulled.skipped, 1, "{pulled:?}");
        assert_eq!(pulled.memberships.len(), 2, "the film and the series on the list");
        assert_eq!(pulled.memberships[0].list, "watchlist");
        assert_eq!(pulled.memberships[0].title.imdb.as_deref(), Some("tt1160419"));
        assert_eq!(pulled.memberships[1].title.kind, "series", "a show can be on the list too");
        assert_eq!(pulled.memberships[1].title.imdb.as_deref(), Some("tt2468"));
    }

    #[test]
    fn playback_becomes_positions_with_a_view() {
        let fake = Fake::new();
        let credentials = credentials();
        let ctx = Ctx { http: &fake, credentials: &credentials };
        let pulled = SimklPull.pull(&ctx, None).unwrap();
        assert_eq!(pulled.positions.len(), 2);
        assert_eq!(pulled.positions[0].percent, 40.0);
        assert_eq!(pulled.positions[0].view, TrackedView::Movie);
        assert_eq!(pulled.positions[1].percent, 12.5);
        assert_eq!(pulled.positions[1].view, TrackedView::Episode { season: 2, number: 3 });
        assert_eq!(pulled.positions[1].title.kind, "series");
    }

    #[test]
    fn without_a_session_a_pull_says_so() {
        let fake = Fake::new();
        struct None_;

        impl Credentials for None_ {
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
        let credentials = None_;
        let ctx = Ctx { http: &fake, credentials: &credentials };
        let error = SimklPull.snapshot(&ctx).unwrap_err();
        assert!(format!("{error:#}").contains("sign in"), "{error:#}");
    }
}
