//! Watch: the space's provider registry, its Collection projection and its
//! playback orchestration. The app imports this crate only; the packages
//! behind it are described in `docs/WATCH-CRATES.md`.
//!
//! Blocking, like every library crate here: the app calls these from a
//! background thread. No GPUI.

pub mod collection;
pub mod metadata;
pub mod playback;
pub mod progress;
pub mod registry;

pub use heronite_watch_core::auth::AuthKey;
pub use heronite_watch_core::http::{Transport, UreqTransport};
#[cfg(feature = "stremio")]
pub use heronite_watch_stremio::endpoints::Endpoints;

/// The account link flow of the Stremio provider.
#[cfg(feature = "stremio")]
pub mod account {
    pub use heronite_watch_stremio::account::{create_link, read_link, LinkOffer, LinkPoll};
}

/// What a finished session reports to the app.
pub mod player {
    pub use heronite_watch_player::player::PlaybackOutcome;
}

/// The local watch state the settings screen reads and writes.
pub mod store {
    pub use heronite_watch_state::store::{clear_auth, read_auth, write_auth};
}

/// The item types the screens show.
pub mod types {
    pub use heronite_watch_core::types::StreamChoice;
}

/// Whether this system ships the Watch space: Linux, with at least one
/// provider built in.
pub fn available() -> bool {
    cfg!(target_os = "linux") && !registry::providers().is_empty()
}

pub type Result<T> = heronite_watch_core::Result<T>;
