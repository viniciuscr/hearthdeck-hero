//! Movies page (☰ → Movies): films from the linked Stremio account and from
//! Cinemeta. The account link lives here, not in Settings. Playback and the
//! console layout arrive in Phases 4–5 (`docs/MOVIES.md`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::AppPaths;
use heronite_i18n::tr;
use heronite_movies::account::{create_link, read_link, LinkOffer, LinkPoll};
use heronite_movies::addon::AddonClient;
use heronite_movies::datastore::{continue_watching, DatastoreClient, LibraryItem};
use heronite_movies::endpoints::Endpoints;
use heronite_movies::store;
use heronite_movies::types::MovieItem;
use heronite_movies::UreqTransport;

use super::{page_close, sheet, PageCtx, PageEvent};
use crate::services::widgets::{cover_box, Covers};
use crate::theme;
use crate::ui::controls::{button, ButtonKind};

const MAX_W: f32 = 1320.;
const CINEMETA: &str = "https://v3-cinemeta.strem.io/manifest.json";
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
    busy: bool,
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
            busy: false,
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(movie.clone());
                        cx.notify();
                    }))
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
        div().flex().items_start().gap(px(24.)).child(cover_box(src, 220., 330.)).child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(10.))
                .child(div().text_size(px(24.)).font_weight(theme::W_BOLD).child(movie.name.clone()))
                .children((!year.is_empty()).then(|| div().text_size(px(12.5)).text_color(theme::text_muted()).child(year)))
                .children(movie.description.clone().map(|text| div().text_size(px(13.)).text_color(theme::text_primary()).child(text)))
                .child(button("movies-back", ButtonKind::Glass, tr!("Back")).on_click(cx.listener(|this, _, _, cx| {
                    this.selected = None;
                    cx.notify();
                }))),
        )
    }
}

impl Render for MoviesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.covers.drop_evicted(window);
        let column = match self.selected.clone() {
            Some(movie) => self.render_details(&movie, window, cx),
            None => self.render_home(window, cx),
        };
        sheet("page-movies", "", None, None, MAX_W, window, page_close(cx), column).into_any_element()
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

fn movie_item(item: &LibraryItem) -> MovieItem {
    MovieItem { id: item.id.clone(), kind: item.kind.clone(), name: item.name.clone(), poster: item.poster.clone(), ..MovieItem::default() }
}

fn progress_bar(progress: f32) -> Div {
    div()
        .h(px(3.))
        .w_full()
        .rounded_full()
        .bg(theme::ink_a(0.12))
        .child(div().h_full().rounded_full().bg(theme::accent()).w(px(COVER_W * progress.clamp(0., 1.))))
}
