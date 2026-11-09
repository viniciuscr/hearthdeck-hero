//! The fork's identity: names, paths, app id and release channel.
//!
//! This crate is fork-owned and has no upstream counterpart. The upstream
//! seams that read it are registered in `docs/FORK.md` (F-001, F-002).
//! Environment variables keep upstream's `HERONITE_*` names so upstream's
//! tooling, tests and documentation keep working.

/// The name people see: window title, tray, notifications.
pub const NAME: &str = "Hearthdeck Hero";

/// Reverse-DNS id of the app: window, tray item, notifications, `.desktop`.
pub const APP_ID: &str = "app.hearthdeck.Hero";

/// The `.desktop` file of the app, named after [`APP_ID`].
pub const DESKTOP_FILE: &str = "app.hearthdeck.Hero.desktop";

/// Folder name under the XDG base directories: `~/.local/share/<DATA_DIR>`,
/// `~/.config/<DATA_DIR>`, `~/.cache/<DATA_DIR>`.
pub const DATA_DIR: &str = "hearthdeck";

/// Short key for everything that must not clash with an upstream install:
/// single-instance sockets, launcher log files, Windows registry values.
pub const KEY: &str = "hearthdeck";

/// URL scheme for links (`<SCHEME>://`).
pub const SCHEME: &str = "hearthdeck";

/// GitHub repository that publishes the fork's releases.
pub const UPDATE_REPO: &str = "viniciuscr/hearthdeck-hero";

/// GitHub API endpoint of the latest (non-draft, non-prerelease) release.
pub const LATEST_API: &str = "https://api.github.com/repos/viniciuscr/hearthdeck-hero/releases/latest";

/// The project on GitHub.
pub const REPO_PAGE: &str = "https://github.com/viniciuscr/hearthdeck-hero";

/// The releases page, for people who update by hand.
pub const RELEASES_PAGE: &str = "https://github.com/viniciuscr/hearthdeck-hero/releases/latest";
