//! Sending watch events to Simkl.
//!
//! Three endpoints, one per kind of thing that happened:
//!
//! - a finished film or episode → `POST /sync/history`, a watch event stamped
//!   with **when it was watched** (the session's end, not the upload time);
//! - an un-marked one → `POST /sync/history/remove`;
//! - one left part-way → `POST /scrobble/stop` with its progress, which Simkl
//!   keeps as a **paused playback** — its own "continue watching" row, since its
//!   history has no notion of half a film.
//!
//! Every request carries the app's `client_id`, `app-name` and `app-version`, and
//! the person's token as a bearer. The client id is not secret; the token never
//! reaches a message, and a refusal is reported by status, never by echoing it.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use heronite_watch_core::http::Transport;
use heronite_watch_core::tracker::{Delivery, ListChange, TrackEvent, TrackedTitle, TrackedView, TrackedWatch, WatchTracker};

/// Simkl's API root.
pub const API: &str = "https://api.simkl.com";
/// The name every request reports, as Simkl's rules ask.
pub const APP_NAME: &str = "hearthdeck-hero";
/// The version every request reports.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Simkl marks watched at 80 % on its own: below it, a stop keeps the position.
const PAUSED_LIMIT: u8 = 79;

/// The delivery side of one signed-in account.
pub struct Sync<'a> {
    http: &'a dyn Transport,
    client_id: &'a str,
    token: &'a str,
}

impl<'a> Sync<'a> {
    pub fn new(http: &'a dyn Transport, client_id: &'a str, token: &'a str) -> Self {
        Self { http, client_id, token }
    }

    /// Sends one event, and says what to do with it.
    pub fn deliver(&self, event: &TrackEvent) -> Result<Delivery> {
        match &event.watch {
            TrackedWatch::Watched => self.history(event, true),
            TrackedWatch::Unwatched => self.history(event, false),
            TrackedWatch::InProgress { position_ms, duration_ms } => self.paused(event, *position_ms, *duration_ms),
        }
    }

    /// A watch event, or its removal: the same body either way, without the
    /// timestamp when it is a removal.
    fn history(&self, event: &TrackEvent, watched: bool) -> Result<Delivery> {
        let path = if watched { "/sync/history" } else { "/sync/history/remove" };
        let url = self.url(path)?;
        // The timestamp belongs to the **episode** in a show payload (Simkl's
        // own example), never to the show: a show-level time says nothing about
        // which episode was watched.
        let mut episode = json!({ "number": episode_number(&event.view) });
        if watched {
            episode["watched_at"] = json!(event.at.to_rfc3339());
        }
        let mut item = self.item(event);
        let body = match event.view {
            TrackedView::Movie => {
                if watched {
                    item["watched_at"] = json!(event.at.to_rfc3339());
                }
                json!({ "movies": [item] })
            }
            TrackedView::Episode { season, .. } => {
                item["seasons"] = json!([{ "number": season, "episodes": [episode] }]);
                json!({ "shows": [item] })
            }
        };
        self.classify(&url, &body)
    }

    /// A watchlist change.
    ///
    /// Adding names the destination (`to: plantowatch`). Simkl has **no
    /// remove-from-the-list-only** call: its own "Remove from list" takes the entry
    /// *and the title's history* away, via `/sync/history/remove`, so that is what
    /// this sends — a person removing a title from their list on simkl.com gets the
    /// same result.
    ///
    /// Favourites and custom lists are a paid, beta surface (`403 premium_only`),
    /// so they answer `Unmatched` here instead of failing on every attempt.
    fn list(&self, change: &ListChange) -> Result<Delivery> {
        if change.list != "watchlist" {
            return Ok(Delivery::Unmatched(format!("Simkl keeps {} behind its paid tier", change.list)));
        }
        let ids = self.ids(&change.title);
        let item = if change.member { json!({ "ids": ids, "to": "plantowatch" }) } else { json!({ "ids": ids }) };
        let body = if change.title.kind == "series" { json!({ "shows": [item] }) } else { json!({ "movies": [item] }) };
        let path = if change.member { "/sync/add-to-list" } else { "/sync/history/remove" };
        self.classify(&self.url(path)?, &body)
    }

    /// A half-watched title: Simkl stores it as a paused playback. Below its 80 %
    /// line nothing enters the history, which is exactly what we want.
    ///
    /// No id is looked up first: Simkl resolves `imdb` (and `title`/`year`)
    /// server-side on every write, and its guide is explicit that a `/search/id`
    /// round-trip is wasted work that doubles the quota for no gain.
    fn paused(&self, event: &TrackEvent, position_ms: u64, duration_ms: u64) -> Result<Delivery> {
        if duration_ms == 0 {
            return Ok(Delivery::Unmatched("the length of the title is unknown".into()));
        }
        let ids = self.ids(&event.title);
        let body = match event.view {
            TrackedView::Movie => json!({ "progress": percent(position_ms, duration_ms), "movie": { "ids": ids } }),
            TrackedView::Episode { season, number } => json!({
                "progress": percent(position_ms, duration_ms),
                "show": { "ids": ids },
                "episode": { "season": season, "number": number },
            }),
        };
        self.classify(&self.url("/scrobble/stop")?, &body)
    }

    /// What identifies the title: the IMDb id Watch resolves titles by, plus the
    /// name and year as the fallback for a title Simkl has no id for.
    fn item(&self, event: &TrackEvent) -> Value {
        let mut item = json!({ "ids": self.ids(&event.title) });
        if !event.title.title.trim().is_empty() {
            item["title"] = json!(event.title.title.trim());
        }
        if let Some(year) = year_of(&event.title) {
            item["year"] = json!(year);
        }
        item
    }

    fn ids(&self, title: &TrackedTitle) -> Value {
        match title.imdb.as_deref().map(str::trim).filter(|imdb| !imdb.is_empty()) {
            Some(imdb) => json!({ "imdb": imdb }),
            None => json!({}),
        }
    }

    /// The API parameters every call carries.
    fn params(&self) -> [(&'static str, String); 3] {
        [("client_id", self.client_id.trim().to_owned()), ("app-name", APP_NAME.to_owned()), ("app-version", APP_VERSION.to_owned())]
    }

    fn url(&self, path: &str) -> Result<Url> {
        let mut url = Url::parse(&format!("{API}{path}")).with_context(|| format!("building the Simkl URL for {path}"))?;
        let params = self.params();
        url.query_pairs_mut().extend_pairs(params.iter().map(|(key, value)| (*key, value.as_str())));
        Ok(url)
    }

    /// One answer, read the way a tracker should: a refusal is permanent, a
    /// rate limit or a server fault is not, and an expired token stops the run.
    fn classify(&self, url: &Url, body: &Value) -> Result<Delivery> {
        let (status, answer) = self.http.post_json_bearer(url, self.token, body).context("Simkl")?;
        let code = error_code(&answer);
        match status {
            200 | 201 => {
                let answer: Answer = serde_json::from_slice(&answer).unwrap_or_default();
                if answer.not_found.any() {
                    Ok(Delivery::Unmatched("Simkl could not match this title".into()))
                } else {
                    Ok(Delivery::Delivered)
                }
            }
            // The same watch is already recorded: what we wanted.
            409 => Ok(Delivery::Delivered),
            401 | 403 => Err(anyhow!("Simkl refused this account's token: sign in again (HTTP {status})")),
            // A malformed request will not fix itself; a rate limit will.
            // The docs print both spellings; the lock is a sub-second condition,
            // not a quota.
            400 if code.as_deref().is_some_and(|code| code.eq_ignore_ascii_case("RATE_LIMIT")) => {
                Ok(Delivery::Retry("Simkl is rate limiting requests".into()))
            }
            400 => Ok(Delivery::Unmatched(
                code.map_or_else(|| "Simkl refused the write".into(), |code| format!("Simkl refused the write ({code})")),
            )),
            404 => Ok(Delivery::Unmatched("Simkl does not know this title".into())),
            429 => Ok(Delivery::Retry("Simkl is rate limiting requests".into())),
            status => Ok(Delivery::Retry(format!("Simkl answered HTTP {status}"))),
        }
    }
}

/// The port a tracker implements, over one signed-in account. The credentials are
/// owned, so the facade can hold the tracker in a trait object for the run.
pub struct SimklTracker<'a> {
    http: &'a dyn Transport,
    client_id: String,
    token: String,
}

impl<'a> SimklTracker<'a> {
    pub fn new(http: &'a dyn Transport, client_id: &str, token: &str) -> Self {
        Self { http, client_id: client_id.to_owned(), token: token.to_owned() }
    }
}

impl WatchTracker for SimklTracker<'_> {
    fn id(&self) -> &'static str {
        crate::PROVIDER
    }

    fn name(&self) -> &'static str {
        "Simkl"
    }

    fn deliver(&self, event: &TrackEvent) -> Result<Delivery> {
        Sync::new(self.http, &self.client_id, &self.token).deliver(event)
    }

    fn sync_list(&self, change: &ListChange) -> Result<Delivery> {
        Sync::new(self.http, &self.client_id, &self.token).list(change)
    }
}

/// How far in, 1..=79: the paused-playback range. Simkl's own line is 80, above
/// which a stop marks the item watched. The division and the clamp both happen
/// in `u128`, so a position past the end cannot wrap into a small percentage.
fn percent(position_ms: u64, duration_ms: u64) -> u8 {
    let percent = u128::from(position_ms) * 100 / u128::from(duration_ms);
    percent.clamp(1, u128::from(PAUSED_LIMIT)) as u8
}

/// The year Simkl takes: an integer, from the first four digits the library
/// knows (a series' "2011–2019" is 2011).
fn year_of(title: &TrackedTitle) -> Option<u16> {
    let year = title.year.as_deref()?.trim();
    let digits: String = year.chars().take_while(|c| c.is_ascii_digit()).collect();
    (digits.len() == 4).then(|| digits.parse().ok())?
}

/// The episode number of an episode view (a film has none).
fn episode_number(view: &TrackedView) -> u32 {
    match view {
        TrackedView::Episode { number, .. } => *number,
        TrackedView::Movie => 0,
    }
}

fn error_code(body: &[u8]) -> Option<String> {
    let answer: Value = serde_json::from_slice(body).ok()?;
    let code = answer.get("error")?.as_str()?.trim();
    (!code.is_empty()).then(|| code.to_owned())
}

#[derive(Deserialize, Default)]
struct Answer {
    #[serde(default)]
    not_found: NotFound,
}

#[derive(Deserialize, Default)]
struct NotFound {
    #[serde(default)]
    movies: Vec<Value>,
    #[serde(default)]
    shows: Vec<Value>,
    #[serde(default)]
    episodes: Vec<Value>,
}

impl NotFound {
    fn any(&self) -> bool {
        !self.movies.is_empty() || !self.shows.is_empty() || !self.episodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use heronite_watch_core::http::http_status;
    use std::sync::Mutex;

    /// Answers every POST with a fixed status/body, and records what was sent.
    struct Fake {
        status: u16,
        answer: String,
        seen: Mutex<Vec<String>>,
        tokens: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(status: u16, answer: &str) -> Self {
            Self { status, answer: answer.into(), seen: Mutex::new(Vec::new()), tokens: Mutex::new(Vec::new()) }
        }

        fn delivered() -> Self {
            Self::new(201, r#"{"added":{"movies":1,"shows":1,"episodes":1},"not_found":{"movies":[],"shows":[],"episodes":[]}}"#)
        }

        fn posts(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            unreachable!("a tracker writes; it does not look a title up first: {url}")
        }
        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            unreachable!("a tracker posts with a token")
        }
        fn post_json_bearer(&self, url: &Url, token: &str, body: &Value) -> Result<(u16, Vec<u8>)> {
            self.seen.lock().unwrap().push(format!("POST {url} {body}"));
            self.tokens.lock().unwrap().push(token.to_owned());
            Ok((self.status, self.answer.as_bytes().to_vec()))
        }
    }

    fn list_change(kind: &str, list: &str, member: bool) -> ListChange {
        ListChange {
            title: TrackedTitle { imdb: Some("tt1".into()), kind: kind.into(), title: "A Film".into(), year: Some("2010".into()) },
            list: list.into(),
            member,
        }
    }

    #[test]
    fn adding_to_the_watchlist_names_the_destination() {
        let fake = Fake::delivered();
        let delivery = SimklTracker::new(&fake, "client-1", "tok").sync_list(&list_change("movie", "watchlist", true)).unwrap();
        assert_eq!(delivery, Delivery::Delivered);
        let posts = fake.posts();
        assert!(posts.iter().any(|post| post.contains("/sync/add-to-list") && post.contains(r#""to":"plantowatch""#)), "{posts:?}");
    }

    #[test]
    fn removing_from_the_watchlist_is_the_history_removal() {
        let fake = Fake::delivered();
        let delivery = SimklTracker::new(&fake, "client-1", "tok").sync_list(&list_change("series", "watchlist", false)).unwrap();
        assert_eq!(delivery, Delivery::Delivered);
        let posts = fake.posts();
        assert!(posts.iter().any(|post| post.contains("/sync/history/remove") && post.contains(r#""shows""#)), "{posts:?}");
        assert!(!posts.iter().any(|post| post.contains("plantowatch")), "{posts:?}");
    }

    #[test]
    fn a_paid_list_is_unmatched_without_a_request() {
        let fake = Fake::delivered();
        let delivery = SimklTracker::new(&fake, "client-1", "tok").sync_list(&list_change("movie", "favourites", true)).unwrap();
        match delivery {
            Delivery::Unmatched(message) => assert!(message.contains("paid"), "{message}"),
            other => panic!("a paid list is unmatched, not {other:?}"),
        }
        assert!(fake.posts().is_empty(), "no request is spent on it");
    }

    fn movie(watch: TrackedWatch) -> TrackEvent {
        TrackEvent {
            seq: 1,
            at: Utc.with_ymd_and_hms(2026, 5, 1, 20, 0, 0).unwrap(),
            title: TrackedTitle { imdb: Some("tt1".into()), kind: "movie".into(), title: "A Film".into(), year: Some("2010".into()) },
            view: TrackedView::Movie,
            watch,
        }
    }

    fn episode(watch: TrackedWatch) -> TrackEvent {
        TrackEvent {
            seq: 2,
            at: Utc.with_ymd_and_hms(2026, 5, 1, 20, 0, 0).unwrap(),
            title: TrackedTitle {
                imdb: Some("tt1".into()), kind: "series".into(), title: "A Show".into(), year: Some("2011–2019".into())
            },
            view: TrackedView::Episode { season: 1, number: 2 },
            watch,
        }
    }

    fn sync(fake: &Fake) -> Sync<'_> {
        Sync::new(fake, "client-1", "token-1")
    }

    #[test]
    fn a_watched_film_is_a_watch_event_with_when_it_happened() {
        let fake = Fake::delivered();
        assert_eq!(sync(&fake).deliver(&movie(TrackedWatch::Watched)).unwrap(), Delivery::Delivered);
        let post = fake.posts().remove(0);
        assert!(post.contains("/sync/history?"), "{post}");
        assert!(post.contains("client_id=client-1") && post.contains("app-name=hearthdeck-hero"), "the app identifies itself: {post}");
        assert!(post.contains(r#""imdb":"tt1""#) && post.contains(r#""year":2010"#), "{post}");
        assert!(post.contains(r#""watched_at":"2026-05-01T20:00:00+00:00""#), "the session's time, not now: {post}");
        assert!(post.contains(r#""movies":[{"ids"#), "a film is a movie entry: {post}");
        assert_eq!(fake.tokens.lock().unwrap().as_slice(), ["token-1"], "the token travels as a bearer");
    }

    #[test]
    fn a_watched_episode_names_its_season_and_number() {
        let fake = Fake::delivered();
        assert_eq!(sync(&fake).deliver(&episode(TrackedWatch::Watched)).unwrap(), Delivery::Delivered);
        let post = fake.posts().remove(0);
        assert!(post.contains("/sync/history?"), "{post}");
        assert!(post.contains(r#""seasons":[{"number":1,"episodes":[{"number":2,"watched_at":"#), "{post}");
        assert!(post.contains(r#""year":2011"#), "a series' span gives the year it started: {post}");
        assert!(
            !post.contains(r#""shows":[{"ids":{"imdb":"tt1"},"title":"A Show","year":2011,"watched_at""#),
            "the show carries no time: {post}"
        );
    }

    #[test]
    fn an_unmarked_title_is_removed_from_the_history() {
        let fake = Fake::delivered();
        assert_eq!(sync(&fake).deliver(&movie(TrackedWatch::Unwatched)).unwrap(), Delivery::Delivered);
        let post = fake.posts().remove(0);
        assert!(post.contains("/sync/history/remove?"), "{post}");
        assert!(!post.contains("watched_at"), "a removal carries no time: {post}");
    }

    #[test]
    fn a_half_watched_episode_is_a_paused_playback_without_looking_the_id_up() {
        let fake = Fake::delivered();
        let event = episode(TrackedWatch::InProgress { position_ms: 1_500_000, duration_ms: 6_000_000 });
        assert_eq!(sync(&fake).deliver(&event).unwrap(), Delivery::Delivered);
        let posts = fake.posts();
        assert_eq!(posts.len(), 1, "one write, no /search/id round-trip (Simkl resolves the id itself): {posts:?}");
        assert!(posts[0].contains("/scrobble/stop?"), "{}", posts[0]);
        assert!(posts[0].contains(r#""progress":25"#), "{}", posts[0]);
        assert!(posts[0].contains(r#""imdb":"tt1""#), "{}", posts[0]);
        assert!(posts[0].contains(r#""episode":{"season":1,"number":2}"#), "{}", posts[0]);
    }

    #[test]
    fn a_half_watched_film_stops_at_a_pause_and_never_reaches_watched() {
        let fake = Fake::delivered();
        let event = movie(TrackedWatch::InProgress { position_ms: 5_900_000, duration_ms: 6_000_000 });
        assert_eq!(sync(&fake).deliver(&event).unwrap(), Delivery::Delivered);
        let posts = fake.posts();
        assert_eq!(posts.len(), 1);
        assert!(posts[0].contains(r#""progress":79"#), "simkl marks watched at 80: {}", posts[0]);
        assert!(!posts[0].contains("/sync/history"), "a pause is not a watch event");
    }

    #[test]
    fn a_title_simkl_does_not_know_is_not_retried() {
        let fake = Fake::new(201, r#"{"not_found":{"movies":[{"title":"A Film"}],"shows":[],"episodes":[]}}"#);
        assert!(matches!(sync(&fake).deliver(&movie(TrackedWatch::Watched)).unwrap(), Delivery::Unmatched(_)));
    }

    #[test]
    fn an_already_recorded_watch_is_delivered_and_a_fault_is_a_retry() {
        let duplicate = Fake::new(409, r#"{"watched_at":"2026-05-01T18:00:00-05:00"}"#);
        assert_eq!(
            sync(&duplicate).deliver(&movie(TrackedWatch::Watched)).unwrap(),
            Delivery::Delivered,
            "already there is what we wanted"
        );
        let limited = Fake::new(429, r#"{}"#);
        assert!(matches!(sync(&limited).deliver(&movie(TrackedWatch::Watched)).unwrap(), Delivery::Retry(_)));
        let fault = Fake::new(503, r#"{}"#);
        assert!(matches!(sync(&fault).deliver(&movie(TrackedWatch::Watched)).unwrap(), Delivery::Retry(_)));
        let malformed = Fake::new(400, r#"{"error":"invalid_request"}"#);
        assert!(matches!(sync(&malformed).deliver(&movie(TrackedWatch::Watched)).unwrap(), Delivery::Unmatched(_)));
    }

    #[test]
    fn a_dead_token_stops_the_run_and_says_what_to_do() {
        let fake = Fake::new(401, r#"{"error":"user_token_failed"}"#);
        let error = sync(&fake).deliver(&movie(TrackedWatch::Watched)).unwrap_err();
        assert!(format!("{error:#}").contains("sign in again"), "{error:#}");
        assert!(!format!("{error:#}").contains("token-1"), "the token never reaches a message");
        assert!(http_status(&error).is_none(), "this is not a status the outbox retries blindly");
    }

    #[test]
    fn a_series_without_a_season_cannot_be_paused() {
        let fake = Fake::delivered();
        let mut event = episode(TrackedWatch::InProgress { position_ms: 1_000, duration_ms: 2_000 });
        event.view = TrackedView::Movie;
        // A film has no season, but a length of zero has nothing to send.
        let event = TrackEvent { watch: TrackedWatch::InProgress { position_ms: 0, duration_ms: 0 }, ..event };
        assert!(matches!(sync(&fake).deliver(&event).unwrap(), Delivery::Unmatched(_)));
        assert!(fake.posts().is_empty(), "nothing is sent for a length nobody knows");
    }

    #[test]
    fn the_tracker_names_itself() {
        let fake = Fake::delivered();
        let tracker = SimklTracker::new(&fake, "client-1", "token-1");
        assert_eq!(tracker.id(), "simkl");
        assert_eq!(tracker.name(), "Simkl");
        assert_eq!(tracker.deliver(&movie(TrackedWatch::Watched)).unwrap(), Delivery::Delivered);
    }

    #[test]
    fn a_refused_scrobble_id_is_permanent() {
        let fake = Fake::new(404, r#"{"error":"id_err"}"#);
        let event = movie(TrackedWatch::InProgress { position_ms: 1_000, duration_ms: 2_000 });
        assert!(matches!(sync(&fake).deliver(&event).unwrap(), Delivery::Unmatched(_)));
    }
}
