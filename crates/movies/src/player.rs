//! mpv command construction and supervision.
//!
//! `--` ends option parsing, only http(s) streams are accepted, and the IPC
//! socket lives in a private `0700` directory. mpv itself keeps the screen
//! awake (`--stop-screensaver` defaults to yes).

use std::path::Path;

use anyhow::{bail, Result};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayRequest {
    pub url: Url,
    pub start_ms: Option<u64>,
    pub title: Option<String>,
}

/// `--` ends option parsing; only http(s) streams are accepted.
pub fn mpv_arguments(request: &PlayRequest, ipc_socket: &Path) -> Result<Vec<String>> {
    if !matches!(request.url.scheme(), "http" | "https") {
        bail!("unsupported stream scheme: {}", request.url.scheme());
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
    use std::fs;
    use std::io::{self, BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result};
    use serde_json::Value;

    use super::{mpv_arguments, PlayRequest};

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

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct PlaybackOutcome {
        pub position_ms: u64,
        pub duration_ms: u64,
    }

    /// One mpv run. Dropping it stops the player and removes the IPC folder.
    pub struct MpvSession {
        child: Child,
        ipc: Option<MpvIpc>,
        dir: PathBuf,
        position_ms: u64,
        duration_ms: u64,
    }

    impl MpvSession {
        pub fn start(mpv: &Path, url: &str, start_ms: Option<u64>, gamepad: bool) -> Result<Self> {
            let url = url::Url::parse(url).context("stream URL")?;
            let dir = std::env::temp_dir().join(format!("hearthdeck-mpv-{}", std::process::id()));
            fs::create_dir_all(&dir)?;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
            let socket = dir.join("mpv.sock");
            let mut args = mpv_arguments(&PlayRequest { url, start_ms, title: None }, &socket)?;
            if gamepad {
                let at = args.len().saturating_sub(2);
                args.insert(at, "--input-gamepad=yes".to_owned());
            }
            let mut cmd = Command::new(mpv);
            cmd.args(&args);
            heronite_data::proc::guard(&mut cmd);
            let child = heronite_data::proc::spawn(&mut cmd).with_context(|| format!("starting {}", mpv.display()))?;
            let ipc = MpvIpc::connect_retry(&socket).ok();
            Ok(Self { child, ipc, dir, position_ms: start_ms.unwrap_or(0), duration_ms: 0 })
        }

        /// Waits for mpv to exit, keeping the last position and duration.
        pub fn wait(&mut self) -> Result<PlaybackOutcome> {
            loop {
                if self.child.try_wait()?.is_some() {
                    break;
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
            let _ = fs::remove_dir_all(&self.dir);
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
pub use unix::{find_mpv, gamepad_supported, MpvSession, PlaybackOutcome};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_comes_after_the_option_terminator() {
        let request =
            PlayRequest { url: Url::parse("http://127.0.0.1:11470/hash/0").unwrap(), start_ms: Some(65_500), title: Some("A Film".into()) };
        let args = mpv_arguments(&request, Path::new("/tmp/ipc")).unwrap();
        let dash = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(dash, args.len() - 2);
        assert_eq!(args.last().unwrap(), "http://127.0.0.1:11470/hash/0");
        assert!(args.contains(&"--start=65.500".to_owned()));
    }

    #[test]
    fn a_non_http_stream_is_refused() {
        let request = PlayRequest { url: Url::parse("file:///etc/passwd").unwrap(), start_ms: None, title: None };
        assert!(mpv_arguments(&request, Path::new("/tmp/ipc")).is_err());
    }
}
