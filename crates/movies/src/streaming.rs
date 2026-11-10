//! The installed Stremio service's local streaming server.
//!
//! **Probe-gated** (docs/MOVIES.md): `/settings` and `/{hash}/create` follow
//! `stremio-core`, but the exact playback URL is confirmed on the target
//! machine before Phase 5 ships. The service is a separate process; "localhost"
//! is not first-party.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::http::Transport;

pub struct StreamingServer<'a> {
    http: &'a dyn Transport,
    base: Url,
}

impl<'a> StreamingServer<'a> {
    pub fn new(http: &'a dyn Transport, base: Url) -> Result<Self> {
        let host = base.host_str().unwrap_or_default();
        if !matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]") {
            bail!("streaming server must be on loopback");
        }
        Ok(Self { http, base: directory(base) })
    }

    /// The service's playback base (`baseUrl`), not the settings URL.
    pub fn base_url(&self) -> Result<Url> {
        #[derive(Deserialize)]
        struct Envelope {
            #[serde(rename = "baseUrl")]
            base_url: Url,
        }
        let body = self.http.get(&self.base.join("settings")?)?;
        let envelope: Envelope = serde_json::from_slice(&body).context("streaming server settings")?;
        Ok(envelope.base_url)
    }

    /// Asks the service to fetch and seed a torrent. The service manages the
    /// cache and peers; we only read the resulting HTTP stream.
    pub fn create(&self, info_hash: &str, announce: &[String]) -> Result<()> {
        validate_hash(info_hash)?;
        let url = self.base.join(&format!("{info_hash}/create"))?;
        let mut body = json!({"torrent": {"infoHash": info_hash}});
        if !announce.is_empty() {
            let mut sources = vec![format!("dht:{info_hash}")];
            sources.extend(announce.iter().cloned());
            body["peerSearch"] = json!({"sources": sources, "min": 40, "max": 200});
        }
        let _: Value = serde_json::from_slice(&self.http.post_json(&url, &body)?).unwrap_or(Value::Null);
        Ok(())
    }

    /// The URL a player opens for `info_hash`'s file at `file_idx`.
    pub fn stream_url(base: &Url, info_hash: &str, file_idx: Option<u16>) -> Result<Url> {
        validate_hash(info_hash)?;
        directory(base.clone()).join(&format!("{info_hash}/{}", file_idx.unwrap_or(0))).context("stream url")
    }
}

fn directory(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let mut path = url.path().to_owned();
        path.push('/');
        url.set_path(&path);
    }
    url
}

fn validate_hash(info_hash: &str) -> Result<()> {
    if info_hash.len() != 40 || !info_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid info hash");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        settings: Vec<u8>,
        body: Mutex<Option<Value>>,
    }

    impl Transport for Fake {
        fn get(&self, _: &Url) -> Result<Vec<u8>> {
            Ok(self.settings.clone())
        }
        fn post_json(&self, _: &Url, body: &Value) -> Result<Vec<u8>> {
            *self.body.lock().unwrap() = Some(body.clone());
            Ok(b"{}".to_vec())
        }
    }

    const HASH: &str = "dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c";

    #[test]
    fn settings_returns_the_playback_base() {
        let fake = Fake { settings: br#"{"baseUrl":"http://127.0.0.1:11470/","values":{}}"#.to_vec(), body: Mutex::new(None) };
        let server = StreamingServer::new(&fake, Url::parse("http://127.0.0.1:11470").unwrap()).unwrap();
        assert_eq!(server.base_url().unwrap().as_str(), "http://127.0.0.1:11470/");
    }

    #[test]
    fn create_sends_the_hash_and_trackers() {
        let fake = Fake { settings: Vec::new(), body: Mutex::new(None) };
        let server = StreamingServer::new(&fake, Url::parse("http://127.0.0.1:11470").unwrap()).unwrap();
        server.create(HASH, &["tracker:udp://tracker.invalid:80".into()]).unwrap();
        let body = fake.body.lock().unwrap().clone().unwrap();
        assert_eq!(body["torrent"]["infoHash"], HASH);
        assert_eq!(body["peerSearch"]["sources"][0], format!("dht:{HASH}"));
        assert_eq!(body["peerSearch"]["sources"][1], "tracker:udp://tracker.invalid:80");
    }

    #[test]
    fn stream_url_uses_the_base_and_file_index() {
        let url = StreamingServer::stream_url(&Url::parse("http://127.0.0.1:11470").unwrap(), HASH, Some(2)).unwrap();
        assert_eq!(url.as_str(), format!("http://127.0.0.1:11470/{HASH}/2"));
    }

    #[test]
    fn a_remote_service_is_refused() {
        let fake = Fake { settings: Vec::new(), body: Mutex::new(None) };
        assert!(StreamingServer::new(&fake, Url::parse("http://192.0.2.1:11470").unwrap()).is_err());
    }

    #[test]
    fn a_bad_hash_is_refused() {
        assert!(StreamingServer::stream_url(&Url::parse("http://127.0.0.1:11470").unwrap(), "nope", None).is_err());
    }
}
