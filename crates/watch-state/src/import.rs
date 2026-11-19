//! The per-provider import cursor: `config/watch-import/<provider>.json`.
//!
//! What an incremental pull needs and nothing else — the provider's **opaque**
//! snapshot, when we last asked, the membership last seen in each list kind (the
//! base a removal is found against), and the last failure. No titles, no items:
//! deleting this file costs one full pull and loses nothing else.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use heronite_data::json_store;
use heronite_data::paths::AppPaths;

/// The folder under the config directory.
pub const DIR: &str = "watch-import";

/// One provider's cursor. Absent file = never pulled.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    /// The provider's own snapshot (Simkl's activities, Trakt's last_activities):
    /// stored verbatim, never interpreted here.
    #[serde(default)]
    pub snapshot: Value,
    /// When a pull last finished successfully.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pull_at: Option<DateTime<Utc>>,
    /// The membership last seen per list kind — the base a removal is found
    /// against, so "not there" is never read as "removed there".
    #[serde(default)]
    pub list_ids: BTreeMap<String, Vec<String>>,
    /// A newer build's fields survive a write here.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The folder the cursors live in.
pub fn dir(app: &AppPaths) -> PathBuf {
    app.config.join(DIR)
}

/// Where one provider's cursor is kept. The provider id is ours (`simkl`), but a
/// file from another build could hold anything: keep it to a file name.
pub fn path(app: &AppPaths, provider: &str) -> PathBuf {
    dir(app).join(format!("{}.json", component(provider)))
}

/// A path component that cannot leave the folder.
fn component(raw: &str) -> String {
    let cleaned: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    if cleaned.is_empty() {
        "_".to_owned()
    } else {
        cleaned
    }
}

/// The cursor as stored; missing or unreadable is "never pulled".
pub fn read(app: &AppPaths, provider: &str) -> Cursor {
    let path = path(app, provider);
    match json_store::read(&path) {
        Ok(cursor) => cursor,
        Err(error) => {
            if path.is_file() {
                log::warn!("{}: {error:#}", path.display());
            }
            Cursor::default()
        }
    }
}

/// Keeps `cursor` for `provider`.
pub fn write(app: &AppPaths, provider: &str, cursor: &Cursor) -> Result<()> {
    let path = path(app, provider);
    json_store::write(&path, cursor).with_context(|| format!("writing {}", path.display()))
}

/// Read-modify-write of one cursor, serialized per path by `json_store`.
pub fn update<R>(app: &AppPaths, provider: &str, f: impl FnOnce(&mut Cursor) -> Result<R>) -> Result<R> {
    let path = path(app, provider);
    json_store::update(&path, f).with_context(|| format!("updating {}", path.display()))
}

/// Forgets one provider's cursor (sign-out, or "pull it all again").
pub fn clear(app: &AppPaths, provider: &str) -> Result<()> {
    let path = path(app, provider);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &std::path::Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    #[test]
    fn a_cursor_round_trips_and_a_missing_one_is_empty() {
        let root = tempfile::tempdir().unwrap();
        let app = paths(root.path());
        assert_eq!(read(&app, "simkl"), Cursor::default(), "no file, never pulled");

        let cursor = Cursor {
            snapshot: serde_json::json!({"all": "2026-01-01T00:00:00Z"}),
            last_pull_at: Some(Utc::now()),
            list_ids: BTreeMap::from([("watchlist".to_owned(), vec!["tt1".to_owned(), "tt2".to_owned()])]),
            ..Cursor::default()
        };
        write(&app, "simkl", &cursor).unwrap();
        assert_eq!(read(&app, "simkl"), cursor);
        assert!(read(&app, "trakt").snapshot.is_null(), "each provider has its own file");
    }

    #[test]
    fn an_update_reads_and_writes_under_the_lock() {
        let root = tempfile::tempdir().unwrap();
        let app = paths(root.path());
        for n in [1, 2] {
            update(&app, "simkl", |cursor| {
                cursor.list_ids.insert(format!("k{n}"), vec![format!("tt{n}")]);
                Ok(())
            })
            .unwrap();
        }
        let cursor = read(&app, "simkl");
        assert_eq!(cursor.list_ids.len(), 2, "one update cannot lose the other");
    }

    #[test]
    fn clearing_forgets_one_provider_only() {
        let root = tempfile::tempdir().unwrap();
        let app = paths(root.path());
        write(&app, "simkl", &Cursor::default()).unwrap();
        write(&app, "trakt", &Cursor::default()).unwrap();
        clear(&app, "simkl").unwrap();
        assert!(!path(&app, "simkl").exists());
        assert!(path(&app, "trakt").exists(), "the other provider keeps its cursor");
    }

    #[test]
    fn a_provider_id_cannot_leave_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let app = paths(root.path());
        let escaped = path(&app, "../../steal");
        assert_eq!(escaped.parent(), Some(super::dir(&app).as_path()), "{escaped:?}");
    }
}
