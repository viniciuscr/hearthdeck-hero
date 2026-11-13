//! The contract every Watch provider implements, and the ports it is given.
//!
//! A provider never opens a file or spawns a process: the transport and its own
//! credentials arrive in [`Ctx`], and the facade owns everything else.

use std::fmt;

use url::Url;

use crate::http::Transport;
use crate::types::{MovieItem, MovieProgress, StreamChoice};
use crate::Result;

/// One home rail a provider contributes, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogDescriptor {
    /// Stable id, e.g. `continue-watching` or `top`.
    pub id: &'static str,
    /// `movie`, `series`, or empty when the rail mixes kinds.
    pub kind: &'static str,
}

/// A catalog and the items it answered with.
#[derive(Clone, Debug)]
pub struct Rail {
    pub catalog: CatalogDescriptor,
    pub items: Vec<MovieItem>,
}

/// What a provider is given: the transport, and its own credentials.
pub struct Ctx<'a> {
    pub http: &'a dyn Transport,
    pub credentials: &'a dyn Credentials,
}

/// A media source the Watch space can show.
///
/// The facade asks every provider for its rails, assembles one library, and
/// routes playback to the provider named by the game's `runner`.
pub trait WatchProvider: Send + Sync {
    /// The value `GameInfo.runner` carries, e.g. `stremio`.
    fn id(&self) -> &'static str;

    /// How the provider is named in an error or a log line.
    fn name(&self) -> &'static str;

    /// Every home rail, in display order. A provider that cannot reach its
    /// service returns an error and the facade keeps the last good library.
    fn browse(&self, ctx: &Ctx<'_>) -> Result<Vec<Rail>>;

    /// The playable streams of one title, best first.
    fn streams(&self, ctx: &Ctx<'_>, kind: &str, id: &str) -> Result<Vec<StreamChoice>>;

    /// The URL the player opens for a picked stream.
    fn resolve(&self, ctx: &Ctx<'_>, stream: &StreamChoice) -> Result<Url>;

    /// Writes cross-device progress. A provider without history does nothing.
    fn write_progress(&self, _ctx: &Ctx<'_>, _progress: &MovieProgress) -> Result<()> {
        Ok(())
    }
}

/// A provider's credentials, opaque to the core: each provider decodes its own
/// shape. Its `Debug` is redacted, like `AuthKey`'s.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Secret(serde_json::Value);

impl Secret {
    pub fn new(value: serde_json::Value) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }

    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_value(self.0.clone())?)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Per-provider credential storage. The state crate implements it over the
/// app's files; tests implement it over a map.
pub trait Credentials: Send + Sync {
    fn get(&self, provider: &str) -> Result<Option<Secret>>;
    fn set(&self, provider: &str, secret: &Secret) -> Result<()>;
    fn clear(&self, provider: &str) -> Result<()>;
}
