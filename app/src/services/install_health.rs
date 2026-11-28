//! Background verification of installed games' presence on disk.
//!
//! Runs off the UI thread (at startup, on Refresh, after reloads and on window
//! focus with a cooldown) and caches every non-`Present` game so the UI never
//! calls `Path::exists()` synchronously during render.
//!
//! Debug: `HERONITE_DEBUG_INSTALL_HEALTH=<key>:<missing|incomplete|offline>[,...]`
//! overrides the state of the named games (no files touched, for screenshots).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::*;
use heronite_data::install_health::{check_game_presence, InstallPresence, MissingReason, MountCache, Mounts};
use heronite_data::{DataPaths, Library};

/// Minimum interval between automatic window-focus scans.
const FOCUS_COOLDOWN: Duration = Duration::from_secs(60);

pub enum InstallHealthEvent {
    Changed,
}

pub struct InstallHealth {
    paths: DataPaths,
    /// Non-`Present` games by Collection key (`runner_appName`).
    states: HashMap<String, InstallPresence>,
    /// Debug overrides parsed from `HERONITE_DEBUG_INSTALL_HEALTH`.
    debug_overrides: HashMap<String, InstallPresence>,
    checking: bool,
    /// Latest scan request queued while a scan was already running.
    pending: Option<(Arc<Library>, HashSet<String>)>,
    last_check: Option<Instant>,
}

impl EventEmitter<InstallHealthEvent> for InstallHealth {}

fn parse_debug_overrides() -> HashMap<String, InstallPresence> {
    let Some(raw) = crate::env::value("HERONITE_DEBUG_INSTALL_HEALTH") else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for item in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let Some((key, kind)) = item.rsplit_once(':') else { continue };
        let presence = match kind.trim().to_ascii_lowercase().as_str() {
            "missing" | "folder" => InstallPresence::Missing(MissingReason::FolderGone),
            "incomplete" | "files" => InstallPresence::Missing(MissingReason::Incomplete),
            "offline" | "drive" => InstallPresence::StorageUnavailable {
                mount_hint: Some(PathBuf::from(if cfg!(windows) { r"D:\" } else { "/run/media/GamesSSD" })),
            },
            _ => continue,
        };
        out.insert(key.trim().to_string(), presence);
    }
    out
}

fn scan_blocking(paths: &DataPaths, lib: &Library, active_apps: &HashSet<String>) -> HashMap<String, InstallPresence> {
    let mounts = Mounts::current();
    let mut cache = MountCache::load(paths);
    let home = heronite_platform::dirs::home().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = HashMap::new();
    for game in &lib.games {
        if !game.installed || active_apps.contains(&game.app_name) {
            continue;
        }
        let presence = check_game_presence(paths, game, &home, &mounts, &mut cache);
        if !presence.is_present() {
            out.insert(game.key.clone(), presence);
        }
    }
    cache.save(paths);
    out
}

impl InstallHealth {
    pub fn new(paths: DataPaths) -> Self {
        let debug_overrides = parse_debug_overrides();
        Self { paths, states: debug_overrides.clone(), debug_overrides, checking: false, pending: None, last_check: None }
    }

    /// Cached presence of `key` (`Present` when healthy or not scanned yet).
    pub fn presence(&self, key: &str) -> InstallPresence {
        if let Some(p) = self.debug_overrides.get(key) {
            return p.clone();
        }
        self.states.get(key).cloned().unwrap_or(InstallPresence::Present)
    }

    /// Fresh single-game presence check for the Play click / launch path
    /// (never call from `render`). Updates the cache if the state changed.
    pub fn check_one(&mut self, game: &heronite_data::CollectionGame, cx: &mut Context<Self>) -> InstallPresence {
        if let Some(p) = self.debug_overrides.get(&game.key) {
            return p.clone();
        }
        if !game.installed {
            return InstallPresence::Present;
        }
        let mounts = Mounts::current();
        let mut cache = MountCache::load(&self.paths);
        let home = heronite_platform::dirs::home().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let presence = check_game_presence(&self.paths, game, &home, &mounts, &mut cache);
        cache.save(&self.paths);
        let prev = self.states.get(&game.key).cloned().unwrap_or(InstallPresence::Present);
        if prev != presence {
            if presence.is_present() {
                self.states.remove(&game.key);
            } else {
                self.states.insert(game.key.clone(), presence.clone());
            }
            cx.emit(InstallHealthEvent::Changed);
            cx.notify();
        }
        presence
    }

    /// Scans `lib` in the background, skipping `active_apps` (downloading,
    /// moving, verifying or running). Queues a follow-up scan if one is
    /// already in flight.
    pub fn scan(&mut self, lib: Arc<Library>, active_apps: HashSet<String>, cx: &mut Context<Self>) {
        if self.checking {
            self.pending = Some((lib, active_apps));
            return;
        }
        self.checking = true;
        let paths = self.paths.clone();
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { scan_blocking(&paths, &lib, &active_apps) }).await;
            this.update(cx, |this, cx| {
                this.checking = false;
                this.last_check = Some(Instant::now());
                let mut next = found;
                for (k, v) in &this.debug_overrides {
                    next.insert(k.clone(), v.clone());
                }
                if this.states != next {
                    this.states = next;
                    cx.emit(InstallHealthEvent::Changed);
                    cx.notify();
                }
                if let Some((next_lib, next_active)) = this.pending.take() {
                    this.scan(next_lib, next_active, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Runs [`Self::scan`] if the last scan was more than [`FOCUS_COOLDOWN`] ago.
    pub fn scan_if_stale(&mut self, lib: Arc<Library>, active_apps: HashSet<String>, cx: &mut Context<Self>) {
        if self.last_check.is_none_or(|t| t.elapsed() >= FOCUS_COOLDOWN) {
            self.scan(lib, active_apps, cx);
        }
    }
}
