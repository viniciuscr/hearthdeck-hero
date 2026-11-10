//! Stremio's datastore: `libraryItem` records and watch progress.
//!
//! Reading is verified against a real account. Writing is probe-gated: the
//! document is replaced whole, so every write reads first and merges into the
//! full item (see `docs/MOVIES.md`).

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::auth::AuthKey;
use crate::endpoints::Endpoints;
use crate::http::Transport;
use crate::types::{MovieProgress, PosterShape};

pub const COLLECTION: &str = "libraryItem";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub poster_shape: PosterShape,
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub temp: bool,
    #[serde(rename = "_ctime", default)]
    pub ctime: Option<DateTime<Utc>>,
    #[serde(rename = "_mtime", default)]
    pub mtime: Option<DateTime<Utc>>,
    #[serde(default)]
    pub state: LibraryItemState,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItemState {
    #[serde(default)]
    pub last_watched: Option<DateTime<Utc>>,
    #[serde(default)]
    pub time_watched: u64,
    #[serde(default)]
    pub time_offset: u64,
    #[serde(default)]
    pub overall_time_watched: u64,
    #[serde(default)]
    pub times_watched: u32,
    #[serde(default)]
    pub flagged_watched: u32,
    #[serde(default)]
    pub duration: u64,
    #[serde(default, rename = "video_id")]
    pub video_id: Option<String>,
    #[serde(default)]
    pub watched: Option<String>,
    #[serde(default, rename = "last_vid_released")]
    pub last_vid_released: Option<DateTime<Utc>>,
    #[serde(default)]
    pub no_notif: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub struct DatastoreClient<'a> {
    http: &'a dyn Transport,
    endpoints: &'a Endpoints,
    key: AuthKey,
}

impl<'a> DatastoreClient<'a> {
    pub fn new(http: &'a dyn Transport, endpoints: &'a Endpoints, key: AuthKey) -> Self {
        Self { http, endpoints, key }
    }

    pub fn get_all(&self) -> Result<Vec<LibraryItem>> {
        let result = self.call("datastoreGet", json!({"authKey": &self.key, "collection": COLLECTION, "ids": [], "all": true}))?;
        serde_json::from_value(result).context("Stremio libraryItem result")
    }

    /// `(id, epoch_ms)` pairs; cheap change detection before a full read.
    pub fn meta(&self) -> Result<Vec<(String, i64)>> {
        let result = self.call("datastoreMeta", json!({"authKey": &self.key, "collection": COLLECTION}))?;
        serde_json::from_value(result).context("Stremio libraryItem meta")
    }

    /// Replaces the given items. The caller sends full records, never a patch.
    pub fn put(&self, changes: &[LibraryItem]) -> Result<()> {
        let _ = self.call("datastorePut", json!({"authKey": &self.key, "collection": COLLECTION, "changes": changes}))?;
        Ok(())
    }

    fn call(&self, method: &str, body: Value) -> Result<Value> {
        let url = self.endpoints.api.join(method).with_context(|| format!("building {method} url"))?;
        let bytes = self.http.post_json(&url, &body)?;
        let value: Value = serde_json::from_slice(&bytes).with_context(|| format!("Stremio {method} answered non-JSON"))?;
        if let Some(error) = value.get("error") {
            bail!("Stremio {method}: {}", error_text(error));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }
}

fn error_text(error: &Value) -> String {
    error.get("message").and_then(Value::as_str).unwrap_or("request failed").to_owned()
}

/// Stremio's Continue Watching predicate and order.
pub fn continue_watching(items: &[LibraryItem]) -> Vec<&LibraryItem> {
    let mut out: Vec<&LibraryItem> =
        items.iter().filter(|item| item.kind != "other" && (!item.removed || item.temp) && item.state.time_offset > 0).collect();
    out.sort_by(|a, b| b.state.last_watched.cmp(&a.state.last_watched).then_with(|| b.mtime.cmp(&a.mtime)));
    out
}

/// Merges a local position into a full record without touching anything else.
pub fn merge_progress(item: &mut LibraryItem, progress: &MovieProgress, now: DateTime<Utc>) {
    item.state.time_offset = progress.position_ms;
    item.state.duration = progress.duration_ms;
    item.state.last_watched = Some(now);
    if let Some(video_id) = &progress.video_id {
        item.state.video_id = Some(video_id.clone());
    } else if item.state.video_id.is_none() {
        item.state.video_id = Some(item.id.clone());
    }
    item.mtime = Some(now);
}

/// A new `temp` record for a title played but not saved to the library.
pub fn new_progress_item(id: &str, name: &str, kind: &str, poster: Option<String>, now: DateTime<Utc>) -> LibraryItem {
    LibraryItem {
        id: id.to_owned(),
        name: name.to_owned(),
        kind: kind.to_owned(),
        poster,
        removed: true,
        temp: true,
        ctime: Some(now),
        mtime: Some(now),
        state: LibraryItemState { last_watched: Some(now), ..Default::default() },
        ..Default::default()
    }
}

/// Reads the current record, merges progress, and returns the full item to put.
pub fn progress_changes(client: &DatastoreClient<'_>, progress: &MovieProgress) -> Result<Vec<LibraryItem>> {
    let items = client.get_all()?;
    let now = Utc::now();
    let mut item = match items.into_iter().find(|item| item.id == progress.id) {
        Some(item) => item,
        None => new_progress_item(&progress.id, &progress.id, "movie", None, now),
    };
    merge_progress(&mut item, progress, now);
    Ok(vec![item])
}

/// Reads, merges and writes one record. On failure the caller keeps the local
/// progress and retries later; nothing is deleted or partially updated.
pub fn write_progress(client: &DatastoreClient<'_>, progress: &MovieProgress) -> Result<LibraryItem> {
    let changes = progress_changes(client, progress)?;
    client.put(&changes)?;
    changes.into_iter().next().ok_or_else(|| anyhow!("progress produced no change"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// Nothing in the library; there is no record to send unchanged.
    Empty,
    /// The full record was accepted by `datastorePut` unchanged.
    Verified { items: usize },
}

/// Proves the write envelope by sending the first existing record unchanged.
/// Do this once before the first real progress write (docs/MOVIES.md).
pub fn probe_put(client: &DatastoreClient<'_>) -> Result<Probe> {
    let items = client.get_all()?;
    let Some(first) = items.first() else {
        return Ok(Probe::Empty);
    };
    client.put(std::slice::from_ref(first))?;
    Ok(Probe::Verified { items: 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(json: &str) -> LibraryItem {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let original = item(
            r#"{"_id":"tt1","name":"A Film","type":"movie","poster":"p","posterShape":"poster",
                "removed":true,"temp":true,"_ctime":"2026-01-01T00:00:00Z","_mtime":"2026-01-02T00:00:00Z",
                "state":{"timeOffset":1000,"duration":2000,"video_id":"tt1","noNotif":false,"futureState":7},
                "futureTop":true}"#,
        );
        let text = serde_json::to_string(&original).unwrap();
        assert!(text.contains("\"futureTop\":true"), "{text}");
        assert!(text.contains("\"futureState\":7"), "{text}");
        assert_eq!(serde_json::from_str::<LibraryItem>(&text).unwrap(), original);
    }

    #[test]
    fn continue_watching_includes_temp_and_excludes_finished() {
        let items = vec![
            item(
                r#"{"_id":"a","name":"A","type":"movie","removed":true,"temp":true,"_mtime":"2026-01-03T00:00:00Z","state":{"timeOffset":1000}}"#,
            ),
            item(
                r#"{"_id":"b","name":"B","type":"movie","removed":true,"temp":false,"_mtime":"2026-01-02T00:00:00Z","state":{"timeOffset":1000}}"#,
            ),
            item(
                r#"{"_id":"c","name":"C","type":"movie","removed":false,"temp":false,"_mtime":"2026-01-01T00:00:00Z","state":{"timeOffset":0}}"#,
            ),
        ];
        let ids: Vec<&str> = continue_watching(&items).iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["a"]);
    }

    #[test]
    fn merge_progress_preserves_other_state() {
        let mut original = item(
            r#"{"_id":"tt1","name":"A","type":"movie","_mtime":"2026-01-01T00:00:00Z",
                "state":{"timeOffset":10,"duration":20,"timesWatched":3,"flaggedWatched":1,"futureState":7}}"#,
        );
        let progress = MovieProgress {
            id: "tt1".into(),
            position_ms: 5000,
            duration_ms: 10_000,
            updated_at: Utc::now(),
            finished: false,
            video_id: None,
        };
        merge_progress(&mut original, &progress, Utc::now());
        assert_eq!(original.state.time_offset, 5000);
        assert_eq!(original.state.duration, 10_000);
        assert_eq!(original.state.times_watched, 3);
        assert_eq!(original.state.flagged_watched, 1);
        assert_eq!(original.state.extra.get("futureState"), Some(&json!(7)));
        assert_eq!(original.state.video_id.as_deref(), Some("tt1"));
    }

    #[test]
    fn a_failed_envelope_is_an_error() {
        let text = r#"{"error":{"code":1,"message":"Session does not exist"}}"#;
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(error_text(value.get("error").unwrap()), "Session does not exist");
        assert!(anyhow::Error::msg(error_text(value.get("error").unwrap())).to_string().contains("Session does not exist"));
    }
}
