//! Settings → Watch → Recap: what was watched, over a week, a month, a year or
//! everything the log holds.
//!
//! The numbers come from `heronite_watch_stats` (pure, over
//! `watch-sessions.json`); this is the drawing. Two shapes of the same recap:
//! cards with bars on the desktop, rows in console mode — a controller can read
//! the totals and the top titles, which is what a couch wants.
//!
//! The span is kept in `ui-state.json` by the settings page's own state, so a
//! look at the year survives a restart.

use chrono::Datelike as _;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_watch::stats::{months, Recap, Span, TitleRow};

use super::SettingsPage;
use crate::console::list::{Outcome, Row, Value};
use crate::theme;
use crate::ui::controls::{caption, card_box};
use crate::ui::fields::meta_text;

/// What a recap screen keeps between visits: the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Recency {
    Week,
    Month,
    #[default]
    Year,
    All,
}

impl Recency {
    pub const ALL: [Recency; 4] = [Recency::Week, Recency::Month, Recency::Year, Recency::All];

    pub fn span(self) -> Span {
        match self {
            Recency::Week => Span::Week,
            Recency::Month => Span::Month,
            Recency::Year => Span::Year,
            Recency::All => Span::All,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Recency::Week => tr!("This week"),
            Recency::Month => tr!("This month"),
            Recency::Year => tr!("This year"),
            Recency::All => tr!("Everything"),
        }
    }

    pub fn index(self) -> usize {
        Recency::ALL.iter().position(|recency| *recency == self).unwrap_or(2)
    }

    pub fn from_index(index: usize) -> Self {
        Recency::ALL.get(index).copied().unwrap_or_default()
    }
}

/// The title said once, over the whole page: what hours mean and what is not
/// counted.
pub fn blurb() -> &'static str {
    tr!(
        "What you watched here, from the sessions this app records: a title counts once per window, a day with nothing watched is a gap, and nothing is fetched to build it."
    )
}

/// The desktop page: the window's chips, then the shared body.
pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let recap = heronite_watch::stats::recap(&d.ctx.app, d.watch.recap.span());
    let recency = d.watch.recap;
    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(caption(tr!("Recap")))
        .child(div().px(px(6.)).child(meta_text(blurb().to_string())))
        .child(window_chips(d, cx))
        .children(body(&recap, recency))
        .into_any_element()
}

/// The numbers, as both surfaces draw them: the totals, the bars for the window
/// and the titles that took its time — or the "nothing here yet" card.
pub fn body(recap: &Recap, recency: Recency) -> Vec<AnyElement> {
    if recap.totals.sessions == 0 {
        return vec![empty().into_any_element()];
    }
    let mut out: Vec<AnyElement> = vec![totals_card(recap).into_any_element()];
    let chart = bars(recap, recency);
    out.push(chart.into_any_element());
    if let Some(titles) = top_titles(recap) {
        out.push(div().flex().flex_col().gap(px(8.)).child(caption(tr!("Most watched"))).child(titles).into_any_element());
    }
    out
}

/// Console mode: the same recap as rows a controller can read.
pub(crate) fn rows(d: &SettingsPage) -> Vec<Row> {
    let recap = heronite_watch::stats::recap(&d.ctx.app, d.watch.recap.span());
    let mut rows = vec![Row::header(tr!("Recap"))];
    rows.push(Row::choice(
        "watch-recap-window",
        tr!("Window"),
        Recency::ALL.iter().map(|recency| SharedString::from(recency.name())).collect(),
        d.watch.recap.index(),
    ));
    rows.push(Row::info("watch-recap-hours", tr!("Watched"), hours(recap.totals.hours())));
    rows.push(Row::info("watch-recap-titles", tr!("Titles"), count(recap.totals.titles)));
    rows.push(Row::info(
        "watch-recap-kinds",
        tr!("Films and series"),
        SharedString::from(format!("{} · {}", count(recap.totals.films), count(recap.totals.series))),
    ));
    rows.push(Row::info("watch-recap-finished", tr!("Finished"), count(recap.totals.finished)));
    rows.push(Row::info("watch-recap-streak", tr!("Longest streak"), days(recap.longest_streak.0)));
    rows.push(Row::info("watch-recap-binges", tr!("Binge days"), count(recap.binge_days)));
    if let Some(day) = recap.busiest_day() {
        rows.push(Row::info("watch-recap-busiest", tr!("Busiest day"), format!("{} · {}", day.date, hours(day.seconds / 3600.))));
    }
    for (at, title) in recap.titles.iter().take(8).enumerate() {
        rows.push(Row::info(
            SharedString::from(format!("watch-recap-title-{at}")),
            SharedString::from(title.title.clone()),
            SharedString::from(hours(title.hours())),
        ));
    }
    rows
}

/// The Console's span choice.
pub(crate) fn apply(d: &mut SettingsPage, outcome: Outcome, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    if let Outcome::Changed(id, Value::Choice { index, .. }) = outcome {
        if id.as_ref() == "watch-recap-window" {
            d.watch.recap = Recency::from_index(index);
            cx.notify();
        }
    }
}

fn window_chips(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let mut chips = div().flex().flex_wrap().items_center().gap(px(8.));
    for recency in Recency::ALL {
        let chosen = d.watch.recap == recency;
        chips = chips.child(
            div()
                .id(SharedString::from(format!("watch-recap-{}", recency.index())))
                .px(px(12.))
                .py(px(6.))
                .rounded(px(999.))
                .border_1()
                .border_color(if chosen { theme::accent_tint_border() } else { theme::ink_a(0.08) })
                .bg(if chosen { theme::accent_tint() } else { theme::ink_a(0.03) })
                .text_size(px(13.))
                .cursor_pointer()
                .child(tr(recency.name()))
                .on_click(cx.listener(move |d, _, _, cx| {
                    d.watch.recap = recency;
                    cx.notify();
                })),
        );
    }
    chips
}

/// The totals, as one row of figures.
fn totals_card(recap: &Recap) -> Div {
    let figure = |label: &str, value: String| {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w(px(110.))
            .child(div().text_size(px(20.)).font_weight(theme::W_BOLD).child(value))
            .child(meta_text(label.to_string()))
    };
    let mut row = div().flex().flex_wrap().items_end().gap(px(22.));
    row = row
        .child(figure(tr!("watched"), hours(recap.totals.hours())))
        .child(figure(tr!("titles"), count(recap.totals.titles)))
        .child(figure(tr!("films"), count(recap.totals.films)))
        .child(figure(tr!("series"), count(recap.totals.series)))
        .child(figure(tr!("finished"), count(recap.totals.finished)));
    if recap.longest_streak.0 > 0 {
        row = row.child(figure(tr!("longest streak"), days(recap.longest_streak.0)));
    }
    if recap.binge_days > 0 {
        row = row.child(figure(tr!("binge days"), count(recap.binge_days)));
    }
    card_box().py(px(14.)).px(px(16.)).child(row)
}

/// The day bars (week and month) or the month bars (year). Everything shows no
/// bars: it has no window to lay out.
fn bars(recap: &Recap, recency: Recency) -> Div {
    let (caption, bars): (&str, Vec<(String, f64)>) = match recency {
        Recency::Week | Recency::Month => (
            tr!("Day by day"),
            recap
                .days
                .iter()
                .map(|day| {
                    // A short label: a day of a week wants its letter, a month its number.
                    let label = match recency {
                        Recency::Week => day.date.format("%a").to_string(),
                        _ => day.date.day().to_string(),
                    };
                    (label, day.seconds)
                })
                .collect(),
        ),
        Recency::Year => {
            (tr!("Month by month"), months(&recap.days).into_iter().map(|(month, seconds)| (month.to_string(), seconds)).collect())
        }
        Recency::All => return div(),
    };
    let peak = bars.iter().map(|(_, seconds)| *seconds).fold(0., f64::max).max(1.);
    // A day of a month is thirty labels in a row: only a week or a year is
    // labelled, where they fit.
    let labelled = bars.len() <= 12;
    let mut chart = div().flex().items_end().gap(px(3.)).h(px(120.)).w_full();
    for (label, seconds) in bars {
        let height = ((seconds / peak * 110.).max(if seconds > 0. { 4. } else { 1. })) as f32;
        chart = chart.child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .justify_end()
                .items_center()
                .gap(px(4.))
                .child(div().w_full().h(px(height)).rounded_t(px(3.)).bg(if seconds > 0. {
                    theme::accent_tint()
                } else {
                    theme::ink_a(0.06)
                }))
                .when(labelled, |column| column.child(meta_text(label))),
        );
    }
    card_box().py(px(14.)).px(px(16.)).flex().flex_col().gap(px(8.)).child(meta_text(caption.to_string())).child(chart)
}

/// The window's titles, most watched first.
fn top_titles(recap: &Recap) -> Option<Div> {
    if recap.titles.is_empty() {
        return None;
    }
    let mut rows = div().flex().flex_col().gap(px(6.));
    for title in recap.titles.iter().take(10) {
        rows = rows.child(title_row(title));
    }
    Some(card_box().py(px(12.)).px(px(16.)).child(rows))
}

fn title_row(title: &TitleRow) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().text_size(px(13.5)).child(title.title.clone()))
                .when(title.kind == "series", |row| row.child(meta_text(tr!("series").to_string())))
                .when(title.finished, |row| row.child(meta_text(tr!("finished").to_string()))),
        )
        .child(div().text_size(px(13.)).text_color(theme::text_muted()).child(hours(title.hours())))
}

fn empty() -> Div {
    card_box()
        .py(px(18.))
        .px(px(16.))
        .child(div().text_size(px(13.5)).child(tr!("Nothing watched in this window yet.")))
        .child(meta_text(tr!("Sessions are recorded from now on, so a new log starts empty.")))
}

/// "12 h 40 m", "40 m", "0 h".
fn hours(value: f64) -> String {
    let minutes = (value * 60.).round().max(0.) as u64;
    let (h, m) = (minutes / 60, minutes % 60);
    match (h, m) {
        (0, 0) => tr!("0 m").to_string(),
        (0, m) => tr!("{m} m", m = m).to_string(),
        (h, 0) => tr!("{h} h", h = h).to_string(),
        (h, m) => tr!("{h} h {m} m", h = h, m = m).to_string(),
    }
}

fn count(value: usize) -> String {
    value.to_string()
}

fn days(value: usize) -> String {
    match value {
        1 => tr!("1 day").to_string(),
        n => tr!("{n} days", n = n).to_string(),
    }
}
