//! The Watch space's Statistics page: the recap, drawn where the games' stats
//! are.
//!
//! `☰ → Statistics` (and the sidebar's Statistics in the desktop window) shows
//! this while the Watch space is on screen, and the games' page otherwise — the
//! same space switch the Collection already does. The numbers come from
//! `heronite-watch-stats`; `Settings → Watch → Recap` draws the same thing beside
//! the source switches.
//!
//! Deliberately one screen, not a scroll: the window, the totals, the bars and
//! the titles that took the time. A couch reads it with LB / RB.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::paths::AppPaths;
use heronite_i18n::tr;
use heronite_watch::stats::{months, Recap};

use crate::console::kit::{self, u};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::fork::watch::settings::recap::{self, Recency};
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::caption;
use crate::ui::fields::meta_text;

pub struct WatchStatsPage {
    app: AppPaths,
    recency: Recency,
    /// Computed once per window: the log is a file read and a pass over it.
    recap: Option<Recap>,
}

impl WatchStatsPage {
    pub fn new(ctx: &PageCtx) -> Self {
        Self { app: ctx.app.clone(), recency: Recency::default(), recap: None }
    }

    /// The window's numbers, read the first time it is shown and again when the
    /// window changes.
    fn recap(&mut self) -> &Recap {
        if self.recap.as_ref().is_none_or(|recap| recap.period.span != self.recency.span()) {
            self.recap = Some(heronite_watch::stats::recap(&self.app, self.recency.span()));
        }
        self.recap.as_ref().expect("just computed")
    }

    fn pick(&mut self, recency: Recency, cx: &mut Context<Self>) {
        if self.recency == recency {
            return;
        }
        self.recency = recency;
        self.recap = None;
        cx.notify();
    }
}

impl Render for WatchStatsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let recency = self.recency;
        let recap = self.recap().clone();
        let mut chips = div().flex().items_center().gap(u(8.));
        for chip in Recency::ALL {
            let chosen = chip == recency;
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("watch-stats-{}", chip.index())))
                    .px(u(12.))
                    .py(u(5.))
                    .rounded_full()
                    .border_1()
                    .border_color(if chosen { theme::accent_tint_border() } else { theme::ink_a(0.08) })
                    .bg(if chosen { theme::accent_tint() } else { theme::ink_a(0.03) })
                    .text_size(u(13.))
                    .cursor_pointer()
                    .child(tr(chip.name()))
                    .on_click(cx.listener(move |page, _, _, cx| page.pick(chip, cx))),
            );
        }

        let body = recap::body(&recap, recency);
        div()
            .id("watch-stats")
            .size_full()
            .overflow_y_scroll()
            .p(u(46.))
            .flex()
            .flex_col()
            .gap(u(14.))
            .child(kit::title(tr!("Recap"), 30.))
            .child(chips)
            .child(meta_text(recap::blurb().to_string()))
            .children(body)
            .children((recap.totals.sessions == 0).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(u(4.))
                    .child(caption(tr!("Nothing watched in this window yet.")))
                    .child(meta_text(tr!("Sessions are recorded from now on, so a new log starts empty.")))
            }))
    }
}

impl PadTarget for WatchStatsPage {
    fn on_pad(&mut self, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        match pad {
            // LB / RB walk the windows, like the tabs do elsewhere.
            Pad::LB | Pad::RB => {
                let at = self.recency.index();
                let len = Recency::ALL.len();
                let next = if pad == Pad::RB { (at + 1) % len } else { (at + len - 1) % len };
                self.pick(Recency::from_index(next), cx);
                true
            }
            _ => false,
        }
    }

    fn hints(&self, _cx: &App) -> Vec<Hint> {
        vec![hint(Btn::LB, tr!("Earlier window")), hint(Btn::RB, tr!("Later window"))]
    }
}

/// The months of a year, for a caller that wants them (the page draws its own).
#[allow(dead_code)]
fn year_months(recap: &Recap) -> Vec<(u32, f64)> {
    months(&recap.days)
}
