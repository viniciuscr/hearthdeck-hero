//! Settings → Watch: the Stremio account and the Watch home rails. The deep
//! theme editor stays in ☰ → Customize (scoped to the Watch space).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{ConsolePrefs, ShelfSpec};
use heronite_i18n::tr;
use heronite_watch::account::{create_link, read_link, LinkOffer, LinkPoll};
use heronite_watch::{Endpoints, UreqTransport};

use crate::collection::Space;
use crate::console::list::{Outcome, Row, Value};
use crate::console::themes;
use crate::fork::watch::watch_theme::watch_theme;
use crate::pages::settings::SettingsPage;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, toggle_row, ButtonKind};
use crate::ui::fields::meta_text;

mod qr;

pub struct State {
    linked: bool,
    busy: bool,
    offer: Option<LinkOffer>,
    notice: Option<SharedString>,
}

impl State {
    pub fn new(ctx: &PageCtx) -> Self {
        let linked = heronite_watch::store::read_auth(&ctx.app).ok().flatten().is_some();
        Self { linked, busy: false, offer: None, notice: None }
    }
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let linked = d.watch.linked;
    let busy = d.watch.busy;
    let mut actions = div().flex().items_center().gap(px(8.));
    if linked {
        actions =
            actions.child(action_button("watch-unlink", ButtonKind::Ghost, tr!("Unlink"), !busy, cx.listener(|d, _, _, cx| unlink(d, cx))));
    } else if d.watch.offer.is_some() {
        actions = actions.child(action_button(
            "watch-check",
            ButtonKind::Primary,
            tr!("Check"),
            !busy,
            cx.listener(|d, _, _, cx| check_link(d, cx)),
        ));
    } else {
        actions = actions.child(action_button(
            "watch-link",
            ButtonKind::Primary,
            tr!("Link with a code"),
            !busy,
            cx.listener(|d, _, _, cx| start_link(d, cx)),
        ));
    }

    let mut account = card_box().py(px(12.)).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(14.)).child(if linked { tr!("Linked") } else { tr!("Not linked") }))
                    .child(meta_text(tr!("Stremio account"))),
            )
            .child(actions),
    );
    if let Some(offer) = &d.watch.offer {
        let mut detail = div().flex().items_center().gap(px(16.)).p(px(12.)).rounded(px(10.)).bg(theme::ink_a(0.03));
        if let Some(qr) = qr::qr_code(&offer.link, 4.0) {
            detail = detail.child(qr);
        }
        detail = detail.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().text_size(px(13.)).child(tr!("Code {code}", code = offer.code.clone())))
                .child(meta_text(tr!("Scan the code with the Stremio app, or open {link}", link = offer.link.clone()))),
        );
        account = div().flex().flex_col().gap(px(8.)).child(account).child(detail);
    }

    let prefs = themes::prefs_for(&d.ctx.app, Space::Watch);
    let rails = card([
        toggle_row(
            "watch-rail-recent",
            tr!("Continue watching"),
            Some(tr!("Films and series you have started.")),
            rail_on(&prefs, "recent"),
        )
        .on_click(cx.listener(|d, _, _, cx| toggle_rail(d, "recent", cx)))
        .into_any_element(),
        toggle_row(
            "watch-rail-top-movies",
            tr!("Top movies"),
            Some(tr!("Films from the provider's catalog.")),
            rail_on(&prefs, "provider:top-movies"),
        )
        .on_click(cx.listener(|d, _, _, cx| toggle_rail(d, "provider:top-movies", cx)))
        .into_any_element(),
        toggle_row(
            "watch-rail-top-series",
            tr!("Top series"),
            Some(tr!("Series from the provider's catalog.")),
            rail_on(&prefs, "provider:top-series"),
        )
        .on_click(cx.listener(|d, _, _, cx| toggle_rail(d, "provider:top-series", cx)))
        .into_any_element(),
    ]);

    let reset = action_button(
        "watch-reset",
        ButtonKind::Glass,
        tr!("Reset Watch theme"),
        true,
        cx.listener(|d, _, _, cx| {
            themes::update_for(&d.ctx.app, Space::Watch, |prefs| {
                prefs.theme = watch_theme();
                prefs.based_on = "watch".into();
                prefs.last_tab = 0;
            });
            cx.notify();
        }),
    );

    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(caption(tr!("Stremio account")))
        .child(account)
        .child(caption(tr!("Home rails")))
        .child(
            div()
                .px(px(6.))
                .pb(px(8.))
                .child(meta_text(tr!("Choose what the Watch home shows. The console uses this theme; ☰ → Customize edits it in detail."))),
        )
        .child(rails)
        .child(div().flex().items_center().gap(px(8.)).child(reset));
    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

fn rail_on(prefs: &ConsolePrefs, kind: &str) -> bool {
    prefs.theme.shelves.iter().any(|shelf| shelf.kind == kind && shelf.on)
}

fn toggle_rail(d: &mut SettingsPage, kind: &str, cx: &mut Context<SettingsPage>) {
    themes::update_for(&d.ctx.app, Space::Watch, |prefs| {
        if let Some(shelf) = prefs.theme.shelves.iter_mut().find(|shelf| shelf.kind == kind) {
            shelf.on = !shelf.on;
        } else {
            prefs.theme.shelves.push(ShelfSpec { kind: kind.into(), on: true, title: rail_title(kind) });
        }
    });
    cx.notify();
}

fn rail_title(kind: &str) -> String {
    match kind {
        "recent" => tr!("Continue watching").to_string(),
        "provider:top-movies" => tr!("Top movies").to_string(),
        "provider:top-series" => tr!("Top series").to_string(),
        _ => kind.to_string(),
    }
}

fn start_link(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.watch.busy {
        return;
    }
    d.watch.busy = true;
    d.watch.notice = None;
    d.watch.offer = None;
    cx.notify();
    let link = Endpoints::default().link;
    cx.spawn(async move |this, cx| {
        let result = cx.background_spawn(async move { create_link(&UreqTransport::new(), &link) }).await;
        this.update(cx, |d, cx| {
            d.watch.busy = false;
            match result {
                Ok(offer) => d.watch.offer = Some(offer),
                Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn check_link(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let Some(offer) = d.watch.offer.clone() else { return };
    if d.watch.busy {
        return;
    }
    d.watch.busy = true;
    d.watch.notice = None;
    cx.notify();
    let link = Endpoints::default().link;
    let code = offer.code;
    cx.spawn(async move |this, cx| {
        let result = cx.background_spawn(async move { read_link(&UreqTransport::new(), &link, &code) }).await;
        this.update(cx, |d, cx| {
            d.watch.busy = false;
            match result {
                Ok(LinkPoll::Linked(key)) => match heronite_watch::store::write_auth(&d.ctx.app, &key) {
                    Ok(()) => {
                        d.watch.linked = true;
                        d.watch.offer = None;
                        d.watch.notice = None;
                    }
                    Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
                },
                Ok(LinkPoll::Pending) => d.watch.notice = Some(tr!("Not linked yet").into()),
                Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn unlink(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    match heronite_watch::store::clear_auth(&d.ctx.app) {
        Ok(()) => {
            d.watch.linked = false;
            d.watch.offer = None;
            d.watch.notice = None;
        }
        Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
    }
    cx.notify();
}

/// Console mode: the account, the code and the rails as rows.
pub(crate) fn console_rows(d: &SettingsPage) -> Vec<Row> {
    let mut rows = Vec::new();
    rows.push(Row::info("watch-account", tr!("Stremio account"), if d.watch.linked { tr!("Linked") } else { tr!("Not linked") }));
    if d.watch.linked {
        rows.push(Row::action("watch-unlink", tr!("Unlink")));
    } else if let Some(offer) = &d.watch.offer {
        rows.push(Row::info("watch-code", tr!("Code {code}", code = offer.code.clone()), offer.link.clone()));
        rows.push(Row::action("watch-check", tr!("Check")));
    } else {
        rows.push(Row::action("watch-link", tr!("Link with a code")));
    }
    rows.push(Row::header(tr!("Home rails")));
    let prefs = themes::prefs_for(&d.ctx.app, Space::Watch);
    rows.push(Row::toggle("watch-rail:recent", tr!("Continue watching"), rail_on(&prefs, "recent")));
    rows.push(Row::toggle("watch-rail:provider:top-movies", tr!("Top movies"), rail_on(&prefs, "provider:top-movies")));
    rows.push(Row::toggle("watch-rail:provider:top-series", tr!("Top series"), rail_on(&prefs, "provider:top-series")));
    rows.push(Row::action("watch-reset", tr!("Reset Watch theme")));
    rows
}

pub(crate) fn console_apply(d: &mut SettingsPage, outcome: Outcome, cx: &mut Context<SettingsPage>) {
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            "watch-link" => start_link(d, cx),
            "watch-check" => check_link(d, cx),
            "watch-unlink" => unlink(d, cx),
            "watch-reset" => {
                themes::update_for(&d.ctx.app, Space::Watch, |prefs| {
                    prefs.theme = watch_theme();
                    prefs.based_on = "watch".into();
                    prefs.last_tab = 0;
                });
                cx.notify();
            }
            _ => {}
        },
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-rail:") => {
            let kind = id.trim_start_matches("watch-rail:");
            set_rail(d, kind, on, cx);
        }
        _ => {}
    }
}

fn set_rail(d: &mut SettingsPage, kind: &str, on: bool, cx: &mut Context<SettingsPage>) {
    themes::update_for(&d.ctx.app, Space::Watch, |prefs| {
        if let Some(shelf) = prefs.theme.shelves.iter_mut().find(|shelf| shelf.kind == kind) {
            shelf.on = on;
        } else {
            prefs.theme.shelves.push(ShelfSpec { kind: kind.into(), on, title: rail_title(kind) });
        }
    });
    cx.notify();
}
