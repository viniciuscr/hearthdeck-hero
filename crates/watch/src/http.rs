//! HTTP, behind a trait so tests use fixtures and a local listener.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use url::Url;

/// A blocking HTTP client. Implementations must cap the body and set a
/// timeout; errors must name the host only, never the path or query (an addon
/// transport URL can carry the person's debrid key).
pub trait Transport: Send + Sync {
    fn get(&self, url: &Url) -> Result<Vec<u8>>;
    fn post_json(&self, url: &Url, body: &Value) -> Result<Vec<u8>>;
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
        let mut response = self
            .agent
            .get(url.as_str())
            .config()
            .timeout_global(Some(self.timeout))
            .build()
            .call()
            .map_err(|e| anyhow!("{}: {e}", redacted(url)))?;
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
            .map_err(|e| anyhow!("{}: {e}", redacted(url)))?;
        read(url, &mut response, self.max_bytes)
    }
}

fn read(url: &Url, response: &mut ureq::http::Response<ureq::Body>, max_bytes: u64) -> Result<Vec<u8>> {
    let status = response.status().as_u16();
    let body = response.body_mut().with_config().limit(max_bytes).read_to_vec().map_err(|e| anyhow!("{}: {e}", redacted(url)))?;
    if !(200..300).contains(&status) {
        bail!("{}: HTTP {status}", redacted(url));
    }
    Ok(body)
}

/// `scheme://host/…`: the path and query stay out of every message because
/// they may carry an addon's configuration.
fn redacted(url: &Url) -> String {
    format!("{}://{}/…", url.scheme(), url.host_str().unwrap_or("?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_hide_the_path_and_query() {
        let url = Url::parse("https://addon.example/secret-debrid-key/manifest.json?token=x").unwrap();
        let text = redacted(&url);
        assert_eq!(text, "https://addon.example/…");
        assert!(!text.contains("secret-debrid-key"));
        assert!(!text.contains("token"));
    }
}
