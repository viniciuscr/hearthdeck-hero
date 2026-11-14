//! HTTP, behind a trait so tests use fixtures and a local listener.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;
use url::Url;

/// A blocking HTTP client. Implementations must cap the body and set a
/// timeout; errors must name the host only, never the path or query (an addon
/// transport URL can carry the person's debrid key).
pub trait Transport: Send + Sync {
    fn get(&self, url: &Url) -> Result<Vec<u8>>;
    fn post_json(&self, url: &Url, body: &Value) -> Result<Vec<u8>>;
}

/// The status a server answered with, as an error. It carries the redacted
/// endpoint so a caller can tell "the addon has no answer for this title"
/// (a 404) from a fault without matching on error text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpStatus {
    endpoint: String,
    status: u16,
}

impl HttpStatus {
    pub fn new(url: &Url, status: u16) -> Self {
        Self { endpoint: redacted(url), status }
    }

    /// The numeric status: `404`, `503`, …
    pub fn status(&self) -> u16 {
        self.status
    }
}

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: HTTP {}", self.endpoint, self.status)
    }
}

impl std::error::Error for HttpStatus {}

/// The status an error answered with, when the failure was an HTTP status at
/// all (possibly wrapped in `context`); `None` for a timeout, a connection
/// failure or anything else.
pub fn http_status(error: &anyhow::Error) -> Option<u16> {
    error.chain().find_map(|cause| cause.downcast_ref::<HttpStatus>()).map(HttpStatus::status)
}

pub struct UreqTransport {
    agent: ureq::Agent,
    timeout: Duration,
    max_bytes: u64,
}

impl UreqTransport {
    pub fn new() -> Self {
        Self::with_limits(Duration::from_secs(20), 1 << 20)
    }

    pub fn with_limits(timeout: Duration, max_bytes: u64) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(10))).build().into();
        Self { agent, timeout, max_bytes }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for UreqTransport {
    fn get(&self, url: &Url) -> Result<Vec<u8>> {
        let mut response =
            self.agent.get(url.as_str()).config().timeout_global(Some(self.timeout)).build().call().map_err(|e| transport_error(url, e))?;
        read(url, &mut response, self.max_bytes)
    }

    fn post_json(&self, url: &Url, body: &Value) -> Result<Vec<u8>> {
        let text = serde_json::to_string(body)?;
        let mut response = self
            .agent
            .post(url.as_str())
            .config()
            .timeout_global(Some(self.timeout))
            .build()
            .header("content-type", "application/json")
            .send(text.as_str())
            .map_err(|e| transport_error(url, e))?;
        read(url, &mut response, self.max_bytes)
    }
}

/// A ureq failure, with a status kept as a type and everything else reduced
/// to text: a transport error may quote the request URI, which can carry an
/// addon's debrid key.
fn transport_error(url: &Url, error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::StatusCode(status) => HttpStatus::new(url, status).into(),
        other => anyhow!("{}", failure(url, other)),
    }
}

fn read(url: &Url, response: &mut ureq::http::Response<ureq::Body>, max_bytes: u64) -> Result<Vec<u8>> {
    let status = response.status().as_u16();
    let body = response.body_mut().with_config().limit(max_bytes).read_to_vec().map_err(|e| anyhow!("{}", failure(url, e)))?;
    if !(200..300).contains(&status) {
        return Err(HttpStatus::new(url, status).into());
    }
    Ok(body)
}

/// `scheme://host/…: error`, with the URL and its path and query kept out of
/// the error text too: a transport error may quote the request URI, which can
/// carry an addon's debrid key.
fn failure(url: &Url, error: impl std::fmt::Display) -> String {
    let mut text = format!("{}: {error}", redacted(url));
    text = text.replace(url.as_str(), &redacted(url));
    let path = url.path();
    if path.len() > 1 {
        text = text.replace(path, "/…");
    }
    if let Some(query) = url.query() {
        text = text.replace(query, "…");
    }
    text
}

/// `scheme://host/…`: the path and query stay out of every message because
/// they may carry an addon's configuration.
pub fn redacted(url: &Url) -> String {
    format!("{}://{}/…", url.scheme(), url.host_str().unwrap_or("?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transport_error_cannot_quote_the_path_or_query() {
        let url = Url::parse("https://addon.example/secret-debrid-key/manifest.json?token=x").unwrap();
        let text = failure(&url, "connection to https://addon.example/secret-debrid-key/manifest.json?token=x failed");
        assert!(!text.contains("secret-debrid-key"), "{text}");
        assert!(!text.contains("token=x"), "{text}");
        assert!(text.starts_with("https://addon.example/…"), "{text}");
    }

    #[test]
    fn a_status_error_names_the_status_and_the_host_only() {
        let url = Url::parse("https://addon.example/secret-debrid-key/stream/movie/tt0133093.json").unwrap();
        let status = HttpStatus::new(&url, 404);
        assert_eq!(status.status(), 404);
        assert_eq!(status.to_string(), "https://addon.example/…: HTTP 404");
        assert!(!status.to_string().contains("secret-debrid-key"));
    }

    #[test]
    fn the_status_survives_context_and_absent_statuses_are_none() {
        let url = Url::parse("https://addon.example/manifest.json").unwrap();
        let error = anyhow::Error::from(HttpStatus::new(&url, 503)).context("addon manifest");
        assert_eq!(http_status(&error), Some(503));
        assert_eq!(http_status(&anyhow!("connection reset")), None);
    }
}
