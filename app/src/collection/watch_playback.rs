//! Watch playback in the Collection: the streams of the focused title, the
//! one Play starts and the mpv session behind it.
//!
//! Fork-owned like `console/watch_panel.rs`. The protocol and the player live
//! in `crates/watch`; this module only turns them into view state, rows and
//! sections, and keeps the blocking work off the UI thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_i18n::{tr, trn};
use heronite_watch::types::StreamChoice;

use super::CollectionView;
use crate::console::list::Row;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

/// What the detail page knows about the focused title's sources.
#[derive(Default)]
pub(super) struct WatchPlayback {
    /// The title `state` describes (the Collection key).
    item: Option<String>,
    state: StreamsState,
    /// The stream the main Play starts; the first when the list arrives.
    selected: usize,
    /// mpv is playing this title.
    running: Option<Running>,
}

#[derive(Clone, Default)]
enum StreamsState {
    #[default]
    Idle,
    Loading,
    Ready(Vec<StreamChoice>),
    Error(String),
}

struct Running {
    key: String,
    stop: Arc<AtomicBool>,
}

impl CollectionView {
    /// Drops the loaded sources and asks a running mpv to quit.
    pub(super) fn watch_reset(&mut self) {
        if let Some(running) = &self.watch.running {
            running.stop.store(true, Ordering::Relaxed);
        }
        self.watch = WatchPlayback::default();
    }

    /// Loads the streams of `game` unless the title already has a result
    /// (loaded, loading or failed). Play resets a failure to ask again.
    pub(super) fn watch_ensure_streams(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        let key = game.key.clone();
        let current = self.watch.item.as_deref() == Some(key.as_str());
        // Loading, loaded or failed: the detail page draws what it has. Play
        // resets a failure to ask again.
        if current && !matches!(self.watch.state, StreamsState::Idle) {
            return;
        }
        self.watch.item = Some(key.clone());
        self.watch.state = StreamsState::Loading;
        self.watch.selected = 0;
        cx.notify();

        let app = self.app_paths.clone();
        let id = game.app_name.clone();
        let provider = game.runner.clone();
        let kind = crate::fork::watch::sources::kind(game).to_owned();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { heronite_watch::playback::streams(&app, &provider, &kind, &id) }).await;
            this.update(cx, |this, cx| {
                // Another title took the selection while this one loaded.
                if this.watch.item.as_deref() != Some(key.as_str()) {
                    return;
                }
                this.watch.state = match result {
                    Ok(streams) => StreamsState::Ready(streams),
                    Err(error) => {
                        log::warn!("watch streams: {error:#}");
                        StreamsState::Error(tr!("Could not load streams: {error}", error = format!("{error:#}")).to_string())
                    }
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Play, Stop or "find the streams": what Play does for a film.
    pub(super) fn watch_primary(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        if self.watch.running.as_ref().is_some_and(|running| running.key == game.key) {
            self.watch_stop(cx);
            return;
        }
        if matches!(self.watch.state, StreamsState::Error(_) | StreamsState::Idle) {
            self.watch.item = None;
            self.watch.state = StreamsState::Idle;
            self.watch_ensure_streams(game, cx);
            return;
        }
        match &self.watch.state {
            StreamsState::Ready(streams) if !streams.is_empty() => {
                let index = self.watch.selected.min(streams.len() - 1);
                self.watch_play_stream(game, index, cx);
            }
            StreamsState::Ready(_) => self.show_toast(tr!("No streams found for this title."), true, cx),
            StreamsState::Loading | StreamsState::Error(_) | StreamsState::Idle => {}
        }
    }

    /// Starts `streams[index]` through the Stremio service and mpv.
    pub(super) fn watch_play_stream(&mut self, game: &CollectionGame, index: usize, cx: &mut Context<Self>) {
        if self.watch.item.as_deref() != Some(game.key.as_str()) {
            return;
        }
        let StreamsState::Ready(streams) = &self.watch.state else { return };
        let Some(stream) = streams.get(index).cloned() else { return };
        if let Some(running) = &self.watch.running {
            if running.key != game.key {
                self.show_toast(tr!("Another title is playing. Stop it first."), true, cx);
            }
            return;
        }

        self.watch.selected = index;
        let stop = Arc::new(AtomicBool::new(false));
        self.watch.running = Some(Running { key: game.key.clone(), stop: stop.clone() });
        self.playing.insert(game.app_name.clone(), true);
        self.refresh_focus();
        cx.notify();

        let app = self.app_paths.clone();
        let request = heronite_watch::playback::PlaybackRequest {
            provider: game.runner.clone(),
            id: game.app_name.clone(),
            kind: crate::fork::watch::sources::kind(game).to_owned(),
            title: game.title.clone(),
            stream,
        };
        let (key, app_name) = (game.key.clone(), game.app_name.clone());
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name("watch-playback".into())
            .spawn(move || {
                let _ = tx.send(heronite_watch::playback::play(&app, &request, &stop));
            })
            .ok();

        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| this.watch_finished(&key, &app_name, result, cx)).ok();
        })
        .detach();
    }

    /// Asks the running mpv to quit; the session ends and clears `playing`.
    fn watch_stop(&mut self, cx: &mut Context<Self>) {
        if let Some(running) = &self.watch.running {
            running.stop.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    fn watch_finished(
        &mut self,
        key: &str,
        app_name: &str,
        result: Result<Result<heronite_watch::player::PlaybackOutcome, anyhow::Error>, futures::channel::oneshot::Canceled>,
        cx: &mut Context<Self>,
    ) {
        self.playing.remove(app_name);
        if self.watch.running.as_ref().is_some_and(|running| running.key == key) {
            self.watch.running = None;
        }
        self.refresh_focus();
        if let Ok(Err(error)) = result {
            log::warn!("watch playback: {error:#}");
            self.show_toast(tr!("Could not play: {error}", error = format!("{error:#}")), true, cx);
        }
        cx.notify();
    }

    /// The rows the console game page adds under Play: one per stream.
    pub(super) fn watch_console_rows(&self, game: &CollectionGame) -> Vec<Row> {
        if self.watch.item.as_deref() != Some(game.key.as_str()) {
            return vec![Row::text("watch:loading", tr!("Streams"), tr!("Finding streams\u{2026}")).icon(Icon::Downloading)];
        }
        match &self.watch.state {
            StreamsState::Idle | StreamsState::Loading => {
                vec![Row::text("watch:loading", tr!("Streams"), tr!("Finding streams\u{2026}")).icon(Icon::Downloading)]
            }
            StreamsState::Error(error) => {
                vec![Row::text("watch:error", tr!("Streams"), error.clone()).icon(Icon::Warning)]
            }
            StreamsState::Ready(streams) if streams.is_empty() => {
                vec![Row::text("watch:empty", tr!("Streams"), tr!("No streams found for this title.")).icon(Icon::Warning)]
            }
            StreamsState::Ready(streams) => {
                let mut rows = vec![Row::header(tr!("Streams"))];
                rows.extend(streams.iter().enumerate().map(|(index, stream)| {
                    let label = stream_label(stream);
                    Row::action(format!("stream:{index}"), label.clone()).icon(Icon::PlayArrow).desc(stream_detail(stream, &label))
                }));
                rows
            }
        }
    }

    /// The desktop detail panel's "Streams" section. Asks for the streams
    /// the first time the film's detail is drawn and returns `None` while the
    /// title is not a film.
    pub(super) fn watch_detail_section(&mut self, key: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.space != super::Space::Watch {
            return None;
        }
        if let Some(game) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() {
            self.watch_ensure_streams(&game, cx);
        }
        self.watch_streams_section(key, cx)
    }

    fn watch_streams_section(&mut self, key: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.watch.item.as_deref() != Some(key) {
            return None;
        }
        let streams = match &self.watch.state {
            StreamsState::Idle => return None,
            StreamsState::Loading => {
                return Some(notice(tr!("Finding streams\u{2026}")));
            }
            StreamsState::Error(error) => return Some(notice(error.clone())),
            StreamsState::Ready(streams) => streams.clone(),
        };

        let heading = div()
            .mb(px(8.8))
            .font_family(theme::font_heading())
            .text_size(px(16.8))
            .line_height(px(20.))
            .font_weight(theme::W_BOLD)
            .text_color(theme::text_bright())
            .child(tr!("Streams"));
        let mut list = div().flex().flex_col().gap(px(6.));
        if streams.is_empty() {
            list = list.child(notice(tr!("No streams found for this title.")));
        }
        for (index, stream) in streams.iter().enumerate() {
            let label = stream_label(stream);
            let detail = stream_detail(stream, &label);
            let play_key = SharedString::from(key.to_owned());
            list = list.child(
                div()
                    .id(SharedString::from(format!("watch-stream-{index}")))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .bg(theme::focus_card_fill(false))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::ink_a(0.12)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(game) = this.lib.as_ref().and_then(|lib| lib.get(&play_key)).cloned() else { return };
                        this.watch_play_stream(&game, index, cx);
                    }))
                    .child(icon(Icon::PlayArrow, px(18.), theme::text_muted()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(div().truncate().text_size(px(14.)).font_weight(theme::W_BOLD).child(label))
                            .child(div().truncate().text_size(px(12.5)).text_color(theme::text_muted()).child(detail)),
                    ),
            );
        }
        Some(div().flex().flex_col().child(heading).child(list).into_any_element())
    }
}

fn notice(text: impl Into<SharedString>) -> AnyElement {
    div()
        .px(px(14.))
        .py(px(12.))
        .rounded(px(10.))
        .bg(theme::focus_card_fill(false))
        .text_size(px(13.))
        .text_color(theme::text_muted())
        .child(text.into())
        .into_any_element()
}

/// A quality line if the addon gave one, else its name, else the addon.
fn stream_label(stream: &StreamChoice) -> String {
    if let Some(line) = stream.description.as_deref().and_then(|text| text.lines().map(str::trim).find(|line| is_quality(line))) {
        return line.to_owned();
    }
    if let Some(name) = stream.name.as_deref().and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty())) {
        return name.to_owned();
    }
    if !stream.addon.is_empty() {
        return stream.addon.clone();
    }
    tr!("Stream").to_string()
}

fn is_quality(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    ["2160p", "1440p", "1080p", "720p", "480p", "4k", "hdr"].iter().any(|quality| line.contains(quality))
}

/// The line under a stream's name: source, size, peers and what is left of
/// the addon's description.
fn stream_detail(stream: &StreamChoice, label: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !stream.addon.is_empty() {
        parts.push(stream.addon.clone());
    }
    if let Some(size) = stream.size_bytes {
        parts.push(heronite_epic::util::file_size_local(size as f64));
    }
    if let Some(seeders) = stream.seeders {
        parts.push(trn!(seeders, "{n} seeder", "{n} seeders"));
    }
    if let Some(description) = &stream.description {
        for line in description.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if line == label || line.starts_with('\u{1f464}') || line.starts_with('\u{1f4be}') {
                continue;
            }
            if !parts.iter().any(|part| part == line) {
                parts.push(line.to_owned());
            }
        }
    }
    parts.join(" \u{b7} ")
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    fn stream(name: &str, description: &str, seeders: u32, size: u64) -> StreamChoice {
        StreamChoice {
            addon: "Torrentio".into(),
            name: Some(name.into()),
            description: Some(description.into()),
            info_hash: Some("dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c".into()),
            seeders: Some(seeders),
            size_bytes: Some(size),
            ..StreamChoice::default()
        }
    }

    #[test]
    fn a_quality_line_names_the_stream() {
        let s = stream("Torrentio", "1080p\n\u{1f464} 123 \u{1f4be} 1.5 GB", 123, 1_500_000_000);
        assert_eq!(stream_label(&s), "1080p");
    }

    #[test]
    fn without_a_quality_the_addon_name_is_used() {
        let s = stream("Torrentio", "Direct\nA film", 0, 0);
        assert_eq!(stream_label(&s), "Torrentio");
    }

    #[test]
    fn the_detail_keeps_source_size_and_peers_once() {
        let s = stream("Torrentio", "1080p\n\u{1f464} 123 \u{1f4be} 1.5 GB", 123, 1_500_000_000);
        let detail = stream_detail(&s, "1080p");
        assert!(detail.contains("Torrentio"), "{detail}");
        assert!(detail.contains("123"), "{detail}");
        assert!(detail.contains(&heronite_epic::util::file_size_local(1_500_000_000.0)), "{detail}");
        assert!(!detail.contains("\u{1f464}"), "{detail}");
        assert!(!detail.contains("\u{1f4be}"), "{detail}");
    }
}
