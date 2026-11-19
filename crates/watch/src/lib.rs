//! Watch: the space's provider registry, its Collection projection and its
//! playback orchestration. The app imports this crate only; the packages
//! behind it are described in `docs/WATCH-CRATES.md`.
//!
//! Blocking, like every library crate here: the app calls these from a
//! background thread. No GPUI.

pub mod catalog;
pub mod collections;
pub mod import;
pub mod library;
pub mod metadata;
pub mod personal;
pub mod playback;
pub mod progress;
pub mod registry;
pub mod service;
pub mod simkl_adapter;
pub mod simkl_sign_in;
pub mod stats;
pub mod tmdb_sign_in;
pub mod trackers;

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
    pub use heronite_watch_state::store::{
        clear_auth, clear_simkl_client_id, clear_tmdb_key, read_auth, read_simkl_client_id, read_tmdb_key, write_auth,
        write_simkl_client_id, write_tmdb_key,
    };
}

/// Settings → Watch: what the person chose beyond account, theme and rails.
pub mod settings {
    pub use heronite_watch_state::settings::{read, write, Settings};
}

/// The item types the screens show.
pub mod types {
    pub use heronite_watch_core::title::{Art, Title, IMPORTED_ADDON};
    pub use heronite_watch_core::types::StreamChoice;
}

/// Whether this system ships the Watch space: Linux, with at least one
/// provider built in.
pub fn available() -> bool {
    cfg!(target_os = "linux") && !registry::providers().is_empty()
}

pub type Result<T> = heronite_watch_core::Result<T>;
