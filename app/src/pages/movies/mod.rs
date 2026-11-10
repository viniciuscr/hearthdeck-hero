//! Movies page (☰ → Movies): films from the linked Stremio account and from
//! Cinemeta, a stream picker from Torrentio, and the handoff to Stremio's
//! player. The console layout is the TV one; playback through mpv is Phase 5.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::AppPaths;
use heronite_i18n::tr;
use heronite_movies::account::{create_link, read_link, LinkOffer, LinkPoll};
use heronite_movies::addon::AddonClient;
use heronite_movies::datastore::{continue_watching, DatastoreClient, LibraryItem};
use heronite_movies::deep_link::{player_deep_link, StreamLink};
use heronite_movies::endpoints::Endpoints;
use heronite_movies::store;
use heronite_movies::types::{MovieItem, StreamChoice};
use heronite_movies::UreqTransport;

use super::{page_close, sheet, PageCtx, PageEvent};
use crate::console::kit::{self, u};
use crate::console::list::{ListNav, Outcome, Row};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::icons::Icon;
use crate::services::widgets::{cover_box, Covers};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};

const MAX_W: f32 = 1320.;
const CINEMETA: &str = "https://v3-cinemeta.strem.io/manifest.json";
const TORRENTIO: &str = "https://torrentio.strem.fun/manifest.json";
const COVER_W: f32 = 140.;
const COVER_H: f32 = 210.;

#[derive(Clone, Default)]
struct Shelf {
    title: String,
    items: Vec<ShelfItem>,
}

#[derive(Clone)]
struct ShelfItem {
    movie: MovieItem,
    /// 0..=1, for Continue Watching.
    progress: Option<f32>,
}

struct Loaded {
    linked: bool,
    shelves: Vec<Shelf>,
}

pub struct MoviesPage {
    ctx: PageCtx,
    covers: Covers,
    linked: bool,
    shelves: Vec<Shelf>,
    loading: bool,
    error: Option<String>,
    notice: Option<String>,
    link: Option<LinkOffer>,
    selected: Option<MovieItem>,
    streams: Vec<StreamChoice>,
    streams_for: Option<String>,
    streams_loading: bool,
    streams_error: Option<String>,
    busy: bool,
    playing: bool,
    console_list: ListNav,
}

impl EventEmitter<PageEvent> for MoviesPage {}

impl MoviesPage {
    pub fn new(ctx: PageCtx, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let covers = Covers::new(&ctx.data_paths, &ctx.app, cx, |this: &mut Self| &mut this.covers);
        let mut page = Self {
            ctx,
            covers,
            linked: false,
            shelves: Vec::new(),
            loading: true,
            error: None,
            notice: None,
            link: None,
            selected: None,
            streams: Vec::new(),
            streams_for: None,
            streams_loading: false,
            streams_error: None,
            busy: false,
            playing: false,
            console_list: ListNav::default(),
        };
        page.reload(cx);
        page
    }

    /// The page went out of sight: free the poster textures (they reload from
    /// the disk cache when it comes back).
    pub fn hidden(&mut self, window: &mut Window) {
        self.covers.release(window);
    }

    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if shown && self.shelves.is_empty() && !self.loading {
            self.reload(cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let app = self.ctx.app.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { load(app) }).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(loaded) => {
                        this.linked = loaded.linked;
                        this.shelves = loaded.shelves;
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_link(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        let link_base = Endpoints::default().link;
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { create_link(&UreqTransport::new(), &link_base) }).await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(offer) => this.link = Some(offer),
                    Err(error) => this.notice = Some(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn check_link(&mut self, cx: &mut Context<Self>) {
        let Some(offer) = self.link.clone() else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        self.notice = None;
        let link_base = Endpoints::default().link;
        let code = offer.code;
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { read_link(&UreqTransport::new(), &link_base, &code) }).await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(LinkPoll::Linked(key)) => match store::write_auth(&this.ctx.app, &key) {
                        Ok(()) => {
                            this.link = None;
                            this.reload(cx);
                            return;
                        }
                        Err(error) => this.notice = Some(format!("{error:#}")),
                    },
                    Ok(LinkPoll::Pending) => this.notice = Some(tr!("Not linked yet").to_string()),
                    Err(error) => this.notice = Some(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn unlink(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = store::clear_auth(&self.ctx.app) {
            self.notice = Some(format!("{error:#}"));
        }
        self.link = None;
        self.linked = false;
        self.reload(cx);
    }

    /// Opens a title: the details, its streams, and the console focus reset.
    fn open_movie(&mut self, movie: MovieItem, cx: &mut Context<Self>) {
        self.selected = Some(movie.clone());
        self.streams.clear();
        self.streams_for = Some(movie.id.clone());
        self.streams_loading = true;
        self.streams_error = None;
        self.console_list.reset();
        cx.spawn(async move |this, cx| {
            let id = movie.id.clone();
            let wanted = id.clone();
            let result = cx.background_spawn(async move { load_streams(&id) }).await;
            this.update(cx, |this, cx| {
                if this.streams_for.as_deref() != Some(wanted.as_str()) {
                    return;
                }
                this.streams_loading = false;
                match result {
                    Ok(mut streams) => {
                        streams.sort_by(|a, b| b.seeders.cmp(&a.seeders).then_with(|| b.size_bytes.cmp(&a.size_bytes)));
                        this.streams = streams;
                    }
                    Err(error) => this.streams_error = Some(format!("{error:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Hands the chosen stream to Stremio's player (`stremio:///player/…`).
    fn handoff_stream(&mut self, choice: &StreamChoice, cx: &mut Context<Self>) {
        let link = if let Some(hash) = &choice.info_hash {
            StreamLink::torrent(hash.clone(), choice.file_idx)
        } else if let Some(url) = &choice.url {
            StreamLink::url(url.clone())
        } else {
            self.notice = Some(tr!("This stream has no playable address").to_string());
            cx.notify();
            return;
        };
        let link = StreamLink { name: choice.name.clone(), description: choice.description.clone(), ..link };
        match player_deep_link(&link).and_then(|uri| heronite_platform::shell::open(uri.as_str()).map_err(anyhow::Error::from)) {
            Ok(()) => self.notice = None,
            Err(error) => self.notice = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    /// Plays in mpv; when native playback is unavailable (or mpv has no
    /// gamepad support), opens the stream in Stremio instead.
    fn play_stream(&mut self, choice: &StreamChoice, cx: &mut Context<Self>) {
        if self.playing {
            return;
        }
        let for_player = choice.clone();
        let for_fallback = choice.clone();
        let movie_id = self.selected.as_ref().map(|movie| movie.id.clone()).unwrap_or_default();
        let app = self.ctx.app.clone();
        self.playing = true;
        self.notice = Some(tr!("Opening the player…").to_string());
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(play_native(&for_player, &movie_id, &app));
        });
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| {
                this.playing = false;
                match result {
                    Ok(Ok(())) => this.notice = None,
                    Ok(Err(error)) => {
                        this.notice = Some(format!("{error:#}"));
                        this.handoff_stream(&for_fallback, cx);
                    }
                    Err(_) => this.notice = Some(tr!("The player stopped").to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The title's own page in Stremio, the verified fallback target.
    fn open_details(&mut self, movie: &MovieItem, cx: &mut Context<Self>) {
        let kind = if movie.kind.is_empty() { "movie" } else { movie.kind.as_str() };
        let uri = format!("stremio:///detail/{kind}/{}", movie.id);
        match heronite_platform::shell::open(uri.as_str()) {
            Ok(()) => self.notice = None,
            Err(error) => self.notice = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    fn render_home(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let mut column = div().flex().flex_col().gap(px(18.)).w_full();
        column = column.child(self.render_account(cx));

        if let Some(notice) = &self.notice {
            column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
        }
        if self.loading && self.shelves.is_empty() {
            column = column.child(div().py(px(24.)).text_color(theme::text_muted()).child(tr!("Loading…")));
        }
        if let Some(error) = &self.error {
            column = column
                .child(div().py(px(8.)).text_color(theme::danger_text()).child(tr!("Could not load the movies")))
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(error.clone()));
        }
        if !self.loading && self.error.is_none() && self.shelves.iter().all(|shelf| shelf.items.is_empty()) {
            column = column.child(div().py(px(24.)).text_color(theme::text_muted()).child(tr!("No movies to show yet.")));
        }

        let shelves = self.shelves.clone();
        for (index, shelf) in shelves.iter().enumerate() {
            if shelf.items.is_empty() {
                continue;
            }
            column = column.child(self.render_shelf(index, shelf, window, cx));
        }
        column
    }

    fn render_account(&self, cx: &mut Context<Self>) -> Div {
        let status = if self.linked { tr!("Linked") } else { tr!("Not linked") };
        let mut actions = div().flex().items_center().gap(px(8.));
        if self.linked {
            actions = actions
                .child(button("movies-unlink", ButtonKind::Ghost, tr!("Unlink")).on_click(cx.listener(|this, _, _, cx| this.unlink(cx))));
        } else if self.link.is_some() {
            actions = actions.child(
                button("movies-check", ButtonKind::Primary, tr!("Check")).on_click(cx.listener(|this, _, _, cx| this.check_link(cx))),
            );
        } else {
            actions = actions.child(
                button("movies-link", ButtonKind::Primary, tr!("Link with a code"))
                    .on_click(cx.listener(|this, _, _, cx| this.start_link(cx))),
            );
        }
        let mut card = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .p(px(14.))
            .rounded(px(12.))
            .bg(theme::ink_a(0.04))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(13.5)).child(status))
                    .child(div().text_size(px(11.5)).text_color(theme::text_muted()).child(tr!("Stremio account"))),
            )
            .child(actions);

        if let Some(offer) = &self.link {
            let mut detail = div().flex().items_center().gap(px(16.)).p(px(14.)).rounded(px(12.)).bg(theme::ink_a(0.03));
            if let Some(qr) = crate::ui::qr::qr_code(&offer.link, 4.0) {
                detail = detail.child(qr);
            }
            detail = detail.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(13.)).child(tr!("Code {code}", code = offer.code.clone())))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(theme::text_muted())
                            .child(tr!("Scan the code with the Stremio app, or open {link}", link = offer.link.clone())),
                    ),
            );
            card = div().flex().flex_col().gap(px(10.)).child(card).child(detail);
        }
        card
    }

    fn render_shelf(&mut self, index: usize, shelf: &Shelf, window: &Window, cx: &mut Context<Self>) -> Div {
        let scale = window.scale_factor();
        let mut cards = div().flex().flex_wrap().gap(px(12.)).pb(px(6.));
        for (position, item) in shelf.items.iter().enumerate() {
            let src = self.covers.get(item.movie.poster.as_deref(), COVER_W, COVER_H, scale);
            let movie = item.movie.clone();
            let id = format!("movie-{index}-{position}");
            cards = cards.child(
                div()
                    .id(id)
                    .flex_none()
                    .w(px(COVER_W))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.open_movie(movie.clone(), cx)))
                    .child(cover_box(src, COVER_W, COVER_H))
                    .child(div().text_size(px(12.5)).truncate().child(item.movie.name.clone()))
                    .children(item.progress.map(progress_bar)),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(16.)).font_weight(theme::W_BOLD).child(shelf.title.clone()))
            .child(cards)
    }

    fn render_details(&mut self, movie: &MovieItem, window: &Window, cx: &mut Context<Self>) -> Div {
        let src = self.covers.get(movie.poster.as_deref(), 220., 330., window.scale_factor());
        let year = movie.year.clone().unwrap_or_default();
        let for_details = movie.clone();
        let mut column = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(10.))
            .child(div().text_size(px(24.)).font_weight(theme::W_BOLD).child(movie.name.clone()))
            .children((!year.is_empty()).then(|| div().text_size(px(12.5)).text_color(theme::text_muted()).child(year)))
            .children(movie.description.clone().map(|text| div().text_size(px(13.)).text_color(theme::text_primary()).child(text)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        button("movies-open-stremio", ButtonKind::Glass, tr!("Open in Stremio"))
                            .on_click(cx.listener(move |this, _, _, cx| this.open_details(&for_details, cx))),
                    )
                    .child(button("movies-back", ButtonKind::Ghost, tr!("Back")).on_click(cx.listener(|this, _, _, cx| {
                        this.selected = None;
                        this.console_list.reset();
                        cx.notify();
                    }))),
            )
            .child(div().text_size(px(16.)).font_weight(theme::W_BOLD).child(tr!("Streams")));

        if self.playing {
            column = column.child(div().text_size(px(12.5)).text_color(theme::accent()).child(tr!("Playing in mpv…")));
        }

        if self.streams_loading {
            column = column.child(div().text_color(theme::text_muted()).child(tr!("Loading…")));
        } else if let Some(error) = &self.streams_error {
            column = column
                .child(div().text_color(theme::danger_text()).child(tr!("Could not load the streams")))
                .child(div().text_size(px(11.5)).text_color(theme::text_muted()).child(error.clone()));
        } else if self.streams.is_empty() {
            column = column.child(div().text_color(theme::text_muted()).child(tr!("No streams found")));
        } else {
            for (index, choice) in self.streams.iter().enumerate() {
                let choice = choice.clone();
                let name = choice.name.clone().unwrap_or_else(|| choice.addon.clone());
                let description = choice.description.clone().unwrap_or_default();
                let meta = stream_meta(&choice);
                column = column.child(
                    div()
                        .id(format!("stream-{index}"))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .p(px(10.))
                        .rounded(px(10.))
                        .bg(theme::ink_a(0.03))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::ink_a(0.06)))
                        .on_click(cx.listener(move |this, _, _, cx| this.play_stream(&choice, cx)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_size(px(13.)).truncate().child(name))
                                .child(div().text_size(px(11.5)).text_color(theme::text_muted()).truncate().child(description)),
                        )
                        .child(div().text_size(px(11.5)).text_color(theme::text_muted()).child(meta)),
                );
            }
        }

        div().flex().items_start().gap(px(24.)).child(cover_box(src, 220., 330.)).child(column)
    }

    fn console_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(movie) = &self.selected {
            rows.push(Row::header(movie.name.clone()));
            rows.push(Row::action("details", tr!("Open in Stremio")).icon(Icon::OpenInNew));
            rows.push(Row::header(tr!("Streams")));
            if self.streams_loading {
                rows.push(Row::info("loading", tr!("Loading…"), ""));
            } else if let Some(error) = &self.streams_error {
                rows.push(Row::info("error", tr!("Could not load the streams"), error.clone()));
            } else if self.streams.is_empty() {
                rows.push(Row::info("none", tr!("No streams found"), ""));
            } else {
                for (index, choice) in self.streams.iter().enumerate() {
                    rows.push(
                        Row::action(format!("stream:{index}"), choice.name.clone().unwrap_or_else(|| choice.addon.clone()))
                            .desc(choice.description.clone().unwrap_or_default())
                            .icon(Icon::PlayArrow),
                    );
                }
            }
            rows.push(Row::action("back", tr!("Back")).icon(Icon::ChevronLeft));
        } else {
            if !self.linked {
                rows.push(Row::action("link", tr!("Link with a code")).icon(Icon::Add));
            }
            for (shelf_index, shelf) in self.shelves.iter().enumerate() {
                if shelf.items.is_empty() {
                    continue;
                }
                rows.push(Row::header(shelf.title.clone()));
                for (item_index, item) in shelf.items.iter().enumerate() {
                    rows.push(
                        Row::action(format!("movie:{shelf_index}:{item_index}"), item.movie.name.clone())
                            .desc(item.movie.year.clone().unwrap_or_default())
                            .icon(Icon::PlayArrow),
                    );
                }
            }
        }
        rows
    }

    pub(super) fn render_console(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let rows = self.console_rows();
        let mut column = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .child(div().flex().flex_none().items_center().h(u(76.)).px(u(46.)).child(kit::title(tr!("Movies"), 30.)));
        if let Some(offer) = &self.link {
            let mut qr = div().flex().items_center().gap(u(18.)).px(u(46.)).pb(u(12.));
            if let Some(code) = crate::ui::qr::qr_code(&offer.link, 6.0) {
                qr = qr.child(code);
            }
            qr = qr.child(
                div().flex().flex_col().gap(u(6.)).child(kit::title(tr!("Code {code}", code = offer.code.clone()), 22.)).child(
                    div()
                        .text_size(u(14.))
                        .text_color(theme::text_muted())
                        .child(tr!("Scan the code with the Stremio app, or open {link}", link = offer.link.clone())),
                ),
            );
            column = column.child(qr);
        }
        column =
            column.child(div().flex_1().min_h_0().px(u(46.)).pb(u(16.)).child(self.console_list.render("console-movies", &rows, true)));
        column.into_any_element()
    }
}

impl Render for MoviesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if crate::console::active() {
            return self.render_console(window, cx);
        }
        self.covers.drop_evicted(window);
        let column = match self.selected.clone() {
            Some(movie) => self.render_details(&movie, window, cx),
            None => self.render_home(window, cx),
        };
        sheet("page-movies", "", None, None, MAX_W, window, page_close(cx), column).into_any_element()
    }
}

impl PadTarget for MoviesPage {
    fn on_pad(&mut self, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if pad == Pad::B {
            if self.selected.is_some() {
                self.selected = None;
                self.console_list.reset();
                cx.notify();
                return true;
            }
            return false;
        }
        let rows = self.console_rows();
        match self.console_list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                let id = id.to_string();
                if id == "back" {
                    self.selected = None;
                    self.console_list.reset();
                } else if id == "details" {
                    if let Some(movie) = self.selected.clone() {
                        self.open_details(&movie, cx);
                    }
                } else if id == "link" {
                    self.start_link(cx);
                } else if let Some((shelf, item)) = parse_pair(&id, "movie:") {
                    if let Some(movie) = self.shelves.get(shelf).and_then(|s| s.items.get(item)).map(|item| item.movie.clone()) {
                        self.open_movie(movie, cx);
                    }
                } else if let Some(index) = id.strip_prefix("stream:").and_then(|n| n.parse::<usize>().ok()) {
                    if let Some(choice) = self.streams.get(index).cloned() {
                        self.play_stream(&choice, cx);
                    }
                }
                cx.notify();
                true
            }
            Outcome::Ignored => false,
            _ => {
                cx.notify();
                true
            }
        }
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![hint(Btn::A, tr!("Select")), hint(Btn::B, tr!("Back"))]
    }
}

fn load(app: AppPaths) -> anyhow::Result<Loaded> {
    let http = UreqTransport::new();
    let endpoints = Endpoints::default();
    let key = store::read_auth(&app)?;
    let linked = key.is_some();
    let mut shelves = Vec::new();

    if let Some(key) = key {
        let client = DatastoreClient::new(&http, &endpoints, key);
        let items = client.get_all()?;
        let items: Vec<ShelfItem> = continue_watching(&items)
            .into_iter()
            .map(|item| ShelfItem {
                movie: movie_item(item),
                progress: (item.state.duration > 0).then(|| (item.state.time_offset as f32 / item.state.duration as f32).clamp(0., 1.)),
            })
            .collect();
        if !items.is_empty() {
            shelves.push(Shelf { title: tr!("Continue watching").to_string(), items });
        }
    }

    let mut addon = AddonClient::new(&http, CINEMETA)?;
    addon.manifest()?;
    let items = addon.catalog("movie", "top", &[])?.into_iter().map(|movie| ShelfItem { movie, progress: None }).collect();
    shelves.push(Shelf { title: tr!("Popular").to_string(), items });

    Ok(Loaded { linked, shelves })
}

fn load_streams(id: &str) -> anyhow::Result<Vec<StreamChoice>> {
    let http = UreqTransport::new();
    let mut addon = AddonClient::new(&http, TORRENTIO)?;
    addon.manifest()?;
    addon.stream("movie", id)
}

fn movie_item(item: &LibraryItem) -> MovieItem {
    MovieItem { id: item.id.clone(), kind: item.kind.clone(), name: item.name.clone(), poster: item.poster.clone(), ..MovieItem::default() }
}

#[cfg(unix)]
fn play_native(choice: &StreamChoice, movie_id: &str, app: &AppPaths) -> anyhow::Result<()> {
    use heronite_movies::player::{find_mpv, gamepad_supported, MpvSession};

    let mpv = find_mpv().ok_or_else(|| anyhow::anyhow!("mpv is not installed"))?;
    if !gamepad_supported(&mpv) {
        anyhow::bail!("this mpv has no gamepad support");
    }
    let url = resolve_stream(choice)?;
    let start = local_position(app, movie_id);
    let mut session = MpvSession::start(&mpv, &url, start, true)?;
    let outcome = session.wait()?;
    record_progress(app, movie_id, &outcome);
    Ok(())
}

#[cfg(not(unix))]
fn play_native(_: &StreamChoice, _: &str, _: &AppPaths) -> anyhow::Result<()> {
    anyhow::bail!("native playback is Linux-only")
}

#[cfg(unix)]
fn resolve_stream(choice: &StreamChoice) -> anyhow::Result<String> {
    if let Some(url) = &choice.url {
        return Ok(url.clone());
    }
    let hash = choice.info_hash.as_deref().ok_or_else(|| anyhow::anyhow!("the stream has no address"))?;
    let http = UreqTransport::new();
    let server = heronite_movies::streaming::StreamingServer::new(&http, Endpoints::default().streaming_server)?;
    let base = server.base_url()?;
    server.create(hash, &[])?;
    let url = heronite_movies::streaming::StreamingServer::stream_url(&base, hash, choice.file_idx)?;
    Ok(url.to_string())
}

#[cfg(unix)]
fn local_position(app: &AppPaths, movie_id: &str) -> Option<u64> {
    let movies = store::read_movies(app);
    store::progress_for(&movies, movie_id).map(|progress| progress.position_ms).filter(|position| *position > 0)
}

#[cfg(unix)]
fn record_progress(app: &AppPaths, movie_id: &str, outcome: &heronite_movies::player::PlaybackOutcome) {
    if outcome.position_ms == 0 && outcome.duration_ms == 0 {
        return;
    }
    let Ok(Some(key)) = store::read_auth(app) else { return };
    let http = UreqTransport::new();
    let endpoints = Endpoints::default();
    let client = DatastoreClient::new(&http, &endpoints, key);
    let mut movies = store::read_movies(app);
    let progress = heronite_movies::types::MovieProgress {
        id: movie_id.to_owned(),
        position_ms: outcome.position_ms,
        duration_ms: outcome.duration_ms,
        updated_at: chrono::Utc::now(),
        finished: outcome.duration_ms > 0 && outcome.position_ms * 100 >= outcome.duration_ms * 70,
        video_id: None,
    };
    if let Err(error) = heronite_movies::progress::record(&client, app, &mut movies, progress) {
        log::warn!("saving the playback position: {error:#}");
    }
}

fn progress_bar(progress: f32) -> Div {
    div()
        .h(px(3.))
        .w_full()
        .rounded_full()
        .bg(theme::ink_a(0.12))
        .child(div().h_full().rounded_full().bg(theme::accent()).w(px(COVER_W * progress.clamp(0., 1.))))
}

fn stream_meta(choice: &StreamChoice) -> String {
    let mut parts = Vec::new();
    if let Some(seeders) = choice.seeders {
        parts.push(format!("👤 {seeders}"));
    }
    if let Some(size) = choice.size_bytes {
        parts.push(size_text(size));
    }
    parts.join(" \u{b7} ")
}

fn size_text(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else {
        format!("{:.0} MiB", bytes / MIB)
    }
}

fn parse_pair(id: &str, prefix: &str) -> Option<(usize, usize)> {
    let rest = id.strip_prefix(prefix)?;
    let (shelf, item) = rest.split_once(':')?;
    Some((shelf.parse().ok()?, item.parse().ok()?))
}
