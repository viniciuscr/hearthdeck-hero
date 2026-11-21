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
use heronite_watch::metadata::{Episode, TitleDetails, TitleRef};
use heronite_watch::types::StreamChoice;

use super::CollectionView;
use crate::console::list::{ListNav, Row};
use crate::console::Pad;
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
    /// Which card the page shows beside the list: the title's details or its
    /// streams.
    tab: DetailTab,
    /// Which side the pad drives: the title's list, its marks or the card.
    pane: Pane,
    /// The focus of the card's stream list.
    streams_nav: ListNav,
    /// The mark of the page's icon row the pad is on (`console/watch_page.rs`).
    mark: usize,
    /// The title the page last opened: another one starts on the list.
    page: Option<String>,
    /// Play the first stream as soon as this title/episode's list arrives (A on
    /// an episode, or Play, starts the best at once). `Some(None)` is a film.
    pending: Option<Option<String>>,
    /// A trailer was asked for before the title's details arrived (from a
    /// cover, where nobody has opened the page yet): it plays with them.
    trailer_pending: Option<String>,
    /// The card of the "Similar" / "Community" rail the pad is on.
    rail: usize,
    /// A title a rail's card opened, by IMDb id: the page turns to it once the
    /// library holds it (the press adds it and reloads).
    opening: Option<String>,
    /// The page whose rail titles are being kept, and where that keep stands:
    /// a rail asks once, not once per frame (and gives up rather than looping).
    rails_page: Option<String>,
    rails_tries: u8,
    rails_loading: bool,
    /// The keep was written and a reload is on its way: asking again until it
    /// lands is what made the same set be kept twice.
    rails_waiting_reload: bool,
    /// The wait has been said once: a render that arrives while the load is in
    /// flight must not repeat it per frame.
    rails_spoke: bool,
    rails_failed: bool,
}

impl WatchPlayback {
    /// The card of the rail the pad is on (clamped to `len`).
    fn rail_focus(&self, len: usize) -> usize {
        self.rail.min(len.saturating_sub(1))
    }
}

/// Where a press on a rail takes the focus, `None` when the press leaves the
/// rail for the title's list: ◀ from the first card, ▲ from the top row (the
/// grid's edges, as B does anywhere).
///
/// The grid is `len` cards laid out `columns` to a row — the page draws it that
/// way, so the arithmetic lives here, where a test can walk it.
pub(super) fn rail_step(focus: usize, columns: usize, len: usize, pad: Pad) -> Option<usize> {
    if len == 0 || columns == 0 {
        return None;
    }
    let last = len - 1;
    let focus = focus.min(last);
    match pad {
        Pad::Left if focus == 0 => None,
        Pad::Left => Some(focus - 1),
        Pad::Right => Some((focus + 1).min(last)),
        Pad::Up if focus < columns => None,
        Pad::Up => Some(focus - columns),
        // The last row is short: a step past it stays on the card it is on.
        Pad::Down => Some((focus + columns).min(last)),
        _ => None,
    }
}

/// The series picker's state: which season and episode the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Selection {
    /// Index into the title's seasons, not the season number.
    pub season: usize,
    /// Index into that season's episodes.
    pub episode: usize,
}

/// The cards the film page shows beside the list: what the title is, where it
/// plays, what people recommend beside it and what Simkl calls alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DetailTab {
    #[default]
    Details,
    Streams,
    /// Titles people recommend alongside this one (Simkl's community list).
    Community,
    /// Titles the sources say are alike (Simkl's `similar`).
    Similar,
}

impl DetailTab {
    /// Whether the card is one of the two rails: the pad walks its cards with
    /// ◀ ▶, and A opens the one it is on.
    pub(super) fn is_rail(self) -> bool {
        matches!(self, Self::Community | Self::Similar)
    }

    /// The card beside this one in the tab bar, wrapping at either end: RB walks
    /// it forwards, LB back.
    pub(super) fn next(self, forward: bool) -> Self {
        let at = DETAIL_TABS.iter().position(|tab| *tab == self).unwrap_or(0);
        let next = if forward { (at + 1) % DETAIL_TABS.len() } else { (at + DETAIL_TABS.len() - 1) % DETAIL_TABS.len() };
        DETAIL_TABS[next]
    }
}

/// The page's cards, in the order the tab bar draws them and LB / RB walk them.
pub(super) const DETAIL_TABS: [DetailTab; 4] = [DetailTab::Details, DetailTab::Streams, DetailTab::Community, DetailTab::Similar];

/// Which side of the film page the pad drives: the title's list (Play, season,
/// episodes), its row of marks (watched, watch later, favourite) or the card
/// beside it (the details to scroll, or the streams).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Pane {
    #[default]
    List,
    /// The watched / watch later / favourite icons above the list.
    Marks,
    Card,
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

/// What the page knows about a title's trailer, before asking anyone.
#[derive(Debug)]
enum Trailer {
    /// The first trailer's page, for mpv.
    Found(String),
    /// The sources answered, and the title has none.
    NotFound,
    /// The sources could not be asked.
    Unavailable,
    /// Nobody has asked yet: the details are still on their way.
    Unasked,
}

/// What is known about `key`'s trailer from the details in hand: `item` names
/// the title `state` describes, which is another one once a cover asks for its
/// own (nothing is loaded for it yet).
fn trailer_of(item: Option<&str>, key: &str, state: &TitleState) -> Trailer {
    if item != Some(key) {
        return Trailer::Unasked;
    }
    match state {
        TitleState::Ready(details) => match details.trailers.first() {
            Some(trailer) => Trailer::Found(trailer.url.clone()),
            None => Trailer::NotFound,
        },
        TitleState::Missing => Trailer::NotFound,
        TitleState::Failed => Trailer::Unavailable,
        TitleState::Idle | TitleState::Loading => Trailer::Unasked,
    }
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
        // What the log names: the title's own id (`stremio:tt0149460`), never the
        // library key, whose `stremio_stremio:` prefix reads like a provider id.
        let label = format!("{provider}:{id}");
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
                // Another title took the page while this one loaded: a press
                // that waited for these details waited for nothing, and must
                // not play a trailer by itself when they are asked for again.
                if this.watch.info.item.as_deref() != Some(key.as_str()) {
                    if this.watch.trailer_pending.as_deref() == Some(key.as_str()) {
                        this.watch.trailer_pending = None;
                    }
                    return;
                }
                this.watch.info.state = match result {
                    Ok((Some(details), resume)) => {
                        log::info!(
                            target: "heronite_app",
                            "watch details: {label} answered — similar {}, recommended {}",
                            details.similar.len(),
                            details.recommended.len()
                        );
                        this.watch.info.resume = resume;
                        // The new details decide which episode the picker opens on.
                        this.watch.pick = Pick::default();
                        TitleState::Ready(Arc::new(details))
                    }
                    Ok((None, _)) => {
                        log::info!(target: "heronite_app", "watch details: the sources know nothing about {key}");
                        TitleState::Missing
                    }
                    Err(error) => {
                        log::warn!(target: "heronite_app", "watch details: {key} could not be asked: {error:#}");
                        TitleState::Failed
                    }
                };
                // A trailer pressed on a cover asked for these details: it plays
                // now, or says why it cannot. A failed answer is said here, so a
                // press never asks the sources twice in a row.
                if this.watch.trailer_pending.as_deref() == Some(key.as_str()) {
                    this.watch.trailer_pending = None;
                    if matches!(this.watch.info.state, TitleState::Failed) {
                        this.show_toast(tr!("Details unavailable right now."), true, cx);
                    } else if let Some(game) = this.lib.as_ref().and_then(|lib| lib.get(&key)).cloned() {
                        this.watch_play_trailer(&game, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The IMDb id of the title the page is open on: the page a keep belongs to,
    /// and the only one whose rails the library keeps (`related:{imdb}`).
    pub(super) fn watch_page_id(&self) -> Option<String> {
        let key = SharedString::from(self.watch.info.item.clone()?);
        let game = self.lib.as_ref()?.get(&key)?;
        Some(crate::fork::watch::sources::id(game).to_owned())
    }

    /// A rail is on screen: its titles join the library, so its cards are the
    /// app's own tiles (theme, focus, titles under them) and a press opens a
    /// title that already exists.
    ///
    /// It runs when a rail is **shown**, not when the details land: a person who
    /// never opens Community or Similar never pays for its titles, and pressing
    /// the trailer on a cover adds nothing. The set is scoped to this page
    /// (`related:{imdb}`), so the next page's rails take these away instead of
    /// the library growing with every page ever looked at.
    pub(super) fn watch_ensure_rails(&mut self, cx: &mut Context<Self>) {
        let TitleState::Ready(details) = &self.watch.info.state else {
            log::debug!(target: "heronite_app", "watch rails: the details are not in yet");
            return;
        };
        let Some(page) = self.watch_page_id() else {
            log::debug!(target: "heronite_app", "watch rails: no page id to keep titles for");
            return;
        };
        if self.watch.rails_loading || self.watch.rails_waiting_reload {
            if !self.watch.rails_spoke {
                self.watch.rails_spoke = true;
                log::debug!(target: "heronite_app", "watch rails: {page} is already being kept or reloading");
            }
            return;
        }
        // A new page starts its own count: the retry bound is per page, not per
        // run, or a page that could not be kept would never be tried again.
        if self.watch.rails_page.as_deref() != Some(page.as_str()) {
            self.watch.rails_tries = 0;
            self.watch.rails_failed = false;
        }
        let rails = details.similar.len() + details.recommended.len();
        let known: std::collections::HashSet<String> =
            self.lib.iter().flat_map(|lib| lib.games.iter()).map(|game| crate::fork::watch::sources::id(game).to_owned()).collect();
        // The **whole** set, never only what looks missing: the keep replaces the
        // page's scoped feed, so handing it a subset deletes the rest — which
        // came back as "missing" on the next reload, and that is a loop.
        let titles: Vec<heronite_watch::types::Title> =
            details.similar.iter().chain(details.recommended.iter()).map(heronite_watch::metadata::related_title).collect();
        let missing =
            details.similar.iter().chain(details.recommended.iter()).filter(|related| !known.contains(related.id.as_str())).count();
        if missing == 0 {
            if self.watch.rails_page.as_deref() != Some(page.as_str()) {
                log::info!(target: "heronite_app", "watch rails: {page} already has all {rails} titles in the library");
                self.watch.rails_page = Some(page);
                self.watch.rails_failed = false;
            }
            return;
        }
        if self.watch.rails_tries >= 3 {
            // The set was written and the load still does not hold it: say so
            // instead of keeping it again and again.
            if !self.watch.rails_failed {
                log::warn!(
                    target: "heronite_app",
                    "watch rails: {page} still misses {missing} of {rails} titles after {} keeps; giving up",
                    self.watch.rails_tries
                );
                self.watch.rails_failed = true;
            }
            return;
        }
        log::info!(
            target: "heronite_app",
            "watch rails: keeping {rails} titles for {page} ({missing} missing, similar {}, recommended {})",
            details.similar.len(),
            details.recommended.len()
        );
        self.watch.rails_tries = self.watch.rails_tries.saturating_add(1);
        self.watch.rails_loading = true;
        self.watch.rails_failed = false;
        self.watch.rails_spoke = false;
        let app = self.app_paths.clone();
        cx.spawn(async move |this, cx| {
            let asking = page.clone();
            let result = cx.background_spawn(async move { heronite_watch::catalog::keep_related(&app, &asking, titles) }).await;
            this.update(cx, |this, cx| {
                this.watch.rails_loading = false;
                match result {
                    Ok(added) => {
                        log::info!(target: "heronite_app", "watch rails: wrote {added} title(s) for {page}, reloading");
                        this.watch.rails_page = Some(page);
                        this.watch.rails_waiting_reload = true;
                        // The reload is what turns them into library titles: the
                        // page's tiles take their art from its cover chains.
                        this.reload(cx);
                    }
                    Err(error) => {
                        log::warn!(target: "heronite_app", "watch rails: keeping {page}'s titles failed: {error:#}");
                        this.watch.rails_failed = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Whether that ask failed (the rail says so instead of waiting for ever).
    pub(super) fn watch_rails_failed(&self) -> bool {
        self.watch.rails_failed
    }

    /// Whether this wait has already been said (renders must not repeat it).
    pub(super) fn watch_rails_spoke(&self) -> bool {
        self.watch.rails_spoke
    }

    /// Says it: the next render of this wait is silent.
    pub(super) fn watch_rails_say(&mut self) {
        self.watch.rails_spoke = true;
    }

    /// A reload landed: say what the library holds of the page's rails, so the
    /// next keep is asked for only when something is really missing.
    pub(super) fn watch_rails_reloaded(&mut self) {
        if !self.watch.rails_waiting_reload {
            return;
        }
        self.watch.rails_waiting_reload = false;
        // The next keep may say its wait again.
        self.watch.rails_spoke = false;
        let TitleState::Ready(details) = &self.watch.info.state else { return };
        let known: std::collections::HashSet<String> =
            self.lib.iter().flat_map(|lib| lib.games.iter()).map(|game| crate::fork::watch::sources::id(game).to_owned()).collect();
        let total = details.similar.len() + details.recommended.len();
        let found = details.similar.iter().chain(details.recommended.iter()).filter(|related| known.contains(related.id.as_str())).count();
        let page = self.watch_page_id().unwrap_or_default();
        if found == total {
            log::info!(target: "heronite_app", "watch rails: reload landed — all {total} titles for {page} have a library key");
        } else {
            log::warn!(target: "heronite_app", "watch rails: reload landed — only {found} of {total} titles for {page} have a library key");
        }
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

    /// What the page's own stream probe found for `game`, when it was about the
    /// title itself: a film's answer is newer than the projection's, which only
    /// knows what the last load asked. A series' answer is one episode's, so it
    /// says nothing here.
    pub(super) fn watch_streams_known(&self, game: &CollectionGame) -> Option<bool> {
        if self.watch.item.as_deref() != Some(game.key.as_str()) || self.watch.video.is_some() {
            return None;
        }
        match &self.watch.state {
            StreamsState::Ready(streams) => Some(!streams.is_empty()),
            _ => None,
        }
    }

    /// The genres a card shows for `game`: the merged metadata's when that
    /// answer has arrived (the focused title, cached for a week), else the
    /// catalogue's own list from the projection.
    pub(super) fn watch_card_genres(&self, game: &CollectionGame) -> Vec<String> {
        self.watch_details(&game.key)
            .map(|details| details.genres.clone())
            .filter(|genres| !genres.is_empty())
            .unwrap_or_else(|| game.details.genres.clone())
    }

    /// The length a card shows for `game` (`2h 53m`): the merged details'
    /// runtime when it has arrived, else the catalogue's. `None` when neither
    /// says — a source that knew the film may name no runtime at all. For a
    /// series this is **one episode's**; the label comes from
    /// `sources::length_label`.
    pub(super) fn watch_card_runtime(&self, game: &CollectionGame) -> Option<String> {
        self.watch_details(&game.key)
            .and_then(|details| details.runtime.as_deref().and_then(crate::fork::watch::sources::runtime_minutes_of))
            .or_else(|| crate::fork::watch::sources::runtime_minutes(game))
            .map(heronite_data::collection::format_playtime_minutes)
    }

    /// The whole show at the length its episodes run: what watching it from the
    /// first episode to the last takes. The episode count is the merged
    /// details', so this arrives with that answer (and not before).
    pub(super) fn watch_card_binge(&self, game: &CollectionGame) -> Option<String> {
        use crate::fork::watch::sources;
        if !sources::is_series(game) {
            return None;
        }
        let details = self.watch_details(&game.key)?;
        let per_episode = details.runtime.as_deref().and_then(sources::runtime_minutes_of).or_else(|| sources::runtime_minutes(game));
        sources::binge_minutes(per_episode, details.episode_count()).map(heronite_data::collection::format_playtime_minutes)
    }

    /// What is known about `key`'s trailer without asking: the details may not
    /// describe this title yet (a cover never opened it).
    fn watch_trailer(&self, key: &str) -> Trailer {
        trailer_of(self.watch.info.item.as_deref(), key, &self.watch.info.state)
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
        if !current {
            self.watch.streams_nav.reset();
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
        let title_id = crate::fork::watch::sources::id(game).to_owned();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let answer = heronite_watch::playback::streams(&app, &provider, &kind, &id);
                    // What the addons answered is the only proof a title can be
                    // played: the projection keeps it so a card can dim a title
                    // no source here has. A film's answer is the title's own; a
                    // series' is one episode's and says nothing about the show.
                    if kind == "movie" {
                        if let Ok(streams) = &answer {
                            if let Err(error) = heronite_watch::streams::note(&app, &provider, &kind, &title_id, streams.len()) {
                                log::warn!("recording the stream probe: {error:#}");
                            }
                        }
                    }
                    answer
                })
                .await;
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
                // A on an episode, or Play: the best stream starts as soon as it
                // is known.
                let play_now = this.watch.pending.take() == Some(this.watch.video.clone());
                let has_streams = matches!(&this.watch.state, StreamsState::Ready(streams) if !streams.is_empty());
                cx.notify();
                if play_now {
                    match (has_streams, this.lib.as_ref().and_then(|lib| lib.get(&key)).cloned()) {
                        (true, Some(game)) => this.watch_play_stream(&game, 0, cx),
                        (false, _) => this.show_toast(tr!("No streams found for this title."), true, cx),
                        (true, None) => {}
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
        self.watch_play_picked(game, Some(video), cx);
    }

    /// The Play button (and X): the best stream of the picked episode, or of the
    /// film, started as soon as the list is known. While the title plays it
    /// stops it.
    pub(super) fn watch_play_picked(&mut self, game: &CollectionGame, video: Option<&str>, cx: &mut Context<Self>) {
        if crate::fork::watch::sources::imported(game) {
            self.show_toast(tr!("Not currently present in any available stream source."), true, cx);
            return;
        }
        if self.watch.running.as_ref().is_some_and(|running| running.key == game.key) {
            self.watch_stop(cx);
            return;
        }
        let same = self.watch.item.as_deref() == Some(game.key.as_str()) && self.watch.video.as_deref() == video;
        match (same, &self.watch.state) {
            (true, StreamsState::Ready(streams)) if !streams.is_empty() => {
                self.watch_play_stream(game, 0, cx);
                return;
            }
            (true, StreamsState::Ready(_)) => {
                self.show_toast(tr!("No streams found for this title."), true, cx);
                return;
            }
            (true, StreamsState::Loading) => {
                self.watch.pending = Some(video.map(str::to_owned));
                cx.notify();
                return;
            }
            _ => {}
        }
        // Idle, failed or another title/episode: ask again and play the best.
        self.watch.item = Some(game.key.clone());
        self.watch.video = video.map(str::to_owned);
        self.watch.state = StreamsState::Idle;
        self.watch.streams_nav.reset();
        self.watch.pending = Some(video.map(str::to_owned));
        self.watch_ensure_streams(game, video, cx);
    }

    /// Which card the page shows beside the list.
    pub(super) fn watch_tab(&self) -> DetailTab {
        self.watch.tab
    }

    /// LB / RB: the card beside this one, with the pad where that card expects
    /// it. RB walks the tab bar forwards, LB back, and either end wraps.
    pub(super) fn watch_switch_tab(&mut self, forward: bool, cx: &mut Context<Self>) -> DetailTab {
        self.watch.tab = self.watch.tab.next(forward);
        self.watch.pane = tab_pane(self.watch.tab);
        // A rail opens on its first card, whichever card the page came from, and
        // its titles join the library the first time it is shown.
        self.watch.rail = 0;
        if self.watch.tab.is_rail() {
            self.watch_ensure_rails(cx);
        }
        self.watch.tab
    }

    /// The card of the rail the pad is on (clamped to `len`).
    pub(super) fn watch_rail_focus(&self, len: usize) -> usize {
        self.watch.rail_focus(len)
    }

    /// The card of a rail's grid the pad is on.
    pub(super) fn watch_set_rail(&mut self, index: usize) {
        self.watch.rail = index;
    }

    /// A rail's card was pressed: the page turns to that title once the library
    /// has it (`watch_take_opening`). The press adds it and reloads.
    pub(super) fn watch_open_soon(&mut self, id: &str) {
        self.watch.opening = Some(id.to_owned());
    }

    /// The title a rail's card opened, once: the page takes it and turns.
    pub(super) fn watch_take_opening(&mut self) -> Option<String> {
        self.watch.opening.take()
    }

    /// L3 (the click of the left stick), when the title has a trailer: it plays
    /// in **mpv**, like a title does, and leaves nothing behind — no progress,
    /// no session, no tracker event. Pressing it again while it plays stops it,
    /// and one player at a time still holds: a stream playing has to stop first.
    ///
    /// A cover on the list never opened the page, so the details may not be in
    /// hand yet: the press asks for them and the trailer starts with the answer.
    pub(super) fn watch_play_trailer(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        if let Some(running) = &self.watch.running {
            if running.key == game.key {
                running.stop.store(true, Ordering::Relaxed);
            } else {
                self.show_toast(tr!("Another title is playing. Stop it first."), true, cx);
            }
            cx.notify();
            return;
        }
        let url = match self.watch_trailer(&game.key) {
            Trailer::Found(url) => url,
            Trailer::NotFound => {
                self.show_toast(tr!("No trailer for this title."), true, cx);
                return;
            }
            Trailer::Unavailable => {
                // The sources could not be asked: the press asks again, and
                // the trailer plays (or the answer is said) with the new one.
                self.watch.info.state = TitleState::Idle;
                self.watch.trailer_pending = Some(game.key.clone());
                self.watch_ensure_details(game, cx);
                return;
            }
            Trailer::Unasked => {
                self.watch.trailer_pending = Some(game.key.clone());
                self.watch_ensure_details(game, cx);
                return;
            }
        };
        let title = format!("{} \u{2014} {}", game.title, self.watch_title(game));
        let stop = Arc::new(AtomicBool::new(false));
        self.watch.running = Some(Running { key: game.key.clone(), stop: stop.clone() });
        // mpv resolves the page through yt-dlp before it shows a frame, which
        // takes a moment: say it is opening, or the press looks like it missed.
        self.show_toast(tr!("Opening the trailer…"), false, cx);
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name("watch-trailer".into())
            .spawn(move || {
                let _ = tx.send(heronite_watch::playback::play_trailer(&url, Some(&title), &stop));
            })
            .ok();
        let key = game.key.clone();
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| this.watch_trailer_finished(&key, result, cx)).ok();
        })
        .detach();
    }

    /// The trailer's mpv exited (or never started): the player is free again,
    /// and a failure is said out loud — `mpv` resolves a YouTube page through
    /// yt-dlp, which the bootstrap installs.
    fn watch_trailer_finished(
        &mut self,
        key: &str,
        result: Result<Result<(), anyhow::Error>, futures::channel::oneshot::Canceled>,
        cx: &mut Context<Self>,
    ) {
        if self.watch.running.as_ref().is_some_and(|running| running.key == key) {
            self.watch.running = None;
        }
        if let Ok(Err(error)) = result {
            log::warn!("watch trailer: {error:#}");
            self.show_toast(tr!("Could not play the trailer: {error}", error = format!("{error:#}")), true, cx);
        }
        cx.notify();
    }

    /// Which side the pad drives (◀ ▶ move between them).
    pub(super) fn watch_pane(&self) -> Pane {
        self.watch.pane
    }

    pub(super) fn watch_focus_pane(&mut self, pane: Pane) {
        self.watch.pane = pane;
    }

    /// The card's stream list, to render it and to route its presses.
    pub(super) fn watch_streams_nav(&mut self) -> &mut ListNav {
        &mut self.watch.streams_nav
    }

    /// Opens the page of `key`: a title the page was not on starts with the list
    /// in hand and the first mark focused, so no focus crosses from one page to
    /// the next (a shorter row would otherwise open with the pad outside it).
    pub(super) fn watch_open_page(&mut self, key: &str) {
        if self.watch.page.as_deref() == Some(key) {
            return;
        }
        self.watch.page = Some(key.to_owned());
        // Another title opens on the list's first row, at the first card of any
        // rail, with the details card showing.
        self.watch.rail = 0;
        self.watch.pane = Pane::List;
        self.watch.mark = 0;
    }

    /// The mark of the icon row the pad is on (clamped to `len`).
    pub(super) fn watch_mark_focus(&self, len: usize) -> usize {
        self.watch.mark.min(len.saturating_sub(1))
    }

    /// ◀ ▶ on the icon row: the mark beside it, around the ends.
    pub(super) fn watch_move_mark(&mut self, right: bool, len: usize) {
        if len == 0 {
            self.watch.mark = 0;
            return;
        }
        let at = self.watch_mark_focus(len);
        self.watch.mark = if right { (at + 1) % len } else { (at + len - 1) % len };
    }

    /// The episode the picker shows, when the title is a series with episodes.
    pub(super) fn watch_picked_video(&mut self, game: &CollectionGame) -> Option<String> {
        let details = self.watch_details(&game.key)?;
        if details.episodes.is_empty() {
            return None;
        }
        let selection = self.watch_ensure_selection(&game.key, &details);
        picked_episode(&details, selection).map(|episode| episode.id.clone())
    }

    /// The picked episode's season and number, for the record and the trackers.
    /// `None` for a film, or when the page never had the episode list.
    pub(super) fn watch_picked_episode(&mut self, game: &CollectionGame) -> Option<(u32, u32)> {
        let details = self.watch_details(&game.key)?;
        if details.episodes.is_empty() {
            return None;
        }
        let selection = self.watch_ensure_selection(&game.key, &details);
        picked_episode(&details, selection).map(|episode| (episode.season, episode.number))
    }

    /// The picked episode's id, season and number: what a mark and a tracker
    /// event name, without parsing the provider's own id.
    pub(super) fn watch_picked_episode_ref(&mut self, game: &CollectionGame) -> Option<(String, u32, u32)> {
        let details = self.watch_details(&game.key)?;
        if details.episodes.is_empty() {
            return None;
        }
        let selection = self.watch_ensure_selection(&game.key, &details);
        picked_episode(&details, selection).map(|episode| (episode.id.clone(), episode.season, episode.number))
    }

    /// A on the watched mark: the film, or the picked episode, flips between
    /// watched and not. The local record is written first; the trackers get it
    /// from the outbox, on their own thread.
    pub(super) fn watch_toggle_watched(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        use heronite_watch::progress::{Mark, MarkTarget};
        let provider = game.runner.clone();
        let id = crate::fork::watch::sources::id(game).to_owned();
        let kind = crate::fork::watch::sources::kind(game).to_owned();
        let name = game.title.clone();
        let year = crate::fork::watch::sources::year(game).map(str::to_owned);
        let imdb = id.starts_with("tt").then(|| id.clone());
        let episode = self.watch_picked_episode_ref(game);
        let progress = heronite_watch::progress::local(&self.app_paths, &provider, &id);
        let (target, watched) = match &episode {
            Some((video_id, season, number)) => {
                // The same lookup the page draws the mark from: by the video id,
                // then by season and number, so a record an import keyed the
                // second way is un-marked by the press that showed it watched.
                let watched = progress
                    .as_ref()
                    .and_then(|progress| progress.episode(video_id, *season, *number))
                    .is_some_and(|record| record.finished);
                (MarkTarget::Episode { video_id, season: *season, number: *number }, !watched)
            }
            None => {
                let watched = progress.as_ref().is_some_and(|progress| progress.finished);
                (MarkTarget::Movie, !watched)
            }
        };
        let mark = Mark { provider: &provider, id: &id, imdb: imdb.as_deref(), kind: &kind, name: &name, year: year.as_deref(), target };
        match heronite_watch::progress::set_watched(&self.app_paths, &mark, watched) {
            Ok(()) => {
                // Say it happened: the row's label flips, but a mark a person is
                // only "pretty sure" about gets pressed again.
                let message = if watched { tr!("Marked as watched") } else { tr!("Marked as not watched") };
                self.show_toast(message, false, cx);
                self.watch_drain_trackers(cx);
            }
            Err(error) => {
                log::warn!("marking {id} watched={watched}: {error:#}");
                self.show_toast(format!("{error:#}"), true, cx);
            }
        }
        cx.notify();
    }

    /// Hands the tracker outbox to the trackers, off the UI thread: the mark is
    /// saved here and sent when the network lets it.
    /// Fifteen minutes between attempts while the Watch space is in use. The
    /// import's cursor decides whether the tick calls anything, so this is a clock
    /// rather than a schedule: a screen that redraws a hundred times spends nothing.
    pub(super) fn watch_import_tick(&mut self, cx: &mut Context<Self>) {
        use std::time::{Duration, Instant};
        const EVERY: Duration = Duration::from_secs(15 * 60);
        if !matches!(self.space, crate::collection::Space::Watch) {
            return;
        }
        if self.import_ticked_at.is_some_and(|at| at.elapsed() < EVERY) {
            return;
        }
        self.import_ticked_at = Some(Instant::now());
        self.watch_tick_imports(cx);
    }

    /// Asks the pull to catch up after a session: the cursor's own schedule decides
    /// whether it runs, so this is one request at most, and never on the UI thread.
    fn watch_tick_imports(&mut self, cx: &mut Context<Self>) {
        let app = self.app_paths.clone();
        cx.spawn(async move |this, cx| {
            let ran = cx
                .background_spawn(async move {
                    // Read off the UI thread: this can run from a render, and a file
                    // read there is a frame spent on a disk.
                    if heronite_watch::import::enabled(&app).is_empty() {
                        return Vec::new();
                    }
                    heronite_watch::import::tick(&app)
                })
                .await;
            this.update(cx, |_, cx| {
                if let Some((provider, Err(error))) = ran.first() {
                    log::warn!("import {provider}: {error:#}");
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn watch_drain_trackers(&self, cx: &mut Context<Self>) {
        let app = self.app_paths.clone();
        cx.spawn(async move |_, cx| {
            let drained = cx.background_spawn(async move { heronite_watch::trackers::drain(&app) }).await;
            match drained {
                Ok(report) if report.pending > 0 => log::info!("trackers: {} still waiting for {}", report.pending, "the outbox"),
                Ok(_) => {}
                Err(error) => log::warn!("trackers: {error:#}"),
            }
        })
        .detach();
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

    /// Play / Stop for a title: a film's own best stream, a series' picked
    /// episode's. Also what the desktop card and the shell's Play call.
    pub(super) fn watch_primary(&mut self, game: &CollectionGame, cx: &mut Context<Self>) {
        let video = self.watch_picked_video(game);
        self.watch_play_picked(game, video.as_deref(), cx);
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
        // A torrent needs the local service: start it first when it is down
        // (`watch_service.rs`); the play here runs again once it answers.
        if stream.info_hash.is_some() && !self.watch_service_ready() && !self.watch_service.starting {
            self.show_toast(tr!("Starting the Stremio service…"), false, cx);
            self.watch_service_start(game.key.clone(), index, cx);
            return;
        }

        self.watch.selected = index;
        let stop = Arc::new(AtomicBool::new(false));
        self.watch.running = Some(Running { key: game.key.clone(), stop: stop.clone() });
        self.playing.insert(game.app_name.clone(), true);
        self.refresh_focus();
        cx.notify();

        let app = self.app_paths.clone();
        let episode = self.watch_picked_episode(game);
        let request = heronite_watch::playback::PlaybackRequest {
            provider: game.runner.clone(),
            // The id Watch resolves titles by is an IMDb id (`tt…`): kept with
            // the record so it survives a provider renaming its own.
            imdb: Some(crate::fork::watch::sources::id(game).to_owned()).filter(|id| id.starts_with("tt")),
            id: crate::fork::watch::sources::id(game).to_owned(),
            kind: crate::fork::watch::sources::kind(game).to_owned(),
            video_id: self.watch.video.clone(),
            season: episode.map(|(season, _)| season),
            number: episode.map(|(_, number)| number),
            // What mpv's window and the desktop panels call it: the episode,
            // when the page picked one.
            title: self.watch_title(game),
            // What a tracker records: the film or the show, not the episode label.
            name: game.title.clone(),
            year: crate::fork::watch::sources::year(game).map(str::to_owned),
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
        // The service we started was for this session.
        self.watch_service_finished(cx);
        self.refresh_focus();
        match result {
            // What the session left in the tracker outbox goes out now.
            Ok(Ok(_)) => {
                self.watch_drain_trackers(cx);
                self.watch_tick_imports(cx);
            }
            Ok(Err(error)) => {
                log::warn!("watch playback: {error:#}");
                self.show_toast(tr!("Could not play: {error}", error = format!("{error:#}")), true, cx);
            }
            Err(_) => {}
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

/// Where the pad goes when the card switches to `tab`: the stream list and the
/// two rails take it, while the details open on the episode list (◀ ▶ reaches the
/// card to scroll it).
fn tab_pane(tab: DetailTab) -> Pane {
    match tab {
        DetailTab::Details => Pane::List,
        DetailTab::Streams | DetailTab::Community | DetailTab::Similar => Pane::Card,
    }
}

/// The episode the picker shows: the chosen one of the chosen season.
pub(super) fn picked_episode(details: &TitleDetails, selection: Selection) -> Option<&Episode> {
    let season = details.seasons().get(selection.season).copied()?;
    details.episodes_of(season).nth(selection.episode)
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
    fn the_card_takes_the_pad_for_the_streams_and_the_rails_and_the_list_for_the_details() {
        assert_eq!(tab_pane(DetailTab::Details), Pane::List);
        assert_eq!(tab_pane(DetailTab::Streams), Pane::Card);
        assert_eq!(tab_pane(DetailTab::Community), Pane::Card);
        assert_eq!(tab_pane(DetailTab::Similar), Pane::Card);
        assert_eq!(Pane::default(), Pane::List);
        assert!(DetailTab::Community.is_rail() && DetailTab::Similar.is_rail());
        assert!(!DetailTab::Details.is_rail() && !DetailTab::Streams.is_rail());
    }

    #[test]
    fn the_tab_bar_walks_all_four_cards_and_wraps() {
        let mut tab = DetailTab::Details;
        let mut order = Vec::new();
        for _ in 0..DETAIL_TABS.len() {
            tab = tab.next(true);
            order.push(tab);
        }
        assert_eq!(order, [DetailTab::Streams, DetailTab::Community, DetailTab::Similar, DetailTab::Details]);
        assert_eq!(DetailTab::Details.next(false), DetailTab::Similar, "LB walks back from the first card");
        assert_eq!(DETAIL_TABS, [DetailTab::Details, DetailTab::Streams, DetailTab::Community, DetailTab::Similar]);
    }

    #[test]
    fn a_rail_is_a_grid_walked_by_the_pad() {
        // Seven cards, three to a row: 0 1 2 / 3 4 5 / 6.
        assert_eq!(rail_step(0, 3, 7, Pad::Right), Some(1));
        assert_eq!(rail_step(2, 3, 7, Pad::Right), Some(3), "past the row's end is the next row");
        assert_eq!(rail_step(6, 3, 7, Pad::Right), Some(6), "the last card is the end");
        assert_eq!(rail_step(4, 3, 7, Pad::Left), Some(3));
        assert_eq!(rail_step(0, 3, 7, Pad::Left), None, "the first card's ◀ leaves for the list");
        assert_eq!(rail_step(4, 3, 7, Pad::Up), Some(1), "a row above");
        assert_eq!(rail_step(1, 3, 7, Pad::Up), None, "the top row's ▲ leaves for the list");
        assert_eq!(rail_step(1, 3, 7, Pad::Down), Some(4));
        assert_eq!(rail_step(6, 3, 7, Pad::Down), Some(6), "the short last row stays put");
        assert_eq!(rail_step(4, 3, 7, Pad::A), None, "A is the caller's: it opens the card");
        assert_eq!(rail_step(0, 3, 0, Pad::Right), None, "an empty rail holds nothing");
        assert_eq!(rail_step(9, 3, 7, Pad::Left), Some(5), "a stale focus is clamped to the list");
    }

    #[test]
    fn a_selection_stays_inside_the_seasons_the_source_lists() {
        let details = series();
        assert_eq!(clamp_selection(&details, Selection { season: 9, episode: 9 }), Selection { season: 1, episode: 2 });
        assert_eq!(clamp_selection(&details, Selection { season: 0, episode: 9 }), Selection { season: 0, episode: 1 });
        assert_eq!(clamp_selection(&TitleDetails::default(), Selection { season: 3, episode: 3 }), Selection::default());
    }

    fn ready_with(trailer: Option<&str>) -> TitleState {
        let trailers = trailer.map(|url| heronite_watch::metadata::Trailer { url: url.into(), ..Default::default() }).into_iter().collect();
        TitleState::Ready(Arc::new(TitleDetails { trailers, ..TitleDetails::default() }))
    }

    #[test]
    fn a_trailer_is_known_only_when_the_details_describe_the_title() {
        let ready = ready_with(Some("https://youtu.be/x"));
        assert!(matches!(trailer_of(Some("a"), "a", &ready), Trailer::Found(url) if url == "https://youtu.be/x"));
        assert!(matches!(trailer_of(Some("a"), "b", &ready), Trailer::Unasked), "the details describe another title");
        assert!(matches!(trailer_of(None, "a", &ready), Trailer::Unasked), "a cover never opened the page");
        assert!(matches!(trailer_of(Some("a"), "a", &ready_with(None)), Trailer::NotFound), "a title with no trailer");
        assert!(matches!(trailer_of(Some("a"), "a", &TitleState::Missing), Trailer::NotFound));
        assert!(matches!(trailer_of(Some("a"), "a", &TitleState::Failed), Trailer::Unavailable));
        assert!(matches!(trailer_of(Some("a"), "a", &TitleState::Loading), Trailer::Unasked));
    }
}
