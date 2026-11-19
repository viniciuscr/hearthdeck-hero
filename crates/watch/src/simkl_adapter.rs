//! Simkl as a tracker adapter: what the facade needs to know about it, and
//! nothing more.
//!
//! Everything Simkl-shaped lives behind this: the sign-in store, the token
//! refresh, the deliverer. A second tracker is another module like this one and a
//! line in `trackers::adapters()` — no branch anywhere else.

use anyhow::{anyhow, Result};
use heronite_data::paths::AppPaths;
use heronite_watch_core::http::UreqTransport;
use heronite_watch_core::tracker::WatchTracker;
use heronite_watch_simkl::{SimklTracker, PROVIDER};

use crate::simkl_sign_in;
use crate::trackers::TrackerAdapter;

pub(crate) struct Simkl;

impl TrackerAdapter for Simkl {
    fn id(&self) -> &'static str {
        PROVIDER
    }

    fn name(&self) -> &'static str {
        "Simkl"
    }

    /// This build can talk to Simkl (the app's client id) and a person signed in.
    fn ready(&self, app: &AppPaths) -> bool {
        simkl_sign_in::configured(app) && simkl_sign_in::session(app).is_some()
    }

    /// One deliverer for a drain, with a session refreshed if it is near expiry.
    fn build<'a>(&self, app: &AppPaths, http: &'a UreqTransport) -> Result<Box<dyn WatchTracker + 'a>> {
        let client_id = simkl_sign_in::client_id(app)?;
        let tokens = simkl_sign_in::tokens_fresh_with(http, app)?.ok_or_else(|| anyhow!("no session on this machine"))?;
        Ok(Box::new(SimklTracker::new(http, &client_id, &tokens.access_token)))
    }
}

/// The one adapter this build ships.
pub(crate) static SIMKL: Simkl = Simkl;
