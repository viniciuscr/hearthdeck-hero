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

    /// A POST whose answer is read whatever the status is: an OAuth token
    /// endpoint answers "not yet" with a 400 and a body (`authorization_pending`,
    /// `slow_down`), which is part of the protocol rather than a failure. A
    /// transport that cannot keep the body of a non-2xx answer reports a
    /// successful one, and the caller reads it as it can.
    fn post_json_answer(&self, url: &Url, body: &Value) -> Result<(u16, Vec<u8>)> {
        Ok((200, self.post_json(url, body)?))
    }

    /// A GET carrying a bearer, with the status **and** the body of a refused
    /// answer: a pull needs to read the error code (`max_items`) to split a big
    /// request, which a status alone cannot tell.
    fn get_bearer_answer(&self, url: &Url, token: &str) -> Result<(u16, Vec<u8>)> {
        let _ = (url, token);
        anyhow::bail!("this transport cannot show a refused answer's body")
    }

    /// A POST whose answer comes back whole: status, headers and body. `token`,
    /// when given, is sent as a bearer. A transport that cannot show headers says
    /// so, and the caller falls back to the body-only call.
    fn post_json_full(&self, url: &Url, token: Option<&str>, body: &Value) -> Result<Answer> {
        let _ = (url, token, body);
        anyhow::bail!("this transport cannot show an answer's headers")
    }

    /// A POST carrying `Authorization: Bearer <token>`, again with the status
    /// returned: a tracker reads what a non-2xx answer says ("already watched",
    /// "rate limited") instead of treating every one as a fault. A transport
    /// that cannot send headers says so rather than sending the request
    /// unauthenticated.
    fn post_json_bearer(&self, url: &Url, token: &str, body: &Value) -> Result<(u16, Vec<u8>)> {
        let _ = (url, token, body);
        anyhow::bail!("this transport cannot carry an Authorization header")
    }

    /// A GET whose redirect is **not** followed: the `Location` header of a 3xx,
    /// which is what an id resolver reads (Simkl's `/redirect` turns an IMDb id
    /// into `https://simkl.com/tv/17465/…` without a JSON body). A transport that
    /// follows redirects or cannot show headers says so.
    fn get_location(&self, url: &Url) -> Result<Option<String>> {
        let _ = url;
        anyhow::bail!("this transport cannot read a redirect's Location")
    }

    fn get_bearer(&self, url: &Url, token: &str) -> Result<Vec<u8>> {
        let _ = (url, token);
        anyhow::bail!("this transport cannot carry an Authorization header")
    }
}

/// A whole answer: status, headers and body.
///
/// A tracker needs the headers when the refusal itself says when to come back —
/// `Retry-After` on a daily-quota 429 — and a provider with its own request
/// headers (Trakt's `trakt-api-version`) needs them on the way out too.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Answer {
    /// The header `name`, case-insensitively — HTTP header names are.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
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
        // A 4xx is an answer, not a transport failure: `post_json_answer` has to
        // keep the body (an OAuth token endpoint says "not yet" with a 400), and
        // the other methods turn the status into an `HttpStatus` themselves.
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // A full request carries a real user agent: Simkl requires one, and
            // an anonymous client is the first thing a WAF turns away.
            .user_agent(concat!("hearthdeck-hero/", env!("CARGO_PKG_VERSION"), " (+https://github.com/viniciuscr/hearthdeck-hero)"))
            .timeout_connect(Some(Duration::from_secs(10)))
            .build()
            .into();
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

    fn get_location(&self, url: &Url) -> Result<Option<String>> {
        // `max_redirects(0)`: the response is the 301 itself, never an error.
        let response = self
            .agent
            .get(url.as_str())
            .config()
            .timeout_global(Some(self.timeout))
            .max_redirects(0)
            .build()
            .call()
            .map_err(|e| transport_error(url, e))?;
        let status = response.status().as_u16();
        if !(300..400).contains(&status) {
            return Err(HttpStatus::new(url, status).into());
        }
        Ok(response.headers().get("location").and_then(|value| value.to_str().ok()).map(str::to_owned))
    }

    fn get_bearer(&self, url: &Url, token: &str) -> Result<Vec<u8>> {
        let (status, bytes) = self.get_bearer_answer(url, token)?;
        accepted(url, status, bytes)
    }

    fn get_bearer_answer(&self, url: &Url, token: &str) -> Result<(u16, Vec<u8>)> {
        let mut response = self
            .agent
            .get(url.as_str())
            .config()
            .timeout_global(Some(self.timeout))
            .build()
            .header("authorization", format!("Bearer {token}"))
            .call()
            .map_err(|e| transport_error(url, e))?;
        let status = response.status().as_u16();
        let bytes = response.body_mut().with_config().limit(self.max_bytes).read_to_vec().map_err(|e| anyhow!("{}", failure(url, e)))?;
        Ok((status, bytes))
    }

    fn post_json_full(&self, url: &Url, token: Option<&str>, body: &Value) -> Result<Answer> {
        let text = serde_json::to_string(body)?;
        let mut request =
            self.agent.post(url.as_str()).config().timeout_global(Some(self.timeout)).build().header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let mut response = request.send(text.as_str()).map_err(|e| transport_error(url, e))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.as_str().to_owned(), value.to_owned())))
            .collect();
        let body = response.body_mut().with_config().limit(self.max_bytes).read_to_vec().map_err(|e| anyhow!("{}", failure(url, e)))?;
        Ok(Answer { status, headers, body })
    }

    fn post_json(&self, url: &Url, body: &Value) -> Result<Vec<u8>> {
        let (status, bytes) = self.post_json_answer(url, body)?;
        accepted(url, status, bytes)
    }

    fn post_json_answer(&self, url: &Url, body: &Value) -> Result<(u16, Vec<u8>)> {
        self.post_json_with(url, None, body)
    }

    fn post_json_bearer(&self, url: &Url, token: &str, body: &Value) -> Result<(u16, Vec<u8>)> {
        self.post_json_with(url, Some(token), body)
    }
}

impl UreqTransport {
    /// The one POST: JSON in, status and body out, an optional bearer token on
    /// the way. The status is kept whether or not it is a success.
    fn post_json_with(&self, url: &Url, token: Option<&str>, body: &Value) -> Result<(u16, Vec<u8>)> {
        let text = serde_json::to_string(body)?;
        let mut request =
            self.agent.post(url.as_str()).config().timeout_global(Some(self.timeout)).build().header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let mut response = request.send(text.as_str()).map_err(|e| transport_error(url, e))?;
        let status = response.status().as_u16();
        let bytes = response.body_mut().with_config().limit(self.max_bytes).read_to_vec().map_err(|e| anyhow!("{}", failure(url, e)))?;
        Ok((status, bytes))
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
    accepted(url, status, body)
}

/// The body of a 2xx answer; anything else is the status, as a type.
fn accepted(url: &Url, status: u16, body: Vec<u8>) -> Result<Vec<u8>> {
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
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One request answered with `status` and a JSON body, over a real socket:
    /// the only way to prove what the client does with a non-2xx answer.
    fn answer_once(status: u16, body: &'static str) -> Url {
        answer_naming(status, &[], body)
    }

    /// The same, with headers on the answer: `Retry-After` on a 429.
    fn answer_naming(status: u16, headers: &'static [(&'static str, &'static str)], body: &'static str) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let address = listener.local_addr().expect("the bound address");
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("one request");
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request);
            let extra: String = headers.iter().map(|(name, value)| format!("{name}: {value}\r\n")).collect();
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
        });
        Url::parse(&format!("http://{address}/oauth2/token")).expect("a local url")
    }

    #[test]
    fn a_refused_get_can_show_its_body() {
        let url = answer_once(400, r#"{"error":"max_items"}"#);
        let (status, body) = UreqTransport::new().get_bearer_answer(&url, "tok").expect("an answer");
        assert_eq!(status, 400);
        assert!(String::from_utf8_lossy(&body).contains("max_items"), "the code is readable");
    }

    #[test]
    fn a_post_can_show_the_headers_it_answered_with() {
        let url = answer_naming(429, &[("Retry-After", "42")], r#"{"error":"user_limit_exceeded"}"#);
        let answer = UreqTransport::new().post_json_full(&url, Some("tok"), &serde_json::json!({})).expect("an answer");
        assert_eq!(answer.status, 429);
        assert_eq!(answer.header("retry-after"), Some("42"), "the name is matched case-insensitively");
        assert!(String::from_utf8_lossy(&answer.body).contains("user_limit_exceeded"));
    }

    #[test]
    fn a_four_hundred_keeps_its_body_for_the_caller_that_wants_it() {
        let url = answer_once(400, r#"{"error":"authorization_pending"}"#);
        let (status, body) = UreqTransport::new().post_json_answer(&url, &serde_json::json!({})).expect("the answer is read");
        assert_eq!(status, 400);
        assert_eq!(String::from_utf8_lossy(&body), r#"{"error":"authorization_pending"}"#, "the OAuth error code is the protocol");
    }

    #[test]
    fn a_bearer_get_keeps_the_body_of_a_refused_answer() {
        let url = answer_once(400, r#"{"error":"max_items"}"#);
        let (status, body) = UreqTransport::new().get_bearer_answer(&url, "token").expect("the answer is read");
        assert_eq!(status, 400);
        assert_eq!(String::from_utf8_lossy(&body), r#"{"error":"max_items"}"#, "an importer reads the refusal to split the pull");
    }

    #[test]
    fn a_bearer_get_without_the_body_still_reports_the_status() {
        let url = answer_once(404, "{}");
        let error = UreqTransport::new().get_bearer(&url, "token").expect_err("a refusal is an error on this path");
        assert_eq!(http_status(&error), Some(404));
    }

    #[test]
    fn a_four_hundred_is_still_an_error_for_a_plain_post() {
        let url = answer_once(400, r#"{"error":"authorization_pending"}"#);
        let error = UreqTransport::new().post_json(&url, &serde_json::json!({})).expect_err("a plain POST fails on the status");
        assert_eq!(http_status(&error), Some(400));
        assert!(!error.to_string().contains("authorization_pending"), "the body is not the error's text");
    }

    #[test]
    fn a_not_found_from_a_get_is_the_status_and_nothing_more() {
        let url = answer_once(404, r#"{"detail":"gone"}"#);
        let error = UreqTransport::new().get(&url).expect_err("a 404 is a status");
        assert_eq!(http_status(&error), Some(404));
    }

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
