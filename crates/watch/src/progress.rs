//! When progress is written: at most once per interval, plus forced writes on
//! pause and stop. Local progress is stored first, so a failed remote write
//! never loses the position.

use std::time::{Duration, Instant};

use anyhow::Result;

use heronite_data::paths::AppPaths;

use crate::datastore::{write_progress, DatastoreClient};
use crate::store::{upsert_progress, write_movies, MoviesFile};
use crate::types::MovieProgress;

/// The default: at most one remote write every 30 seconds.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self { interval, last: None }
    }

    /// `force` is a pause or stop; otherwise the interval must have elapsed.
    pub fn should_write(&self, now: Instant, force: bool) -> bool {
        force || self.last.is_none_or(|last| now.duration_since(last) >= self.interval)
    }

    pub fn wrote(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new(DEFAULT_INTERVAL)
    }
}

/// Stores progress locally, then tries Stremio. A failed remote write leaves
/// the local record in place for the next attempt; it never deletes anything.
pub fn record(client: &DatastoreClient<'_>, app: &AppPaths, movies: &mut MoviesFile, progress: MovieProgress) -> Result<()> {
    upsert_progress(movies, progress.clone());
    write_movies(app, movies)?;
    write_progress(client, &progress)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use anyhow::{anyhow, Result};
    use serde_json::Value;
    use url::Url;

    use crate::auth::AuthKey;
    use crate::endpoints::Endpoints;
    use crate::http::Transport;

    struct Failing {
        gets: Mutex<usize>,
    }

    impl Transport for Failing {
        fn get(&self, _: &Url) -> Result<Vec<u8>> {
            *self.gets.lock().unwrap() += 1;
            Ok(br#"{"result":[]}"#.to_vec())
        }
        fn post_json(&self, _: &Url, _: &Value) -> Result<Vec<u8>> {
            Err(anyhow!("offline"))
        }
    }

    #[test]
    fn the_throttle_allows_the_first_write_and_then_waits() {
        let mut throttle = Throttle::new(Duration::from_secs(30));
        let start = Instant::now();
        assert!(throttle.should_write(start, false));
        throttle.wrote(start);
        assert!(!throttle.should_write(start + Duration::from_secs(29), false));
        assert!(throttle.should_write(start + Duration::from_secs(30), false));
        assert!(throttle.should_write(start + Duration::from_secs(1), true));
    }

    #[test]
    fn a_failed_remote_write_keeps_local_progress() {
        let dir = tempfile::tempdir().unwrap();
        let app = AppPaths { config: dir.path().join("config"), cache: dir.path().join("cache") };
        let fake = Failing { gets: Mutex::new(0) };
        let endpoints = Endpoints::default();
        let client = DatastoreClient::new(&fake, &endpoints, AuthKey::new("key"));
        let mut movies = MoviesFile::default();
        let progress = MovieProgress {
            id: "tt1".into(),
            position_ms: 1234,
            duration_ms: 5678,
            updated_at: chrono::Utc::now(),
            finished: false,
            video_id: None,
        };
        assert!(record(&client, &app, &mut movies, progress).is_err());
        let stored = crate::store::read_movies(&app);
        assert_eq!(stored.progress.len(), 1);
        assert_eq!(stored.progress[0].position_ms, 1234);
    }
}
