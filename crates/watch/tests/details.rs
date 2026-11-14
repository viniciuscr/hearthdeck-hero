//! The metadata facade: what a title is, from the sources and their cache.
//! No test here reaches the network.

use std::sync::Mutex;

use anyhow::{anyhow, Result};
use serde_json::Value;
use url::Url;

use heronite_watch::metadata::{details_with, sources, TitleRef};
use heronite_watch_core::http::{http_status, HttpStatus, Transport};
use heronite_watch_core::metadata::TitleDetails;
use heronite_watch_state::details;

const META: &str = r#"{"meta":{"id":"tt0133093","type":"movie","name":"The Matrix","imdbRating":"8.7"}}"#;

#[derive(Clone, Copy)]
enum Answer {
    Body(&'static str),
    Status(u16),
    Fault,
}

/// Answers with each of `answers` in turn, repeating the last one.
struct Fake {
    answers: Mutex<Vec<Answer>>,
    seen: Mutex<usize>,
}

impl Fake {
    fn new(answers: Vec<Answer>) -> Self {
        Self { answers: Mutex::new(answers), seen: Mutex::new(0) }
    }

    fn asked(&self) -> usize {
        *self.seen.lock().unwrap()
    }
}

impl Transport for Fake {
    fn get(&self, url: &Url) -> Result<Vec<u8>> {
        *self.seen.lock().unwrap() += 1;
        let answer = {
            let mut answers = self.answers.lock().unwrap();
            match answers.len() {
                0 => return Err(anyhow!("no answer queued")),
                1 => answers[0],
                _ => answers.remove(0),
            }
        };
        match answer {
            Answer::Body(body) => Ok(body.as_bytes().to_vec()),
            Answer::Status(status) => Err(HttpStatus::new(url, status).into()),
            Answer::Fault => Err(anyhow!("connection reset by peer")),
        }
    }

    fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
        Err(anyhow!("unexpected POST"))
    }
}

fn app(root: &std::path::Path) -> heronite_data::paths::AppPaths {
    heronite_data::paths::AppPaths { config: root.join("config"), cache: root.join("cache") }
}

fn movie() -> TitleRef {
    TitleRef::new("stremio", "tt0133093", "movie").named("The Matrix", Some("1999"))
}

#[test]
fn an_answer_is_cached_and_the_next_open_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let http = Fake::new(vec![Answer::Body(META)]);
    let first = details_with(&http, &app, &movie()).unwrap().expect("the fixture answers");
    assert_eq!(first.name, "The Matrix");
    assert_eq!(first.rating.as_deref(), Some("8.7"));
    assert_eq!(first.source, "cinemeta");

    let second = details_with(&http, &app, &movie()).unwrap().expect("the cache answers");
    assert_eq!(second, first);
    assert_eq!(http.asked(), 1, "a fresh answer is not asked for again");
    assert!(details::read(&app.cache.join(details::ROOT), &movie()).is_some());
}

#[test]
fn a_stale_answer_is_served_when_the_source_cannot_be_asked() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let cached = TitleDetails { id: "tt0133093".into(), kind: "movie".into(), name: "The Matrix".into(), ..TitleDetails::default() };
    let fetched = chrono::Utc::now() - chrono::Duration::days(30);
    details::write(&app.cache.join(details::ROOT), &movie(), &cached, fetched).unwrap();

    let http = Fake::new(vec![Answer::Fault]);
    let answer = details_with(&http, &app, &movie()).unwrap().expect("the stale answer is kept");
    assert_eq!(answer.name, "The Matrix");
    assert_eq!(http.asked(), 1, "the stale answer is only served after asking");
}

#[test]
fn an_answer_without_a_cache_is_refreshed_and_written() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let old = TitleDetails { id: "tt0133093".into(), kind: "movie".into(), name: "Old name".into(), ..TitleDetails::default() };
    let fetched = chrono::Utc::now() - chrono::Duration::days(30);
    details::write(&app.cache.join(details::ROOT), &movie(), &old, fetched).unwrap();

    let http = Fake::new(vec![Answer::Body(META)]);
    let answer = details_with(&http, &app, &movie()).unwrap().expect("the source answers");
    assert_eq!(answer.name, "The Matrix", "a stale answer is replaced");
    let cached = details::read(&app.cache.join(details::ROOT), &movie()).expect("the new answer was written");
    assert!(cached.is_fresh(chrono::Utc::now()));
}

#[test]
fn a_title_no_source_knows_is_no_details_and_is_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let http = Fake::new(vec![Answer::Status(404)]);
    assert!(details_with(&http, &app, &movie()).unwrap().is_none());
    assert!(details::read(&app.cache.join(details::ROOT), &movie()).is_none());
}

#[test]
fn a_source_that_could_not_be_asked_is_an_error_without_a_cache() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let http = Fake::new(vec![Answer::Status(503)]);
    let error = details_with(&http, &app, &movie()).unwrap_err();
    assert_eq!(http_status(&error), Some(503));
}

#[test]
fn the_facade_publishes_the_metadata_sources_in_order() {
    let ids: Vec<&str> = sources().iter().map(|source| source.descriptor().id).collect();
    assert_eq!(ids, ["cinemeta"]);
}
