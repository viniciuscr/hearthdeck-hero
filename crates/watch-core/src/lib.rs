//! Watch's domain: the item and stream types the screens read, the provider
//! contract, and the transport and credential ports every provider is given.
//!
//! No file, process or `heronite-*` access lives here; `docs/WATCH-CRATES.md`
//! owns the boundary.

pub mod auth;
pub mod http;
pub mod provider;
pub mod types;

pub use provider::{CatalogDescriptor, Credentials, Ctx, Rail, Secret, WatchProvider};

pub type Result<T> = anyhow::Result<T>;
