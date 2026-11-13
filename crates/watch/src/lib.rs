//! Watch: the space's provider registry, its Collection projection and its
//! playback orchestration. The app imports this crate only; the packages
//! behind it are described in `docs/WATCH-CRATES.md`.
//!
//! Blocking, like every library crate here: the app calls these from a
//! background thread. No GPUI.

pub mod collection;
pub mod playback;

pub use heronite_watch_addon::addon;
pub use heronite_watch_core::auth::AuthKey;
pub use heronite_watch_core::http::{Transport, UreqTransport};
pub use heronite_watch_core::types;
pub use heronite_watch_player::player;
pub use heronite_watch_state::store;
pub use heronite_watch_stremio::endpoints::Endpoints;
pub use heronite_watch_stremio::{account, datastore, endpoints, streaming};

/// Whether this system ships the Watch space. Linux is the only target in v1.
pub fn available() -> bool {
    cfg!(target_os = "linux")
}

pub type Result<T> = heronite_watch_core::Result<T>;
