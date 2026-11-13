//! The Stremio addon protocol, shared by every provider that is an addon
//! (Stremio, YouTube, OpenTV). Transport URLs can carry the person's own
//! configuration, so they are never quoted in an error or a log line.

pub mod addon;
