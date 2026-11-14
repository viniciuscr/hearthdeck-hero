//! Cached title details: what a metadata source said about a title, kept so a
//! page opens without asking again.
//!
//! Regenerable and disposable: the files live under the app's cache directory,
//! a stale answer is still served when no source can be reached, and a missing
//! or corrupt file is simply no answer. Nothing here goes to the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use heronite_data::json_store;
use heronite_watch_core::metadata::{TitleDetails, TitleRef};

/// Where the cache lives under the app's cache directory.
pub const ROOT: &str = "watch-details";

/// How long an answer counts as fresh. Older answers are still served when the
/// sources cannot be asked, so a flight or an outage never empties the page.
pub const FRESH_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// One cached answer: the details and when they were fetched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedDetails {
    pub fetched_at: DateTime<Utc>,
    pub details: TitleDetails,
}

impl CachedDetails {
    /// Whether the answer is young enough to use without asking again.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.fetched_at).num_seconds() < FRESH_FOR.as_secs() as i64
    }
}

/// Where the answer for `title` is kept: one folder per media provider, so two
/// providers that share an id never share a record.
pub fn path(root: &Path, title: &TitleRef) -> PathBuf {
    root.join(component(&title.provider)).join(component(&title.kind)).join(file_name(&title.id))
}

/// The cached answer, `None` when there is none or it cannot be read.
pub fn read(root: &Path, title: &TitleRef) -> Option<CachedDetails> {
    let file = path(root, title);
    let bytes = std::fs::read(&file).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(cached) => Some(cached),
        Err(error) => {
            log::debug!("watch details cache {}: {error}", file.display());
            None
        }
    }
}

/// Keeps `details` as the answer for `title`.
pub fn write(root: &Path, title: &TitleRef, details: &TitleDetails, fetched_at: DateTime<Utc>) -> Result<()> {
    let file = path(root, title);
    json_store::write(&file, &CachedDetails { fetched_at, details: details.clone() }).with_context(|| format!("writing {}", file.display()))
}

/// A path component that cannot leave the cache folder: an id comes from a
/// media provider, so only letters, digits, `-` and `_` survive.
fn component(raw: &str) -> String {
    let cleaned: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    if cleaned.is_empty() {
        "_".to_owned()
    } else {
        cleaned
    }
}

/// The file name of a title's answer: its safe id, clamped to a file name's
/// length, plus a hash of the raw id — two ids that sanitise alike (a slash and
/// an underscore, say) must not share a cache file.
fn file_name(id: &str) -> String {
    let safe: String = component(id).chars().take(80).collect();
    format!("{safe}-{:016x}.json", fnv1a(id))
}

/// FNV-1a: a stable file-name tail for an id, without another dependency.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(id: &str) -> TitleRef {
        TitleRef::new("stremio", id, "movie")
    }

    fn details(id: &str) -> TitleDetails {
        TitleDetails { id: id.into(), kind: "movie".into(), name: "The Matrix".into(), ..TitleDetails::default() }
    }

    #[test]
    fn an_answer_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        write(dir.path(), &title("tt0133093"), &details("tt0133093"), now).unwrap();
        let cached = read(dir.path(), &title("tt0133093")).expect("the answer was written");
        assert_eq!(cached.fetched_at, now);
        assert_eq!(cached.details.name, "The Matrix");
        assert!(cached.is_fresh(now));
    }

    #[test]
    fn a_seven_day_old_answer_is_not_fresh_but_is_still_readable() {
        let dir = tempfile::tempdir().unwrap();
        let fetched = Utc::now() - chrono::Duration::days(8);
        write(dir.path(), &title("tt1"), &details("tt1"), fetched).unwrap();
        let cached = read(dir.path(), &title("tt1")).expect("the answer is still there");
        assert!(!cached.is_fresh(Utc::now()));
        assert!(read(dir.path(), &title("tt2")).is_none(), "no answer is not an answer");
    }

    #[test]
    fn an_id_cannot_leave_the_cache_and_names_its_provider() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(ROOT);
        let hostile = TitleRef::new("stremio", "../../etc/passwd", "movie");
        let file = path(&root, &hostile);
        assert!(file.starts_with(&root), "{}", file.display());
        let other = TitleRef::new("youtube", "tt1", "movie");
        assert_ne!(path(&root, &title("tt1")), path(&root, &other), "the provider is part of the key");
        let slash = TitleRef::new("stremio", "a/b", "movie");
        let underscore = TitleRef::new("stremio", "a_b", "movie");
        assert_ne!(path(&root, &slash), path(&root, &underscore), "ids that sanitise alike keep their own answer");
    }

    #[test]
    fn a_corrupt_file_is_no_answer() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path(), &title("tt1"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"not json").unwrap();
        assert!(read(dir.path(), &title("tt1")).is_none());
    }
}
