//! Player command construction. The supervisor (spawn, IPC, focus return) is
//! Phase 5; this module owns the argument rules that keep a stream URL from
//! being parsed as an mpv option.

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
