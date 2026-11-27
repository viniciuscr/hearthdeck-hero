//! What the stream addons answered when a title was last asked.
//!
//! A catalogue says a title exists; only a stream resource says someone can play
//! it. The page asks for a film's streams when it opens (and Play asks again), so
//! that answer is recorded here: a film a probe found empty is `Unavailable`
//! until the mark ages out or a later probe finds a stream. Deleting the file
//! only means the next page open asks again — it is a cache, not a fact.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use heronite_data::json_store;
use heronite_data::paths::AppPaths;

use crate::collection::ROOT;

/// The file, inside the projection root (regenerable, delete-safe).
pub const FILE: &str = "streams.json";

/// How long a probe is trusted. Sources and addons change, and a title that had
/// no stream can gain one; a week keeps a stale verdict from sticking.
pub const TTL_DAYS: i64 = 7;

/// What the last probe of a title found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// A stream was found.
    Available,
    /// Asked, and no addon offered one.
    Unavailable,
    /// Never asked, or the answer is older than [`TTL_DAYS`].
    Unknown,
}

/// One answer, and when it came.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    /// How many streams the addons offered; `0` is the answer that dims a card.
    pub streams: u32,
    pub at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbesFile {
    #[serde(default)]
    probes: HashMap<String, Probe>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// The key a probe is kept under. The kind is part of it, so a film and a series
/// sharing an id never share an answer.
pub fn key(provider: &str, kind: &str, id: &str) -> String {
    format!("{provider}:{kind}:{id}")
}

fn path(app: &AppPaths) -> PathBuf {
    app.cache.join(ROOT).join(FILE)
}

/// Every probe as stored; a missing file is none.
///
/// A file that is there but unreadable is a fault, not an empty cache: it is said
/// in the log, because every title then reads as `Unknown` and no card dims
/// (`docs/WATCH-SYNC-RULES.md` on why the probe has to be visible).
pub fn read(root: &Path) -> HashMap<String, Probe> {
    let path = root.join(FILE);
    match json_store::read::<ProbesFile>(&path) {
        Ok(file) => file.probes,
        Err(error) => {
            log::warn!("watch streams: {} could not be read ({error:#}); no title dims from it", path.display());
            HashMap::new()
        }
    }
}

/// What the probes say about a title now.
pub fn availability(probes: &HashMap<String, Probe>, provider: &str, kind: &str, id: &str) -> Availability {
    let Some(probe) = probes.get(&key(provider, kind, id)) else { return Availability::Unknown };
    if probe.at < Utc::now() - Duration::days(TTL_DAYS) {
        return Availability::Unknown;
    }
    if probe.streams == 0 {
        Availability::Unavailable
    } else {
        Availability::Available
    }
}

/// Records one probe: the newest answer wins, whichever way it went.
pub fn note(app: &AppPaths, provider: &str, kind: &str, id: &str, streams: usize) -> Result<()> {
    // The count is only ever read as "none" or "some"; saturating keeps a
    // pathological answer from being a panic in a cache write.
    let streams = u32::try_from(streams).unwrap_or(u32::MAX);
    json_store::update::<ProbesFile, _>(&path(app), |file| {
        file.probes.insert(key(provider, kind, id), Probe { streams, at: Utc::now() });
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn app(dir: &Path) -> AppPaths {
        AppPaths::for_data(&heronite_data::DataPaths::at(dir.to_path_buf()))
    }

    #[test]
    fn a_corrupt_probe_file_reads_as_none_and_says_so() {
        // A file that is there but broken is a fault, not an empty cache: it must
        // not panic, and every title then reads `Unknown` (nothing dims) — the
        // warning is the only trace, so the read cannot fail silently.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(ROOT);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(FILE), b"{ not json").unwrap();

        assert!(read(&root).is_empty(), "a broken file is no probes");
        assert_eq!(availability(&read(&root), "stremio", "movie", "tt1"), Availability::Unknown);
    }

    #[test]
    fn a_probe_is_kept_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        assert!(read(&app.cache.join(ROOT)).is_empty(), "nothing asked yet");

        note(&app, "stremio", "movie", "tt1", 0).unwrap();
        note(&app, "stremio", "movie", "tt2", 7).unwrap();
        let probes = read(&app.cache.join(ROOT));
        assert_eq!(availability(&probes, "stremio", "movie", "tt1"), Availability::Unavailable);
        assert_eq!(availability(&probes, "stremio", "movie", "tt2"), Availability::Available);
        assert_eq!(availability(&probes, "stremio", "movie", "tt3"), Availability::Unknown, "never asked");
        assert_eq!(availability(&probes, "stremio", "series", "tt1"), Availability::Unknown, "the kind is part of the key");

        // The newest answer is the one that counts.
        note(&app, "stremio", "movie", "tt1", 3).unwrap();
        let probes = read(&app.cache.join(ROOT));
        assert_eq!(availability(&probes, "stremio", "movie", "tt1"), Availability::Available);
    }

    #[test]
    fn an_old_answer_is_unknown_again() {
        let old = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut probes = HashMap::new();
        probes.insert(key("stremio", "movie", "tt1"), Probe { streams: 0, at: old });
        probes.insert(key("stremio", "movie", "tt2"), Probe { streams: 0, at: Utc::now() });
        assert_eq!(availability(&probes, "stremio", "movie", "tt1"), Availability::Unknown, "older than the TTL");
        assert_eq!(availability(&probes, "stremio", "movie", "tt2"), Availability::Unavailable, "today's answer");
    }
}
