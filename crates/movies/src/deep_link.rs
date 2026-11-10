//! Stremio's player deep link: `stremio:///player/<encoded stream>`.
//!
//! The stream is JSON, zlib-compressed with no compression, base64-encoded and
//! percent-encoded with Stremio's own set. The test pins the torrent form to
//! the value `stremio-core` produces, so the encoding is verified even before
//! the link is tried on hardware (docs/MOVIES.md, probe 4).

use std::io::Write;

use anyhow::Result;
use base64::Engine;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum StreamSource {
    Url {
        url: String,
    },
    #[serde(rename_all = "camelCase")]
    Torrent {
        info_hash: String,
        file_idx: Option<u16>,
        announce: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamLink {
    #[serde(flatten)]
    pub source: StreamSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl StreamLink {
    pub fn torrent(info_hash: impl Into<String>, file_idx: Option<u16>) -> Self {
        Self {
            source: StreamSource::Torrent { info_hash: info_hash.into(), file_idx, announce: Vec::new() },
            name: None,
            description: None,
        }
    }

    pub fn url(url: impl Into<String>) -> Self {
        Self { source: StreamSource::Url { url: url.into() }, name: None, description: None }
    }
}

pub fn player_deep_link(stream: &StreamLink) -> Result<String> {
    let json = serde_json::to_vec(stream)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::none());
    encoder.write_all(&json)?;
    let compressed = encoder.finish()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(compressed);
    Ok(format!("stremio:///player/{}", percent_encode(&encoded)))
}

/// The set `stremio-core` keeps unescaped (`URI_COMPONENT_ENCODE_SET`).
fn percent_encode(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c";

    #[test]
    fn the_torrent_player_link_matches_stremio_core() {
        let link = player_deep_link(&StreamLink::torrent(HASH, None)).unwrap();
        assert_eq!(
            link,
            "stremio:///player/eAEBVACr%2F3siaW5mb0hhc2giOiJkZDgyNTVlY2RjN2NhNTVmYjBiYmY4MTMyM2Q4NzA2MmRiMWY2ZDFjIiwiZmlsZUlkeCI6bnVsbCwiYW5ub3VuY2UiOltdfVAyGnc%3D"
        );
    }

    #[test]
    fn a_url_stream_still_encodes() {
        let link = player_deep_link(&StreamLink::url("http://domain.root/path")).unwrap();
        assert!(link.starts_with("stremio:///player/"), "{link}");
        assert!(!link.contains(' '));
    }

    #[test]
    fn the_encoding_set_matches_stremio() {
        assert_eq!(percent_encode("a+/= z"), "a%2B%2F%3D%20z");
        assert_eq!(percent_encode("-_.!~*'()"), "-_.!~*'()");
    }
}
