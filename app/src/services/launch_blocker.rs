//! Why a game cannot be launched right now. The Play button and the launches
//! that have no window (tray, `heronite://`, Steam shortcuts) share it.

use std::path::PathBuf;

use gpui_kit::*;
use heronite_data::install_health::{InstallPresence, MissingReason};
use heronite_data::CollectionGame;
use heronite_i18n::tr;

/// What stands between a game and its launch.
#[derive(Debug, Clone, PartialEq)]
pub enum Blocker {
    /// In the download queue.
    Queued,
    /// An install, update, verification or move is running; its label.
    Busy(String),
    /// The storage drive/mount holding the game is disconnected or unreadable.
    StorageUnavailable(Option<PathBuf>),
    /// The install folder is gone.
    FolderMissing,
    /// The install folder exists, but the game's essential files are missing.
    FilesIncomplete,
    /// An update is waiting (the game can still be played).
    Update,
}

impl Blocker {
    /// A download or an operation owns the game's files.
    pub fn is_busy(&self) -> bool {
        matches!(self, Blocker::Queued | Blocker::Busy(_))
    }

    /// What to tell the user when nobody can be asked (no window): `None` for
    /// an update, which does not stop a launch there.
    pub fn refusal(&self, title: &str) -> Option<String> {
        match self {
            Blocker::Queued => Some(tr!("{title} is waiting in the download queue", title)),
            Blocker::Busy(task) => Some(tr!("{title} is busy ({task})", title, task)),
            Blocker::StorageUnavailable(_) => Some(tr!("The drive holding {title} is not connected", title)),
            Blocker::FolderMissing => Some(tr!("The install folder of {title} is missing", title)),
            Blocker::FilesIncomplete => Some(tr!("The game files of {title} are missing", title)),
            Blocker::Update => None,
        }
    }
}

/// Returns the cached presence of `game` without doing blocking I/O on the UI
/// thread when `InstallHealth` is running (falling back to a single stat in
/// one-shot/headless mode).
pub fn game_presence(cx: &App, game: &CollectionGame) -> InstallPresence {
    if !game.installed {
        return InstallPresence::Present;
    }
    if let Some(health) = super::install_health_started(cx) {
        return health.read(cx).presence(&game.key);
    }
    if game.install_folder_missing() {
        InstallPresence::Missing(MissingReason::FolderGone)
    } else {
        InstallPresence::Present
    }
}

/// Fresh single-game presence check for the Play click / launch path (never
/// called during `render`). Updates the `InstallHealth` cache when started.
fn check_presence_fresh(cx: &mut App, game: &CollectionGame) -> InstallPresence {
    if !game.installed {
        return InstallPresence::Present;
    }
    if let Some(health) = super::install_health_started(cx) {
        return health.update(cx, |s, cx| s.check_one(game, cx));
    }
    let paths = heronite_data::DataPaths::detect();
    let mounts = heronite_data::install_health::Mounts::current();
    let mut cache = heronite_data::install_health::MountCache::load(&paths);
    let home = heronite_platform::dirs::home().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let presence = heronite_data::install_health::check_game_presence(&paths, game, &home, &mounts, &mut cache);
    cache.save(&paths);
    presence
}

/// The first thing that stops `game` from launching, in the order the Play
/// button deals with them. Performs a fresh presence check for this single
/// game so an external uninstall within the focus cooldown is caught before
/// launching.
pub fn launch_blocker(cx: &mut App, game: &CollectionGame) -> Option<Blocker> {
    let is_store = matches!(game.runner.as_str(), "legendary" | "gog" | "nile");
    let downloads = is_store.then(|| super::try_downloads(cx)).flatten();
    if let Some(d) = downloads.as_ref().map(|d| d.read(cx)) {
        if d.queued(&game.app_name).is_some() {
            return Some(Blocker::Queued);
        }
        if let Some(busy) = d.busy_of(&game.app_name) {
            return Some(Blocker::Busy(busy.label.to_string()));
        }
    }
    if !game.installed {
        return None;
    }
    match check_presence_fresh(cx, game) {
        InstallPresence::StorageUnavailable { mount_hint } => return Some(Blocker::StorageUnavailable(mount_hint)),
        InstallPresence::Missing(MissingReason::FolderGone) if is_store && game.store_installed() => {
            return Some(Blocker::FolderMissing);
        }
        InstallPresence::Missing(MissingReason::Incomplete) if is_store && game.store_installed() => {
            return Some(Blocker::FilesIncomplete);
        }
        _ => {}
    }
    if !is_store {
        return None;
    }
    downloads.is_some_and(|d| d.read(cx).has_update(&game.app_name)).then_some(Blocker::Update)
}

#[cfg(test)]
mod tests {
    use super::Blocker;

    #[test]
    fn downloads_block_a_launch_and_an_update_does_not() {
        assert!(Blocker::Queued.is_busy());
        assert!(Blocker::Busy("Verifying".into()).is_busy());
        assert!(!Blocker::FolderMissing.is_busy());
        assert!(!Blocker::FilesIncomplete.is_busy());
        assert!(!Blocker::StorageUnavailable(None).is_busy());
        assert!(!Blocker::Update.is_busy());
        assert_eq!(Blocker::Queued.refusal("Calluna").unwrap(), "Calluna is waiting in the download queue");
        assert_eq!(Blocker::Busy("Verifying".into()).refusal("Calluna").unwrap(), "Calluna is busy (Verifying)");
        assert_eq!(Blocker::FolderMissing.refusal("Calluna").unwrap(), "The install folder of Calluna is missing");
        assert_eq!(Blocker::FilesIncomplete.refusal("Calluna").unwrap(), "The game files of Calluna are missing");
        assert_eq!(Blocker::StorageUnavailable(None).refusal("Calluna").unwrap(), "The drive holding Calluna is not connected");
        assert_eq!(Blocker::Update.refusal("Calluna"), None);
    }
}
