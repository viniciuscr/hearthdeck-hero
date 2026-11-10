//! End-to-end protocol tests against fixtures and a fake transport. No test
//! here reaches the network.

use std::sync::Mutex;

use anyhow::{anyhow, Result};
use serde_json::Value;
use url::Url;

use heronite_movies::account::{create_link, read_link, LinkPoll};
use heronite_movies::addon::AddonClient;
use heronite_movies::datastore::{continue_watching, DatastoreClient};
use heronite_movies::endpoints::Endpoints;
use heronite_movies::streaming::StreamingServer;
use heronite_movies::{AuthKey, Transport};

const HASH: &str = "dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c";

struct Fake {
    calls: Mutex<Vec<(String, Option<Value>)>>,
}

impl Fake {
    fn new() -> Self {
        Self { calls: Mutex::new(Vec::new()) }
    }

    fn body(&self, suffix: &str) -> Value {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .find(|(path, body)| path.ends_with(suffix) && body.is_some())
            .and_then(|(_, body)| body.clone())
            .unwrap_or_else(|| panic!("no request body ending in {suffix}"))
    }
}

impl Transport for Fake {
    fn get(&self, url: &Url) -> Result<Vec<u8>> {
        self.calls.lock().unwrap().push((url.path().to_owned(), None));
        let body = match url.path() {
            "/api/create" => include_str!("fixtures/link_create.json"),
            "/api/read" => include_str!("fixtures/link_linked.json"),
            path if path.ends_with("/manifest.json") => include_str!("fixtures/manifest.json"),
            path if path.ends_with("/settings") => include_str!("fixtures/settings.json"),
            path if path.contains("/stream/") => include_str!("fixtures/streams.json"),
            other => return Err(anyhow!("unexpected GET path {other}")),
        };
        Ok(body.as_bytes().to_vec())
    }

    fn post_json(&self, url: &Url, body: &Value) -> Result<Vec<u8>> {
        self.calls.lock().unwrap().push((url.path().to_owned(), Some(body.clone())));
        if url.path().ends_with("datastorePut") {
            Ok(br#"{"result":{"success":true}}"#.to_vec())
        } else {
            Ok(include_str!("fixtures/datastore.json").as_bytes().to_vec())
        }
    }
}

#[test]
fn link_flow_returns_a_code_and_a_session_key() {
    let fake = Fake::new();
    let base = Url::parse("https://link.stremio.com").unwrap();
    let offer = create_link(&fake, &base).unwrap();
    assert_eq!(offer.code, "GYWV");
    assert_eq!(offer.link, "https://link.stremio.com/GYWV");
    assert!(offer.qrcode.is_some());
    assert!(matches!(read_link(&fake, &base, "GYWV").unwrap(), LinkPoll::Linked(_)));
}

#[test]
fn the_library_keeps_unknown_fields_and_continue_watching_works() {
    let fake = Fake::new();
    let endpoints = Endpoints::default();
    let client = DatastoreClient::new(&fake, &endpoints, AuthKey::new("test-session-key"));
    let items = client.get_all().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "tt26657236");
    assert_eq!(items[0].extra.get("futureTop"), Some(&Value::Bool(true)));
    assert_eq!(items[0].state.extra.get("futureState"), Some(&Value::from(7)));
    assert_eq!(continue_watching(&items).len(), 1);

    client.put(&items).unwrap();
    let body = fake.body("datastorePut");
    assert_eq!(body["collection"], "libraryItem");
    assert_eq!(body["authKey"], "test-session-key");
    assert_eq!(body["changes"][0]["_id"], "tt26657236");
    assert_eq!(body["changes"][0]["futureTop"], true);
}

#[test]
fn an_addon_keeps_its_configured_path_and_parses_streams() {
    let fake = Fake::new();
    let mut client = AddonClient::new(&fake, "https://torrentio.strem.fun/providers=yts/manifest.json").unwrap();
    let manifest = client.manifest().unwrap();
    assert_eq!(manifest.id, "community.torrentio");
    assert!(manifest.has_resource("stream"));

    let streams = client.stream("movie", "tt26657236").unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].addon, "community.torrentio");
    assert_eq!(streams[0].info_hash.as_deref(), Some(HASH));
    assert_eq!(streams[0].file_idx, Some(0));
    assert_eq!(streams[0].seeders, Some(123));
    assert_eq!(streams[0].size_bytes, Some(1_500_000_000));
    assert!(fake.calls.lock().unwrap().iter().any(|(path, _)| path.starts_with("/providers=yts/stream/")));
}

#[test]
fn the_streaming_server_creates_a_torrent_and_builds_the_stream_url() {
    let fake = Fake::new();
    let server = StreamingServer::new(&fake, Url::parse("http://127.0.0.1:11470").unwrap()).unwrap();
    assert_eq!(server.base_url().unwrap().as_str(), "http://127.0.0.1:11470/");
    server.create(HASH, &[]).unwrap();
    assert_eq!(fake.body("/create")["torrent"]["infoHash"], HASH);
    let url = StreamingServer::stream_url(&Url::parse("http://127.0.0.1:11470/").unwrap(), HASH, Some(1)).unwrap();
    assert_eq!(url.as_str(), format!("http://127.0.0.1:11470/{HASH}/1"));
}
