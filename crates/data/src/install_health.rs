//! Unified on-disk presence verification for installed games.
//!
//! Replaces scattered `Path::exists()` checks on the UI thread with a
//! background-friendly probe that distinguishes:
//! - [`InstallPresence::Present`]: install folder and game signature exist.
//! - [`InstallPresence::StorageUnavailable`]: the volume/mount holding the
//!   game is disconnected or inaccessible (never auto-uninstall).
//! - [`InstallPresence::Missing`]: the parent volume is mounted and readable,
//!   but the game's folder disappeared or lost its essential files (e.g.
//!   leftover `mods/` or logs after an external uninstall).

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::game_config;
use crate::install_origins::canonicalize_prefix;
use crate::json_store;
use crate::library::CollectionGame;
use crate::paths::DataPaths;

/// Why an installed game's files are missing on a mounted volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingReason {
    /// The install directory no longer exists.
    FolderGone,
    /// The install directory exists, but the game's marker/executable is gone
    /// (empty folder or leftover mods/logs/saves).
    Incomplete,
    /// Sideload executable or command target is gone (or broken symlink).
    ExecutableGone,
    /// All configured ROM files for an emulated game are gone.
    RomGone,
}

/// On-disk status of a game currently marked as installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPresence {
    /// Folder and expected game files are present.
    Present,
    /// The storage volume is not mounted or returned an I/O / permission error.
    StorageUnavailable {
        /// Mount point or drive root that appears offline, when known.
        mount_hint: Option<PathBuf>,
    },
    /// The volume is mounted, but the game's folder or files are gone.
    Missing(MissingReason),
}

impl InstallPresence {
    pub fn is_present(&self) -> bool {
        matches!(self, InstallPresence::Present)
    }

    pub fn is_storage_unavailable(&self) -> bool {
        matches!(self, InstallPresence::StorageUnavailable { .. })
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, InstallPresence::Missing(_))
    }

    pub fn missing_reason(&self) -> Option<MissingReason> {
        match self {
            InstallPresence::Missing(r) => Some(*r),
            _ => None,
        }
    }
}

/// One mounted filesystem entry used for volume matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPoint {
    pub point: PathBuf,
    pub fs: String,
    pub source: String,
}

/// Snapshot of currently mounted filesystems (taken once per scan pass).
#[derive(Debug, Clone, Default)]
pub struct Mounts {
    entries: Vec<MountPoint>,
}

/// Pseudo-filesystems on Linux that never hold game libraries.
#[cfg_attr(windows, allow(dead_code))]
const PSEUDO_FS: &[&str] = &[
    "proc",
    "sysfs",
    "tmpfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "mqueue",
    "debugfs",
    "tracefs",
    "securityfs",
    "pstore",
    "efivarfs",
    "bpf",
    "configfs",
    "fusectl",
    "autofs",
    "binfmt_misc",
    "ramfs",
    "nsfs",
    "squashfs",
];

impl Mounts {
    /// Reads the currently mounted filesystems from the OS.
    pub fn current() -> Self {
        #[cfg(windows)]
        {
            let entries = heronite_platform::disks::volumes()
                .into_iter()
                .flat_map(|v| {
                    let fs = v.fs;
                    let source = v.id;
                    v.mount_points.into_iter().map(move |point| MountPoint { point, fs: fs.clone(), source: source.clone() })
                })
                .collect();
            Self { entries }
        }
        #[cfg(not(windows))]
        {
            let entries = std::fs::read_to_string("/proc/self/mountinfo").map(|t| parse_mountinfo(&t)).unwrap_or_default();
            Self { entries }
        }
    }

    /// Builds a snapshot from explicit entries (for unit tests).
    pub fn from_entries(entries: Vec<MountPoint>) -> Self {
        Self { entries }
    }

    /// The longest mounted point that contains `path`.
    pub fn mount_of(&self, path: &Path) -> Option<&MountPoint> {
        let norm = canonicalize_prefix(path);
        self.entries.iter().filter(|m| path_starts_with(&norm, &m.point)).max_by_key(|m| m.point.as_os_str().len())
    }

    /// Whether `point` is currently mounted (comparing canonicalized paths).
    pub fn is_mounted(&self, point: &Path) -> bool {
        let norm = canonicalize_prefix(point);
        self.entries.iter().any(|m| same_mount_path(&m.point, &norm))
    }

    /// Whether `path` looks like it belongs to an unmounted volume (by drive
    /// letter / UNC on Windows, or known external mount prefixes on Linux, or
    /// a previously remembered non-root mount point).
    pub fn unmounted_hint(&self, path: &Path, remembered: Option<&RememberedMount>) -> Option<PathBuf> {
        if let Some(rem) = remembered {
            let rem_point = PathBuf::from(&rem.mount_point);
            if !is_root_mount(&rem_point) {
                if !self.is_mounted(&rem_point) {
                    return Some(rem_point);
                }
                // Still mounted at the same path, check if it now resolves to a
                // different mount (e.g., empty stub dir on `/`).
                if let Some(cur) = self.mount_of(path) {
                    if !same_mount_path(&cur.point, &rem_point) {
                        return Some(rem_point);
                    }
                }
            }
        }
        if cfg!(windows) {
            return windows_unmounted_hint(path, self);
        }
        linux_unmounted_hint(path, self)
    }
}

/// Last known mount point of a game's install path, cached locally in
/// `local_library/install-mounts.json` so even custom mount paths (like
/// `/home/user/GamesHDD`) are recognized as unmounted when disconnected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedMount {
    pub mount_point: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MountCacheFile {
    #[serde(default)]
    mounts: BTreeMap<String, RememberedMount>,
}

/// Persistent cache of observed mount points per game key.
#[derive(Debug, Clone, Default)]
pub struct MountCache {
    mounts: BTreeMap<String, RememberedMount>,
    dirty: bool,
}

impl MountCache {
    pub fn load(paths: &DataPaths) -> Self {
        let mounts = json_store::read::<MountCacheFile>(&paths.install_mounts()).map(|f| f.mounts).unwrap_or_default();
        Self { mounts, dirty: false }
    }

    pub fn get(&self, key: &str) -> Option<&RememberedMount> {
        self.mounts.get(key)
    }

    /// Updates the cached mount point for `key` when the path is currently
    /// accessible.
    pub fn observe(&mut self, key: &str, path: &Path, mounts: &Mounts) {
        let Some(m) = mounts.mount_of(path) else { return };
        let next = RememberedMount { mount_point: m.point.to_string_lossy().into_owned(), source: m.source.clone() };
        if self.mounts.get(key) != Some(&next) {
            self.mounts.insert(key.to_string(), next);
            self.dirty = true;
        }
    }

    /// Writes changes back to `local_library/install-mounts.json` if dirty.
    pub fn save(&mut self, paths: &DataPaths) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let data = self.mounts.clone();
        if let Err(err) = json_store::update_if_changed::<MountCacheFile, _>(&paths.install_mounts(), |file| {
            file.mounts = data;
            Ok(())
        }) {
            log::warn!("saving install mount cache: {err:#}");
        }
    }
}

/// Checks whether `path` is present, on unmounted/inaccessible storage, or
/// genuinely missing (`MissingReason::FolderGone` / `ExecutableGone`).
pub fn check_path(path: &Path, remembered: Option<&RememberedMount>, mounts: &Mounts, not_found_reason: MissingReason) -> InstallPresence {
    match std::fs::metadata(path) {
        Ok(_) => InstallPresence::Present,
        Err(err) if err.kind() != ErrorKind::NotFound => {
            if let Some(hint) = mounts.unmounted_hint(path, remembered) {
                InstallPresence::StorageUnavailable { mount_hint: Some(hint) }
            } else {
                // Permission denied (e.g. protected WindowsApps or restricted parent)
                // or similar non-NotFound errors must not falsely block Play.
                InstallPresence::Present
            }
        }
        Err(_) => {
            if let Some(hint) = mounts.unmounted_hint(path, remembered) {
                InstallPresence::StorageUnavailable { mount_hint: Some(hint) }
            } else {
                InstallPresence::Missing(not_found_reason)
            }
        }
    }
}

/// Checks whether `path` is currently on unmounted or inaccessible storage.
pub fn is_path_storage_unavailable(path: &Path, remembered: Option<&RememberedMount>, mounts: &Mounts) -> bool {
    check_path(path, remembered, mounts, MissingReason::FolderGone).is_storage_unavailable()
}

/// Evaluates the on-disk presence of one installed [`CollectionGame`].
/// Updates `cache` with observed mount points when the game is present.
pub fn check_game_presence(
    paths: &DataPaths,
    game: &CollectionGame,
    home: &str,
    mounts: &Mounts,
    cache: &mut MountCache,
) -> InstallPresence {
    if !game.installed {
        return InstallPresence::Present;
    }
    // Steam copy of a non-Steam entry is managed by Steam, not this store's folder.
    if game.via_steam {
        return InstallPresence::Present;
    }
    match game.runner.as_str() {
        "gog" => check_gog(paths, game, home, mounts, cache),
        "legendary" => check_epic(paths, game, home, mounts, cache),
        "nile" => check_amazon(paths, game, home, mounts, cache),
        "sideload" => check_sideload(paths, game, home, mounts, cache),
        _ => InstallPresence::Present,
    }
}

fn target_exe(paths: &DataPaths, app_name: &str, home: &str) -> Option<String> {
    let settings = game_config::load_game(paths, app_name, home);
    settings.get("targetExe").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn check_gog(paths: &DataPaths, game: &CollectionGame, home: &str, mounts: &Mounts, cache: &mut MountCache) -> InstallPresence {
    let Some(raw) = game.game.install.install_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return InstallPresence::Present;
    };
    let dir = Path::new(raw);
    match check_path(dir, cache.get(&game.key), mounts, MissingReason::FolderGone) {
        InstallPresence::Present => {}
        other => return other,
    }
    if !dir.is_dir() {
        return InstallPresence::Missing(MissingReason::FolderGone);
    }
    cache.observe(&game.key, dir, mounts);

    // Custom targetExe overrides the standard GOG launch markers.
    if let Some(exe) = target_exe(paths, &game.app_name, home) {
        if Path::new(&exe).exists() {
            return InstallPresence::Present;
        }
    }
    let info = dir.join(format!("goggame-{}.info", game.app_name));
    let is_linux = game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("linux"));
    let valid = info.is_file() || (is_linux && (dir.join("start.sh").is_file() || dir.join("gameinfo").is_file()));
    if valid {
        InstallPresence::Present
    } else {
        InstallPresence::Missing(MissingReason::Incomplete)
    }
}

fn check_epic(paths: &DataPaths, game: &CollectionGame, home: &str, mounts: &Mounts, cache: &mut MountCache) -> InstallPresence {
    // Third-party managed Epic games (EA app / Ubisoft Connect) live inside
    // their own client prefix and have no legendary `install_path`.
    let Some(raw) = game.game.install.install_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return InstallPresence::Present;
    };
    let dir = Path::new(raw);
    match check_path(dir, cache.get(&game.key), mounts, MissingReason::FolderGone) {
        InstallPresence::Present => {}
        other => return other,
    }
    if !dir.is_dir() {
        return InstallPresence::Missing(MissingReason::FolderGone);
    }
    cache.observe(&game.key, dir, mounts);

    if let Some(exe) = target_exe(paths, &game.app_name, home) {
        if Path::new(&exe).exists() {
            return InstallPresence::Present;
        }
    }
    if let Some(exe) = game.game.install.executable.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if exists_relative_ci(dir, exe) {
            return InstallPresence::Present;
        }
        return InstallPresence::Missing(MissingReason::Incomplete);
    }
    // No executable recorded: at least require the folder not to be empty.
    if dir_has_entries(dir) {
        InstallPresence::Present
    } else {
        InstallPresence::Missing(MissingReason::Incomplete)
    }
}

fn check_amazon(paths: &DataPaths, game: &CollectionGame, home: &str, mounts: &Mounts, cache: &mut MountCache) -> InstallPresence {
    let Some(raw) = game.game.install.install_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return InstallPresence::Present;
    };
    let dir = Path::new(raw);
    match check_path(dir, cache.get(&game.key), mounts, MissingReason::FolderGone) {
        InstallPresence::Present => {}
        other => return other,
    }
    if !dir.is_dir() {
        return InstallPresence::Missing(MissingReason::FolderGone);
    }
    cache.observe(&game.key, dir, mounts);

    if let Some(exe) = target_exe(paths, &game.app_name, home) {
        if Path::new(&exe).exists() {
            return InstallPresence::Present;
        }
    }
    if dir.join("fuel.json").is_file() {
        InstallPresence::Present
    } else {
        InstallPresence::Missing(MissingReason::Incomplete)
    }
}

fn check_sideload(_paths: &DataPaths, game: &CollectionGame, _home: &str, mounts: &Mounts, cache: &mut MountCache) -> InstallPresence {
    if game.game.install.platform.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("browser")) {
        return InstallPresence::Present;
    }
    let kind = game.launch_kind.as_deref().unwrap_or("");
    match kind {
        "steam-uri" => {
            // Steam presence is refreshed via manifests; if the game has a
            // cached folder we only surface `StorageUnavailable` when offline.
            if let Some(raw) = game.game.install.install_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let p = Path::new(raw);
                let res = check_path(p, cache.get(&game.key), mounts, MissingReason::FolderGone);
                if res.is_present() {
                    cache.observe(&game.key, p, mounts);
                } else if res.is_storage_unavailable() {
                    return res;
                }
            }
            InstallPresence::Present
        }
        crate::launchers::LAUNCH_KIND | crate::clients::LAUNCH_KIND | crate::xbox::LAUNCH_KIND => {
            let Some(raw) = game.game.install.install_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
                return InstallPresence::Present;
            };
            let p = Path::new(raw);
            let res = check_path(p, cache.get(&game.key), mounts, MissingReason::FolderGone);
            if res.is_present() {
                cache.observe(&game.key, p, mounts);
            }
            res
        }
        "emulator" => check_emulator(game, mounts, cache),
        _ => {
            if let Some((_, folder)) = crate::sideload::command_of(&game.game) {
                let p = Path::new(folder);
                let res = check_path(p, cache.get(&game.key), mounts, MissingReason::FolderGone);
                if res.is_present() {
                    cache.observe(&game.key, p, mounts);
                }
                return res;
            }
            let exe = game
                .meta
                .as_deref()
                .and_then(|m| m.remapped_executable.as_deref())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| game.game.install.executable.as_deref().filter(|s| !s.trim().is_empty()));
            let Some(exe) = exe else {
                return InstallPresence::Present;
            };
            let p = Path::new(exe);
            // Ignore cross-platform Windows paths on Unix (e.g. Playnite import).
            if !p.is_absolute() {
                return InstallPresence::Present;
            }
            let res = check_path(p, cache.get(&game.key), mounts, MissingReason::ExecutableGone);
            if res.is_present() {
                cache.observe(&game.key, p, mounts);
            }
            res
        }
    }
}

fn check_emulator(game: &CollectionGame, mounts: &Mounts, cache: &mut MountCache) -> InstallPresence {
    let Some(meta) = game.meta.as_deref() else {
        return InstallPresence::Present;
    };
    if let Some(roms) = meta.roms.as_ref().filter(|r| !r.is_empty()) {
        let mut saw_offline = None;
        for rom in roms {
            let p = Path::new(&rom.path);
            if !p.is_absolute() {
                continue;
            }
            match check_path(p, cache.get(&game.key), mounts, MissingReason::RomGone) {
                InstallPresence::Present => {
                    cache.observe(&game.key, p, mounts);
                    return InstallPresence::Present;
                }
                InstallPresence::StorageUnavailable { mount_hint } => {
                    saw_offline.get_or_insert(mount_hint);
                }
                InstallPresence::Missing(_) => {}
            }
        }
        if let Some(hint) = saw_offline {
            return InstallPresence::StorageUnavailable { mount_hint: hint };
        }
        return InstallPresence::Missing(MissingReason::RomGone);
    }
    InstallPresence::Present
}

/// Checks whether a relative path like `Bin\Game.exe` exists under `base`,
/// falling back to a case-insensitive walk on case-sensitive filesystems.
fn exists_relative_ci(base: &Path, rel: &str) -> bool {
    let cleaned = rel.replace('\\', "/");
    let direct = base.join(&cleaned);
    if direct.exists() {
        return true;
    }
    let mut dir = base.to_path_buf();
    for part in cleaned.split('/').filter(|p| !p.is_empty() && *p != ".") {
        let exact = dir.join(part);
        if exact.exists() {
            dir = exact;
            continue;
        }
        let found = std::fs::read_dir(&dir)
            .ok()
            .and_then(|rd| rd.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().eq_ignore_ascii_case(part)));
        match found {
            Some(name) => dir.push(name),
            None => return false,
        }
    }
    dir.exists()
}

fn dir_has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut rd| rd.next().is_some())
}

fn is_root_mount(p: &Path) -> bool {
    p == Path::new("/")
}

fn path_starts_with(path: &Path, prefix: &Path) -> bool {
    if cfg!(windows) {
        let a = path.to_string_lossy().replace('/', "\\").to_lowercase();
        let b = prefix.to_string_lossy().replace('/', "\\").to_lowercase();
        let b_trim = b.trim_end_matches('\\');
        if b_trim.is_empty() {
            return false;
        }
        a == b_trim || a.starts_with(&format!("{b_trim}\\"))
    } else {
        path.starts_with(prefix)
    }
}

fn same_mount_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
        norm(a) == norm(b)
    } else {
        canonicalize_prefix(a) == canonicalize_prefix(b)
    }
}

/// On Linux, detects common external/secondary mount prefixes (`/run/media/<user>/<vol>`,
/// `/media/<user>/<vol>`, `/media/<vol>`, `/mnt/<vol>`, `/var/mnt/<vol>`) whose
/// mount point is absent from `mounts`.
#[cfg_attr(windows, allow(dead_code))]
fn linux_unmounted_hint(path: &Path, mounts: &Mounts) -> Option<PathBuf> {
    let norm = canonicalize_prefix(path);
    let candidate = linux_external_mount_candidate(&norm)?;
    // If the candidate itself is mounted, the drive IS mounted (so the game was deleted).
    if mounts.is_mounted(&candidate) {
        return None;
    }
    // If the candidate directory does not exist, or exists only as an empty stub
    // directory on another filesystem, the external drive is unmounted.
    if !candidate.exists() || !dir_has_entries(&candidate) {
        return Some(candidate);
    }
    None
}

/// Extracts the expected mount point under standard Linux mount roots.
fn linux_external_mount_candidate(path: &Path) -> Option<PathBuf> {
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        ["run", "media", user, vol, ..] => Some(PathBuf::from(format!("/run/media/{user}/{vol}"))),
        ["var", "mnt", vol, ..] => Some(PathBuf::from(format!("/var/mnt/{vol}"))),
        ["mnt", vol, ..] => Some(PathBuf::from(format!("/mnt/{vol}"))),
        ["media", a, b, ..] => {
            // `/media/<user>/<vol>` (udisks2) vs `/media/<vol>` (fstab).
            let two = PathBuf::from(format!("/media/{a}/{b}"));
            let one = PathBuf::from(format!("/media/{a}"));
            if one.is_dir() && !two.exists() {
                Some(two)
            } else {
                Some(one)
            }
        }
        _ => None,
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_unmounted_hint(path: &Path, mounts: &Mounts) -> Option<PathBuf> {
    let s = path.to_string_lossy();
    // UNC network share (`\\server\share\...`): if the share root doesn't exist, offline.
    if let Some(rest) = s.strip_prefix(r"\\") {
        let mut parts = rest.split('\\').filter(|p| !p.is_empty());
        if let (Some(server), Some(share)) = (parts.next(), parts.next()) {
            let root = PathBuf::from(format!(r"\\{server}\{share}"));
            if !root.exists() {
                return Some(root);
            }
        }
        return None;
    }
    // Drive letter (`D:\...`): if the drive root is absent, the drive is disconnected.
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let root = PathBuf::from(format!("{}:\\", bytes[0] as char));
        if !mounts.is_mounted(&root) && !root.exists() {
            return Some(root);
        }
    }
    None
}

/// Parses Linux `/proc/self/mountinfo` lines into real filesystem mount points.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn parse_mountinfo(text: &str) -> Vec<MountPoint> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let point = left.split(' ').nth(4)?;
            let mut right = right.split(' ');
            let fs = right.next()?;
            let source = right.next()?;
            if PSEUDO_FS.contains(&fs) {
                return None;
            }
            Some(MountPoint { point: unescape_mountinfo(point), fs: fs.to_string(), source: source.to_string() })
        })
        .collect()
}

/// Octal escapes in `/proc/self/mountinfo` (`\040` -> space, `\011` -> tab),
/// preserving UTF-8 and raw bytes on Unix so non-ASCII volume labels match.
#[cfg_attr(windows, allow(dead_code))]
fn unescape_mountinfo(s: &str) -> PathBuf {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() {
            if let Some(oct) = std::str::from_utf8(&b[i + 1..i + 4]).ok().and_then(|o| u8::from_str_radix(o, 8).ok()) {
                out.push(oct);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(out))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(&out).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::library::Library;
    #[cfg(unix)]
    use serde_json::json;

    #[cfg(unix)]
    #[test]
    fn parses_mountinfo_and_unescapes_spaces_and_utf8() {
        let sample = "\
36 1 259:2 / / rw,noatime - ext4 /dev/nvme0n1p2 rw\n\
55 36 0:20 / /proc rw - proc proc rw\n\
77 36 8:17 / /run/media/rafa/My\\040SSD rw - btrfs /dev/sdb1 rw\n\
78 36 8:33 / /run/media/rafa/Jogos\\040Ação rw - ext4 /dev/sdc1 rw\n";
        let mounts = Mounts::from_entries(parse_mountinfo(sample));
        assert!(mounts.is_mounted(Path::new("/")));
        assert!(mounts.is_mounted(Path::new("/run/media/rafa/My SSD")));
        assert!(mounts.is_mounted(Path::new("/run/media/rafa/Jogos Ação")));
        assert!(!mounts.is_mounted(Path::new("/proc")));
        assert_eq!(
            mounts.mount_of(Path::new("/run/media/rafa/My SSD/Games/Witcher")).map(|m| m.point.as_path()),
            Some(Path::new("/run/media/rafa/My SSD"))
        );
        assert_eq!(
            mounts.mount_of(Path::new("/run/media/rafa/Jogos Ação/Control")).map(|m| m.point.as_path()),
            Some(Path::new("/run/media/rafa/Jogos Ação"))
        );
    }

    #[test]
    fn strips_windows_verbatim_prefix() {
        use crate::install_origins::strip_verbatim;
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\D:\Games\Control")), PathBuf::from(r"D:\Games\Control"));
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\UNC\nas\games\Control")), PathBuf::from(r"\\nas\games\Control"));
        assert_eq!(strip_verbatim(PathBuf::from("/mnt/games/Control")), PathBuf::from("/mnt/games/Control"));
    }

    #[cfg(unix)]
    #[test]
    fn non_not_found_error_on_mounted_volume_is_not_storage_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("regular-file");
        std::fs::write(&file, "x").unwrap();
        // Statting a child beneath a regular file returns NotADirectory (not NotFound).
        let bogus_child = file.join("child");
        let mounts = Mounts::from_entries(vec![MountPoint { point: PathBuf::from("/"), fs: "ext4".into(), source: "/dev/root".into() }]);
        assert_eq!(check_path(&bogus_child, None, &mounts, MissingReason::FolderGone), InstallPresence::Present);
    }

    #[cfg(unix)]
    #[test]
    fn distinguishes_unmounted_external_drive_from_deleted_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mounts =
            Mounts::from_entries(vec![MountPoint { point: PathBuf::from("/"), fs: "ext4".into(), source: "/dev/nvme0n1p2".into() }]);

        // Under `/run/media/<user>/<vol>` with nothing mounted there -> StorageUnavailable.
        let on_sd = Path::new("/run/media/rafa/SDCard/Games/Witcher 3");
        assert_eq!(
            check_path(on_sd, None, &mounts, MissingReason::FolderGone),
            InstallPresence::StorageUnavailable { mount_hint: Some(PathBuf::from("/run/media/rafa/SDCard")) }
        );

        // Remembered custom mount point `/home/rafa/GamesHDD` now unmounted -> StorageUnavailable.
        let custom = dir.path().join("GamesHDD/Control");
        let remembered =
            RememberedMount { mount_point: dir.path().join("GamesHDD").to_string_lossy().into_owned(), source: "/dev/sdb1".into() };
        assert_eq!(
            check_path(&custom, Some(&remembered), &mounts, MissingReason::FolderGone),
            InstallPresence::StorageUnavailable { mount_hint: Some(dir.path().join("GamesHDD")) }
        );

        // Regular local folder deleted on mounted filesystem -> FolderGone.
        let local_gone = dir.path().join("Games/Deleted");
        assert_eq!(check_path(&local_gone, None, &mounts, MissingReason::FolderGone), InstallPresence::Missing(MissingReason::FolderGone));
    }

    #[cfg(unix)]
    #[test]
    fn gog_and_amazon_detect_leftover_folders_as_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let gog_dir = dir.path().join("Games/Witcher");
        let nile_dir = dir.path().join("Games/Fallout");
        std::fs::create_dir_all(&gog_dir).unwrap();
        std::fs::create_dir_all(&nile_dir).unwrap();
        // Leftover mod file only: no goggame-*.info and no fuel.json.
        std::fs::write(gog_dir.join("mod.log"), "leftover").unwrap();
        std::fs::write(nile_dir.join("notes.txt"), "leftover").unwrap();

        json_store::write(
            &paths.gog_library(),
            &json!({"games":[{"runner":"gog","app_name":"123","title":"Witcher","is_installed":true,"install":{"platform":"windows","install_path":gog_dir}}],"__timestamp":{"games":"x"}}),
        )
        .unwrap();
        json_store::write(&paths.gog_installed(), &json!({"installed":[{"appName":"123","platform":"windows","install_path":gog_dir}]}))
            .unwrap();
        json_store::write(
            &paths.nile_library(),
            &json!({"library":[{"runner":"nile","app_name":"amzn1.a","title":"Fallout","is_installed":true,"install":{"platform":"Windows","install_path":nile_dir}}],"__timestamp":{"library":"x"}}),
        )
        .unwrap();

        let lib = Library::load(&paths).unwrap();
        let mounts = Mounts::current();
        let mut cache = MountCache::default();
        assert_eq!(
            check_game_presence(&paths, lib.get("gog_123").unwrap(), "/tmp", &mounts, &mut cache),
            InstallPresence::Missing(MissingReason::Incomplete)
        );
        assert_eq!(
            check_game_presence(&paths, lib.get("nile_amzn1.a").unwrap(), "/tmp", &mounts, &mut cache),
            InstallPresence::Missing(MissingReason::Incomplete)
        );

        // Adding the real markers turns both Present.
        std::fs::write(gog_dir.join("goggame-123.info"), "{}").unwrap();
        std::fs::write(nile_dir.join("fuel.json"), "{}").unwrap();
        assert_eq!(check_game_presence(&paths, lib.get("gog_123").unwrap(), "/tmp", &mounts, &mut cache), InstallPresence::Present);
        assert_eq!(check_game_presence(&paths, lib.get("nile_amzn1.a").unwrap(), "/tmp", &mounts, &mut cache), InstallPresence::Present);
    }
}
