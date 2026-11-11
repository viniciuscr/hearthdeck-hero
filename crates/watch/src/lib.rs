//! Watch: Stremio account, addon protocol, local streaming server and playback.
//!
//! Blocking, like every library crate here: the app calls these from a
//! background thread. No GPUI. The contracts and rules are in `docs/WATCH.md`.

pub mod account;
pub mod addon;
pub mod auth;
pub mod collection;
pub mod datastore;
pub mod deep_link;
pub mod endpoints;
pub mod http;
pub mod player;
pub mod progress;
pub mod store;
pub mod streaming;
pub mod types;

pub use auth::AuthKey;
pub use endpoints::Endpoints;
pub use http::{Transport, UreqTransport};

/// Whether this system ships the Watch space. Linux is the only target in v1.
pub fn available() -> bool {
    cfg!(target_os = "linux")
}

pub type Result<T> = anyhow::Result<T>;
