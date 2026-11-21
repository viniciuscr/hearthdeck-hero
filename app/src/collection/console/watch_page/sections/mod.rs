//! The film page's sections, one file per part of the page.
//!
//! Fork-owned (`docs/WATCH-PAGE.md`, Phase 2). Each section takes the data it
//! draws and returns elements or rows; none of them owns view state, so the page
//! (`../mod.rs`) can compose them and the engine (`../engine.rs`) routes the pad
//! over them without either knowing the other's internals.

pub(super) mod details;
pub(super) mod episode;
pub(super) mod marks;
