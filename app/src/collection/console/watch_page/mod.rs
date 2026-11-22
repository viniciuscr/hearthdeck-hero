//! The film page in console mode: our own page for Watch titles.
//!
//! Fork-owned (F-012). It is composed from the house pieces — `console::kit`,
//! the console list, the Collection cover loader, the focus card surface and
//! the games' own facts table — so it looks and behaves like the rest of the
//! console. The games' page in `console/game/` stays upstream's: it gains one
//! dispatch line and nothing else, so its improvements keep coming.
//!
//! What it shows is what a film is, never what a game is: no installed state,
//! playtime, status, folders, screenshots or HowLongToBeat. The description
//! and the details come from the metadata sources (`heronite_watch::metadata`:
//! Cinemeta today), while Play and its rows are the provider's streams.
//!
//! A series gets a picker in the same list: a season row moved with ◀ ▶, one row
//! per episode of that season, and the picked episode's streams under them. A
//! on an episode plays its best stream at once; the card previews what the
//! picker is on.
//!
//! The title, its marks (watched, watch later, favourite) and the list keep the
//! left column; the details panel takes the right one **from the page's top edge
//! to its bottom**, so the facts fit with almost no scrolling — it is taller
//! than a panel set under the title ever was, and nothing else about the page
//! moved. The marks are icons, not rows: ◀ ▶ move along them, A presses one, and
//! R3 (the right stick's click) adds the title to the watchlist from anywhere on
//! the page.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::text::DateLocale;
use heronite_data::CollectionGame;
use heronite_i18n::tr;
use heronite_images::{ImageRef, Variant};
use heronite_watch::metadata::{Rail, Related, TitleDetails};
use heronite_watch::progress::WatchedState;

mod engine;
use engine::{focused_episode, mark_at, mark_route, marks_len, rail_press, rail_route};

mod sections;
use sections::details::{detail_lines, facts_line, title_status, watched_line};
use sections::episode::{episode_card, episode_row};

mod style;

use super::super::watch_personal::Flag;
use super::super::watch_playback::{picked_episode, DetailTab, Pane, Selection, DETAIL_TABS};
use super::super::CollectionView;
use super::game::facts_table;
use super::hero::{card_surface, fitted_title, pill_button, PillSize};
use crate::collection::focus_left::STAR;
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::motion;
use crate::console::pointer::Clickable;
use crate::console::{hint, Btn, Hint, Pad};
use crate::icons::Icon;
use crate::theme;

/// The page's own margins, design px: the console screens' 46 at the sides, 20
/// under everything, and 40 between the title's column and the panel.
const PAGE_PAD: f32 = 46.;
const BOTTOM_PAD: f32 = 20.;
const COLUMN_GAP: f32 = 40.;
/// The column the title, its marks and its list take. The details panel takes
/// the rest of the page, from its top edge to its bottom.
const LEFT_W: f32 = 420.;
/// The list card's width in Cinema, design px.
const WIDE_LIST_W: f32 = 460.;
/// The details panel's corners and its own padding, design px.
const PANEL_RADIUS: f32 = 20.;
const PANEL_PAD: f32 = 26.;
/// The gaps between two of a rail's tiles, side by side and row under row. The
/// tile itself is the theme's own (`console_tile_size`), never a size of this
/// page's.
const RAIL_GAP: f32 = 16.;
const RAIL_ROW_GAP: f32 = 18.;

/// One label / value line of the page's card.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DetailLine {
    label: SharedString,
    value: String,
}

/// One of the page's three marks: the person's two shelf flags and the watched
/// mark of the film (or of the episode the card shows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Watched,
    Watchlist,
    Favourite,
}

impl Mark {
    /// The icon it wears: the shelf flags fill theirs while the mark is on and
    /// outline it while off; the watched mark is a tick either way, and its
    /// tint (green while on, muted while off) is what says where it stands.
    fn icon(self, on: bool) -> Icon {
        match self {
            Self::Watched => Icon::Check,
            Self::Watchlist => Flag::Watchlist.icon(on),
            Self::Favourite => Flag::Favourite.icon(on),
        }
    }

    /// The colour of a mark that is on: a tick is the good green, a favourite
    /// the games' star, the watchlist the console's accent.
    fn tint(self, on: bool) -> Option<Rgba> {
        match (self, on) {
            (Self::Watched, true) => Some(theme::success()),
            (Self::Favourite, true) => Some(rgb(STAR)),
            (Self::Watchlist, true) => Some(theme::accent()),
            _ => None,
        }
    }

    /// Which mark it is: its place in the row, and its element id.
    fn rank(self) -> usize {
        match self {
            Self::Watched => 0,
            Self::Watchlist => 1,
            Self::Favourite => 2,
        }
    }

    /// The flag the mark writes, for the two that are shelf flags.
    fn flag(self) -> Option<Flag> {
        match self {
            Self::Watchlist => Some(Flag::Watchlist),
            Self::Favourite => Some(Flag::Favourite),
            Self::Watched => None,
        }
    }
}

/// The episode the card shows, named the way a mark and a tracker need it
/// (never by parsing the provider's id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Picked<'a> {
    id: &'a str,
    season: u32,
    number: u32,
}

/// A mark as the page draws it: where it stands and what a press does (the
/// label is the state itself, §7).
#[derive(Clone, Debug, PartialEq, Eq)]
struct MarkState {
    mark: Mark,
    on: bool,
    label: SharedString,
}

/// Whether the title has any mark to draw. An imported title has none — there is
/// no local record to mark — and every other title has at least its two flags.
fn has_marks(game: &CollectionGame) -> bool {
    !crate::fork::watch::sources::imported(game)
}

/// Whether the row draws the watched mark: a film always has one (marked or
/// not), a series once the page knows its episodes — a film's mark on a show is
/// not something a tracker can record.
fn shows_watched(kind: &str, episodes_known: bool) -> bool {
    kind != "series" || episodes_known
}

/// Where the watched mark stands: the film's own record, or the picked
/// episode's. `None` when the page has nothing to mark at all (a series whose
/// episodes it does not know). **No record is a mark that is off**, not a mark
/// that is missing: that is what a press writes.
fn watched_state(progress: Option<&heronite_watch::progress::MovieProgress>, picked: Option<Picked<'_>>, kind: &str) -> Option<bool> {
    match picked {
        Some(picked) => Some(
            progress.and_then(|progress| progress.episode(picked.id, picked.season, picked.number)).is_some_and(|record| record.finished),
        ),
        None if kind != "series" => Some(progress.is_some_and(|progress| progress.finished)),
        None => None,
    }
}

/// Why a title has no mark: it is in no source's catalogue here, so the page has
/// no local record to write (one message for the row and for the R3 press).
fn not_present() -> &'static str {
    tr!("Not currently present in any available stream source.")
}

/// The watched mark's label: the state itself, and which episode it is about
/// when the card shows one — the card follows the list, and "Mark as watched"
/// alone would not say which episode.
fn watched_mark_label(on: bool, episode: Option<&str>) -> SharedString {
    match (episode, on) {
        (Some(episode), true) => tr!("Mark {episode} as not watched", episode = episode).into(),
        (Some(episode), false) => tr!("Mark {episode} as watched", episode = episode).into(),
        (None, true) => tr!("Mark as not watched").into(),
        (None, false) => tr!("Mark as watched").into(),
    }
}

/// One far side of the page's two rails, as the tab that shows it.
fn rail_of(tab: DetailTab) -> Rail {
    match tab {
        DetailTab::Community => Rail::Community,
        _ => Rail::Similar,
    }
}

/// The room the details panel really has inside it, design px: the page's
/// width after its own margins, the title's column and the panel's padding.
/// That width is the body's, never the window's: a theme whose menu is a Rail
/// keeps it on screen and insets the page by it (`layouts::body_width`), so a
/// panel measured from the window would claim room it does not have.
fn panel_room(window: &Window) -> f32 {
    super::layouts::body_width(window) - 2. * PAGE_PAD - LEFT_W - COLUMN_GAP - 2. * PANEL_PAD
}

/// How many of the theme's tiles fit a row of `room` design px: as many as
/// the row of `tile_w`-wide tiles, `RAIL_GAP` apart, holds.
fn columns_in(room: f32, tile_w: f32) -> usize {
    let fit = ((room + RAIL_GAP) / (tile_w + RAIL_GAP)).floor();
    (fit as usize).max(1)
}

/// How many of the theme's tiles a rail's row holds: as many as fit the
/// panel's real room (`panel_room`). The tile's width is the theme's
/// (`console_tile_size`), so a square or wide tile makes fewer columns. Never
/// fewer than one and never so many that a row runs out of the panel — the
/// grid scrolls down, not across.
fn rail_columns(window: &Window, tile_w: f32) -> usize {
    columns_in(panel_room(window), tile_w)
}

/// The width a rail's full row takes, design px: `columns` tiles with the gap
/// between them. A shorter last row keeps it too, so its cards stay in their
/// columns, and the whole block is centred in the panel.
fn rail_line_width(columns: usize, tile_w: f32) -> f32 {
    columns as f32 * tile_w + columns.saturating_sub(1) as f32 * RAIL_GAP
}

impl CollectionView {
    /// The console page of a film or a series. The title, its marks and its list
    /// keep the left column; the details panel beside them runs from the page's
    /// top edge to its bottom — the room the header band used to sit in.
    /// The page in the space's arrangement. Stage and Stage Wide today; a new
    /// style adds an arm here, and the engine and the sections do not change
    /// (`style.rs`, `docs/WATCH-PAGE.md` §6).
    pub(super) fn render_watch_game_page(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let layout = style::current(&self.app_paths);
        self.render_watch_page(layout, key, window, cx)
    }

    fn render_watch_page(
        &mut self,
        layout: crate::fork::watch::page_config::PageStyle,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(g) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else {
            return kit::empty(Icon::Warning, tr!("This title is gone"), tr!("Press B to go back.")).into_any_element();
        };
        self.console_focus_game(Some(key.clone()), cx);
        // Another title opens on the list, with the first mark focused: no focus
        // crosses from one page to the next.
        self.watch_open_page(key);
        self.watch_ensure_details(&g, cx);
        // A page can open on a rail (the card survives a page change), so the
        // titles a rail draws are asked for here and not only when the card is
        // switched to. The ask is latched: a render that arrives while it is in
        // flight does nothing.
        if self.watch_tab().is_rail() {
            self.watch_ensure_rails(cx);
        }

        // Everything the page draws is taken before the view is borrowed mutably
        // (the list, the cover and the scroll driver all need `&mut self`).
        let details = self.watch_details(key);
        let failed = self.watch_details_failed(key);
        // A series opens on the episode the stored progress points at.
        let selection =
            details.as_deref().filter(|details| !details.episodes.is_empty()).map(|details| self.watch_ensure_selection(key, details));
        let locale = self.lib.as_ref().map(|lib| lib.date_locale).unwrap_or_default();
        let origins = crate::fork::watch::sources::origins_with(&g, self.watch_streams_known(&g));
        let facts = facts_line(&origins, details.as_deref(), crate::fork::watch::sources::year(&g));
        let progress = heronite_watch::progress::local(&self.app_paths, &g.runner, crate::fork::watch::sources::id(&g));
        let status = title_status(progress.as_ref(), crate::fork::watch::sources::kind(&g), details.as_deref());
        let watched = watched_line(g.playtime_minutes, g.last_played.as_deref(), status, locale);
        let lines = detail_lines(details.as_deref(), &g.details.genres, locale);
        let overview = details
            .as_deref()
            .and_then(|details| details.description_or(Some(g.details.description.as_str())))
            .map(str::to_owned)
            .filter(|text| !text.trim().is_empty());

        let pane = self.watch_pane();
        let rows = self.watch_rows(&g, details.as_deref(), locale);
        let list = self.console.game.list.render("console-game-actions", &rows, pane == Pane::List);
        // After `render`, which clamps the focus to a focusable row: the row the
        // pad is on is the one the card follows.
        let list_focus = self.console.game.list.focus;
        // The card follows the list: the row the pad is on decides the episode.
        if let Some(selection) = selection {
            if let Some(episode) = focused_episode(&rows, list_focus) {
                self.watch_pick_episode(&g.key, Selection { season: selection.season, episode });
            }
        }
        let selection = details.as_deref().filter(|details| !details.episodes.is_empty()).map(|d| self.watch_ensure_selection(key, d));
        let picked = match (details.as_deref(), selection) {
            (Some(details), Some(selection)) => picked_episode(details, selection).cloned(),
            _ => None,
        };
        let header = self.watch_header(key, &g, &facts, watched, &*window);

        let marks = self.watch_marks(&g, progress.as_ref(), picked.as_ref());
        let mark_focus = self.watch_mark_focus(marks.len());
        // What the focused mark is, beside the icons: the label the tooltip shows,
        // drawn while the pad is on the row so a person does not have to press to
        // find out. It disappears with the focus and the icons stay put.
        let marks_label =
            (pane == Pane::Marks).then(|| marks.get(mark_focus)).flatten().map(|state| (state.mark.rank(), state.label.clone()));

        let s = kit::scale();
        let still = picked.as_ref().and_then(|episode| episode.thumbnail.as_deref()).and_then(|url| self.episode_still(url, 420., 236., s));
        let columns = if card_is_wide(window, s) { 2 } else { 1 };

        // The marks: icons, not rows. ◀ ▶ move along them, A presses one; ↑ from
        // the first row of the list arrives here, ↓ (or B) goes back.
        let marks_row = div()
            .flex()
            .items_center()
            .gap(u(10.))
            .children(marks.into_iter().enumerate().map(|(index, state)| {
                let focused = pane == Pane::Marks && index == mark_focus;
                pill_button(focused, state.mark.icon(state.on), None, state.mark.tint(state.on), PillSize::MARK)
                    .id(SharedString::from(format!("watch-mark-{}", state.mark.rank())))
                    .tooltip(crate::ui::tooltip(state.label))
                    .on_pads(mark_route(pane, list_focus, mark_focus, index))
            }))
            .children(marks_label.map(|(rank, label)| {
                motion::fade_in(
                    SharedString::from(format!("watch-mark-label-{rank}")),
                    div().min_w_0().truncate().text_size(u(14.)).text_color(theme::text_muted()).child(label),
                    160,
                )
            }));

        // The card's two sides: what the title or the picked episode is, and the
        // streams it can be played from. The trailer is not a card: it is a press
        // of L3 (the left stick's click), offered in the hint bar while there is
        // one.
        let tab = self.watch_tab();
        let tabs: Vec<SharedString> = DETAIL_TABS
            .iter()
            .map(|tab| {
                SharedString::from(match tab {
                    DetailTab::Details => tr!("Details"),
                    DetailTab::Streams => tr!("Streams"),
                    DetailTab::Community => tr!("Community"),
                    DetailTab::Similar => tr!("Similar"),
                })
            })
            .collect();
        let active = DETAIL_TABS.iter().position(|known| *known == tab).unwrap_or(0);
        let body = match tab {
            DetailTab::Details => {
                let mut about = div()
                    .id("console-watch-about")
                    .flex()
                    .flex_col()
                    .gap(u(20.))
                    .overflow_y_scroll()
                    .track_scroll(self.console.game.about.handle());
                if let Some(episode) = picked.as_ref() {
                    about = about.child(episode_card(episode, still, locale));
                }
                if let Some(overview) = overview {
                    about = about.child(
                        kit::body_text(overview).max_w(u(980.)).text_size(u(16.)).line_height(u(25.)).text_color(theme::text_primary()),
                    );
                }
                if !lines.is_empty() {
                    let table: Vec<(SharedString, String)> = lines.iter().map(|line| (line.label.clone(), line.value.clone())).collect();
                    about = about.child(div().h(px(1.)).flex_none().bg(kit::ink(0.08))).child(facts_table(&table, columns));
                }
                if failed {
                    about = about.child(div().text_size(u(14.)).text_color(theme::text_dim()).child(tr!("Details unavailable right now.")));
                }
                about.into_any_element()
            }
            DetailTab::Streams => {
                let video = picked.as_ref().map(|episode| episode.id.clone());
                self.watch_ensure_streams(&g, video.as_deref(), cx);
                let rows = self.watch_stream_rows(&g, video.as_deref());
                let focused = pane == Pane::Card;
                let list = self.watch_streams_nav().render("console-watch-streams", &rows, focused);
                list.flex_1().min_h_0().into_any_element()
            }
            DetailTab::Community => self.watch_rail(Rail::Community, details.as_deref(), window),
            DetailTab::Similar => self.watch_rail(Rail::Similar, details.as_deref(), window),
        };

        // Each arrangement assembles the same pieces its own way: the title and
        // its marks, the list, and the details. Only one arm runs, so the pieces
        // are moved there and nowhere else.
        let page = match layout {
            // Stage: the title, its marks and its list lead; the details read
            // beside them.
            crate::fork::watch::page_config::PageStyle::Stage => div()
                .relative()
                .size_full()
                .flex()
                .gap(u(COLUMN_GAP))
                .px(u(PAGE_PAD))
                .pb(u(BOTTOM_PAD))
                .child(
                    div()
                        .flex_none()
                        .w(u(LEFT_W))
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .gap(u(14.))
                        .child(header)
                        .child(marks_row)
                        // The list is the scrolling element itself: `flex_1` +
                        // `min_h_0` on the row that owns the scroll, never on a
                        // wrapper, or the scroll area is sized by its content.
                        .child(list.flex_1().min_h_0()),
                )
                .child(
                    card_surface()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .p(u(PANEL_PAD))
                        .rounded(u(PANEL_RADIUS))
                        .shadow(kit::shadow(0.5, 12., 44.))
                        .flex()
                        .flex_col()
                        .gap(u(16.))
                        .when(pane == Pane::Card, |card| card.border_color(theme::console_focus()))
                        .child(kit::tabs(&tabs, active))
                        .child(div().flex_1().min_h_0().flex().flex_col().child(body)),
                ),
            // Stage Wide: a title band across the top, then two full-width cards —
            // the details and the list side by side under it.
            crate::fork::watch::page_config::PageStyle::StageWide => div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .gap(u(18.))
                .px(u(PAGE_PAD))
                .pb(u(BOTTOM_PAD))
                .child(div().flex().items_end().gap(u(28.)).child(header).child(marks_row))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .gap(u(COLUMN_GAP))
                        .child(
                            card_surface()
                                .w(u(WIDE_LIST_W))
                                .min_h_0()
                                .p(u(18.))
                                .rounded(u(PANEL_RADIUS))
                                .shadow(kit::shadow(0.5, 12., 44.))
                                .flex()
                                .flex_col()
                                .child(list.flex_1().min_h_0()),
                        )
                        .child(
                            card_surface()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .p(u(PANEL_PAD))
                                .rounded(u(PANEL_RADIUS))
                                .shadow(kit::shadow(0.5, 12., 44.))
                                .flex()
                                .flex_col()
                                .gap(u(16.))
                                .when(pane == Pane::Card, |card| card.border_color(theme::console_focus()))
                                .child(kit::tabs(&tabs, active))
                                .child(div().flex_1().min_h_0().flex().flex_col().child(body)),
                        ),
                ),
        };
        page.child(self.console.game.about.driver()).into_any_element()
    }

    /// One of the page's two rails: the cards a source's list holds, with the pad
    /// on one of them. An empty list says so instead of drawing nothing.
    fn watch_rail(&mut self, rail: Rail, details: Option<&TitleDetails>, window: &Window) -> AnyElement {
        let items = details.map_or(&[][..], |details| details.rail(rail));
        if items.is_empty() {
            let (title, hint) = match rail {
                Rail::Similar => (tr!("Nothing similar yet"), tr!("Simkl knows nothing alike for this title.")),
                Rail::Community => (tr!("No recommendations yet"), tr!("Nobody has recommended a title beside this one.")),
            };
            return kit::empty(Icon::GridView, title, hint).into_any_element();
        }
        // A rail's cards are library tiles: until the reload brings their titles
        // in there is no tile to draw, and an empty box is not a card.
        if self.watch_rails_failed() {
            return kit::empty(Icon::Warning, tr!("Simkl's lists could not be loaded."), tr!("The app's log says why.")).into_any_element();
        }
        let missing = items.iter().filter(|related| self.rail_key(related).is_none()).count();
        if missing > 0 {
            // Said once per keep, not once per frame: the reload is fast, and a
            // debug log does not need forty copies of the same line.
            if !self.watch_rails_spoke() {
                log::debug!(target: "heronite_app", "watch rails: {missing} of {} titles have no library key yet", items.len());
                self.watch_rails_say();
            }
            return kit::empty(Icon::GridView, tr!("Loading titles…"), tr!("Asking Simkl for its lists.")).into_any_element();
        }
        let (tile_w, tile_h) = self.console_tile_size();
        let room = super::tile::label_room();
        // One list for the render and for the pad: the focus count, the routes
        // and the cards drawn all read it, so they cannot disagree.
        let keyed = self.rail_cards(rail, details);
        let focus = self.watch_rail_focus(keyed.len());
        let columns = rail_columns(window, tile_w);
        let line_w = rail_line_width(columns, tile_w);
        let pane = self.watch_pane();
        // A browser page, not a shelf: the theme's tiles fill the panel row
        // under row, and the panel scrolls (the right stick, or the pad moving
        // between rows). Each card is `console_tile`, so it wears the theme's
        // shape, corners, focus effect, title placement and badges like every
        // other cover on the console.
        let id = match rail {
            Rail::Similar => "console-watch-similar",
            Rail::Community => "console-watch-community",
        };
        // The rows are the width of a full one and centred in the panel (a
        // shorter last row keeps its columns), so the covers sit in the middle
        // with room on both sides for the focus's growth — a row wider than the
        // panel would push the last column's card past its right edge.
        let mut grid = div()
            .id(SharedString::from(id))
            .flex()
            .flex_col()
            .items_center()
            .gap(u(RAIL_ROW_GAP))
            .overflow_y_scroll()
            .track_scroll(self.console.game.about.handle());
        for (row, chunk) in keyed.chunks(columns).enumerate() {
            let mut tiles: Vec<AnyElement> = Vec::with_capacity(chunk.len());
            for (in_row, (key, related)) in chunk.iter().enumerate() {
                let index = row * columns + in_row;
                let on = pane == Pane::Card && index == focus;
                let face = self.console_tile(key, tile_w, tile_h, on, true, window);
                tiles.push(
                    div()
                        .id(SharedString::from(format!("watch-rail-{}", related.id)))
                        .flex_none()
                        .w(u(tile_w))
                        .h(u(tile_h + room))
                        .on_pads(rail_route(pane, focus, index, columns))
                        .child(face)
                        .into_any_element(),
                );
            }
            grid = grid.child(div().flex_none().w(u(line_w)).flex().gap(u(RAIL_GAP)).children(tiles));
        }
        grid.flex_1().min_h_0().into_any_element()
    }

    /// The cards of a rail the library already holds, in display order. The
    /// render, the focus count and the pad routes all read this list, so they
    /// cannot disagree about how many cards there are.
    fn rail_cards<'a>(&self, rail: Rail, details: Option<&'a TitleDetails>) -> Vec<(SharedString, &'a Related)> {
        let items = details.map_or(&[][..], |details| details.rail(rail));
        items.iter().filter_map(|related| self.rail_key(related).map(|key| (key, related))).collect()
    }

    /// The library key of a rail's card, once the reload has brought it in: a
    /// tile is drawn (and opened) by the key the library knows.
    fn rail_key(&self, related: &Related) -> Option<SharedString> {
        self.lib
            .as_ref()?
            .games
            .iter()
            .find(|game| crate::fork::watch::sources::id(game) == related.id)
            .map(|game| SharedString::from(game.key.clone()))
    }

    /// A rail's card was pressed: the title becomes a library title of its own
    /// (`simkl/opened`) and the page turns to it.
    ///
    /// The adoption is what keeps the title alive: until it happens the title is
    /// the **page's** scratch feed, and the next page's rails would take it away —
    /// which is exactly how a title opened from a rail used to vanish under the
    /// page ("This title is gone"). A press that beats the keep still works: the
    /// title is written (adopted) and the page turns when the load lands.
    fn watch_open_related(&mut self, related: &Related, cx: &mut Context<Self>) {
        let title = heronite_watch::metadata::related_title(related);
        if let Some(key) = self.rail_key(related) {
            log::info!(target: "heronite_app", "watch rails: opening {} ({}) at {key}", related.name, related.id);
            self.watch_adopt_related(title, false, cx);
            self.open_game_page(key, cx);
            return;
        }
        log::info!(
            target: "heronite_app",
            "watch rails: {} ({}) is not in the library yet; keeping it and opening when the load lands",
            related.name,
            related.id
        );
        self.watch_open_soon(&related.id);
        self.watch_adopt_related(title, true, cx);
    }

    /// Writes `title` as a title the person opened (`simkl/opened`), and reloads
    /// when the page is waiting for it to exist before it can turn to it.
    fn watch_adopt_related(&mut self, title: heronite_watch::types::Title, reload: bool, cx: &mut Context<Self>) {
        let app = self.app_paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { heronite_watch::catalog::adopt_related(&app, title) }).await;
            this.update(cx, |this, cx| match result {
                Ok(_) => {
                    if reload {
                        this.reload(cx);
                    }
                }
                Err(error) => {
                    this.watch_take_opening();
                    log::warn!(target: "heronite_app", "watch rails: adopting the opened title failed: {error:#}");
                    this.show_toast(tr!("Could not open that title: {error}", error = format!("{error:#}")), true, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The title's band at the top of the left column: the cover, the title, who
    /// owns it and what the source knows, and the line about what was watched.
    /// It is the page's own header, unchanged by the panel beside it.
    fn watch_header(&mut self, key: &SharedString, g: &CollectionGame, facts: &str, watched: Option<String>, window: &Window) -> Div {
        let s = kit::scale();
        let (cw, ch) = (136., 136. * self.cover_aspect);
        let cover = self.cover(key, cw * s, ch * s, window.scale_factor(), 300);
        let title = fitted_title(42., g.title.chars().count());
        div()
            .flex()
            .items_end()
            .gap(u(28.))
            .pt(u(64.))
            .child(
                div()
                    .flex_none()
                    .w(u(cw))
                    .h(u(ch))
                    .rounded(u(crate::console::look::tile_radius() + 2.))
                    .overflow_hidden()
                    .bg(kit::ink(0.08))
                    .border_1()
                    .border_color(kit::ink(0.12))
                    .shadow(kit::shadow(0.6, 12., 34.))
                    .children(cover.map(|cover| img(cover).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(motion::enter(
                SharedString::from(format!("watch-head-{key}")),
                div()
                    .flex()
                    .flex_col()
                    .gap(u(8.))
                    .min_w_0()
                    .child(kit::title(g.title.clone(), title).line_clamp(2))
                    .child(div().text_size(u(15.)).text_color(theme::text_muted()).child(facts.to_owned()))
                    .children(watched.map(|line| div().text_size(u(13.)).text_color(theme::text_dim()).child(line))),
                320,
                0.,
                10.,
            ))
    }

    /// A press on one of the marks, or the R3 shortcut: the watchlist's own, and
    /// A's for whichever mark has the focus.
    fn watch_press_mark(&mut self, game: &CollectionGame, mark: Mark, cx: &mut Context<Self>) {
        match mark.flag() {
            Some(flag) => self.watch_toggle_personal(game, flag, cx),
            None => self.watch_toggle_watched(game, cx),
        }
    }
}

impl CollectionView {
    /// The page's rows: Play, then for a series the season row and that
    /// season's episodes. The marks and the streams are not rows: the marks are
    /// the icons above the list, and the streams are the card's second tab.
    fn watch_rows(&mut self, game: &CollectionGame, details: Option<&TitleDetails>, locale: DateLocale) -> Vec<Row> {
        // A title no source here has: one line, and nothing that pretends it can
        // play or be marked (there is no local record to mark).
        if crate::fork::watch::sources::imported(game) {
            return vec![Row::info("watch-unavailable", tr!("Playback"), not_present())];
        }
        let (label, icon) = self.primary_state(game);
        let mut rows = vec![Row::action("primary", label).icon(icon)];
        // The local record: a series' per-episode states. A film's streams are
        // the card's second tab, and its mark is the tick of the icon row.
        let progress = heronite_watch::progress::local(&self.app_paths, &game.runner, crate::fork::watch::sources::id(game));
        let Some(details) = details.filter(|details| !details.episodes.is_empty()) else {
            return rows;
        };
        let selection = self.watch_ensure_selection(&game.key, details);
        let seasons = details.seasons();
        let season = seasons.get(selection.season).copied().unwrap_or(1);
        let options: Vec<SharedString> = seasons.iter().map(|number| SharedString::from(tr!("Season {n}", n = number))).collect();
        rows.push(Row::choice("season", tr!("Season"), options, selection.season));
        // A tick means this episode was finished, taken from the per-episode
        // records: the resume pointer only says where playback starts, so it
        // cannot stand for "watched" (it moves to the next episode the moment
        // that one is opened).
        let episodes = progress.map(|progress| progress.episodes).unwrap_or_default();
        let resuming = self.watch_resume().map(str::to_owned);
        for (index, episode) in details.episodes_of(season).enumerate() {
            // By the video id, then by season and number: an imported episode is
            // keyed the second way, a played one the first.
            let state = episodes
                .get(episode.id.as_str())
                .or_else(|| episodes.values().find(|record| record.season == Some(season) && record.number == Some(episode.number)))
                .map_or(WatchedState::Unwatched, |record| record.state());
            let resuming = resuming.as_deref() == Some(episode.id.as_str());
            rows.push(episode_row(episode, index, state, resuming, locale));
        }
        rows
    }

    /// A on a row of the list: Play starts the best of the picked title or
    /// episode, an episode row starts that episode's.
    fn watch_press_row(&mut self, game: &CollectionGame, id: &str, cx: &mut Context<Self>) {
        if id == "primary" {
            let video = self.watch_picked_video(game);
            self.watch_play_picked(game, video.as_deref(), cx);
            return;
        }
        if let Some(index) = id.strip_prefix("episode:").and_then(|index| index.parse::<usize>().ok()) {
            self.watch_press_episode(game, index, cx);
        }
    }

    /// A on an episode: it becomes the pick and its best stream starts at once.
    fn watch_press_episode(&mut self, game: &CollectionGame, index: usize, cx: &mut Context<Self>) {
        let Some(details) = self.watch_details(&game.key) else { return };
        let selection = self.watch_ensure_selection(&game.key, &details);
        let picked = Selection { season: selection.season, episode: index };
        let Some(episode) = picked_episode(&details, picked) else { return };
        let video = episode.id.clone();
        self.watch_pick_episode(&game.key, picked);
        self.watch_play_episode(game, &video, cx);
    }

    /// A 16:9 still from its URL, once loaded (the loader caches it by size).
    fn episode_still(&mut self, url: &str, w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("still:{url}:{pw}x{ph}").into();
        let chain = ImageRef::Url(url.to_owned());
        self.images.get_or_request(&id, || vec![chain], pw, ph, Variant::Cover, 200).map(ImageSource::Render)
    }

    pub(super) fn watch_page_pad(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(game) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else { return false };
        let details = self.watch_details(key);
        let tab = self.watch_tab();
        // L3 (the left stick's click) is the trailer's: the hint bar says
        // "Play trailer" while the title has one, and this is the press it
        // promises. X keeps what it does on the list — it starts the title —
        // and Y keeps the list's search, so no two presses trade places. A
        // title with no trailer answers the press itself (`watch_play_trailer`).
        if trailer_press(pad) {
            self.watch_play_trailer(&game, cx);
            return true;
        }
        // R3 (the right stick's click) is the watchlist's, from anywhere on the
        // page: the hint bar names it (`watch_later_hint`), and this is the
        // press that promise takes. An imported title has no local record to
        // mark, and the page says so instead of writing a mark nowhere.
        if watch_later_press(pad) {
            if !has_marks(&game) {
                self.show_toast(not_present(), true, cx);
            } else {
                self.watch_toggle_personal(&game, Flag::Watchlist, cx);
            }
            return true;
        }
        // LB / RB: the card beside this one, with the pad where that card
        // expects it.
        if matches!(pad, Pad::LB | Pad::RB) {
            let tab = self.watch_switch_tab(pad == Pad::RB, cx);
            // The panel is one scroller shared by the cards: each one opens at
            // its top.
            self.console.game.about.reset();
            if tab == DetailTab::Streams {
                let video = self.watch_picked_video(&game);
                self.watch_ensure_streams(&game, video.as_deref(), cx);
            }
            cx.notify();
            return true;
        }
        let locale = self.lib.as_ref().map(|lib| lib.date_locale).unwrap_or_default();
        let picked_video = self.watch_picked_video(&game);
        // The details card scrolls with the right stick whichever side has the
        // focus, and with ↑ ↓ while it holds the focus.
        if tab == DetailTab::Details {
            let step = f32::from(u(120.));
            let on_card = self.watch_pane() == Pane::Card;
            if matches!(pad, Pad::ScrollUp | Pad::ScrollDown) || (on_card && matches!(pad, Pad::Up | Pad::Down)) {
                let down = matches!(pad, Pad::Down | Pad::ScrollDown);
                self.console.game.about.by(if down { step } else { -step });
                return true;
            }
        }
        // The marks row: ◀ ▶ move along it, A presses the mark, ↓ (or B) leaves
        // it for the list. Nothing under it runs while it has the pad.
        if self.watch_pane() == Pane::Marks && has_marks(&game) {
            // The row's shape, not its contents: a press moves along the marks
            // the page drew, and reading the record again would be a disk read
            // per cursor move.
            let watched = shows_watched(
                crate::fork::watch::sources::kind(&game),
                details.as_deref().is_some_and(|details| !details.episodes.is_empty()),
            );
            let len = marks_len(watched);
            let focus = self.watch_mark_focus(len);
            return match pad {
                Pad::Left => {
                    self.watch_move_mark(false, len);
                    cx.notify();
                    true
                }
                Pad::Right => {
                    self.watch_move_mark(true, len);
                    cx.notify();
                    true
                }
                Pad::A => {
                    if let Some(mark) = mark_at(focus, watched) {
                        self.watch_press_mark(&game, mark, cx);
                    }
                    cx.notify();
                    true
                }
                // X starts the title from this row too: the hint bar promises it.
                Pad::X => {
                    self.watch_play_picked(&game, picked_video.as_deref(), cx);
                    true
                }
                Pad::Down | Pad::B => {
                    self.watch_focus_pane(Pane::List);
                    cx.notify();
                    true
                }
                _ => true,
            };
        }
        if self.watch_pane() == Pane::Card {
            // A rail: ◀ ▶ walk its cards, A opens the one the pad is on, ◀ on the
            // first leaves for the list (as it does on every card).
            if tab.is_rail() {
                let rail = rail_of(tab);
                let keyed = self.rail_cards(rail, details.as_deref());
                let len = keyed.len();
                let focus = self.watch_rail_focus(len);
                let columns = rail_columns(window, self.console_tile_size().0);
                // The right stick scrolls the grid, as it scrolls the details.
                if matches!(pad, Pad::ScrollUp | Pad::ScrollDown) {
                    let step = f32::from(u(160.));
                    self.console.game.about.by(if pad == Pad::ScrollDown { step } else { -step });
                    return true;
                }
                return match pad {
                    Pad::A => {
                        if let Some(index) = rail_press(len, focus) {
                            if let Some((_, related)) = keyed.get(index) {
                                self.watch_open_related(related, cx);
                            }
                        }
                        true
                    }
                    Pad::B => {
                        self.watch_focus_pane(Pane::List);
                        cx.notify();
                        true
                    }
                    Pad::Left | Pad::Right | Pad::Up | Pad::Down => {
                        match crate::collection::watch_playback::rail_step(focus, columns, len, pad) {
                            Some(next) => {
                                self.watch_set_rail(next);
                                // The row the pad is on stays in view.
                                self.console.game.about.go(motion::Goal::Reveal { ix: next / columns.max(1), margin: 8. });
                                cx.notify();
                                true
                            }
                            // The grid's edge goes back to the title's list.
                            None => {
                                self.watch_focus_pane(Pane::List);
                                cx.notify();
                                true
                            }
                        }
                    }
                    _ => false,
                };
            }
            return match tab {
                DetailTab::Details => match pad {
                    Pad::Left => {
                        self.watch_focus_pane(Pane::List);
                        cx.notify();
                        true
                    }
                    Pad::X => {
                        self.watch_play_picked(&game, picked_video.as_deref(), cx);
                        true
                    }
                    _ => false,
                },
                DetailTab::Community | DetailTab::Similar => false,
                DetailTab::Streams => {
                    self.watch_ensure_streams(&game, picked_video.as_deref(), cx);
                    let rows = self.watch_stream_rows(&game, picked_video.as_deref());
                    match self.watch_streams_nav().on_pad(pad, &rows) {
                        Outcome::Activate(id) => {
                            if let Some(index) = id.strip_prefix("stream:").and_then(|index| index.parse::<usize>().ok()) {
                                self.watch_play_stream(&game, index, cx);
                            }
                            cx.notify();
                            true
                        }
                        // ◀ leaves the streams for the episode list.
                        Outcome::Edge(Pad::Left) => {
                            self.watch_focus_pane(Pane::List);
                            cx.notify();
                            true
                        }
                        Outcome::Moved | Outcome::Edge(_) | Outcome::Changed(..) => {
                            cx.notify();
                            true
                        }
                        Outcome::Ignored => match pad {
                            Pad::X => {
                                self.watch_play_picked(&game, picked_video.as_deref(), cx);
                                true
                            }
                            _ => false,
                        },
                    }
                }
            };
        }
        // The list has the focus: it navigates, and ▶ reaches the card when the
        // row has no value of its own (the season row keeps it for ◀ ▶).
        let rows = self.watch_rows(&game, details.as_deref(), locale);
        match self.console.game.list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                self.watch_press_row(&game, &id, cx);
                cx.notify();
                true
            }
            Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "season" => {
                if let Some(details) = self.watch_details(key) {
                    self.watch_pick_season(&game.key, &details, index);
                    self.watch_forget_streams();
                    cx.notify();
                }
                true
            }
            // ↑ on the first row: the marks above it, when the title has any
            // (an imported title has no local record to mark).
            Outcome::Edge(Pad::Up) => {
                if has_marks(&game) {
                    self.watch_focus_pane(Pane::Marks);
                }
                cx.notify();
                true
            }
            Outcome::Edge(Pad::Right) => {
                self.watch_focus_pane(Pane::Card);
                cx.notify();
                true
            }
            Outcome::Moved | Outcome::Edge(_) | Outcome::Changed(..) => {
                cx.notify();
                true
            }
            Outcome::Ignored => match pad {
                Pad::X => {
                    self.watch_play_picked(&game, picked_video.as_deref(), cx);
                    true
                }
                _ => false,
            },
        }
    }

    /// The page's hint bar: what A, the bumpers, X, L3 and R3 do on the card
    /// shown. X starts the title, as it does on the list; L3 is the trailer while
    /// the title has one, and R3 adds the title to the watchlist. Each press and
    /// its label come from one function, so they cannot disagree.
    pub(super) fn watch_page_hints(&self, key: &SharedString) -> Vec<Hint> {
        let primary = self.lib.as_ref().and_then(|lib| lib.get(key)).map(|game| self.primary_label(game)).unwrap_or(tr!("Play"));
        let has_trailer = self.watch_details(key).is_some_and(|details| !details.trailers.is_empty());
        let on_the_shelf = self.lib.as_ref().and_then(|lib| lib.get(key)).is_some_and(|game| has_marks(game));
        // On a rail, A opens the card the pad is on rather than selecting a row.
        let primary_hint = if self.watch_tab().is_rail() { tr!("Open") } else { tr!("Select") };
        let mut hints = vec![hint(Btn::A, primary_hint), hint(Btn::LeftRight, tr!("Panels")), hint(Btn::Bumpers, tr!("Tabs"))];
        hints.push(hint(Btn::X, primary));
        hints.extend(trailer_hint(has_trailer));
        hints.extend(watch_later_hint(on_the_shelf));
        hints.push(hint(Btn::B, tr!("Back")));
        hints.push(hint(Btn::Start, tr!("Menu")));
        hints
    }
}

/// The hint bar's L3: the press it names is the one `trailer_press` takes, so
/// the label and what it does cannot drift apart.
const TRAILER_BTN: Btn = Btn::L3;

/// The hint bar's L3, when the title has a trailer to watch: the press plays it
/// in mpv instead of the stream. No trailer, no hint — the press itself is what
/// says there is none (`watch_play_trailer`).
fn trailer_hint(has_trailer: bool) -> Option<Hint> {
    has_trailer.then(|| hint(TRAILER_BTN, tr!("Play trailer")))
}

/// Whether `pad` is the trailer's press: the click of the left stick, never the
/// primary's X (that one starts the title, as it does on the list) and never Y
/// (the list keeps it for the search). The title's own state says what the press
/// does — play the trailer, or answer that there is none — so the press is the
/// same on the page and on the covers, even before the details are in hand.
fn trailer_press(pad: Pad) -> bool {
    pad == Pad::L3
}

/// The hint bar's R3: the watch-later press, named as `watch_later_press` takes
/// it, so the label and what it does cannot drift apart.
const WATCH_LATER_BTN: Btn = Btn::R3;

/// The hint bar's R3, on a title the shelf can hold: the press adds it to (or
/// takes it from) the watchlist. An imported title has no local record to mark,
/// so it has no hint either.
fn watch_later_hint(on_the_shelf: bool) -> Option<Hint> {
    on_the_shelf.then(|| hint(WATCH_LATER_BTN, tr!("Watch later")))
}

/// Whether `pad` is the watch-later press: the click of the right stick, never
/// the left one (that is the trailer's).
fn watch_later_press(pad: Pad) -> bool {
    pad == kit::btn_pad(WATCH_LATER_BTN)
}

/// Whether the card has room for two columns of facts (design px, as the
/// games' page measures it).
fn card_is_wide(window: &Window, scale: f32) -> bool {
    let viewport = f32::from(crate::ui::content_size(window).width) / scale;
    viewport - 2. * PAGE_PAD - LEFT_W - COLUMN_GAP > 760.
}

#[cfg(test)]
mod tests {
    use super::sections::episode::{episode_desc, episode_label, episode_name};
    use super::*;
    use heronite_i18n::trn;
    use heronite_watch::metadata::Episode;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    fn matrix() -> TitleDetails {
        TitleDetails {
            name: "The Matrix".into(),
            year: Some("1999".into()),
            runtime: Some("136 min".into()),
            rating: Some("8.7".into()),
            released: Some("1999-03-31".into()),
            genres: vec!["Action".into(), "Sci-Fi".into()],
            directors: vec!["Lana Wachowski".into(), "Lilly Wachowski".into()],
            writers: vec!["Lilly Wachowski".into()],
            cast: vec!["Keanu Reeves".into(), "Carrie-Anne Moss".into()],
            countries: vec!["United States".into()],
            awards: Some("Won 4 Oscars.".into()),
            ..TitleDetails::default()
        }
    }

    #[test]
    fn the_header_names_where_the_title_comes_from_and_what_the_source_knows() {
        assert_eq!(facts_line(&["Cinemeta"], Some(&matrix()), None), "Cinemeta · 1999 · 136 min · IMDb 8.7");
        assert_eq!(
            facts_line(&["Cinemeta", "Stremio"], Some(&matrix()), None),
            "Cinemeta · Stremio · 1999 · 136 min · IMDb 8.7",
            "the stream source follows the catalogue once a stream is known"
        );
        assert_eq!(facts_line(&["Stremio"], None, Some("1999")), "Stremio · 1999");
        assert_eq!(facts_line(&["Cinemeta"], None, None), "Cinemeta");
    }

    #[test]
    fn the_card_lists_the_films_people_and_no_game_row() {
        let lines = detail_lines(Some(&matrix()), &[], DateLocale::En);
        let labels: Vec<&str> = lines.iter().map(|line| line.label.as_ref()).collect();
        assert_eq!(labels, ["Directed by", "Written by", "Starring", "Released", "Runtime", "Rating", "Genres", "Country", "Awards"]);
        assert_eq!(lines[0].value, "Lana Wachowski, Lilly Wachowski");
        assert_eq!(lines[3].value, "3/31/1999", "the released date follows the library's locale");
        for game in ["Playtime", "Last Played", "Installed", "Folders", "Steam tags", "Series"] {
            assert!(!labels.contains(&game), "{game} is not a film fact");
        }
    }

    fn episode(season: u32, number: u32, name: &str) -> Episode {
        Episode {
            id: format!("tt0944947:{season}:{number}"),
            season,
            number,
            name: name.into(),
            thumbnail: Some(format!("https://episodes.metahub.space/tt0944947/{season}/{number}/w780.jpg")),
            released: Some(format!("2011-0{season}-1{number}")),
            ..Episode::default()
        }
    }

    fn series() -> TitleDetails {
        TitleDetails {
            kind: "series".into(),
            series_status: Some("Ended".into()),
            episodes: vec![episode(1, 1, "Winter Is Coming"), episode(1, 2, "The Kingsroad"), episode(2, 1, "The North Remembers")],
            ..matrix()
        }
    }

    #[test]
    fn a_series_adds_its_status_and_counts() {
        let details = series();
        let lines = detail_lines(Some(&details), &[], DateLocale::En);
        let value = |label: &str| lines.iter().find(|line| line.label == label).map(|line| line.value.as_str());
        assert_eq!(value("Status"), Some("Ended"));
        assert_eq!(value("Seasons"), Some("2"));
        assert_eq!(value("Episodes"), Some("3"));
        assert_eq!(value("Episode runtime"), Some("136 min"), "a show's length is one episode's");
        assert_eq!(value("Time to binge"), Some("6h 48m"), "three episodes at 136 min");
        assert_eq!(value("Runtime"), None, "a show never says a whole runtime");
    }

    #[test]
    fn the_picker_names_each_episode_of_the_chosen_season() {
        let details = series();
        let selection = Selection { season: 0, episode: 1 };
        let picked = picked_episode(&details, selection).expect("the season has two episodes");
        assert_eq!(picked.code(), "S1E2");
        assert_eq!(episode_label(picked), "2 \u{b7} The Kingsroad");
        assert_eq!(episode_desc(picked, DateLocale::En), "S1E2 \u{b7} 1/12/2011");
        // The row's id says which episode of the season the list means.
        assert_eq!(episode_row(picked, 1, WatchedState::Unwatched, false, DateLocale::En).id.as_ref(), "episode:1");
        assert!(episode_row(picked, 1, WatchedState::Watched, false, DateLocale::En).icon.is_some(), "a watched episode wears a tick");
        let started =
            episode_row(picked, 1, WatchedState::InProgress { position_ms: 1_200_000, duration_ms: 2_400_000 }, true, DateLocale::En);
        assert!(started.icon.is_some(), "a half-watched episode wears the other mark");
        let desc = started.desc.clone().unwrap_or_default();
        assert!(desc.contains("50%") && desc.contains("Resume"), "{desc}");

        let other = Selection { season: 1, episode: 0 };
        assert_eq!(picked_episode(&details, other).map(|episode| episode.name.as_str()), Some("The North Remembers"));
        assert_eq!(picked_episode(&details, Selection { season: 9, episode: 0 }), None, "no such season");
    }

    #[test]
    fn the_card_follows_the_episode_the_list_is_on() {
        let rows = vec![
            Row::action("primary", "Play"),
            Row::choice("season", "Season", vec!["Season 1".into()], 0),
            episode_row(&episode(1, 1, "A"), 0, WatchedState::Unwatched, false, DateLocale::En),
            episode_row(&episode(1, 2, "B"), 1, WatchedState::Unwatched, false, DateLocale::En),
        ];
        assert_eq!(focused_episode(&rows, 0), None, "Play is not an episode");
        assert_eq!(focused_episode(&rows, 1), None, "a season is not an episode");
        assert_eq!(focused_episode(&rows, 2), Some(0));
        assert_eq!(focused_episode(&rows, 3), Some(1));
        assert_eq!(focused_episode(&rows, 9), None);
    }

    #[test]
    fn an_unnamed_episode_is_its_number() {
        let unnamed = Episode { season: 1, number: 7, ..Episode::default() };
        assert_eq!(episode_name(&unnamed), "Episode 7");
        assert_eq!(episode_label(&unnamed), "7 \u{b7} Episode 7");
    }

    #[test]
    fn without_a_source_the_library_genres_stand_in() {
        let lines = detail_lines(None, &["Drama".into()], DateLocale::En);
        assert_eq!(lines, [DetailLine { label: "Genres".into(), value: "Drama".into() }]);
        assert!(detail_lines(None, &[], DateLocale::En).is_empty());
    }

    #[test]
    fn a_watched_film_says_how_much_and_when() {
        assert_eq!(watched_line(23, None, None, DateLocale::En).as_deref(), Some("23m watched"));
        assert_eq!(watched_line(0, None, None, DateLocale::En), None);
        assert_eq!(
            watched_line(0, None, Some(tr!("Watched").to_string()), DateLocale::En).as_deref(),
            Some("Watched"),
            "finished is the mark"
        );
        let line = watched_line(0, Some("2026-11-12"), None, DateLocale::En).expect("a date is a fact");
        assert!(line.starts_with("Last watched "), "{line}");
    }

    #[test]
    fn the_hint_bar_offers_the_trailer_only_when_there_is_one() {
        assert!(trailer_hint(false).is_none(), "no trailer, nothing to offer");
        let offered = trailer_hint(true).expect("a trailer is offered on the stick's click");
        assert_eq!(offered.btn, Btn::L3, "the stick's click, never the primary's X");
        assert_eq!(offered.label, "Play trailer");
    }

    #[test]
    fn only_the_sticks_click_presses_the_trailer() {
        assert!(trailer_press(Pad::L3), "the click plays it");
        for pad in [Pad::X, Pad::A, Pad::B, Pad::Y, Pad::Up, Pad::Down, Pad::Left, Pad::LB, Pad::Start] {
            assert!(!trailer_press(pad), "{pad:?} keeps its own job");
        }
    }

    #[test]
    fn a_title_left_part_way_says_how_far_and_a_series_counts_its_episodes() {
        use chrono::Utc;
        let mut progress = heronite_watch::progress::MovieProgress {
            provider: "stremio".into(),
            id: "tt1".into(),
            position_ms: 2_400_000,
            duration_ms: 6_000_000,
            updated_at: Utc::now(),
            finished: false,
            video_id: None,
            imdb: Some("tt1".into()),
            episodes: Default::default(),
            ..Default::default()
        };
        assert_eq!(title_status(Some(&progress), "movie", None).as_deref(), Some("In progress · 40%"));

        let mut details = TitleDetails { id: "tt1".into(), kind: "series".into(), name: "A Show".into(), ..Default::default() };
        details.episodes =
            (1..=8).map(|number| Episode { id: format!("tt1:1:{number}"), season: 1, number, ..Default::default() }).collect();
        for number in 1..=3 {
            progress.episodes.insert(
                format!("tt1:1:{number}"),
                heronite_watch::progress::EpisodeProgress {
                    position_ms: 2_400_000,
                    duration_ms: 2_400_000,
                    updated_at: Utc::now(),
                    finished: true,
                    season: Some(1),
                    number: Some(number),
                    ..Default::default()
                },
            );
        }
        assert_eq!(title_status(Some(&progress), "series", Some(&details)).as_deref(), Some("3 of 8 episodes watched"));
        assert_eq!(title_status(Some(&progress), "series", None), Some(trn!(3, "{n} episode watched", "{n} episodes watched")));
        assert_eq!(title_status(None, "movie", None), None, "no record, no line");
    }

    #[test]
    fn a_mark_is_filled_and_coloured_only_while_it_is_on() {
        for mark in [Mark::Watched, Mark::Watchlist, Mark::Favourite] {
            assert_eq!(mark.tint(false), None, "{mark:?} is untinted while it is off");
            assert!(mark.tint(true).is_some(), "{mark:?} is coloured while it is on");
        }
        // The two shelf flags swap their outline for the full icon; the watched
        // mark stays a tick, and its colour is what says where it stands.
        for mark in [Mark::Watchlist, Mark::Favourite] {
            assert_ne!(mark.icon(false), mark.icon(true), "{mark:?} fills its icon with its state");
        }
        assert_eq!(Mark::Watched.icon(false), Icon::Check, "a tick either way: the tint says which");
        assert_ne!(Mark::Watchlist.icon(true), Mark::Favourite.icon(true), "the two shelf flags never look alike");
        assert_eq!([Mark::Watched.rank(), Mark::Watchlist.rank(), Mark::Favourite.rank()], [0, 1, 2], "the row's order");
    }

    #[test]
    fn a_title_nobody_has_played_still_has_a_watched_mark() {
        assert_eq!(watched_state(None, None, "movie"), Some(false), "a film never played can be marked");
        let episode = Picked { id: "tt1:1:2", season: 1, number: 2 };
        assert_eq!(watched_state(None, Some(episode), "series"), Some(false), "an episode watched elsewhere can be marked");
        assert_eq!(watched_state(None, None, "series"), None, "a show without its episodes has nothing to mark");

        let played = heronite_watch::progress::MovieProgress { finished: true, ..Default::default() };
        assert_eq!(watched_state(Some(&played), None, "movie"), Some(true), "a finished film is the tick");

        let mut episodes = played;
        episodes.episodes.insert(
            "tt1:1:2".into(),
            heronite_watch::progress::EpisodeProgress {
                finished: true,
                season: Some(1),
                number: Some(2),
                updated_at: chrono::Utc::now(),
                ..Default::default()
            },
        );
        assert_eq!(watched_state(Some(&episodes), Some(episode), "series"), Some(true), "its own record is the tick");
        let other = Picked { id: "tt1:1:3", season: 1, number: 3 };
        assert_eq!(watched_state(Some(&episodes), Some(other), "series"), Some(false), "another episode is its own mark");
    }

    #[test]
    fn a_press_on_a_mark_writes_the_flag_it_draws() {
        assert_eq!(Mark::Watchlist.flag(), Some(Flag::Watchlist));
        assert_eq!(Mark::Favourite.flag(), Some(Flag::Favourite));
        assert_eq!(Mark::Watched.flag(), None, "the watched mark writes the record, not a shelf");
    }

    #[test]
    fn the_watched_mark_names_the_episode_it_is_about() {
        assert_eq!(watched_mark_label(false, None).as_ref(), "Mark as watched");
        assert_eq!(watched_mark_label(true, None).as_ref(), "Mark as not watched");
        assert_eq!(watched_mark_label(false, Some("S1E2")).as_ref(), "Mark S1E2 as watched");
        assert_eq!(watched_mark_label(true, Some("S1E2")).as_ref(), "Mark S1E2 as not watched");
    }

    #[test]
    fn the_row_lists_the_watched_mark_then_the_two_flags() {
        assert!(shows_watched("movie", false), "a film always has a mark, played or not");
        assert!(shows_watched("series", true));
        assert!(!shows_watched("series", false), "a show whose episodes are not here has nothing to mark");

        assert_eq!(marks_len(true), 3, "the tick and the two flags");
        assert_eq!(marks_len(false), 2, "a series without its episodes keeps its two flags");

        assert_eq!(mark_at(0, true), Some(Mark::Watched));
        assert_eq!(mark_at(1, true), Some(Mark::Watchlist));
        assert_eq!(mark_at(2, true), Some(Mark::Favourite));
        assert_eq!(mark_at(3, true), None);
        assert_eq!(mark_at(0, false), Some(Mark::Watchlist), "the row closes over a missing watched mark");
        assert_eq!(mark_at(1, false), Some(Mark::Favourite));
        assert_eq!(mark_at(2, false), None);
    }

    #[test]
    fn a_click_on_a_mark_makes_the_presses_that_reach_it() {
        // The marks have the pad: a click on another one moves along and presses.
        assert_eq!(mark_route(Pane::Marks, 4, 0, 2), [Pad::Right, Pad::Right, Pad::A]);
        assert_eq!(mark_route(Pane::Marks, 0, 1, 0), [Pad::Left, Pad::A]);
        // The list has it: up its rows to the first, then up into the marks.
        assert_eq!(mark_route(Pane::List, 0, 1, 0), [Pad::Up, Pad::Left, Pad::A]);
        assert_eq!(mark_route(Pane::List, 2, 0, 1), [Pad::Up, Pad::Up, Pad::Up, Pad::Right, Pad::A]);
        // The card has it: ◀ back to the list, then up.
        assert_eq!(mark_route(Pane::Card, 0, 0, 0), [Pad::Left, Pad::Up, Pad::A]);
    }

    #[test]
    fn the_watch_later_hint_names_the_press_that_writes_it() {
        assert!(watch_later_hint(false).is_none(), "an imported title has no record to mark");
        let offered = watch_later_hint(true).expect("the shelf holds what a source here has");
        assert_eq!(offered.btn, Btn::R3, "the right stick's click");
        assert_eq!(offered.label, "Watch later");
        assert!(watch_later_press(Pad::R3), "the hint's press is the one the page takes");
        for pad in [Pad::L3, Pad::X, Pad::A, Pad::B, Pad::Left, Pad::Right, Pad::Start] {
            assert!(!watch_later_press(pad), "{pad:?} keeps its own job");
        }
        assert!(!trailer_press(Pad::R3), "the two sticks' clicks do different things");
    }

    #[test]
    fn a_rails_row_never_runs_out_of_the_panel() {
        // Every theme's tile (portrait 120–215, square, wide up to 376) at the
        // panels a window can give: the row is never wider than its room, so
        // the last card cannot be pushed past the panel's right edge.
        for room in [200., 316., 462., 604., 900., 1180.] {
            for tile_w in [120., 150., 180., 215., 262.5, 315., 376.] {
                let columns = columns_in(room, tile_w);
                assert!(columns >= 1, "a rail always has a row");
                if room >= tile_w {
                    assert!(rail_line_width(columns, tile_w) <= room, "{columns} tiles of {tile_w} fit {room}");
                    assert!(rail_line_width(columns + 1, tile_w) > room, "one more tile of {tile_w} would not fit {room}");
                } else {
                    assert_eq!(columns, 1, "a room narrower than a tile keeps the one column it can");
                }
            }
        }
    }
}
