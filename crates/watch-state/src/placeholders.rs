//! Titles an import knows that **no stream source has**: the "not currently
//! present in any available source" entries.
//!
//! They are library items, not watch records: they appear where titles appear,
//! greyed the way an uninstalled game is, and they leave when the library gains
//! the title. `watch-imported.json`, user data, hand-editable.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use heronite_data::json_store;
use heronite_data::paths::AppPaths;

/// The file, under the config directory.
pub const FILE: &str = "watch-imported.json";

/// One title a provider knows and this machine cannot play.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placeholder {
    /// The IMDb id: what every pull and every local record is matched by.
    pub imdb: String,
    /// `movie` or `series`.
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    /// The provider that brought it in.
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<DateTime<Utc>>,
}

/// The file's shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placeholders {
    #[serde(default)]
    pub items: Vec<Placeholder>,
    /// A newer build's fields survive a write here.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn path(app: &AppPaths) -> PathBuf {
    app.config.join(FILE)
}

/// The placeholders as stored; missing or unreadable is none.
pub fn read(app: &AppPaths) -> Placeholders {
    let path = path(app);
    match json_store::read(&path) {
        Ok(placeholders) => placeholders,
        Err(error) => {
            if path.is_file() {
                log::warn!("{}: {error:#}", path.display());
            }
            Placeholders::default()
        }
    }
}

/// Adds (or refreshes) one placeholder, matched by IMDb id and kind.
pub fn upsert(app: &AppPaths, placeholder: Placeholder) -> Result<()> {
    json_store::update(&path(app), |placeholders: &mut Placeholders| {
        let position = placeholders.items.iter().position(|known| known.imdb == placeholder.imdb && known.kind == placeholder.kind);
        match position {
            Some(index) => placeholders.items[index] = placeholder,
            None => placeholders.items.push(placeholder),
        }
        Ok(())
    })
}

/// Forgets one: the library has the title now, or nothing lists it any more.
pub fn remove(app: &AppPaths, imdb: &str, kind: &str) -> Result<()> {
    json_store::update(&path(app), |placeholders: &mut Placeholders| {
        placeholders.items.retain(|known| !(known.imdb == imdb && known.kind == kind));
        Ok(())
    })
}

/// Forgets every placeholder, for a reset.
pub fn clear(app: &AppPaths) -> Result<()> {
    let path = path(app);
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(root: &std::path::Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    fn one(imdb: &str) -> Placeholder {
        Placeholder {
            imdb: imdb.into(),
            kind: "movie".into(),
            title: "A Film".into(),
            year: Some("1999".into()),
            provider: "simkl".into(),
            first_seen: Some(Utc::now()),
        }
    }

    #[test]
    fn a_placeholder_round_trips_and_is_matched_by_id_and_kind() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        upsert(&app, one("tt1")).unwrap();
        upsert(&app, one("tt1")).unwrap();
        assert_eq!(read(&app).items.len(), 1, "the same title is one entry");

        let mut series = one("tt1");
        series.kind = "series".into();
        upsert(&app, series).unwrap();
        assert_eq!(read(&app).items.len(), 2, "a film and a series may share an id");
    }

    #[test]
    fn removing_one_leaves_the_rest_and_clear_takes_all() {
        let root = tempfile::tempdir().unwrap();
        let app = app(root.path());
        upsert(&app, one("tt1")).unwrap();
        upsert(&app, one("tt2")).unwrap();
        remove(&app, "tt1", "movie").unwrap();
        let items = read(&app).items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].imdb, "tt2");
        clear(&app).unwrap();
        assert!(!path(&app).exists());
    }
}
