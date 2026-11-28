//! `gog_store/installed.json`: `{"installed": [ {...}, ... ]}`, one entry per
//! installed game. Keys the app does not model are kept on every rewrite.

use anyhow::Result;
use heronite_data::json_store;
use heronite_data::models::lenient;
use heronite_data::DataPaths;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Key of the entries [`crate::galaxy`] adds.
pub const GALAXY_KEY: &str = "galaxy";

/// One installed game. `executable` (always `""`) and `is_dlc` live in
/// `extra` with whatever else the file holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Installed {
    #[serde(rename = "appName", default, deserialize_with = "lenient::string")]
    pub app_name: String,
    /// `windows`, `linux` or `osx`.
    #[serde(default, deserialize_with = "lenient::string")]
    pub platform: String,
    #[serde(default, deserialize_with = "lenient::string")]
    pub install_path: String,
    /// Text such as `1.5 GiB`.
    #[serde(default, deserialize_with = "lenient::string")]
    pub install_size: String,
    /// gogdl's `versionName`.
    #[serde(default, deserialize_with = "lenient::string")]
    pub version: String,
    /// Language code, e.g. `en-US`.
    #[serde(default, deserialize_with = "lenient::string")]
    pub language: String,
    #[serde(rename = "installedDLCs", default, deserialize_with = "lenient::vec_string")]
    pub installed_dlcs: Vec<String>,
    /// `ETag` of the build's manifest; compared by the update check.
    #[serde(rename = "versionEtag", default, deserialize_with = "lenient::string")]
    pub version_etag: String,
    #[serde(rename = "buildId", default, deserialize_with = "lenient::string")]
    pub build_id: String,
    /// Games pinned to a version are never offered updates.
    #[serde(rename = "pinnedVersion", default, deserialize_with = "lenient::bool_default")]
    pub pinned_version: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Installed {
    /// A new entry with the fixed keys filled in.
    pub fn new(app_name: &str) -> Self {
        let mut extra = Map::new();
        extra.insert("executable".into(), json!(""));
        extra.insert("is_dlc".into(), json!(false));
        Self { app_name: app_name.to_string(), extra, ..Default::default() }
    }

    /// Found among GOG Galaxy's installs ([`crate::galaxy`]): Galaxy owns the
    /// files, so they are never moved, updated, repaired or deleted here.
    pub fn from_galaxy(&self) -> bool {
        self.extra.get(GALAXY_KEY).and_then(Value::as_bool) == Some(true)
    }

    /// The `install` object of a `GameInfo`.
    pub fn game_install(&self) -> Value {
        json!({
            "executable": self.extra.get("executable").and_then(Value::as_str).unwrap_or(""),
            "platform": self.platform,
            "is_dlc": false,
            "install_path": self.install_path,
            "install_size": self.install_size,
            "version": self.version,
        })
    }
}

fn entry_app_name(v: &Value) -> Option<String> {
    match v.get("appName")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn load(paths: &DataPaths) -> Vec<Installed> {
    let doc = match json_store::read_value(&paths.gog_installed()) {
        Ok(v) => v,
        Err(err) => {
            log::warn!("reading installed GOG games: {err:#}");
            return Vec::new();
        }
    };
    let Some(list) = doc.get("installed").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        match serde_json::from_value::<Installed>(item.clone()) {
            Ok(i) if !i.app_name.is_empty() => out.push(i),
            Ok(_) => {}
            Err(err) => log::warn!("skipping malformed GOG install entry: {err:#}"),
        }
    }
    out
}

pub fn get(paths: &DataPaths, app_name: &str) -> Option<Installed> {
    load(paths).into_iter().find(|i| i.app_name == app_name)
}

/// Adds the entry or replaces the one with the same app name (its unknown
/// keys carry over unless the new entry sets them). Operates on the raw JSON
/// array so one malformed neighbor entry never blocks updating another game.
pub fn upsert(paths: &DataPaths, entry: &Installed) -> Result<()> {
    json_store::update_value(&paths.gog_installed(), |doc| {
        if !doc.is_object() {
            *doc = json!({});
        }
        let list = doc.as_object_mut().expect("object").entry("installed").or_insert_with(|| json!([]));
        if !list.is_array() {
            *list = json!([]);
        }
        let items = list.as_array_mut().expect("array");
        match items.iter_mut().find(|i| entry_app_name(i).as_deref() == Some(entry.app_name.as_str())) {
            Some(old_val) => {
                let mut merged = entry.clone();
                if let Ok(mut old) = serde_json::from_value::<Installed>(old_val.clone()) {
                    for (k, v) in std::mem::take(&mut old.extra) {
                        merged.extra.entry(k).or_insert(v);
                    }
                }
                *old_val = serde_json::to_value(&merged)?;
            }
            None => items.push(serde_json::to_value(entry)?),
        }
        Ok(())
    })
}

pub fn remove(paths: &DataPaths, app_name: &str) -> Result<()> {
    if !paths.gog_installed().exists() {
        return Ok(());
    }
    json_store::update_value(&paths.gog_installed(), |doc| {
        if let Some(items) = doc.get_mut("installed").and_then(Value::as_array_mut) {
            items.retain(|i| entry_app_name(i).as_deref() != Some(app_name));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_round_trip_with_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("gog_store")).unwrap();
        std::fs::write(
            paths.gog_installed(),
            r#"{"installed":[{"appName":"1","platform":"windows","install_path":"/g/one","executable":"","install_size":"1 GiB","is_dlc":false,"version":"1.0","language":"en-US","installedDLCs":["9"],"versionEtag":"\"e\"","buildId":"77","pinnedVersion":true,"somethingNew":5}],"other":1}"#,
        )
        .unwrap();
        let got = get(&paths, "1").unwrap();
        assert_eq!((got.build_id.as_str(), got.pinned_version, got.installed_dlcs.clone()), ("77", true, vec!["9".to_string()]));
        assert_eq!(got.version_etag, "\"e\"");

        let mut changed = got.clone();
        changed.version = "2.0".into();
        upsert(&paths, &changed).unwrap();
        let mut second = Installed::new("2");
        second.install_path = "/g/two".into();
        upsert(&paths, &second).unwrap();

        let doc = json_store::read_value(&paths.gog_installed()).unwrap();
        assert_eq!(doc["other"], 1);
        assert_eq!(doc["installed"][0]["version"], "2.0");
        assert_eq!(doc["installed"][0]["somethingNew"], 5);
        assert_eq!(doc["installed"][1]["executable"], "");
        assert_eq!(doc["installed"][1]["is_dlc"], false);
        assert_eq!(doc["installed"][1]["pinnedVersion"], false);
        assert_eq!(load(&paths).len(), 2);

        remove(&paths, "1").unwrap();
        assert_eq!(load(&paths).iter().map(|i| i.app_name.as_str()).collect::<Vec<_>>(), ["2"]);
        assert!(get(&paths, "1").is_none());
    }

    #[test]
    fn missing_files_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(load(&paths).is_empty());
        remove(&paths, "x").unwrap();
        assert!(!paths.gog_installed().exists());
    }

    #[test]
    fn heroic_numeric_and_null_fields_decode_and_remove_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        std::fs::create_dir_all(dir.path().join("gog_store")).unwrap();
        std::fs::write(
            paths.gog_installed(),
            r#"{"installed":[
                {"appName":"1207658924","platform":"windows","install_path":"/games/Witcher3","install_size":45000000,"version":404,"language":null,"installedDLCs":[123,{"id":456}],"versionEtag":null,"buildId":44535815,"pinnedVersion":null},
                {"appName":999,"platform":"linux","install_path":"/games/Other","buildId":77}
            ]}"#,
        )
        .unwrap();
        let all = load(&paths);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].build_id, "44535815");
        assert_eq!(all[0].version, "404");
        assert_eq!(all[0].install_size, "45000000");
        assert_eq!(all[0].installed_dlcs, ["123", "456"]);
        assert_eq!(all[1].app_name, "999");

        remove(&paths, "1207658924").unwrap();
        assert!(get(&paths, "1207658924").is_none());
        assert_eq!(load(&paths).len(), 1);
        remove(&paths, "999").unwrap();
        assert!(load(&paths).is_empty());
    }
}
