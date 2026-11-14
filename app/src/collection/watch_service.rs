//! The Stremio service in the Collection: started when a torrent play needs it,
//! stopped again when the session ends if it is ours.
//!
//! Fork-owned, like `watch_playback.rs`. The app owns the process — it may use
//! `heronite_data::proc` and `heronite_platform::process` — while the protocol
//! crates never spawn anything. A service that was already running belongs to
//! the person (their desktop session usually starts it), so it is never
//! stopped here; only one this app started is, and only when
//! `heronite_watch::settings` says so.

use std::io::BufRead;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context as _, Result};
use gpui_kit::prelude::*;
use heronite_i18n::tr;

use super::CollectionView;

/// The service's state, and the play waiting for it.
#[derive(Default)]
pub(super) struct WatchService {
    /// The process this app started, while it runs. `None` when the service is
    /// down, or was running before this app asked (it is not ours then).
    child: Option<Child>,
    /// The service answered: ours, or the person's.
    up: bool,
    /// A start is in flight (the play that asked waits for it).
    pub(super) starting: bool,
    /// What to play once the service answers: the Collection key and the index
    /// of the stream in the picked list.
    pending: Option<(String, usize)>,
}

impl CollectionView {
    /// Whether the service answered already (no need to look again).
    pub(super) fn watch_service_ready(&self) -> bool {
        self.watch_service.up
    }

    /// Starts the service if it is down, then plays `index` of `key`'s current
    /// streams. Runs off the UI thread; the caller is the play that needed it.
    pub(super) fn watch_service_start(&mut self, key: String, index: usize, cx: &mut Context<Self>) {
        self.watch_service.starting = true;
        self.watch_service.pending = Some((key.clone(), index));
        cx.notify();
        let endpoints = heronite_watch::Endpoints::default();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { start(&endpoints) }).await;
            this.update(cx, |this, cx| {
                this.watch_service.starting = false;
                this.watch_service.pending = None;
                match result {
                    Ok(child) => {
                        // `None`: it was already running, so it is not ours to stop.
                        this.watch_service.child = child;
                        this.watch_service.up = true;
                        if let Some(game) = this.lib.as_ref().and_then(|lib| lib.get(key.as_str())).cloned() {
                            this.watch_play_stream(&game, index, cx);
                        }
                    }
                    Err(error) => {
                        log::warn!("watch service: {error:#}");
                        this.show_toast(tr!("Could not start the Stremio service: {error}", error = format!("{error:#}")), true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The session ended: stop the service this app started, unless the person
    /// asked to keep it warm.
    pub(super) fn watch_service_finished(&mut self, cx: &mut Context<Self>) {
        let Some(mut child) = self.watch_service.child.take() else { return };
        self.watch_service.up = false;
        if !heronite_watch::settings::read(&self.app_paths).stop_service_when_done {
            // Dropping the handle leaves it running; the next play finds it.
            log::debug!("watch service: left running (Settings → Watch)");
            return;
        }
        // Signalling and waiting a grace period must not hold the UI thread.
        std::thread::Builder::new().name("watch-service-stop".into()).spawn(move || stop(&mut child)).ok();
        cx.notify();
    }
}

/// Makes the local service answer: `Ok(None)` when it already did, `Ok(Some)`
/// with the process this app started otherwise.
pub(super) fn start(endpoints: &heronite_watch::Endpoints) -> Result<Option<Child>> {
    endpoints.validate()?;
    let base = &endpoints.streaming_server;
    if heronite_watch::service::is_up(base) {
        return Ok(None);
    }
    let Some(launcher) = heronite_watch::service::launcher() else {
        bail!(
            "Stremio Service is not installed. Install the stremio-service package, or set {} to its launcher.",
            heronite_watch::service::LAUNCHER_ENV
        );
    };
    let mut command = heronite_watch::service::command(&launcher);
    heronite_platform::process::new_group(&mut command);
    let mut child = heronite_data::proc::spawn(&mut command).with_context(|| format!("starting {}", launcher.display()))?;
    log::info!("watch service: started {}", launcher.display());
    let errors = Tail::follow(&mut child);

    let deadline = Instant::now() + heronite_watch::service::START_TIMEOUT;
    loop {
        if heronite_watch::service::is_up(base) {
            return Ok(Some(child));
        }
        if let Some(status) = child.try_wait().context("asking the service")? {
            bail!("the service stopped at once ({status}){}", errors.last());
        }
        if Instant::now() >= deadline {
            stop(&mut child);
            bail!("the service did not answer within {} s{}", heronite_watch::service::START_TIMEOUT.as_secs(), errors.last());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Ends the service this app started: its whole process group (the launcher
/// starts the runtime as a child of its own), asked first and killed after a
/// grace period.
pub(super) fn stop(child: &mut Child) {
    use heronite_platform::process::{group_alive, signal_group, Signal};
    let pid = child.id();
    if pid <= 1 {
        return;
    }
    signal_group(pid, Signal::Term);
    for _ in 0..30 {
        if !group_alive(pid) {
            let _ = child.wait();
            log::debug!("watch service: stopped");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    signal_group(pid, Signal::Kill);
    let _ = child.wait();
    log::debug!("watch service: killed after the grace period");
}

/// The service's last error lines, kept for a failure report (a broken service
/// that exits at once should say why).
struct Tail(Arc<Mutex<String>>);

impl Tail {
    fn follow(child: &mut Child) -> Self {
        let tail = Arc::new(Mutex::new(String::new()));
        let Some(stderr) = child.stderr.take() else { return Self(tail) };
        let sink = tail.clone();
        std::thread::Builder::new()
            .name("watch-service-stderr".into())
            .spawn(move || {
                let mut reader = std::io::BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let text = line.trim_end();
                            if text.is_empty() {
                                continue;
                            }
                            log::debug!("watch service: {text}");
                            let mut kept = sink.lock().unwrap_or_else(|poison| poison.into_inner());
                            kept.push_str(text);
                            kept.push('\n');
                            if kept.len() > 4096 {
                                let mut cut = kept.len() - 4096;
                                while !kept.is_char_boundary(cut) {
                                    cut += 1;
                                }
                                kept.drain(..cut);
                            }
                        }
                    }
                }
            })
            .ok();
        Self(tail)
    }

    /// ` — the last line`, or nothing when the service said nothing.
    fn last(&self) -> String {
        let kept = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        match kept.lines().rev().find(|line| !line.trim().is_empty()) {
            Some(line) => format!(" — {}", line.trim()),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    #[test]
    fn a_silent_service_says_nothing_in_the_report() {
        let tail = Tail(Arc::new(Mutex::new(String::new())));
        assert_eq!(tail.last(), "");
    }

    #[test]
    fn only_the_last_error_line_is_reported() {
        let tail = Tail(Arc::new(Mutex::new("first line\nlast line\n".into())));
        assert_eq!(tail.last(), " — last line");
    }

    #[cfg(unix)]
    #[test]
    fn stopping_takes_the_whole_process_group_down() {
        use heronite_platform::process::{group_alive, new_group};
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        new_group(&mut command);
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        assert!(group_alive(pid), "the child leads its own group");
        stop(&mut child);
        assert!(!group_alive(pid), "the group is gone");
    }
}
