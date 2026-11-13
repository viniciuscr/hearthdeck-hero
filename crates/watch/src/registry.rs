//! The providers the Watch space ships with, in the order their rails appear.

use heronite_watch_core::provider::WatchProvider;

#[cfg(feature = "stremio")]
static STREMIO: heronite_watch_stremio::Stremio = heronite_watch_stremio::Stremio;

#[cfg(feature = "stremio")]
static PROVIDERS: [&dyn WatchProvider; 1] = [&STREMIO];

#[cfg(not(feature = "stremio"))]
static PROVIDERS: [&dyn WatchProvider; 0] = [];

/// Every provider, in display order. A build without `stremio` ships none.
pub fn providers() -> &'static [&'static dyn WatchProvider] {
    &PROVIDERS
}

/// The provider a game belongs to, by the `runner` value it carries.
pub fn find(id: &str) -> Option<&'static dyn WatchProvider> {
    providers().iter().copied().find(|provider| provider.id() == id)
}
