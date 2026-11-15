//! Watch's domain: the item and stream types the screens read, the provider
//! contract, and the transport and credential ports every provider is given.
//!
//! No file, process or `heronite-*` access lives here; `docs/WATCH-CRATES.md`
//! owns the boundary.

pub mod auth;
pub mod catalog;
pub mod http;
pub mod metadata;
pub mod provider;
pub mod types;

pub use catalog::{
    CatalogCache, CatalogCtx, CatalogDescriptor, CatalogFilter, CatalogPage, CatalogRequest, CatalogSort, CatalogSourceDescriptor,
    WatchCatalogSource,
};
pub use metadata::{Episode, Fact, MetaCtx, MetaDescriptor, TitleDetails, TitleRef, WatchMetadataSource};
pub use provider::{Credentials, Ctx, Rail, RailDescriptor, ResolvedStream, Secret, StreamOrigin, WatchProvider};

pub type Result<T> = anyhow::Result<T>;
