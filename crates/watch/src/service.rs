//! The Stremio service: the local process torrent playback needs.
//!
//! It answers on loopback (`http://127.0.0.1:11470` by default) and is not
//! ours: the packaged `stremio-service` launcher starts a `stremio-runtime`
//! with its `server.js`. The launcher also builds a tray icon, which fails
//! without a desktop session — on a TV or in a sandbox it exits at once — so
//! the headless runtime is started first, and the launcher only as a fallback.
//!
//! The app owns the process it starts, and stops it again when the session ends
//! if the person asked (`heronite_watch::settings`); a service it did not start
//! is never touched. This module only finds and describes things: the probe,
//! the launches and their commands. Spawning and signalling a process belong to
//! the app, which may use `heronite_data::proc` and
//! `heronite_platform::process`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use url::Url;

use heronite_watch_core::http::{Transport, UreqTransport};

/// How long `/settings` has to answer for a probe (a live service answers at
/// once; a stopped one refuses the connection even faster).
pub const PROBE: Duration = Duration::from_millis(900);
/// How long one launch has to come up before the next one is tried.
pub const ATTEMPT: Duration = Duration::from_secs(8);

/// Where a packaged Stremio puts the launcher when it is not on `PATH`.
const PACKAGED: [&str; 2] = ["/usr/share/stremio-service/stremio-service", "/opt/stremio/stremio-service"];
/// The directories a package may keep the runtime and its `server.js` in.
const PACKAGED_DIRS: [&str; 1] = ["/usr/share/stremio-service"];

/// The variable a person can set to name their own launcher, or the runtime
/// itself (an AppImage's extracted service, say).
pub const LAUNCHER_ENV: &str = "STREMIO_SERVICE";

const RUNTIME: &str = "stremio-runtime";
const SERVER: &str = "server.js";

/// How the service can be started on this system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// The server alone: `stremio-runtime <server.js>`. No tray, no updater,
    /// no desktop session needed.
    Runtime { runtime: PathBuf, server: PathBuf },
    /// The packaged launcher, which manages updates and shows a tray icon.
    Launcher(PathBuf),
}

impl Launch {
    /// The command that starts it: no input, no window, errors captured. The
    /// runtime gets the package's own `ffmpeg`/`ffprobe`, as the launcher gives
    /// it.
    pub fn command(&self) -> Command {
        let mut command = match self {
            Self::Runtime { runtime, server } => {
                let mut command = Command::new(runtime);
                command.arg(server);
                if let Some(dir) = runtime.parent() {
                    command.env("FFMPEG_BIN", dir.join("ffmpeg"));
                    command.env("FFPROBE_BIN", dir.join("ffprobe"));
                }
                command
            }
            Self::Launcher(launcher) => Command::new(launcher),
        };
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        command
    }

    /// What a failure report calls it.
    pub fn name(&self) -> String {
        match self {
            Self::Runtime { runtime, .. } => format!("{} {SERVER}", runtime.display()),
            Self::Launcher(launcher) => launcher.display().to_string(),
        }
    }
}

/// Every way to start the service, in the order to try them: each launcher's
/// headless runtime first, then the launcher itself. Only files that exist are
/// listed, and each launch once.
pub fn launches_in(path_var: Option<&OsStr>, custom: Option<&OsStr>) -> Vec<Launch> {
    let mut found: Vec<Launch> = Vec::new();
    if let Some(custom) = custom {
        let custom = PathBuf::from(custom);
        // `STREMIO_SERVICE` may name the runtime itself.
        if custom.file_name() == Some(OsStr::new(RUNTIME)) {
            if let Some(launch) = runtime_pair(custom.clone()) {
                push_launch(&mut found, launch);
            }
        } else {
            add_launcher(&mut found, custom);
        }
    }
    if let Some(path_var) = path_var {
        for dir in std::env::split_paths(path_var) {
            if !dir.as_os_str().is_empty() {
                add_launcher(&mut found, dir.join("stremio-service"));
            }
        }
    }
    for packaged in PACKAGED {
        add_launcher(&mut found, PathBuf::from(packaged));
    }
    for dir in PACKAGED_DIRS {
        if let Some(launch) = runtime_pair(Path::new(dir).join(RUNTIME)) {
            push_launch(&mut found, launch);
        }
    }
    found
}

fn push_launch(found: &mut Vec<Launch>, launch: Launch) {
    if !found.contains(&launch) {
        found.push(launch);
    }
}

/// The headless runtime that ships next to `launcher` (when it is there), then
/// the launcher itself.
fn add_launcher(found: &mut Vec<Launch>, launcher: PathBuf) {
    if let Some(runtime) = runtime_of(&launcher) {
        push_launch(found, runtime);
    }
    if launcher.is_file() {
        push_launch(found, Launch::Launcher(launcher));
    }
}

/// The runtime that ships next to `launcher`, when it is there.
fn runtime_of(launcher: &Path) -> Option<Launch> {
    // `/usr/bin/stremio-service` is a symlink into the package: follow it, so
    // the runtime and `server.js` next to the real file are found.
    let launcher = std::fs::canonicalize(launcher).ok()?;
    runtime_pair(launcher.parent()?.join(RUNTIME))
}

/// The runtime and its `server.js`, when both exist.
fn runtime_pair(runtime: PathBuf) -> Option<Launch> {
    if !runtime.is_file() {
        return None;
    }
    let server = runtime.parent()?.join(SERVER);
    server.is_file().then_some(Launch::Runtime { runtime, server })
}

/// [`launches_in`] over the running environment.
pub fn launches() -> Vec<Launch> {
    let path_var = std::env::var_os("PATH");
    let custom = std::env::var_os(LAUNCHER_ENV);
    launches_in(path_var.as_deref(), custom.as_deref())
}

/// Whether the service answers `/settings`, and so can be asked for streams.
pub fn is_up(base: &Url) -> bool {
    let Ok(url) = base.join("settings") else { return false };
    UreqTransport::with_limits(PROBE, 1 << 16).get(&url).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        path
    }

    /// A package directory with a launcher, the runtime and `server.js`.
    fn package(root: &Path) -> PathBuf {
        let dir = root.join("stremio-service");
        std::fs::create_dir_all(&dir).unwrap();
        executable(&dir, "stremio-service");
        executable(&dir, RUNTIME);
        std::fs::write(dir.join(SERVER), b"// server").unwrap();
        dir.join("stremio-service")
    }

    #[test]
    fn the_headless_runtime_is_tried_before_the_launcher() {
        let root = tempfile::tempdir().unwrap();
        let launcher = package(root.path());
        // The packaged paths of the machine are listed too, so only the pair
        // that comes from this temp package is asserted.
        let found = launches_in(Some(OsStr::new("")), Some(launcher.as_os_str()));
        match &found[0] {
            Launch::Runtime { runtime, server } => {
                assert_eq!(runtime, &launcher.with_file_name(RUNTIME));
                assert_eq!(server, &launcher.with_file_name(SERVER));
            }
            other => panic!("the runtime comes first, got {other:?}"),
        }
        assert!(matches!(&found[1], Launch::Launcher(path) if path == &launcher));
    }

    #[test]
    fn a_launcher_without_its_runtime_is_still_a_launch() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = executable(dir.path(), "stremio-service");
        let found = launches_in(Some(OsStr::new("")), Some(launcher.as_os_str()));
        assert!(found.contains(&Launch::Launcher(launcher)));
        assert!(
            !found.iter().any(|launch| matches!(launch, Launch::Runtime { runtime, .. } if runtime.parent() == Some(dir.path()))),
            "no runtime next to a launcher that has none: {found:?}"
        );
    }

    #[test]
    fn the_runtime_command_passes_the_server_and_the_packages_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = dir.path().join(RUNTIME);
        let server = dir.path().join(SERVER);
        let command = Launch::Runtime { runtime: runtime.clone(), server: server.clone() }.command();
        assert_eq!(command.get_program(), runtime.as_os_str());
        assert_eq!(command.get_args().collect::<Vec<_>>(), [server.as_os_str()]);
        let (ffmpeg, ffprobe) = (dir.path().join("ffmpeg"), dir.path().join("ffprobe"));
        let env: Vec<_> = command.get_envs().collect();
        assert!(env.iter().any(|(key, value)| *key == OsStr::new("FFMPEG_BIN") && *value == Some(ffmpeg.as_os_str())));
        assert!(env.iter().any(|(key, value)| *key == OsStr::new("FFPROBE_BIN") && *value == Some(ffprobe.as_os_str())));
    }

    #[test]
    fn a_listening_service_is_up_and_a_closed_port_is_not() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(1).flatten() {
                let mut stream = stream;
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap()).read_line(&mut request).unwrap();
                assert!(request.starts_with("GET /settings"), "{request}");
                let body = br#"{"baseUrl":"http://192.168.178.100:11470/"}"#;
                let _ = write!(stream, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                let _ = stream.write_all(body);
            }
        });
        let base = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        assert!(is_up(&base));
        assert!(!is_up(&Url::parse("http://127.0.0.1:1").unwrap()), "nothing listens on port 1");
    }
}
