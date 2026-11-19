//! The person's own shelf: what they mean to watch, what they like, and the lists
//! they keep. **Local first**: a toggle is a write to a small file, instant and
//! offline, and the TMDB account slice will mirror these flags rather than own them.
//!
//! The app reads the shelf once into its state and draws from that value; it does
//! not re-read the file per frame (the same rule the TMDB key follows).

use std::sync::Mutex;

use anyhow::Result;
use heronite_data::paths::AppPaths;

pub use heronite_watch_core::personal::{same_title, PersonalLibrary, PersonalList, ShelfTitle};

/// The shelf as stored: empty when the file is missing or unreadable.
pub fn read(app: &AppPaths) -> PersonalLibrary {
    heronite_watch_state::personal::read(app)
}

pub fn write(app: &AppPaths, library: &PersonalLibrary) -> Result<()> {
    heronite_watch_state::personal::write(app, library)
}

/// Adds or removes the title, and says where it stands afterwards: `true` = in.
pub fn toggle_watchlist(app: &AppPaths, item: &ShelfTitle) -> Result<bool> {
    change(app, |library| library.toggle_watchlist(item))
}

/// The same for favourites — a separate flag, not a second watchlist.
pub fn toggle_favourite(app: &AppPaths, item: &ShelfTitle) -> Result<bool> {
    change(app, |library| library.toggle_favourite(item))
}

/// Reads, changes, writes: every write takes this path, so two toggles cannot
/// interleave a read-modify-write and lose one of them.
fn change(app: &AppPaths, edit: impl FnOnce(&mut PersonalLibrary) -> bool) -> Result<bool> {
    static WRITES: Mutex<()> = Mutex::new(());
    let _writing = WRITES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut library = heronite_watch_state::personal::read(app);
    let state = edit(&mut library);
    heronite_watch_state::personal::write(app, &library)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, id: &str) -> ShelfTitle {
        ShelfTitle { provider: "stremio".into(), kind: kind.into(), id: id.into(), name: id.into(), ..ShelfTitle::default() }
    }

    fn app(dir: &std::path::Path) -> AppPaths {
        AppPaths { config: dir.join("config"), cache: dir.join("cache") }
    }

    #[test]
    fn a_toggle_is_written_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let matrix = item("movie", "tt0133093");

        assert!(toggle_watchlist(&app, &matrix).unwrap(), "the first press puts it in");
        assert!(read(&app).watchlisted(&matrix), "and the file says so");
        assert!(!read(&app).favourited(&matrix), "a watchlist entry is not a favourite");

        assert!(!toggle_watchlist(&app, &matrix).unwrap(), "the second press takes it out");
        assert!(!read(&app).watchlisted(&matrix));
    }

    #[test]
    fn the_two_flags_are_independent_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let film = item("movie", "tt1");
        toggle_favourite(&app, &film).unwrap();
        assert!(read(&app).favourited(&film));
        assert!(!read(&app).watchlisted(&film));
    }

    #[test]
    fn a_toggle_keeps_what_a_newer_build_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        std::fs::create_dir_all(&app.config).unwrap();
        std::fs::write(heronite_watch_state::personal::path(&app), r#"{"fromTheFuture":7}"#).unwrap();

        toggle_watchlist(&app, &item("movie", "tt1")).unwrap();
        let stored: serde_json::Value = heronite_data::json_store::read(&heronite_watch_state::personal::path(&app)).unwrap();
        assert_eq!(stored["fromTheFuture"], serde_json::Value::from(7), "{stored}");
    }
}
