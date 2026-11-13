//! Stremio's hosts, as configuration rather than constants: tests inject
//! local fakes and never reach a real service.

use anyhow::{bail, Result};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// `https://api.strem.io/api/`: datastore and addon collection.
    pub api: Url,
    /// `link.stremio.com`: the QR/link-code flow.
    pub link: Url,
    /// Cinemeta: catalogs and metadata.
    pub cinemeta: Url,
    /// The installed Stremio service's local streaming server.
    pub streaming_server: Url,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            api: Url::parse("https://api.strem.io/api/").expect("valid API url"),
            link: Url::parse("https://link.stremio.com").expect("valid link url"),
            cinemeta: Url::parse("https://v3-cinemeta.strem.io").expect("valid Cinemeta url"),
            streaming_server: Url::parse("http://127.0.0.1:11470").expect("valid streaming server url"),
        }
    }
}

impl Endpoints {
    /// Rejects a host that would send a session key or an addon URL somewhere
    /// unexpected, and a "local" streaming server that is not loopback.
    pub fn validate(&self) -> Result<()> {
        for (name, url) in [("api", &self.api), ("link", &self.link), ("cinemeta", &self.cinemeta)] {
            if url.scheme() != "https" {
                bail!("{name} must use https");
            }
        }
        let server = &self.streaming_server;
        if !matches!(server.scheme(), "http" | "https") {
            bail!("streaming server must use http or https");
        }
        let host = server.host_str().unwrap_or_default();
        if !matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]") {
            bail!("streaming server must be on loopback");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        Endpoints::default().validate().unwrap();
    }

    #[test]
    fn the_api_root_keeps_its_path() {
        let api = Endpoints::default().api;
        assert_eq!(api.as_str(), "https://api.strem.io/api/");
        assert_eq!(api.join("addonCollectionGet").unwrap().as_str(), "https://api.strem.io/api/addonCollectionGet");
    }

    #[test]
    fn plain_http_api_is_refused() {
        let e = Endpoints { api: Url::parse("http://api.strem.io").unwrap(), ..Endpoints::default() };
        assert!(e.validate().is_err());
    }

    #[test]
    fn a_remote_streaming_server_is_refused() {
        let e = Endpoints { streaming_server: Url::parse("http://192.0.2.10:11470").unwrap(), ..Endpoints::default() };
        assert!(e.validate().is_err());
    }
}
