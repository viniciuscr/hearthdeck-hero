//! The Stremio provider: its hosts, the link-code account, the `libraryItem`
//! datastore, Cinemeta's catalogs and the local Stremio Service.
//!
//! Everything here is reached through the ports in `Ctx`; the crate opens no
//! files and spawns no processes.

pub mod account;
pub mod catalog;
pub mod datastore;
pub mod endpoints;
pub mod streaming;
