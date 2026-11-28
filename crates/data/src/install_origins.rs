//! Provenance of store installs imported from another launcher or store client
//! (`local_library/install-origins.json`): records which Epic, GOG or Amazon
//! installs came from Heroic, the Epic Games Launcher, GOG Galaxy or the
//! Amazon Games app, so later syncs can automatically clear them when the
//! source launcher uninstalls them (while leaving Heronite's own installs
//! untouched).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::json_store;
use crate::models::game_key;
use crate::paths::DataPaths;

/// External source that owns an imported store install record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Heroic,
    EpicLauncher,
    GogGalaxy,
    AmazonApp,
}

/// One imported install's origin and the folder it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginRecord {
    pub origin: Origin,
    pub install_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginsFile {
    #[serde(default)]
    pub installs: BTreeMap<String, OriginRecord>,
}

/// Loads all recorded external install origins. Missing or unreadable files
/// yield an empty map.
pub fn load(paths: &DataPaths) -> BTreeMap<String, OriginRecord> {
    json_store::read::<OriginsFile>(&paths.install_origins()).map(|f| f.installs).unwrap_or_default()
}

/// Returns the recorded origin for `(runner, app_name)`, if any.
pub fn get(paths: &DataPaths, runner: &str, app_name: &str) -> Option<OriginRecord> {
    load(paths).remove(&game_key(runner, app_name))
}

/// Records that `(runner, app_name)` was imported from `origin` at
/// `install_path`. No-op if the same record is already saved.
pub fn record(paths: &DataPaths, runner: &str, app_name: &str, origin: Origin, install_path: &str) -> Result<()> {
    let key = game_key(runner, app_name);
    let entry = OriginRecord { origin, install_path: install_path.to_string() };
    json_store::update_if_changed::<OriginsFile, _>(&paths.install_origins(), |file| {
        file.installs.insert(key, entry);
        Ok(())
    })
}

/// Clears any recorded external origin for `(runner, app_name)` (when
/// Heronite installs, moves, or forgets the game itself).
pub fn forget(paths: &DataPaths, runner: &str, app_name: &str) {
    let file = paths.install_origins();
    if !file.exists() {
        return;
    }
    let key = game_key(runner, app_name);
    if let Err(err) = json_store::update_if_changed::<OriginsFile, _>(&file, |f| {
        f.installs.remove(&key);
        Ok(())
    }) {
        log::warn!("updating {}: {err:#}", file.display());
    }
}

/// All `(runner, app_name, install_path)` entries recorded under `origin`.
pub fn by_origin(paths: &DataPaths, origin: Origin) -> Vec<(String, String, String)> {
    load(paths)
        .into_iter()
        .filter(|(_, rec)| rec.origin == origin)
        .filter_map(|(key, rec)| {
            let (runner, app) = key.split_once(':')?;
            Some((runner.to_string(), app.to_string(), rec.install_path))
        })
        .collect()
}

/// Compares two install paths tolerating trailing slashes, Windows case/slash
/// differences, and Linux system symlinks (such as `/home` -> `/var/home` on
/// Fedora Atomic / Bazzite).
pub fn same_install_path(a: &str, b: &str) -> bool {
    let a = a.trim().trim_end_matches(['/', '\\']);
    let b = b.trim().trim_end_matches(['/', '\\']);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if cfg!(windows) {
        return a.replace('/', "\\").eq_ignore_ascii_case(&b.replace('/', "\\"));
    }
    if a == b {
        return true;
    }
    canonicalize_prefix(Path::new(a)) == canonicalize_prefix(Path::new(b))
}

/// Strips the Windows `\\?\` verbatim prefix returned by `std::fs::canonicalize`
/// so canonicalized paths match drive roots (`D:\`) and user-facing paths.
pub(crate) fn strip_verbatim(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

/// Resolves symlinks on the longest existing ancestor of `path` and reattaches
/// the remaining tail, so paths to deleted folders under `/home/...` and
/// `/var/home/...` still compare equal.
pub fn canonicalize_prefix(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return strip_verbatim(real);
    }
    let mut tail = Vec::new();
    let mut cur = path;
    while let Some(name) = cur.file_name() {
        tail.push(name.to_os_string());
        match cur.parent() {
            Some(parent) => {
                cur = parent;
                if let Ok(mut real) = std::fs::canonicalize(cur) {
                    for part in tail.into_iter().rev() {
                        real.push(part);
                    }
                    return strip_verbatim(real);
                }
            }
            None => break,
        }
    }
    strip_verbatim(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_queries_and_forgets_origins() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        assert!(load(&paths).is_empty());

        record(&paths, "gog", "1207658924", Origin::Heroic, "/games/Witcher3").unwrap();
        record(&paths, "legendary", "Calluna", Origin::Heroic, "/games/Control").unwrap();
        record(&paths, "nile", "amzn1.a", Origin::AmazonApp, "/games/Fallout").unwrap();

        assert_eq!(get(&paths, "gog", "1207658924"), Some(OriginRecord { origin: Origin::Heroic, install_path: "/games/Witcher3".into() }));
        let heroic = by_origin(&paths, Origin::Heroic);
        assert_eq!(heroic.len(), 2);

        forget(&paths, "gog", "1207658924");
        assert!(get(&paths, "gog", "1207658924").is_none());
        assert_eq!(by_origin(&paths, Origin::Heroic).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn same_install_path_resolves_symlinked_ancestors_for_missing_leaf() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("var_home");
        std::fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("home");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let via_link = link.join("Games/DeletedGame");
        let via_real = real.join("Games/DeletedGame");
        assert!(same_install_path(&via_link.to_string_lossy(), &via_real.to_string_lossy()));
        assert!(!same_install_path(&via_link.to_string_lossy(), &real.join("Games/Other").to_string_lossy()));
    }
}
