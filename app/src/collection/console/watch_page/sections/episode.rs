//! A series' episode picker: one row per episode of the chosen season, and the
//! card that previews the picked one.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::text::DateLocale;
use heronite_i18n::tr;
use heronite_watch::metadata::Episode;
use heronite_watch::progress::WatchedState;

use crate::console::kit::{self, u};
use crate::console::list::Row;
use crate::icons::Icon;
use crate::theme;

/// "Winter Is Coming", or "Episode 3" when the source has no name.
pub(in crate::collection::console::watch_page) fn episode_name(episode: &Episode) -> String {
    match episode.name.trim() {
        "" => tr!("Episode {n}", n = episode.number).to_string(),
        name => name.to_owned(),
    }
}

/// "1 · Winter Is Coming": the picker row's label.
pub(in crate::collection::console::watch_page) fn episode_label(episode: &Episode) -> String {
    format!("{} · {}", episode.number, episode_name(episode))
}

/// "S1E2 · 18 Apr 2011": the picker row's second line.
pub(in crate::collection::console::watch_page) fn episode_desc(episode: &Episode, locale: DateLocale) -> String {
    let mut parts = vec![episode.code()];
    parts
        .extend(episode.released.as_deref().map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty()));
    parts.join(" \u{b7} ")
}

/// One row of a season's episodes ("episode:{index}" in the list). A tick means
/// finished, the half mark means it was started and left, and a play mark means
/// nothing was played yet; the row says how far in when it knows.
pub(in crate::collection::console::watch_page) fn episode_row(
    episode: &Episode,
    index: usize,
    state: WatchedState,
    resuming: bool,
    locale: DateLocale,
) -> Row {
    let mut parts = vec![episode_desc(episode, locale)];
    match state {
        WatchedState::InProgress { .. } => {
            let percent = state.percent().unwrap_or_default();
            parts.push(if resuming {
                tr!("Resume from here · {percent}%", percent = percent).to_string()
            } else {
                tr!("In progress · {percent}%", percent = percent).to_string()
            });
        }
        WatchedState::Unwatched if resuming => parts.push(tr!("Resume from here").to_string()),
        WatchedState::Watched | WatchedState::Unwatched => {}
    }
    let icon = match state {
        WatchedState::Watched => Icon::Check,
        // A clock for "started, not finished" — the set already has one.
        WatchedState::InProgress { .. } => Icon::AccessTime,
        WatchedState::Unwatched => Icon::PlayArrow,
    };
    Row::action(SharedString::from(format!("episode:{index}")), episode_label(episode)).desc(parts.join(" \u{b7} ")).icon(icon)
}

/// The picked episode above the title's own details: still, code, name, date,
/// rating and what it is about.
pub(in crate::collection::console::watch_page) fn episode_card(episode: &Episode, still: Option<ImageSource>, locale: DateLocale) -> Div {
    let mut line = vec![episode.code()];
    line.extend(episode.released.as_deref().map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty()));
    line.extend(episode.rating.as_ref().map(|rating| tr!("IMDb {rating}", rating = rating).to_string()));
    div()
        .flex()
        .flex_col()
        .gap(u(10.))
        .child(
            div()
                .flex_none()
                .w(u(420.))
                .h(u(236.))
                .rounded(u(14.))
                .overflow_hidden()
                .bg(kit::ink(0.08))
                .children(still.map(|still| img(still).size_full().object_fit(ObjectFit::Cover))),
        )
        .child(div().text_size(u(20.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(format!(
            "{} \u{b7} {}",
            episode.code(),
            episode_name(episode)
        )))
        .children((!line.is_empty()).then(|| div().text_size(u(14.)).text_color(theme::text_muted()).child(line.join(" \u{b7} "))))
        .children(
            episode.overview.clone().filter(|overview| !overview.trim().is_empty()).map(|overview| {
                div().max_w(u(640.)).text_size(u(15.)).line_height(u(22.)).text_color(theme::text_primary()).child(overview)
            }),
        )
}
