//! The contract every Watch provider implements, and the ports it is given.
//!
//! A provider never opens a file or spawns a process: the transport and its own
//! credentials arrive in [`Ctx`], and the facade owns everything else.

use std::fmt;

use url::Url;

use crate::http::Transport;
use crate::types::{MovieItem, MovieProgress, StreamChoice};
use crate::Result;

/// Which page of which catalog: what the Movies and Series tabs ask for.
///
/// The provider decides the page size (Cinemeta answers 50); `skip` is how many
/// items of this catalog the caller already has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogRequest {
    /// `movie` or `series`.
    pub kind: String,
    /// The provider's catalog id (`top`), empty for its default.
    pub catalog: String,
    /// Items to skip.
    pub skip: u32,
    /// A search of the provider's catalog; `None` for the plain catalog.
    pub search: Option<String>,
    /// The order to ask for; a sort the descriptor does not declare is not sent.
    pub sort: CatalogSort,
    /// The chosen filters as `filter id → value` (`genre` → `Action`).
    pub filters: Vec<(String, String)>,
}

impl CatalogRequest {
    /// The first page of `kind`'s default catalog.
    pub fn first(kind: &str) -> Self {
        Self { kind: kind.to_owned(), ..Self::default() }
    }
}

/// One home rail a provider contributes, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RailDescriptor {
    /// Stable id, e.g. `continue-watching` or `top`.
    pub id: &'static str,
    /// `movie`, `series`, or empty when the rail mixes kinds.
    pub kind: &'static str,
}

/// A rail and the items it answered with.
#[derive(Clone, Debug)]
pub struct Rail {
    pub catalog: RailDescriptor,
    pub items: Vec<MovieItem>,
}

/// How a catalog can be ordered by the provider.
///
/// A closed set on purpose: the screens label and order these, and the provider
/// picks the ones it can page by. A provider that can only send its own order
/// declares none (or just [`Popular`](Self::Popular)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogSort {
    /// The provider's own order (Cinemeta's "Popular").
    #[default]
    Popular,
    /// Best rated first.
    Rating,
    /// Newest first.
    Newest,
    /// Alphabetically by title.
    Title,
}

/// A filter a catalog takes, with the values it offers. The ids are the
/// provider's (`genre`, `year`, …); the screens draw one row per filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogFilter {
    pub id: String,
    /// How the row is named; the screens localize the ids they know.
    pub label: String,
    pub values: Vec<String>,
    /// A value has to be chosen before the catalog can be asked.
    pub required: bool,
}

/// One catalog a provider offers the Movies and Series tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogDescriptor {
    pub id: String,
    /// How the chip is named (localized by the screens for known ids).
    pub name: String,
    /// `movie` or `series`.
    pub kind: String,
    /// The orders it can page by; empty means only the order it sends.
    pub sorts: Vec<CatalogSort>,
    pub filters: Vec<CatalogFilter>,
    /// Whether it takes a search.
    pub search: bool,
    /// Whether it honours `skip`.
    pub paging: bool,
}

/// One page of a catalog, and whether more follows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogPage {
    pub items: Vec<MovieItem>,
    /// The provider's own answer to "is there another page"; an empty page is
    /// the end even when it says otherwise.
    pub more: bool,
}

/// What a provider is given: the transport, and its own credentials.
pub struct Ctx<'a> {
    pub http: &'a dyn Transport,
    pub credentials: &'a dyn Credentials,
}

/// Where a provider's URL points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamOrigin {
    /// The provider's own service on loopback (the Stremio Service).
    LocalService,
    /// Anywhere else: https only.
    Remote,
}

impl StreamOrigin {
    /// The scheme and host a resolved URL may use for this origin.
    pub fn allows(self, url: &Url) -> bool {
        match self {
            Self::Remote => url.scheme() == "https",
            Self::LocalService => {
                matches!(url.scheme(), "http" | "https") && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1" | "[::1]"))
            }
        }
    }
}

/// A stream turned into the URL the player opens, and where that URL points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedStream {
    pub url: Url,
    pub origin: StreamOrigin,
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

    /// The URL the player opens for a picked stream, and where it points.
    fn resolve(&self, ctx: &Ctx<'_>, stream: &StreamChoice) -> Result<ResolvedStream>;

    /// The catalogs this provider offers the Movies and Series tabs, in the
    /// order their chips appear. A provider that browses nothing returns none,
    /// and the tabs keep what the rails carry.
    fn catalogs(&self, _ctx: &Ctx<'_>) -> Result<Vec<CatalogDescriptor>> {
        Ok(Vec::new())
    }

    /// One page of a catalog, for browsing beyond the rails. A provider that
    /// cannot browse returns an empty last page.
    fn catalog(&self, _ctx: &Ctx<'_>, _request: &CatalogRequest) -> Result<CatalogPage> {
        Ok(CatalogPage::default())
    }

    /// The provider's own record of where the title stopped. A provider
    /// without history returns `None`.
    fn read_progress(&self, _ctx: &Ctx<'_>, _id: &str) -> Result<Option<MovieProgress>> {
        Ok(None)
    }

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
