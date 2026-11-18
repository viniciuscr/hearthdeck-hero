//! One record per playback — what the Watch recap is built from.
//!
//! The progress record (`movies.json`) keeps only where a title stands now: a
//! rewatch overwrites it, and it says nothing about when anything was watched.
//! The games' Statistics page is built on `local_library/sessions.json` for the
//! same reason, so this is Watch's: an append-only list of what was played, when
//! it started and ended, and how far it got.
//!
//! Nothing here is read to play something — a title plays from its progress
//! record — so this file may be deleted without losing a position.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use heronite_data::json_store;
use heronite_data::paths::AppPaths;
use serde::{Deserialize, Serialize};

/// The file, under the app's config folder, like `movies.json`.
pub const FILE: &str = "watch-sessions.json";

/// One title's sessions kept before its oldest are dropped. A film watched every
/// day for five years is 1800; a series' episodes share the title's id, so the
/// cap is generous on purpose.
pub const MAX_PER_TITLE: usize = 2000;

/// Shorter than this is a mis-click or an mpv that never started, not watching.
pub const MIN_SESSION_SECS: f64 = 45.;

/// What was played, and how far it got. Times are UTC, as stored everywhere else.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchSession {
    /// The provider that owns the title (`stremio`).
    pub provider: String,
    /// `movie` or `series`.
    pub kind: String,
    /// The provider's id for the title (`tt0133093`).
    pub id: String,
    /// The episode, for a series; `None` for a film.
    #[serde(default)]
    pub video_id: Option<String>,
    /// The episode's season and number, when the page knew them: what a tracker
    /// sends instead of guessing from `video_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    /// The name as it was when it played: a title renamed at its source keeps
    /// the old one in the recap, which is the honest record.
    #[serde(default)]
    pub title: String,
    /// The title the episode belongs to — the film's or the show's name — and the
    /// year: what a tracker records, where `title` may name the episode. Empty on
    /// a record written before they were kept.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Time this session covered, which is not the position: resuming at 40 min
    /// and stopping at 55 is fifteen minutes of watching.
    pub elapsed_seconds: f64,
    #[serde(default)]
    pub position_from_ms: u64,
    #[serde(default)]
    pub position_to_ms: u64,
    /// The title's length, when the player reported one.
    #[serde(default)]
    pub duration_ms: u64,
    /// Reached the finished threshold (Stremio's 70 %, see `finished`).
    #[serde(default)]
    pub finished: bool,
}

impl WatchSession {
    /// Watch this session covered, in hours.
    pub fn hours(&self) -> f64 {
        self.elapsed_seconds / 3600.
    }
}

/// The file's shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsFile {
    #[serde(default)]
    pub sessions: Vec<WatchSession>,
}

pub fn path(app: &AppPaths) -> PathBuf {
    app.config.join(FILE)
}

/// Every session recorded, oldest first. A file that does not parse reads as
/// empty: a recap is never worth failing a start for.
pub fn read(app: &AppPaths) -> SessionsFile {
    json_store::read_value(&path(app)).ok().and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default()
}

/// Appends one session, dropping a title's oldest past [`MAX_PER_TITLE`].
pub fn append(app: &AppPaths, session: WatchSession) -> Result<()> {
    let mut file = read(app);
    file.sessions.push(session);
    cap(&mut file);
    write(&path(app), &file)
}

/// Keeps each title's newest sessions within the cap.
fn cap(file: &mut SessionsFile) {
    let mut seen: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    // Oldest first, so the count per title runs forward and the newest survive.
    let mut kept = Vec::with_capacity(file.sessions.len());
    for session in &file.sessions {
        *seen.entry((session.provider.clone(), session.id.clone())).or_default() += 1;
    }
    let mut dropped: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    for session in &file.sessions {
        let key = (session.provider.clone(), session.id.clone());
        let over = seen.get(&key).copied().unwrap_or(0).saturating_sub(MAX_PER_TITLE);
        let already = dropped.entry(key).or_default();
        if *already < over {
            *already += 1;
            continue;
        }
        kept.push(session.clone());
    }
    file.sessions = kept;
}

fn write(path: &Path, file: &SessionsFile) -> Result<()> {
    json_store::write(path, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(root: &Path) -> AppPaths {
        AppPaths { config: root.join("config"), cache: root.join("cache") }
    }

    fn session(id: &str, started: i64, elapsed: f64) -> WatchSession {
        let at = DateTime::from_timestamp(started, 0).unwrap_or_default();
        WatchSession {
            provider: "stremio".into(),
            kind: "movie".into(),
            id: id.into(),
            video_id: None,
            season: None,
            number: None,
            name: format!("Film {id}"),
            year: None,
            title: format!("Film {id}"),
            started_at: at,
            ended_at: at,
            elapsed_seconds: elapsed,
            position_from_ms: 0,
            position_to_ms: (elapsed * 1000.) as u64,
            duration_ms: 0,
            finished: false,
        }
    }

    #[test]
    fn sessions_append_and_read_back_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        append(&app, session("tt1", 1_700_000_000, 3_600.)).unwrap();
        append(&app, session("tt2", 1_700_003_600, 1_800.)).unwrap();
        let read_back = read(&app);
        assert_eq!(read_back.sessions.len(), 2);
        assert_eq!(read_back.sessions[0].id, "tt1");
        assert!((read_back.sessions[0].hours() - 1.).abs() < 1e-9);
        assert_eq!(read_back.sessions[1].id, "tt2", "the order is the order they happened");
    }

    #[test]
    fn a_missing_or_broken_file_reads_as_no_history() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        assert!(read(&app).sessions.is_empty(), "no file yet");
        std::fs::create_dir_all(&app.config).unwrap();
        std::fs::write(path(&app), b"{ not json").unwrap();
        assert!(read(&app).sessions.is_empty(), "a broken file is not a start-up fault");
    }

    #[test]
    fn a_titles_oldest_sessions_go_past_the_cap() {
        let mut file = SessionsFile::default();
        for i in 0..(MAX_PER_TITLE + 5) {
            file.sessions.push(session("tt1", 1_700_000_000 + i as i64, 600.));
        }
        file.sessions.push(session("tt2", 1_700_000_000, 60.));
        cap(&mut file);
        assert_eq!(file.sessions.iter().filter(|session| session.id == "tt1").count(), MAX_PER_TITLE);
        assert_eq!(file.sessions.iter().filter(|session| session.id == "tt2").count(), 1, "another title keeps its own");
        let kept: Vec<i64> = file.sessions.iter().filter(|s| s.id == "tt1").map(|s| s.started_at.timestamp()).collect();
        assert!(!kept.contains(&1_700_000_000), "the oldest left first");
        assert!(kept.contains(&(1_700_000_000 + MAX_PER_TITLE as i64 + 4)), "the newest stayed");
    }

    #[test]
    fn a_rewatch_is_its_own_session() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        append(&app, session("tt1", 1_700_000_000, 7_200.)).unwrap();
        append(&app, session("tt1", 1_710_000_000, 7_100.)).unwrap();
        assert_eq!(read(&app).sessions.len(), 2, "the second viewing does not overwrite the first");
    }
}
