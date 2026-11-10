//! The QR/link-code flow: `link.stremio.com/api/create` and `api/read`.
//!
//! The code is single-use and short-lived. Neither the code nor the resulting
//! key is ever put in an error or a log line.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::auth::AuthKey;
use crate::http::Transport;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkOffer {
    pub code: String,
    pub link: String,
    pub qrcode: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkPoll {
    /// The person has not confirmed yet, or the code expired.
    Pending,
    Linked(AuthKey),
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    qrcode: Option<String>,
}

pub fn create_link(http: &dyn Transport, link_base: &Url) -> Result<LinkOffer> {
    let body = http.get(&link_base.join("api/create")?)?;
    let envelope: Envelope = serde_json::from_slice(&body).context("link create answered non-JSON")?;
    let from_result = |name: &str| envelope.result.as_ref().and_then(|result| result.get(name)).and_then(Value::as_str).map(str::to_owned);
    let code = envelope.code.or_else(|| from_result("code")).ok_or_else(|| anyhow!("link create answered without a code"))?;
    let link = envelope.link.or_else(|| from_result("link")).ok_or_else(|| anyhow!("link create answered without a link"))?;
    let qrcode = envelope.qrcode.or_else(|| from_result("qrcode"));
    Ok(LinkOffer { code, link, qrcode })
}

pub fn read_link(http: &dyn Transport, link_base: &Url, code: &str) -> Result<LinkPoll> {
    let mut url = link_base.join("api/read")?;
    url.query_pairs_mut().append_pair("code", code);
    let body = http.get(&url)?;
    let envelope: Envelope = serde_json::from_slice(&body).context("link read answered non-JSON")?;
    let key = envelope.result.as_ref().and_then(|result| result.get("authKey")).and_then(Value::as_str);
    match key {
        Some(key) if !key.is_empty() => Ok(LinkPoll::Linked(AuthKey::new(key))),
        _ => Ok(LinkPoll::Pending),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        create: Vec<u8>,
        read: Vec<u8>,
        seen: Mutex<Vec<String>>,
    }

    impl Transport for Fake {
        fn get(&self, url: &Url) -> Result<Vec<u8>> {
            self.seen.lock().unwrap().push(url.to_string());
            if url.path().ends_with("/api/create") {
                Ok(self.create.clone())
            } else {
                Ok(self.read.clone())
            }
        }
        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            unreachable!()
        }
    }

    #[test]
    fn create_returns_the_code_and_link() {
        let fake = Fake {
            create: br#"{"success":true,"code":"GYWV","link":"https://link.stremio.com/GYWV","qrcode":"https://link.stremio.com/qr?data=x","result":{"code":"GYWV","link":"https://link.stremio.com/GYWV"},"error":null}"#.to_vec(),
            read: Vec::new(),
            seen: Mutex::new(Vec::new()),
        };
        let offer = create_link(&fake, &Url::parse("https://link.stremio.com").unwrap()).unwrap();
        assert_eq!(offer.code, "GYWV");
        assert_eq!(offer.link, "https://link.stremio.com/GYWV");
        assert!(offer.qrcode.is_some());
    }

    #[test]
    fn pending_and_linked_are_distinct() {
        let pending = Fake {
            create: Vec::new(),
            read: br#"{"result":null,"error":{"code":101,"message":"Invalid or expired token"}}"#.to_vec(),
            seen: Mutex::new(Vec::new()),
        };
        let linked = Fake {
            create: Vec::new(),
            read: br#"{"result":{"authKey":"secret-key","success":true}}"#.to_vec(),
            seen: Mutex::new(Vec::new()),
        };
        let base = Url::parse("https://link.stremio.com").unwrap();
        assert_eq!(read_link(&pending, &base, "GYWV").unwrap(), LinkPoll::Pending);
        assert_eq!(read_link(&linked, &base, "GYWV").unwrap(), LinkPoll::Linked(AuthKey::new("secret-key")));
    }

    #[test]
    fn the_code_goes_in_the_query() {
        let fake = Fake { create: Vec::new(), read: br#"{"result":null}"#.to_vec(), seen: Mutex::new(Vec::new()) };
        let base = Url::parse("https://link.stremio.com").unwrap();
        let _ = read_link(&fake, &base, "GYWV");
        assert!(fake.seen.lock().unwrap()[0].contains("code=GYWV"));
    }
}
