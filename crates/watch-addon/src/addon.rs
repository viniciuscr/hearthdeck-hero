//! Stremio addons: manifest, catalog, meta and stream resources.
//!
//! The addon protocol is HTTP. Cinemeta supplies catalogs and metadata;
//! Torrentio supplies streams. Transport URLs can carry the user's own
//! configuration, so they are never quoted in an error or a log line.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use heronite_watch_core::http::{http_status, redacted, Transport};
use heronite_watch_core::metadata::TitleDetails;
use heronite_watch_core::types::{MovieItem, StreamChoice};

/// Addons serialize optional lists as `null`, and `serde(default)` only
/// covers a missing field; make `null` mean the same.
pub fn null_as_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default, deserialize_with = "null_as_default")]
    pub id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub name: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub resources: Vec<Value>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub types: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub catalogs: Vec<Catalog>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub id_prefixes: Vec<String>,
}

impl Manifest {
    pub fn has_resource(&self, name: &str) -> bool {
        self.resources.iter().any(|resource| resource.as_str() == Some(name) || resource.get("name").and_then(Value::as_str) == Some(name))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    #[serde(rename = "type", default, deserialize_with = "null_as_default")]
    pub kind: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPreview {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub year: Option<Value>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub runtime: Option<Value>,
}

impl MetaPreview {
    pub fn into_item(self, addon: &str) -> MovieItem {
        MovieItem {
            id: self.id,
            kind: self.kind,
            name: self.name,
            year: text(self.year),
            poster: self.poster,
            background: self.background,
            logo: self.logo,
            description: self.description,
            runtime: self.runtime,
            genres: self.genres,
            addon: addon.to_owned(),
            ..MovieItem::default()
        }
    }
}

/// One `videos` entry of a series' meta. Only the season and episode numbers
/// are read: they are counted, never listed.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MetaVideo {
    #[serde(default)]
    pub season: Option<u32>,
}

/// The addon protocol's `meta` object: what a `meta/{type}/{id}.json` answers.
/// Addons send `null` for what they do not know, and Cinemeta joins its people
/// and countries with `, `.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaDetails {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub year: Option<Value>,
    #[serde(default)]
    pub release_info: Option<Value>,
    #[serde(default)]
    pub released: Option<Value>,
    #[serde(default)]
    pub runtime: Option<Value>,
    #[serde(default)]
    pub imdb_rating: Option<Value>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub genres: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub genre: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub director: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub writer: Vec<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub cast: Vec<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub awards: Option<String>,
    #[serde(default)]
    pub status: Option<Value>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub videos: Vec<MetaVideo>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
}

impl MetaDetails {
    /// The wire object as the screens' details type, `None` without a name.
    /// The id and kind are the requested ones: they key the cache and the
    /// record, and an addon that omits them must not break that. `source` is
    /// left to the registry, which knows which source answered.
    pub fn into_details(self, id: &str, kind: &str) -> Option<TitleDetails> {
        let name = self.name.filter(|name| !name.trim().is_empty())?;
        let genres = if self.genres.is_empty() { self.genre } else { self.genres };
        // Season 0 is the specials: counted neither as a season nor an episode.
        let seasons: Vec<u32> = self.videos.iter().filter_map(|video| video.season.filter(|s| *s > 0)).collect();
        let seasons_count = seasons.iter().copied().collect::<std::collections::BTreeSet<_>>().len();
        Some(TitleDetails {
            source: String::new(),
            id: id.to_owned(),
            kind: kind.to_owned(),
            name,
            year: text(self.release_info).or_else(|| text(self.year)),
            released: text(self.released).and_then(|date| day_of(&date)),
            runtime: text(self.runtime),
            rating: text(self.imdb_rating),
            description: self.description.filter(|text| !text.trim().is_empty()),
            genres: clean(genres),
            directors: clean(self.director),
            writers: clean(self.writer),
            cast: clean(self.cast),
            countries: split_names(self.country.as_deref()),
            awards: self.awards.filter(|awards| !awards.trim().is_empty()),
            series_status: text(self.status),
            seasons: (seasons_count > 0).then_some(seasons_count as u32),
            episodes: (!seasons.is_empty()).then_some(seasons.len() as u32),
            poster: self.poster,
            background: self.background,
            logo: self.logo,
        })
    }
}

/// `1999-03-31T00:00:00.000Z` as `1999-03-31`; a date alone is unchanged.
fn day_of(text: &str) -> Option<String> {
    let day = text.split('T').next().unwrap_or(text).trim();
    (!day.is_empty()).then(|| day.to_owned())
}

/// The names as the source lists them, without blanks.
fn clean(names: Vec<String>) -> Vec<String> {
    names.into_iter().map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()).collect()
}

/// `"United States, United Kingdom"` as two names.
fn split_names(list: Option<&str>) -> Vec<String> {
    list.map(|list| clean(list.split(',').map(str::to_owned).collect())).unwrap_or_default()
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, alias = "title")]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub info_hash: Option<String>,
    #[serde(default)]
    pub file_idx: Option<u16>,
    #[serde(default)]
    pub behavior_hints: Value,
}

impl Stream {
    pub fn into_choice(self, addon: &str) -> StreamChoice {
        let text = self.description.clone().unwrap_or_default();
        let size_bytes = parse_size(&text);
        let seeders = parse_seeders(&text);
        StreamChoice {
            addon: addon.to_owned(),
            name: self.name,
            description: self.description,
            url: self.url,
            info_hash: self.info_hash,
            file_idx: self.file_idx,
            behavior_hints: self.behavior_hints,
            size_bytes,
            seeders,
        }
    }
}

/// What an addon answered for a stream request.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamAnswer {
    /// The addon serves this title; the list may be empty.
    Streams(Vec<StreamChoice>),
    /// The addon answered 404: it does not serve this title. That is not a
    /// fault, and it must not hide the streams of the other addons.
    NotServed,
}

pub struct AddonClient<'a> {
    http: &'a dyn Transport,
    base: Url,
    id: String,
}

impl<'a> AddonClient<'a> {
    /// `transport_url` is the addon's manifest URL (or its base).
    pub fn new(http: &'a dyn Transport, transport_url: &str) -> Result<Self> {
        let url = Url::parse(transport_url).with_context(|| "addon transport URL")?;
        Ok(Self { http, base: base_url(url)?, id: String::new() })
    }

    pub fn manifest(&mut self) -> Result<Manifest> {
        let body = self.http.get(&self.base.join("manifest.json")?)?;
        let manifest: Manifest = serde_json::from_slice(&body).context("addon manifest")?;
        self.id.clone_from(&manifest.id);
        Ok(manifest)
    }

    pub fn catalog(&self, kind: &str, id: &str, extra: &[(&str, &str)]) -> Result<Vec<MovieItem>> {
        let mut url = self.base.join(&format!("catalog/{kind}/{id}.json"))?;
        if !extra.is_empty() {
            let mut query = url.query_pairs_mut();
            for (name, value) in extra {
                query.append_pair(name, value);
            }
        }
        let body = self.http.get(&url)?;
        let value: Value = serde_json::from_slice(&body).context("addon catalog")?;
        let metas = value.get("metas").ok_or_else(|| anyhow!("addon catalog answered without metas"))?;
        let previews: Vec<MetaPreview> = serde_json::from_value(metas.clone()).context("addon catalog metas")?;
        Ok(previews.into_iter().map(|preview| preview.into_item(&self.id)).collect())
    }

    /// What the addon knows about one title, or `None` when it does not serve
    /// it (a 404). Any other answer is a fault: a body without `meta` is not
    /// the same as a title the addon does not have.
    pub fn details(&self, kind: &str, id: &str) -> Result<Option<TitleDetails>> {
        let body = match self.http.get(&self.base.join(&format!("meta/{kind}/{id}.json"))?) {
            Ok(body) => body,
            Err(error) if http_status(&error) == Some(404) => return Ok(None),
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_slice(&body).context("addon meta")?;
        let Some(meta) = value.get("meta").filter(|meta| !meta.is_null()) else {
            return Ok(None);
        };
        let meta: MetaDetails = serde_json::from_value(meta.clone()).context("addon meta object")?;
        Ok(meta.into_details(id, kind))
    }

    /// A missing `streams` list is an error; an empty list is a valid answer.
    /// A 404 is neither: it means the addon does not serve this title.
    pub fn stream(&self, kind: &str, id: &str) -> Result<StreamAnswer> {
        let body = match self.http.get(&self.base.join(&format!("stream/{kind}/{id}.json"))?) {
            Ok(body) => body,
            Err(error) if http_status(&error) == Some(404) => return Ok(StreamAnswer::NotServed),
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_slice(&body).context("addon stream")?;
        let streams = value.get("streams").ok_or_else(|| anyhow!("addon stream answered without a streams list"))?;
        let streams: Vec<Stream> = serde_json::from_value(streams.clone()).context("addon streams")?;
        Ok(StreamAnswer::Streams(streams.into_iter().map(|stream| stream.into_choice(&self.id)).collect()))
    }

    /// `scheme://host/…`: what a log line may name. The path is where the
    /// person's configuration, and possibly a debrid key, waits.
    pub fn host(&self) -> String {
        redacted(&self.base)
    }
}

/// Strips `manifest.json`, keeps a configured path, and leaves a trailing `/`
/// so `Url::join` replaces the last segment correctly.
fn base_url(mut url: Url) -> Result<Url> {
    let path = url.path().to_owned();
    if let Some(stripped) = path.strip_suffix("manifest.json") {
        url.set_path(stripped);
    }
    if !url.path().ends_with('/') {
        let mut path = url.path().to_owned();
        path.push('/');
        url.set_path(&path);
    }
    Ok(url)
}

fn text(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

/// `💾 1.2 GB` or `1.2 GB`; the description format is addon-owned.
fn parse_size(text: &str) -> Option<u64> {
    let start = text.find('💾').map(|i| i + '💾'.len_utf8()).unwrap_or(0);
    let tail = &text[start..];
    let number_start = tail.find(|c: char| c.is_ascii_digit())?;
    let tail = &tail[number_start..];
    let number_len = tail.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(tail.len());
    let number: f64 = tail[..number_len].parse().ok()?;
    let unit = tail[number_len..].trim_start();
    let multiplier = if unit.starts_with("TB") {
        1_000_000_000_000.0
    } else if unit.starts_with("GB") {
        1_000_000_000.0
    } else if unit.starts_with("MB") {
        1_000_000.0
    } else if unit.starts_with("KB") {
        1_000.0
    } else {
        1.0
    };
    Some((number * multiplier) as u64)
}

/// `👤 123`; Torrentio puts the peers after the person glyph.
fn parse_seeders(text: &str) -> Option<u32> {
    let start = text.find('👤').map(|i| i + '👤'.len_utf8()).unwrap_or(0);
    let tail = &text[start..];
    let tail = tail.trim_start_matches(|c: char| !c.is_ascii_digit());
    let len = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
    tail[..len].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch_core::http::HttpStatus;

    #[test]
    fn a_configured_transport_keeps_its_path() {
        let base = base_url(Url::parse("https://torrentio.strem.fun/providers=yts/debrid=realdebrid/manifest.json").unwrap()).unwrap();
        assert_eq!(base.as_str(), "https://torrentio.strem.fun/providers=yts/debrid=realdebrid/");
    }

    #[test]
    fn a_bare_host_gets_a_trailing_slash() {
        let base = base_url(Url::parse("https://torrentio.strem.fun").unwrap()).unwrap();
        assert_eq!(base.as_str(), "https://torrentio.strem.fun/");
    }

    #[test]
    fn descriptions_yield_size_and_seeders() {
        assert_eq!(parse_size("Torrentio\n1080p\n👤 123 💾 1.5 GB"), Some(1_500_000_000));
        assert_eq!(parse_seeders("Torrentio\n1080p\n👤 123 💾 1.5 GB"), Some(123));
        assert_eq!(parse_size("no size"), None);
    }

    #[test]
    fn a_manifest_can_declare_streams_as_a_string_or_an_object() {
        let manifest: Manifest =
            serde_json::from_str(r#"{"id":"x","name":"X","resources":["catalog",{"name":"stream","types":["movie"]}],"types":["movie"]}"#)
                .unwrap();
        assert!(manifest.has_resource("catalog"));
        assert!(manifest.has_resource("stream"));
        assert!(!manifest.has_resource("meta"));
    }

    #[test]
    fn a_manifest_reads_null_lists_as_empty() {
        let manifest: Manifest =
            serde_json::from_str(r#"{"id":null,"name":null,"resources":null,"types":null,"catalogs":null,"idPrefixes":null}"#).unwrap();
        assert!(manifest.resources.is_empty());
        assert!(manifest.types.is_empty());
        assert!(manifest.catalogs.is_empty());
        assert!(manifest.id_prefixes.is_empty());
        assert!(!manifest.has_resource("stream"));
    }

    /// One fixed outcome for every GET, so a test names what the addon
    /// answered rather than which URL it asked.
    enum Answer {
        Body(&'static str),
        Status(u16),
        Fault,
    }

    struct Stub(Answer);

    impl Transport for Stub {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            match self.0 {
                Answer::Body(body) => Ok(body.as_bytes().to_vec()),
                Answer::Status(status) => Err(HttpStatus::new(url, status).into()),
                Answer::Fault => Err(anyhow!("connection reset by peer")),
            }
        }

        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            Err(anyhow!("unexpected POST"))
        }
    }

    fn client(transport: &Stub) -> AddonClient<'_> {
        let mut client = AddonClient::new(transport, "https://addon.example/debrid=key/manifest.json").unwrap();
        client.id = "addon".into();
        client
    }

    #[test]
    fn a_404_means_the_addon_does_not_serve_the_title() {
        let transport = Stub(Answer::Status(404));
        assert_eq!(client(&transport).stream("movie", "tt0133093").unwrap(), StreamAnswer::NotServed);
    }

    #[test]
    fn a_500_is_still_a_fault_with_its_status() {
        let transport = Stub(Answer::Status(500));
        let error = client(&transport).stream("movie", "tt0133093").unwrap_err();
        assert_eq!(http_status(&error), Some(500));
        assert!(!format!("{error:#}").contains("debrid=key"), "{error:#}");
    }

    #[test]
    fn a_connection_failure_is_a_fault() {
        let transport = Stub(Answer::Fault);
        assert!(client(&transport).stream("movie", "tt0133093").is_err());
    }

    #[test]
    fn a_body_without_a_streams_list_is_a_fault() {
        let transport = Stub(Answer::Body(r#"{"metas":[]}"#));
        let error = client(&transport).stream("movie", "tt0133093").unwrap_err();
        assert!(format!("{error:#}").contains("without a streams list"), "{error:#}");
    }

    #[test]
    fn a_body_with_streams_is_an_answer() {
        let transport =
            Stub(Answer::Body(r#"{"streams":[{"title":"1080p","infoHash":"dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c","fileIdx":0}]}"#));
        let StreamAnswer::Streams(streams) = client(&transport).stream("movie", "tt0133093").unwrap() else {
            panic!("a body with streams is a stream answer");
        };
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].addon, "addon");
        assert_eq!(streams[0].file_idx, Some(0));
    }

    fn details(transport: &Stub, kind: &str, id: &str) -> Option<TitleDetails> {
        client(transport).details(kind, id).unwrap()
    }

    #[test]
    fn a_movie_meta_becomes_the_pages_details() {
        let transport = Stub(Answer::Body(include_str!("../tests/fixtures/meta_movie.json")));
        let details = details(&transport, "movie", "tt0133093").expect("the fixture has a meta");
        assert_eq!(details.id, "tt0133093");
        assert_eq!(details.kind, "movie");
        assert_eq!(details.name, "The Matrix");
        assert_eq!(details.year.as_deref(), Some("1999"));
        assert_eq!(details.released.as_deref(), Some("1999-03-31"));
        assert_eq!(details.runtime.as_deref(), Some("136 min"));
        assert_eq!(details.rating.as_deref(), Some("8.7"));
        assert_eq!(details.genres, ["Action", "Sci-Fi"]);
        assert_eq!(details.directors, ["Lana Wachowski", "Lilly Wachowski"]);
        assert_eq!(details.writers.len(), 2);
        assert_eq!(details.cast.len(), 3);
        assert_eq!(details.countries, ["United States"]);
        assert!(details.awards.as_deref().is_some_and(|awards| awards.contains("Oscars")));
        assert_eq!(details.series_status, None);
        assert_eq!(details.seasons, None);
        assert_eq!(details.episodes, None);
    }

    #[test]
    fn a_series_counts_its_seasons_and_episodes_without_the_specials() {
        let transport = Stub(Answer::Body(include_str!("../tests/fixtures/meta_series.json")));
        let details = details(&transport, "series", "tt0944947").expect("the fixture has a meta");
        assert_eq!(details.year.as_deref(), Some("2011–2019"));
        assert_eq!(details.series_status.as_deref(), Some("Ended"));
        // Season 0 holds the specials: neither a season nor an episode here.
        assert_eq!(details.seasons, Some(2));
        assert_eq!(details.episodes, Some(3));
        assert_eq!(details.countries, ["United States", "United Kingdom"]);
        assert!(details.directors.is_empty());
    }

    #[test]
    fn a_body_without_meta_is_not_an_answer() {
        let transport = Stub(Answer::Body(r#"{"meta":null}"#));
        assert_eq!(details(&transport, "movie", "tt1"), None);
        let transport = Stub(Answer::Body(r#"{"meta":{"id":"tt1","type":"movie"}}"#));
        assert_eq!(details(&transport, "movie", "tt1"), None, "a meta without a name says nothing");
    }

    #[test]
    fn a_404_on_meta_means_the_addon_does_not_know_the_title() {
        let transport = Stub(Answer::Status(404));
        assert_eq!(details(&transport, "movie", "tt1"), None);
    }

    #[test]
    fn a_500_on_meta_is_a_fault_with_its_status() {
        let transport = Stub(Answer::Status(500));
        let error = client(&transport).details("movie", "tt1").unwrap_err();
        assert_eq!(http_status(&error), Some(500));
    }
}
