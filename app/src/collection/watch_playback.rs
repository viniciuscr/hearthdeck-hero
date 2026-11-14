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
use heronite_watch::metadata::{TitleDetails, TitleRef};
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
    /// The episode `state` is for, when the title is a series.
    video: Option<String>,
    state: StreamsState,
    /// The stream the main Play starts; the first when the list arrives.
    selected: usize,
    /// mpv is playing this title.
    running: Option<Running>,
    /// What the metadata sources say about the title (`console/watch_page.rs`).
    info: TitleInfo,
    /// The season and episode the picker shows (`console/watch_page.rs`).
    pick: Pick,
    /// Play the first stream as soon as this episode's list arrives (A on an
    /// episode plays it at once).
    pending: Option<String>,
}

/// The series picker's state: which season and episode the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Selection {
    /// Index into the title's seasons, not the season number.
    pub season: usize,
    /// Index into that season's episodes.
    pub episode: usize,
}

/// The selection and the title it belongs to.
#[derive(Default)]
struct Pick {
    item: Option<String>,
    selection: Selection,
}

/// The metadata sources' answer for the title, and where the load stands.
#[derive(Default)]
struct TitleInfo {
    /// The title `state` describes (the Collection key).
    item: Option<String>,
    state: TitleState,
    /// The episode the stored progress points at, when the page opened.
    resume: Option<String>,
}

#[derive(Default)]
enum TitleState {
    #[default]
    Idle,
    Loading,
    Ready(Arc<TitleDetails>),
    /// No metadata source knows this title; the page shows the library's own.
    Missing,
    /// The sources could not be asked; the page says so.
    Failed,
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

    /// Loads what the metadata sources know about `game`, once per title: the
    /// facade answers from its week-long cache before it asks anyone.
    pub(super) fn watch_ensure_details(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        let key = game.key.clone();
        if self.watch.info.item.as_deref() == Some(key.as_str()) && !matches!(self.watch.info.state, TitleState::Idle) {
            return;
        }
        self.watch.info.item = Some(key.clone());
        self.watch.info.state = TitleState::Loading;
        cx.notify();

        let app = self.app_paths.clone();
        let provider = game.runner.clone();
        let id = crate::fork::watch::sources::id(game).to_owned();
        let title = TitleRef::new(&provider, &id, crate::fork::watch::sources::kind(game))
            .named(&game.title, crate::fork::watch::sources::year(game));
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let details = heronite_watch::metadata::details(&app, &title)?;
                    // Where the person stopped, read from the local records: offline
                    // and free, and what the picker opens on.
                    let resume = heronite_watch::progress::local(&app, &provider, &id).and_then(|progress| progress.video_id);
                    Ok::<_, anyhow::Error>((details, resume))
                })
                .await;
            this.update(cx, |this, cx| {
                // Another title took the page while this one loaded.
                if this.watch.info.item.as_deref() != Some(key.as_str()) {
                    return;
                }
                this.watch.info.state = match result {
                    Ok((Some(details), resume)) => {
                        this.watch.info.resume = resume;
                        // The new details decide which episode the picker opens on.
                        this.watch.pick = Pick::default();
                        TitleState::Ready(Arc::new(details))
                    }
                    Ok((None, _)) => TitleState::Missing,
                    Err(error) => {
                        log::debug!("watch details: {error:#}");
                        TitleState::Failed
                    }
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The details of `key`, when they arrived. The `Arc` keeps the page able to
    /// draw them while it borrows the view mutably.
    pub(super) fn watch_details(&self, key: &str) -> Option<Arc<TitleDetails>> {
        if self.watch.info.item.as_deref() != Some(key) {
            return None;
        }
        match &self.watch.info.state {
            TitleState::Ready(details) => Some(details.clone()),
            _ => None,
        }
    }

    /// Whether the sources could not be asked for `key`'s details.
    pub(super) fn watch_details_failed(&self, key: &str) -> bool {
        self.watch.info.item.as_deref() == Some(key) && matches!(self.watch.info.state, TitleState::Failed)
    }

    /// Loads the streams of `game` (or of one episode, when `video` names it)
    /// unless that answer is already there. Play resets a failure to ask again.
    pub(super) fn watch_ensure_streams(&mut self, game: &CollectionGame, video: Option<&str>, cx: &mut Context<Self>) {
        let key = game.key.clone();
        let video = video.map(str::to_owned);
        let current = self.watch.item.as_deref() == Some(key.as_str()) && self.watch.video == video;
        // Loading, loaded or failed: the page draws what it has. Play resets a
        // failure to ask again.
        if current && !matches!(self.watch.state, StreamsState::Idle) {
            return;
        }
        self.watch.item = Some(key.clone());
        self.watch.video = video.clone();
        self.watch.state = StreamsState::Loading;
        self.watch.selected = 0;
        cx.notify();

        let app = self.app_paths.clone();
        let id = video.clone().unwrap_or_else(|| crate::fork::watch::sources::id(game).to_owned());
        let provider = game.runner.clone();
        let kind = crate::fork::watch::sources::kind(game).to_owned();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { heronite_watch::playback::streams(&app, &provider, &kind, &id) }).await;
            this.update(cx, |this, cx| {
                // Another title or episode took the page while this one loaded.
                if this.watch.item.as_deref() != Some(key.as_str()) || this.watch.video != video {
                    return;
                }
                this.watch.state = match result {
                    Ok(streams) => StreamsState::Ready(streams),
                    Err(error) => {
                        log::warn!("watch streams: {error:#}");
                        StreamsState::Error(tr!("Could not load streams: {error}", error = format!("{error:#}")).to_string())
                    }
                };
                // A on an episode: its best stream starts as soon as it is known.
                let play_now = this.watch.pending.take().is_some_and(|pending| Some(pending) == this.watch.video);
                let has_streams = matches!(&this.watch.state, StreamsState::Ready(streams) if !streams.is_empty());
                cx.notify();
                if play_now && has_streams {
                    if let Some(game) = this.lib.as_ref().and_then(|lib| lib.get(&key)).cloned() {
                        this.watch_play_stream(&game, 0, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Opening a title's page: a film asks for its streams now; a series waits
    /// for the page to pick an episode.
    pub(super) fn watch_open_title(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        if crate::fork::watch::sources::kind(game) == "series" {
            return;
        }
        self.watch_ensure_streams(game, None, cx);
    }

    /// A on an episode: it becomes the picked one, its streams are asked for and
    /// the best starts when they arrive.
    pub(super) fn watch_play_episode(&mut self, game: &CollectionGame, video: &str, cx: &mut Context<Self>) {
        let same = self.watch.item.as_deref() == Some(game.key.as_str()) && self.watch.video.as_deref() == Some(video);
        match (same, &self.watch.state) {
            (true, StreamsState::Ready(streams)) if !streams.is_empty() => {
                self.watch_play_stream(game, 0, cx);
                return;
            }
            (true, StreamsState::Loading) => {
                self.watch.pending = Some(video.to_owned());
                cx.notify();
                return;
            }
            _ => {}
        }
        // Idle, failed, empty or another episode: ask again and play the best.
        self.watch.item = Some(game.key.clone());
        self.watch.video = Some(video.to_owned());
        self.watch.state = StreamsState::Idle;
        self.watch.pending = Some(video.to_owned());
        self.watch_ensure_streams(game, Some(video), cx);
    }

    /// The picker's selection for `key`: the episode the stored progress points
    /// at, else the first episode of the first season.
    pub(super) fn watch_ensure_selection(&mut self, key: &str, details: &TitleDetails) -> Selection {
        let selection = match self.watch.pick.item.as_deref() {
            Some(item) if item == key => self.watch.pick.selection,
            _ => default_selection(details, self.watch.info.resume.as_deref()),
        };
        let selection = clamp_selection(details, selection);
        if self.watch.pick.item.as_deref() != Some(key) || self.watch.pick.selection != selection {
            self.watch.pick = Pick { item: Some(key.to_owned()), selection };
        }
        selection
    }

    /// ◀ ▶ on the season row: the new season's first episode.
    pub(super) fn watch_pick_season(&mut self, key: &str, details: &TitleDetails, season: usize) -> Selection {
        let season = season.min(details.season_count().saturating_sub(1));
        let selection = clamp_selection(details, Selection { season, episode: 0 });
        self.watch.pick = Pick { item: Some(key.to_owned()), selection };
        selection
    }

    /// The episode the focus settled on: what A would play.
    pub(super) fn watch_pick_episode(&mut self, key: &str, selection: Selection) -> Selection {
        self.watch.pick = Pick { item: Some(key.to_owned()), selection };
        selection
    }

    /// The episode the stored progress points at: the picker ticks it.
    pub(super) fn watch_resume(&self) -> Option<&str> {
        self.watch.info.resume.as_deref()
    }

    /// Forgets the loaded streams: another season's episodes come next.
    pub(super) fn watch_forget_streams(&mut self) {
        self.watch.item = None;
        self.watch.video = None;
        self.watch.pending = None;
        self.watch.state = StreamsState::Idle;
    }

    /// Play, Stop or "find the streams": what Play does for a film.
    pub(super) fn watch_primary(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        if self.watch.running.as_ref().is_some_and(|running| running.key == game.key) {
            self.watch_stop(cx);
            return;
        }
        if matches!(self.watch.state, StreamsState::Error(_) | StreamsState::Idle) {
            let video = self.watch.video.clone();
            self.watch.state = StreamsState::Idle;
            self.watch_ensure_streams(game, video.as_deref(), cx);
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
            id: crate::fork::watch::sources::id(game).to_owned(),
            kind: crate::fork::watch::sources::kind(game).to_owned(),
            video_id: self.watch.video.clone(),
            // What mpv's window and the desktop panels call it: the episode,
            // when the page picked one.
            title: self.watch_title(game),
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

    /// The title as a playing session names it: the episode's code and name when
    /// the page picked one, else the title.
    fn watch_title(&self, game: &CollectionGame) -> String {
        let Some(video) = self.watch.video.as_deref() else { return game.title.clone() };
        let Some(details) = self.watch_details(&game.key) else { return game.title.clone() };
        let Some(episode) = details.episode_with_id(video) else { return game.title.clone() };
        match episode.name.trim() {
            "" => format!("{} · {}", game.title, episode.code()),
            name => format!("{} · {} {name}", game.title, episode.code()),
        }
    }

    /// The rows the console page adds under Play for a film: one per stream.
    pub(super) fn watch_console_rows(&self, game: &CollectionGame) -> Vec<Row> {
        self.watch_stream_rows(game, None)
    }

    /// The rows of one episode's streams (`None`: the title's own, for a film).
    pub(super) fn watch_stream_rows(&self, game: &CollectionGame, video: Option<&str>) -> Vec<Row> {
        if self.watch.item.as_deref() != Some(game.key.as_str()) || self.watch.video.as_deref() != video {
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
        let game = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned()?;
        // A series' streams belong to an episode, and only the console page
        // picks one; the desktop panel lists them for films.
        if crate::fork::watch::sources::kind(&game) == "series" {
            return None;
        }
        self.watch_ensure_streams(&game, None, cx);
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

/// The picker's first selection: the episode the stored pointer names, else the
/// first episode of the first season.
fn default_selection(details: &TitleDetails, resume: Option<&str>) -> Selection {
    let seasons = details.seasons();
    let Some(resume) = resume else { return Selection::default() };
    let Some(episode) = details.episode_with_id(resume) else { return Selection::default() };
    let season = seasons.iter().position(|season| *season == episode.season).unwrap_or(0);
    let listed = seasons.get(season).copied().unwrap_or(episode.season);
    let episode = details.episodes_of(listed).position(|listed| listed.id == resume).unwrap_or(0);
    Selection { season, episode }
}

/// Keeps a selection inside the seasons and episodes the source lists.
fn clamp_selection(details: &TitleDetails, selection: Selection) -> Selection {
    let seasons = details.seasons();
    let season = selection.season.min(seasons.len().saturating_sub(1));
    let episodes = seasons.get(season).map(|season| details.episodes_of(*season).count()).unwrap_or(0);
    Selection { season, episode: selection.episode.min(episodes.saturating_sub(1)) }
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
    use heronite_watch::metadata::Episode;
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

    fn episode(season: u32, number: u32) -> Episode {
        Episode { id: format!("tt1:{season}:{number}"), season, number, name: format!("S{season}E{number}"), ..Episode::default() }
    }

    fn series() -> TitleDetails {
        TitleDetails {
            kind: "series".into(),
            id: "tt1".into(),
            name: "A Series".into(),
            episodes: vec![episode(1, 1), episode(1, 2), episode(2, 1), episode(2, 2), episode(2, 3)],
            ..TitleDetails::default()
        }
    }

    #[test]
    fn the_picker_opens_on_the_stored_episode() {
        let details = series();
        assert_eq!(default_selection(&details, Some("tt1:2:2")), Selection { season: 1, episode: 1 });
        assert_eq!(default_selection(&details, None), Selection::default(), "no progress: the first episode");
        assert_eq!(default_selection(&details, Some("tt9:1:1")), Selection::default(), "an episode this title does not list");
    }

    #[test]
    fn a_selection_stays_inside_the_seasons_the_source_lists() {
        let details = series();
        assert_eq!(clamp_selection(&details, Selection { season: 9, episode: 9 }), Selection { season: 1, episode: 2 });
        assert_eq!(clamp_selection(&details, Selection { season: 0, episode: 9 }), Selection { season: 0, episode: 1 });
        assert_eq!(clamp_selection(&TitleDetails::default(), Selection { season: 3, episode: 3 }), Selection::default());
    }
}
