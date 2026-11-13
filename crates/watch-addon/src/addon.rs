//! Stremio addons: manifest, catalog, meta and stream resources.
//!
//! The addon protocol is HTTP. Cinemeta supplies catalogs and metadata;
//! Torrentio supplies streams. Transport URLs can carry the user's own
//! configuration, so they are never quoted in an error or a log line.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use heronite_watch_core::http::Transport;
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

    pub fn meta(&self, kind: &str, id: &str) -> Result<Option<MovieItem>> {
        let body = self.http.get(&self.base.join(&format!("meta/{kind}/{id}.json"))?)?;
        let value: Value = serde_json::from_slice(&body).context("addon meta")?;
        let Some(meta) = value.get("meta").filter(|meta| !meta.is_null()) else {
            return Ok(None);
        };
        let preview: MetaPreview = serde_json::from_value(meta.clone()).context("addon meta object")?;
        Ok(Some(preview.into_item(&self.id)))
    }

    /// A missing `streams` list is an error; an empty list is a valid answer.
    pub fn stream(&self, kind: &str, id: &str) -> Result<Vec<StreamChoice>> {
        let body = self.http.get(&self.base.join(&format!("stream/{kind}/{id}.json"))?)?;
        let value: Value = serde_json::from_slice(&body).context("addon stream")?;
        let streams = value.get("streams").ok_or_else(|| anyhow!("addon stream answered without a streams list"))?;
        let streams: Vec<Stream> = serde_json::from_value(streams.clone()).context("addon streams")?;
        Ok(streams.into_iter().map(|stream| stream.into_choice(&self.id)).collect())
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
}
