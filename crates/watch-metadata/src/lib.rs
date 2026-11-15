//! The metadata sources Watch ships with, in the order they are asked.
//!
//! A metadata source says **what a title is**. It is not a media provider:
//! Stremio, YouTube and the like say where a title comes from and how to play
//! it ([`WatchProvider`]), and nothing here is ever given an account, a stream
//! or a player. The two have their own registries, and later their own
//! settings, so a new description source never touches the provider story.
//!
//! The sources answer through the addon protocol (`heronite-watch-addon`):
//! Cinemeta is Stremio's own `meta` addon, and a second source can point at
//! another addon's manifest the same way.
//!
//! [`WatchProvider`]: heronite_watch_core::provider::WatchProvider

use anyhow::Result;
use heronite_watch_core::metadata::{MetaCtx, TitleDetails, TitleRef, WatchMetadataSource};

pub mod cinemeta;

static CINEMETA: cinemeta::Cinemeta = cinemeta::Cinemeta::builtin();
static SOURCES: [&dyn WatchMetadataSource; 1] = [&CINEMETA];

/// Every metadata source, in the order their answers are asked.
pub fn sources() -> &'static [&'static dyn WatchMetadataSource] {
    &SOURCES
}

/// The sources that can answer for titles of `provider`, in that order.
pub fn sources_for(sources: &[&'static dyn WatchMetadataSource], provider: &str) -> Vec<&'static dyn WatchMetadataSource> {
    sources.iter().copied().filter(|source| source.descriptor().covers(provider)).collect()
}

/// The sources named by `enabled`, in the order given.
///
/// An id this build does not have is skipped, so a settings file from another
/// version is not a fault — and a source that is off is never asked at all.
pub fn enabled(ids: &[String]) -> Vec<&'static dyn WatchMetadataSource> {
    ids.iter().filter_map(|id| sources().iter().copied().find(|source| source.descriptor().id == id.as_str())).collect()
}

/// The merged answer for `title` from `sources`, asked in order.
///
/// The first source to fill a field keeps it, a later one fills only what is
/// still empty, and the extras are joined with the first of each id winning — so
/// a source that knows more adds to the card, and one that knows less cannot take
/// away what another gave. A source that could not be asked (a timeout, a server
/// error) is set aside and the next is tried: a broken source must not hide the
/// others. When none answers, the first failure is what the caller sees.
pub fn details(sources: &[&'static dyn WatchMetadataSource], ctx: &MetaCtx<'_>, title: &TitleRef) -> Result<Option<TitleDetails>> {
    if !title.is_complete() {
        return Ok(None);
    }
    let mut merged: Option<TitleDetails> = None;
    let mut failure = None;
    for source in sources_for(sources, &title.provider) {
        match source.details(ctx, title) {
            Ok(Some(mut details)) => {
                details.source = source.descriptor().id.to_owned();
                merged = Some(match merged.take() {
                    Some(mut kept) => {
                        kept.merge(details);
                        kept
                    }
                    None => details,
                });
            }
            Ok(None) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    match merged {
        Some(details) => Ok(Some(details)),
        None => match failure {
            Some(error) => Err(error),
            None => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use heronite_watch_core::http::{HttpStatus, Transport};
    use heronite_watch_core::metadata::MetaDescriptor;
    use serde_json::Value;
    use url::Url;

    /// One fixed outcome for every GET, remembering what it was asked.
    enum Answer {
        Body(&'static str),
        Status(u16),
        Fault,
    }

    struct Fake {
        answer: Answer,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(answer: Answer) -> Self {
            Self { answer, seen: Mutex::new(Vec::new()) }
        }

        fn asked(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.as_str().to_owned());
            match self.answer {
                Answer::Body(body) => Ok(body.as_bytes().to_vec()),
                Answer::Status(status) => Err(HttpStatus::new(url, status).into()),
                Answer::Fault => Err(anyhow::anyhow!("connection reset by peer")),
            }
        }

        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            Err(anyhow::anyhow!("unexpected POST"))
        }
    }

    fn movie() -> TitleRef {
        TitleRef::new("stremio", "tt0133093", "movie").named("The Matrix", Some("1999"))
    }

    #[test]
    fn cinemeta_asks_its_own_meta_resource() {
        let fake = Fake::new(Answer::Body(r#"{"meta":{"id":"tt0133093","type":"movie","name":"The Matrix","imdbRating":"8.7"}}"#));
        let details = details(sources(), &MetaCtx::new(&fake), &movie()).unwrap().expect("the fixture answers");
        assert_eq!(
            fake.asked(),
            ["https://v3-cinemeta.strem.io/meta/movie/tt0133093.json"],
            "the descriptor's host and the addon protocol's path"
        );
        assert_eq!(details.name, "The Matrix");
        assert_eq!(details.rating.as_deref(), Some("8.7"));
        assert_eq!(details.source, "cinemeta", "the registry stamps the source that answered");
    }

    #[test]
    fn a_title_no_source_knows_is_none() {
        let fake = Fake::new(Answer::Status(404));
        assert_eq!(details(sources(), &MetaCtx::new(&fake), &movie()).unwrap(), None);
    }

    #[test]
    fn a_source_that_could_not_be_asked_is_an_error() {
        let fake = Fake::new(Answer::Status(503));
        let error = details(sources(), &MetaCtx::new(&fake), &movie()).unwrap_err();
        assert_eq!(heronite_watch_core::http::http_status(&error), Some(503));
        let fake = Fake::new(Answer::Fault);
        assert!(details(sources(), &MetaCtx::new(&fake), &movie()).is_err());
    }

    #[test]
    fn an_incomplete_reference_is_never_asked() {
        let fake = Fake::new(Answer::Body(r#"{"meta":null}"#));
        let mut title = movie();
        title.id.clear();
        assert_eq!(details(sources(), &MetaCtx::new(&fake), &title).unwrap(), None);
        assert!(fake.asked().is_empty());
    }

    #[test]
    fn the_registry_names_its_sources_and_what_they_cover() {
        let ids: Vec<&str> = sources().iter().map(|source| source.descriptor().id).collect();
        assert_eq!(ids, ["cinemeta"]);
        assert_eq!(sources_for(sources(), "stremio").len(), 1);
        assert_eq!(sources_for(sources(), "youtube").len(), 1, "an empty applies_to covers every media provider");
        let descriptor = MetaDescriptor { id: "x", name: "X", applies_to: &["stremio"] };
        assert!(descriptor.covers("stremio"));
        assert!(!descriptor.covers("youtube"));
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use heronite_watch_core::metadata::{Episode, Fact};

    struct Fixed(&'static str, TitleDetails);

    impl WatchMetadataSource for Fixed {
        fn descriptor(&self) -> &'static heronite_watch_core::metadata::MetaDescriptor {
            // Leaked on purpose: the registry holds sources for the process' life.
            Box::leak(Box::new(heronite_watch_core::metadata::MetaDescriptor { id: self.0, name: self.0, applies_to: &[] }))
        }

        fn details(&self, _: &MetaCtx<'_>, _: &TitleRef) -> Result<Option<TitleDetails>> {
            Ok(Some(self.1.clone()))
        }
    }

    #[test]
    fn two_sources_fill_one_card() {
        let rich: &'static dyn WatchMetadataSource = Box::leak(Box::new(Fixed(
            "rich",
            TitleDetails {
                id: "tt1".into(),
                kind: "series".into(),
                name: "A Series".into(),
                runtime: Some("50m".into()),
                episodes: vec![Episode { id: "tt1:1:1".into(), season: 1, number: 1, ..Episode::default() }],
                extras: vec![Fact { id: "network".into(), label: "Network".into(), value: "HBO".into(), source: "rich".into() }],
                ..TitleDetails::default()
            },
        )));
        let thin: &'static dyn WatchMetadataSource = Box::leak(Box::new(Fixed(
            "thin",
            TitleDetails {
                id: "tt1".into(),
                kind: "series".into(),
                name: "A Series".into(),
                runtime: Some("0m".into()),
                cast: vec!["Someone".into()],
                extras: vec![Fact { id: "tagline".into(), label: "Tagline".into(), value: "Oh".into(), source: "thin".into() }],
                ..TitleDetails::default()
            },
        )));
        let sources = vec![rich, thin];

        let merged = details(&sources, &MetaCtx::new(&NoNet), &TitleRef::new("stremio", "tt1", "series")).unwrap().expect("answered");
        assert_eq!(merged.source, "rich", "the record came from the first source that answered");
        assert_eq!(merged.runtime.as_deref(), Some("50m"), "the first answer keeps its field");
        assert_eq!(merged.cast, ["Someone"], "and the second fills what was empty");
        assert_eq!(merged.episodes.len(), 1, "one source's episode list, not two halves");
        assert_eq!(merged.extras.len(), 2, "both extras, the first of each id winning");
    }

    struct NoNet;

    impl heronite_watch_core::http::Transport for NoNet {
        fn get(&self, _: &url::Url) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn post_json(&self, _: &url::Url, _: &serde_json::Value) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }
}
