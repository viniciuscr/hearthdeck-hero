//! Settings → Watch: the Stremio account and the Watch home rails. The deep
//! theme editor stays in ☰ → Customize (scoped to the Watch space).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::console_theme::{ConsolePrefs, ShelfSpec};
use heronite_i18n::tr;
use heronite_watch::account::{create_link, read_link, LinkOffer, LinkPoll};
use heronite_watch::{Endpoints, UreqTransport};

use crate::collection::Space;
use crate::console::layers::{self, HasLayer};
use crate::console::list::{Outcome, Row, Value};
use crate::console::themes;
use crate::fork::watch::prefs::default_theme as watch_default_theme;
use crate::pages::settings::SettingsPage;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, toggle_row, ButtonKind};
use crate::ui::fields::meta_text;

pub mod qr;

pub mod registration;

pub use registration as watch_registration;

pub struct State {
    linked: bool,
    busy: bool,
    offer: Option<LinkOffer>,
    notice: Option<SharedString>,
    /// Whether a TMDB key is stored: read once, because Settings must not touch
    /// the credential file on every frame, and updated when it changes.
    pub tmdb_key: bool,
    /// The collection rails the build declares, with their source: read once for
    /// the same reason — asking a source to declare builds a transport, which a
    /// frame must not pay.
    pub collections: Vec<(String, heronite_watch::collections::CollectionDescriptor)>,
}

impl State {
    pub fn new(ctx: &PageCtx) -> Self {
        let linked = heronite_watch::store::read_auth(&ctx.app).ok().flatten().is_some();
        let tmdb_key = heronite_watch::store::read_tmdb_key(&ctx.app).ok().flatten().is_some();
        let collections = heronite_watch::collections::descriptors(&ctx.app);
        Self { linked, busy: false, offer: None, notice: None, tmdb_key, collections }
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
    let rails = card([toggle_row(
        "watch-rail-recent",
        tr!("Continue watching"),
        Some(tr!("Films and series you have started.")),
        rail_on(&prefs, "recent"),
    )
    .on_click(cx.listener(|d, _, _, cx| toggle_rail(d, "recent", cx)))
    .into_any_element()]);

    let settings = heronite_watch::settings::read(&d.ctx.app);
    let service = card([toggle_row(
        "watch-service-stop",
        tr!("Stop the Stremio service after watching"),
        Some(tr!("Frees its memory; it is started again when you play a torrent.")),
        settings.stop_service_when_done,
    )
    .on_click(cx.listener(|d, _, _, cx| toggle_service(d, cx)))
    .into_any_element()]);

    let reset = action_button(
        "watch-reset",
        ButtonKind::Glass,
        tr!("Reset Watch theme"),
        true,
        cx.listener(|d, _, _, cx| {
            themes::update_for(&d.ctx.app, Space::Watch, |prefs| {
                prefs.theme = watch_default_theme();
                prefs.based_on = crate::fork::watch::prefs::base_id().into();
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
        .child(caption(tr!("Stremio service")))
        .child(service)
        .child(div().flex().items_center().gap(px(8.)).child(reset));
    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

/// Settings → Watch → Metadata providers: what describes a title, asked in the
/// order they are listed.
pub fn render_metadata(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let rows: Vec<AnyElement> = heronite_watch::metadata::sources()
        .iter()
        .map(|source| {
            let descriptor = source.descriptor();
            let on = settings.metadata_sources.iter().any(|id| id == descriptor.id);
            toggle_row(format!("watch-metadata-{}", descriptor.id), SharedString::from(descriptor.name.to_owned()), None, on)
                .on_click(cx.listener(move |d, _, _, cx| set_metadata_source(d, descriptor.id, !on, cx)))
                .into_any_element()
        })
        .collect();
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(caption(tr!("Metadata providers")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "What describes a film or a series on its page. They are asked in this order: the first answer for a field wins, the next fills what is still empty."
        ))))
        .child(card(rows));
    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

/// Settings → Watch → Metadata providers, as rows.
pub(crate) fn metadata_rows(d: &SettingsPage) -> Vec<Row> {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let mut rows = vec![Row::header(tr!("Metadata providers"))];
    for source in heronite_watch::metadata::sources() {
        let descriptor = source.descriptor();
        let on = settings.metadata_sources.iter().any(|id| id == descriptor.id);
        rows.push(Row::toggle(
            SharedString::from(format!("watch-metadata:{}", descriptor.id)),
            SharedString::from(descriptor.name.to_owned()),
            on,
        ));
    }
    rows.push(Row::info(
        "watch-metadata-order",
        tr!("Order"),
        tr!("The first answer for a field wins; the next fills what is still empty."),
    ));
    rows
}

/// The metadata page's rows: the source toggles.
pub(crate) fn metadata_apply(d: &mut SettingsPage, outcome: Outcome, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    if let Outcome::Changed(id, Value::Toggle(on)) = outcome {
        if let Some(source) = id.strip_prefix("watch-metadata:") {
            set_metadata_source(d, source, on, cx);
        }
    }
}

/// Turns a metadata source on or off. The list is also the priority order, so a
/// source turned on joins at the end — behind the one that answers first.
fn set_metadata_source(d: &mut SettingsPage, source: &str, on: bool, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    if on {
        if !settings.metadata_sources.iter().any(|id| id == source) {
            settings.metadata_sources.push(source.to_owned());
        }
    } else {
        settings.metadata_sources.retain(|id| id != source);
    }
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    cx.notify();
}

/// Settings → Watch → Catalog providers: what the Movies and Series tabs may
/// browse. Nothing here can change how a title plays.
pub fn render_catalogs(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let source_rows: Vec<AnyElement> = heronite_watch::catalog::sources()
        .iter()
        .map(|source| {
            let descriptor = source.descriptor();
            let on = settings.catalog_sources.iter().any(|id| id == descriptor.id);
            toggle_row(format!("watch-catalog-{}", descriptor.id), SharedString::from(descriptor.name.to_owned()), None, on)
                .on_click(cx.listener(move |d, _, _, cx| set_catalog_source(d, descriptor.id, !on, cx)))
                .into_any_element()
        })
        .collect();
    let sources = card(source_rows);

    let keyed = d.watch.tmdb_key;
    let key_buttons = {
        let mut buttons = div().flex().items_center().gap(px(8.));
        buttons = buttons.child(action_button(
            "watch-tmdb-key-paste",
            ButtonKind::Primary,
            tr!("Paste the key"),
            true,
            cx.listener(|d, _, _, cx| paste_tmdb_key(d, cx)),
        ));
        buttons = buttons.child(action_button(
            "watch-tmdb-key",
            ButtonKind::Glass,
            tr!("Type it instead"),
            true,
            cx.listener(|d, _, window, cx| set_tmdb_key(d, window, cx)),
        ));
        if keyed {
            buttons = buttons.child(action_button(
                "watch-tmdb-key-remove",
                ButtonKind::Ghost,
                tr!("Remove"),
                true,
                cx.listener(|d, _, _, cx| clear_tmdb_key(d, cx)),
            ));
        }
        buttons
    };
    let key = card_box().py(px(12.)).child(
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
                    .child(div().text_size(px(14.)).child(tr!("TMDB API key")))
                    .child(meta_text(tr!("Needed to browse TMDB's catalogs. Copy it on another device and paste it here: no typing."))),
            )
            .child(key_buttons),
    );

    // The collection rails: the source order is the rail order, and the ids are
    // the sources' own, so a settings file stays readable. The list is read with
    // the page (a source's declaration costs a transport), not per frame.
    let mut collections: Vec<AnyElement> = Vec::new();
    for (source, rail) in &d.watch.collections {
        let on = settings.collections.contains(&rail.id);
        let note = rail_note(d, source);
        let label = SharedString::from(crate::fork::watch::rails::title(rail));
        let id = rail.id.clone();
        collections.push(
            toggle_row(format!("watch-collection-{id}"), label, Some(note.as_str()), on)
                .on_click(cx.listener(move |d, _, _, cx| set_collection(d, &id, !on, cx)))
                .into_any_element(),
        );
    }

    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(caption(tr!("Catalog providers")))
        .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!(
            "What the Movies and Series tabs may browse. A catalog decides what is listed; it never changes what plays."
        ))))
        .child(sources)
        .child(key);
    if !collections.is_empty() {
        column = column
            .child(caption(tr!("Collections")))
            .child(div().px(px(6.)).pb(px(8.)).child(meta_text(tr!("Rails the first page draws, in this order."))))
            .child(card(collections));
    }
    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

fn rail_on(prefs: &ConsolePrefs, kind: &str) -> bool {
    prefs.theme.shelves.iter().any(|shelf| shelf.kind == kind && shelf.on)
}

/// Whether the app can see that a collection source has its key. A source whose
/// key the app does not track is taken as keyed: its rows say nothing.
fn source_keyed(d: &SettingsPage, source: &str) -> bool {
    match source {
        "tmdb" => d.watch.tmdb_key,
        _ => true,
    }
}

/// What a collection row says under its name: where the rail is drawn, or the key
/// its source still needs.
fn rail_note(d: &SettingsPage, source: &str) -> String {
    if source_keyed(d, source) {
        return tr!("Shown on the first page.").to_string();
    }
    let name = heronite_watch::collections::source_name(source);
    tr!("{source} has no key yet: add it above, or this rail stays empty.", source = name).to_string()
}

/// Turns a collection rail on or off. The list is also the rail order: a rail
/// turned on joins at the end, so a new one is drawn last. The rails are resolved
/// with the library, so the Collection looks again.
fn set_collection(d: &mut SettingsPage, id: &str, on: bool, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    toggled(&mut settings.collections, id, on);
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    cx.emit(crate::pages::PageEvent::LibraryChanged);
    cx.notify();
}

/// The enabled rails after a toggle: a rail turned on joins at the end (the list
/// is the rail order), and one already there keeps its place — a press must not
/// reorder the tab.
fn toggled(collections: &mut Vec<String>, id: &str, on: bool) {
    if on {
        if !collections.iter().any(|rail| rail == id) {
            collections.push(id.to_owned());
        }
    } else {
        collections.retain(|rail| rail != id);
    }
}

/// Turns a catalog source on or off. A catalog decides what the tabs list; it
/// never touches the stream side, so nothing about playback changes.
fn set_catalog_source(d: &mut SettingsPage, source: &str, on: bool, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    if on {
        if !settings.catalog_sources.iter().any(|id| id == source) {
            settings.catalog_sources.push(source.to_owned());
        }
    } else {
        settings.catalog_sources.retain(|id| id != source);
    }
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    crate::collection::watch_catalog::catalogs_changed();
    cx.notify();
}

/// The keyboard, for a key typed by hand. It opens empty: the stored key is a
/// secret and this screen is a television.
fn set_tmdb_key(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) {
    layers::open_osk(d.layer_mut(), tr!("TMDB API key"), "", window, cx, |d, text, _, cx| {
        save_tmdb_key(d, &text, cx);
    });
}

/// The key from the clipboard: no typing, which is the only workable way with a
/// gamepad. A person copies the token on their phone or computer and pastes it
/// here.
fn paste_tmdb_key(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let text = cx.read_from_clipboard().and_then(|item| item.text()).map(|text| text.to_owned()).unwrap_or_default();
    if text.trim().is_empty() {
        d.watch.notice = Some(tr!("The clipboard has no text: copy the key first, then paste it here.").into());
        cx.notify();
        return;
    }
    save_tmdb_key(d, &text, cx);
}

/// Stores a pasted or typed key and says what happened.
fn save_tmdb_key(d: &mut SettingsPage, text: &str, cx: &mut Context<SettingsPage>) {
    let key = text.trim();
    if key.is_empty() {
        d.watch.notice = Some(tr!("That key is empty.").into());
        cx.notify();
        return;
    }
    match heronite_watch::store::write_tmdb_key(&d.ctx.app, key) {
        Ok(()) => {
            d.watch.tmdb_key = true;
            // A source's rails can depend on its key, so the list is read again.
            d.watch.collections = heronite_watch::collections::descriptors(&d.ctx.app);
            d.watch.notice = Some(tr!("The TMDB API key was saved.").into());
            crate::collection::watch_catalog::catalogs_changed();
            // The collection rails are resolved with the library: the ones that
            // were waiting for this key are drawn from now on.
            cx.emit(crate::pages::PageEvent::LibraryChanged);
        }
        Err(error) => {
            log::warn!("saving the TMDB key: {error:#}");
            d.watch.notice = Some(tr!("The TMDB API key could not be saved.").into());
        }
    }
    cx.notify();
}

fn clear_tmdb_key(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if let Err(error) = heronite_watch::store::clear_tmdb_key(&d.ctx.app) {
        log::warn!("clearing the TMDB key: {error:#}");
    }
    d.watch.tmdb_key = false;
    d.watch.collections = heronite_watch::collections::descriptors(&d.ctx.app);
    crate::collection::watch_catalog::catalogs_changed();
    // What that key was resolving (the collection rails, TMDB's catalogs) is no
    // longer there to draw.
    cx.emit(crate::pages::PageEvent::LibraryChanged);
    cx.notify();
}

fn toggle_service(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    settings.stop_service_when_done = !settings.stop_service_when_done;
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    cx.notify();
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
    rows.push(Row::action("watch-reset", tr!("Reset Watch theme")));
    rows.push(Row::header(tr!("Stremio service")));
    let settings = heronite_watch::settings::read(&d.ctx.app);
    rows.push(Row::toggle("watch-service-stop", tr!("Stop the Stremio service after watching"), settings.stop_service_when_done));
    rows
}

/// Settings → Watch → Catalog providers, as rows.
pub(crate) fn catalog_rows(d: &SettingsPage) -> Vec<Row> {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let mut rows = vec![Row::header(tr!("Catalog providers"))];
    for source in heronite_watch::catalog::sources() {
        let descriptor = source.descriptor();
        let on = settings.catalog_sources.iter().any(|id| id == descriptor.id);
        rows.push(Row::toggle(
            SharedString::from(format!("watch-catalog:{}", descriptor.id)),
            SharedString::from(descriptor.name.to_owned()),
            on,
        ));
    }
    let keyed = d.watch.tmdb_key;
    rows.push(Row::info("watch-tmdb-key", tr!("TMDB API key"), if keyed { tr!("Set") } else { tr!("Not set") }));
    rows.push(Row::action("watch-tmdb-key-paste", tr!("Paste the key from the clipboard")));
    rows.push(Row::action("watch-tmdb-key-set", tr!("Type it instead")));
    if keyed {
        rows.push(Row::action("watch-tmdb-key-remove", tr!("Remove the TMDB API key")));
    }
    // The collection rails: one row each, in the order they are drawn.
    if !d.watch.collections.is_empty() {
        rows.push(Row::header(tr!("Collections")));
        for (source, rail) in &d.watch.collections {
            let on = settings.collections.contains(&rail.id);
            let title = SharedString::from(crate::fork::watch::rails::title(rail));
            let note = rail_note(d, source);
            rows.push(Row::toggle(SharedString::from(format!("watch-collection:{}", rail.id)), title, on).desc(note));
        }
        rows.push(Row::info("watch-collections-note", tr!("Collections"), tr!("Rails the first page draws, in this order.")));
    }
    rows.push(Row::info("watch-catalogs-note", tr!("Catalogs"), tr!("A catalog decides what is listed; it never changes what plays.")));
    rows
}

/// The catalog page's rows: the source toggles and the key.
pub(crate) fn catalog_apply(d: &mut SettingsPage, outcome: Outcome, window: &mut Window, cx: &mut Context<SettingsPage>) {
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            // A copied key needs one press: no typing on a couch.
            "watch-tmdb-key-paste" => paste_tmdb_key(d, cx),
            "watch-tmdb-key-set" => set_tmdb_key(d, window, cx),
            "watch-tmdb-key-remove" => clear_tmdb_key(d, cx),
            _ => {}
        },
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-catalog:") => {
            let source = id.trim_start_matches("watch-catalog:").to_owned();
            set_catalog_source(d, &source, on, cx);
        }
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-collection:") => {
            let rail = id.trim_start_matches("watch-collection:").to_owned();
            set_collection(d, &rail, on, cx);
        }
        _ => {}
    }
}

pub(crate) fn console_apply(d: &mut SettingsPage, outcome: Outcome, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            "watch-link" => start_link(d, cx),
            "watch-check" => check_link(d, cx),
            "watch-unlink" => unlink(d, cx),
            "watch-reset" => {
                themes::update_for(&d.ctx.app, Space::Watch, |prefs| {
                    prefs.theme = watch_default_theme();
                    prefs.based_on = crate::fork::watch::prefs::base_id().into();
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
        Outcome::Changed(id, Value::Toggle(on)) if id.as_ref() == "watch-service-stop" => {
            let mut settings = heronite_watch::settings::read(&d.ctx.app);
            settings.stop_service_when_done = on;
            if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
                log::warn!("saving the Watch settings: {error:#}");
            }
            cx.notify();
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

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    #[test]
    fn a_rail_joins_the_end_of_the_order_and_leaves_it() {
        let mut collections = vec!["tmdb:top-rated:movie".to_owned()];
        toggled(&mut collections, "tmdb:popular:movie", true);
        assert_eq!(collections, ["tmdb:top-rated:movie", "tmdb:popular:movie"], "a new rail is drawn last");

        toggled(&mut collections, "tmdb:popular:movie", true);
        assert_eq!(collections.len(), 2, "pressing it on twice does not move it");

        toggled(&mut collections, "tmdb:top-rated:movie", false);
        assert_eq!(collections, ["tmdb:popular:movie"], "the one left keeps its place");

        toggled(&mut collections, "gone", false);
        assert_eq!(collections, ["tmdb:popular:movie"], "an id the build does not have is not a fault");
    }
}
