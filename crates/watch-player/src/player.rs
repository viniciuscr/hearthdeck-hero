//! mpv command construction and supervision.
//!
//! `--` ends option parsing, only http(s) streams are accepted, and the IPC
//! socket lives in a private `0700` directory. mpv itself keeps the screen
//! awake (`--stop-screensaver` defaults to yes).

use std::path::Path;

use anyhow::{bail, Result};
use heronite_watch_core::provider::StreamOrigin;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayRequest {
    pub url: Url,
    /// Where the URL points; the scheme and host it may use follow from it.
    pub origin: StreamOrigin,
    pub start_ms: Option<u64>,
    pub title: Option<String>,
}

/// Where a session ended, in milliseconds. Target-independent, so a build
/// without a Unix player can still name its result type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaybackOutcome {
    pub position_ms: u64,
    pub duration_ms: u64,
}

/// `--` ends option parsing; the URL must fit its origin (https remotely, a
/// loopback service locally).
pub fn mpv_arguments(request: &PlayRequest, ipc_socket: &Path) -> Result<Vec<String>> {
    if !request.origin.allows(&request.url) {
        let expected = match request.origin {
            StreamOrigin::Remote => "an https URL",
            StreamOrigin::LocalService => "a loopback service URL",
        };
        bail!("refusing a stream that is not {expected}: {}://{}", request.url.scheme(), request.url.host_str().unwrap_or("?"));
    }
    if request.url.as_str().starts_with('-') {
        bail!("refusing a stream URL that looks like an option");
    }
    let mut args = vec!["--fullscreen".to_owned(), "--force-window=yes".to_owned(), format!("--input-ipc-server={}", ipc_socket.display())];
    if let Some(ms) = request.start_ms {
        args.push(format!("--start={:.3}", ms as f64 / 1000.0));
    }
    if let Some(title) = &request.title {
        args.push(format!("--title={title}"));
    }
    args.push("--".to_owned());
    args.push(request.url.to_string());
    Ok(args)
}

#[cfg(unix)]
mod unix {
    use std::io::{self, BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result};
    use serde_json::Value;

    use super::{mpv_arguments, PlayRequest, PlaybackOutcome};

    /// `mpv` on `PATH`.
    pub fn find_mpv() -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).map(|dir| dir.join("mpv")).find(|candidate| candidate.is_file())
    }

    /// Whether this mpv was built with SDL gamepad support (`--input-gamepad`).
    pub fn gamepad_supported(mpv: &Path) -> bool {
        static SUPPORTED: OnceLock<bool> = OnceLock::new();
        *SUPPORTED.get_or_init(|| {
            Command::new(mpv)
                .arg("--list-options")
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).contains("input-gamepad"))
                .unwrap_or(false)
        })
    }

    /// One mpv run. Dropping it stops the player and removes the IPC folder.
    pub struct MpvSession {
        child: Child,
        ipc: Option<MpvIpc>,
        /// Held for its drop: a `TempDir` removes the IPC folder.
        _dir: tempfile::TempDir,
        position_ms: u64,
        duration_ms: u64,
    }

    impl MpvSession {
        pub fn start(mpv: &Path, request: &PlayRequest, gamepad: bool) -> Result<Self> {
            // `TempDir` creates the folder `0700` atomically under a random
            // name, so a same-user process cannot pre-plant it as a symlink.
            let dir = tempfile::Builder::new().prefix("hearthdeck-mpv-").tempdir()?;
            let socket = dir.path().join("mpv.sock");
            let mut args = mpv_arguments(request, &socket)?;
            if gamepad {
                let at = args.len().saturating_sub(2);
                args.insert(at, "--input-gamepad=yes".to_owned());
            }
            let mut cmd = Command::new(mpv);
            cmd.args(&args);
            heronite_data::proc::guard(&mut cmd);
            let child = heronite_data::proc::spawn(&mut cmd).with_context(|| format!("starting {}", mpv.display()))?;
            let ipc = MpvIpc::connect_retry(&socket).ok();
            Ok(Self { child, ipc, _dir: dir, position_ms: request.start_ms.unwrap_or(0), duration_ms: 0 })
        }

        /// Waits for mpv to exit, keeping the last position and duration.
        pub fn wait(&mut self) -> Result<PlaybackOutcome> {
            self.wait_until(&std::sync::atomic::AtomicBool::new(false))
        }

        /// [`wait`](Self::wait), quitting mpv once `stop` is set.
        pub fn wait_until(&mut self, stop: &std::sync::atomic::AtomicBool) -> Result<PlaybackOutcome> {
            use std::sync::atomic::Ordering;
            let mut stop_sent = None;
            loop {
                if stop.load(Ordering::Relaxed) && stop_sent.is_none() {
                    self.stop();
                    stop_sent = Some(Instant::now());
                }
                if self.child.try_wait()?.is_some() {
                    break;
                }
                // A player that ignored the IPC quit still ends within a wait.
                if stop_sent.is_some_and(|since| since.elapsed() > Duration::from_secs(5)) {
                    let _ = self.child.kill();
                }
                if let Some(ipc) = &mut self.ipc {
                    if let Some(position) = ipc.property("time-pos") {
                        self.position_ms = (position.max(0.) * 1000.) as u64;
                    }
                    if let Some(duration) = ipc.property("duration") {
                        self.duration_ms = (duration.max(0.) * 1000.) as u64;
                    }
                }
                std::thread::sleep(Duration::from_millis(700));
            }
            Ok(PlaybackOutcome { position_ms: self.position_ms, duration_ms: self.duration_ms })
        }

        pub fn stop(&mut self) {
            if let Some(ipc) = &mut self.ipc {
                let _ = ipc.command(&serde_json::json!({"command": ["quit"]}));
            }
        }
    }

    impl Drop for MpvSession {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            // `_dir` (a `TempDir`) removes the IPC folder here.
        }
    }

    struct MpvIpc {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
        next_id: u64,
    }

    impl MpvIpc {
        fn connect(path: &Path) -> io::Result<Self> {
            let stream = UnixStream::connect(path)?;
            stream.set_read_timeout(Some(Duration::from_millis(250)))?;
            Ok(Self { reader: BufReader::new(stream.try_clone()?), writer: stream, next_id: 1 })
        }

        fn connect_retry(path: &Path) -> io::Result<Self> {
            let start = Instant::now();
            loop {
                match Self::connect(path) {
                    Ok(ipc) => return Ok(ipc),
                    Err(_) if start.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(100)),
                    Err(error) => return Err(error),
                }
            }
        }

        fn command(&mut self, command: &Value) -> io::Result<()> {
            let mut line = serde_json::to_string(command).map_err(io::Error::other)?;
            line.push('\n');
            self.writer.write_all(line.as_bytes())
        }

        fn property(&mut self, name: &str) -> Option<f64> {
            let id = self.next_id;
            self.next_id += 1;
            let command = serde_json::json!({"command": ["get_property", name], "request_id": id});
            if self.command(&command).is_err() {
                return None;
            }
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                let mut line = String::new();
                match self.reader.read_line(&mut line) {
                    Ok(0) => return None,
                    Ok(_) => {
                        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                        if value.get("request_id").and_then(Value::as_u64) != Some(id) {
                            continue;
                        }
                        return value
                            .get("data")
                            .and_then(|data| data.as_f64().or_else(|| data.as_str().and_then(|text| text.parse().ok())));
                    }
                    Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
                    Err(_) => return None,
                }
            }
            None
        }
    }

    /// Keeps the import used on a build that turns out to have no Unix path.
    #[allow(dead_code)]
    fn _assert_path(_: &Path) {}
}

#[cfg(unix)]
pub use unix::{find_mpv, gamepad_supported, MpvSession};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_comes_after_the_option_terminator() {
        let request = PlayRequest {
            url: Url::parse("http://127.0.0.1:11470/hash/0").unwrap(),
            origin: StreamOrigin::LocalService,
            start_ms: Some(65_500),
            title: Some("A Film".into()),
        };
        let args = mpv_arguments(&request, Path::new("/tmp/ipc")).unwrap();
        let dash = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(dash, args.len() - 2);
        assert_eq!(args.last().unwrap(), "http://127.0.0.1:11470/hash/0");
        assert!(args.contains(&"--start=65.500".to_owned()));
    }

    #[test]
    fn a_non_http_stream_is_refused() {
        let request =
            PlayRequest { url: Url::parse("file:///etc/passwd").unwrap(), origin: StreamOrigin::Remote, start_ms: None, title: None };
        assert!(mpv_arguments(&request, Path::new("/tmp/ipc")).is_err());
    }

    #[test]
    fn a_remote_stream_must_be_https() {
        let args = |url: &str| {
            let request = PlayRequest { url: Url::parse(url).unwrap(), origin: StreamOrigin::Remote, start_ms: None, title: None };
            mpv_arguments(&request, Path::new("/tmp/ipc"))
        };
        assert!(args("https://debrid.example/film").is_ok());
        assert!(args("http://debrid.example/film").is_err());
    }

    #[test]
    fn the_local_service_must_stay_on_loopback() {
        let args = |url: &str| {
            let request = PlayRequest { url: Url::parse(url).unwrap(), origin: StreamOrigin::LocalService, start_ms: None, title: None };
            mpv_arguments(&request, Path::new("/tmp/ipc"))
        };
        assert!(args("http://127.0.0.1:11470/hash/0").is_ok());
        assert!(args("http://localhost:11470/hash/0").is_ok());
        assert!(args("http://192.0.2.10:11470/hash/0").is_err());
    }
}
