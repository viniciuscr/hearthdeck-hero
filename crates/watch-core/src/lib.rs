//! Watch's domain: the item and stream types the screens read, the provider
//! contract, and the transport and credential ports every provider is given.
//!
//! No file, process or `heronite-*` access lives here; `docs/WATCH-CRATES.md`
//! owns the boundary.

pub mod auth;
pub mod http;
pub mod types;

pub type Result<T> = anyhow::Result<T>;
