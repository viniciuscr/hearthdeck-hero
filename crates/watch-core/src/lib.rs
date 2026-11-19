//! Watch's domain: the item and stream types the screens read, the provider
//! contract, and the transport and credential ports every provider is given.
//!
//! No file, process or `heronite-*` access lives here; `docs/WATCH-CRATES.md`
//! owns the boundary.

pub mod auth;
pub mod catalog;
pub mod collections;
pub mod http;
pub mod metadata;
pub mod personal;
pub mod progress;
pub mod provider;
pub mod sync_rules;
pub mod tracker;
pub mod types;

pub use catalog::{
    CatalogCache, CatalogCtx, CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor,
    WatchCatalogSource,
};
pub use collections::{CollectionDescriptor, CollectionSourceDescriptor, WatchCollectionSource};
pub use metadata::{Episode, Fact, MetaCtx, MetaDescriptor, TitleDetails, TitleRef, Trailer, WatchMetadataSource};
pub use personal::{same_title, PersonalLibrary, PersonalList};
pub use progress::{finished, SeriesStanding, WatchedState};
pub use provider::{Credentials, Ctx, Rail, RailDescriptor, ResolvedStream, Secret, StreamOrigin, WatchProvider};
pub use tracker::{Delivery, TrackEvent, TrackedTitle, TrackedView, TrackedWatch, WatchTracker};

pub type Result<T> = anyhow::Result<T>;
