//! The Stremio service: the local process torrent playback needs.
//!
//! It answers on loopback (`http://127.0.0.1:11470` by default) and is not
//! ours: on Linux it is the standalone `stremio-service` launcher, which stays
//! in the foreground and starts `stremio-runtime server.js` itself. When it is
//! not running, the app starts it — the app owns that process, and stops it
//! again when the session ends if the person asked (`heronite_watch::settings`)
//! — while a service it did not start is never touched.
//!
//! This module only finds and describes things: the probe, the launcher and the
//! command. Spawning and signalling a process belong to the app, which may use
//! `heronite_data::proc` and `heronite_platform::process`.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use url::Url;

use heronite_watch_core::http::{Transport, UreqTransport};

/// How long `/settings` has to answer for a probe (a live service answers at
/// once; a stopped one refuses the connection even faster).
pub const PROBE: Duration = Duration::from_millis(900);
/// How long a freshly started service has to come up (the torrent engine boots
/// its runtime before it listens).
pub const START_TIMEOUT: Duration = Duration::from_secs(20);

/// Where a packaged Stremio puts the launcher when it is not on `PATH`.
const PACKAGED: [&str; 2] = ["/usr/share/stremio-service/stremio-service", "/opt/stremio/stremio-service"];

/// The variable a person can set to name their own launcher (an AppImage's
/// extracted service, say).
pub const LAUNCHER_ENV: &str = "STREMIO_SERVICE";

/// Every launcher this system has, in the order to try them: `STREMIO_SERVICE`
/// first, then `PATH`, then the packaged paths. Only files that exist are
/// listed, and each path once.
pub fn candidates_in(path_var: Option<&OsStr>, custom: Option<&OsStr>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    };
    if let Some(custom) = custom {
        push(PathBuf::from(custom));
    }
    if let Some(path_var) = path_var {
        for dir in std::env::split_paths(path_var) {
            if !dir.as_os_str().is_empty() {
                push(dir.join("stremio-service"));
            }
        }
    }
    for packaged in PACKAGED {
        push(PathBuf::from(packaged));
    }
    found
}

/// [`candidates_in`] over the running environment.
pub fn candidates() -> Vec<PathBuf> {
    let path_var = std::env::var_os("PATH");
    let custom = std::env::var_os(LAUNCHER_ENV);
    candidates_in(path_var.as_deref(), custom.as_deref())
}

/// The launcher to start, if this system has one.
pub fn launcher() -> Option<PathBuf> {
    candidates().into_iter().next()
}

/// Whether the service answers `/settings`, and so can be asked for streams.
pub fn is_up(base: &Url) -> bool {
    let Ok(url) = base.join("settings") else { return false };
    UreqTransport::with_limits(PROBE, 1 << 16).get(&url).is_ok()
}

/// The command that starts `launcher`: no input, no window of its own, its
/// error output read for a failure report.
pub fn command(launcher: &std::path::Path) -> Command {
    let mut command = Command::new(launcher);
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    fn executable(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        path
    }

    #[test]
    fn the_candidates_are_the_existing_launchers_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path_dir = dir.path().join("bin");
        std::fs::create_dir_all(&path_dir).unwrap();
        let on_path = executable(&path_dir, "stremio-service");
        let custom = executable(dir.path(), "my-service");

        let path_var = path_dir.clone().into_os_string();
        let found = candidates_in(Some(path_var.as_os_str()), Some(custom.as_os_str()));
        assert_eq!(found.first(), Some(&custom), "an explicit launcher wins");
        assert_eq!(found.get(1), Some(&on_path), "then PATH");
        assert!(found.iter().filter(|candidate| **candidate == custom || **candidate == on_path).count() == 2);
    }

    #[test]
    fn a_missing_launcher_is_not_a_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nowhere").into_os_string();
        let empty = dir.path().as_os_str().to_owned();
        let found = candidates_in(Some(empty.as_os_str()), Some(missing.as_os_str()));
        assert!(!found.iter().any(|candidate| candidate.ends_with("nowhere")));
        assert!(found.iter().all(|candidate| candidate.is_file()));
    }

    #[test]
    fn the_command_gives_the_service_no_stdin_and_captures_its_errors() {
        let command = command(std::path::Path::new("/usr/bin/stremio-service"));
        assert_eq!(command.get_program(), OsStr::new("/usr/bin/stremio-service"));
        assert!(command.get_args().next().is_none(), "the launcher takes no arguments");
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
                let body = br#"{"baseUrl":"http://127.0.0.1:1/"}"#;
                let _ = write!(stream, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                let _ = stream.write_all(body);
            }
        });
        let base = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        assert!(is_up(&base));
        assert!(!is_up(&Url::parse("http://127.0.0.1:1").unwrap()), "nothing listens on port 1");
    }
}
