//! Settings → Watch: the Stremio account and the Watch home rails. The deep
//! theme editor stays in ☰ → Customize (scoped to the Watch space).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_watch::account::{create_link, read_link, LinkOffer, LinkPoll};
use heronite_watch::{Endpoints, UreqTransport};

use crate::collection::Space;
use crate::console::layers::HasLayer;
use crate::console::list::{Outcome, Row, Value};
use crate::console::themes;
use crate::fork::watch::prefs::default_theme as watch_default_theme;
use crate::pages::settings::SettingsPage;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, card_box, toggle_row, ButtonKind};
use crate::ui::fields::meta_text;

pub mod qr;
mod qr_overlay;
pub mod recap;

pub mod registration;

pub use registration as watch_registration;

pub struct State {
    linked: bool,
    busy: bool,
    offer: Option<LinkOffer>,
    notice: Option<SharedString>,
    /// Whether TMDB can be used in this build: it carries the application key
    /// (`HEARTHDECK_TMDB_API_KEY`), or an older build stored one. The person is
    /// never asked for it.
    pub tmdb_ready: bool,
    /// The TMDB account signed in on this machine, by name (empty when TMDB did
    /// not say, `None` when there is no session).
    pub tmdb_username: Option<SharedString>,
    /// The request token waiting for the person to approve it in their browser.
    pub tmdb_pending: Option<String>,
    /// The window the recap is showing.
    pub recap: recap::Recency,
    /// The collection rails the build declares, with their source: read once for
    /// the same reason — asking a source to declare builds a transport, which a
    /// frame must not pay.
    pub collections: Vec<(String, heronite_watch::collections::CollectionDescriptor)>,
}

impl State {
    pub fn new(ctx: &PageCtx) -> Self {
        let linked = heronite_watch::store::read_auth(&ctx.app).ok().flatten().is_some();
        let tmdb_ready = heronite_watch::tmdb_sign_in::configured(&ctx.app);
        let tmdb_username = heronite_watch::tmdb_sign_in::session(&ctx.app).map(|session| SharedString::from(session.username));
        let collections = heronite_watch::collections::descriptors(&ctx.app);
        Self {
            linked,
            busy: false,
            offer: None,
            notice: None,
            tmdb_ready,
            tmdb_username,
            tmdb_pending: None,
            recap: recap::Recency::default(),
            collections,
        }
    }
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
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
    let theme = card_box().py(px(12.)).child(
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
                    .child(div().text_size(px(14.)).child(tr!("Watch theme")))
                    .child(meta_text(tr!("Colours, layouts and tabs. ☰ → Customize edits the same theme from the couch."))),
            )
            .child(reset),
    );
    let mut column = div().flex().flex_col().gap(px(14.)).child(caption(tr!("Appearance"))).child(theme);
    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

/// Settings → Watch → Home: the rails the first page draws, each with the
/// provider it comes from. This list is the drawing order, and the only owner of
/// that page: a theme's shelves do not apply in Watch.
pub fn render_home(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let on = |id: &str| settings.home_rails.iter().any(|rail| rail == id);
    let mut column = div().flex().flex_col().gap(px(14.)).child(caption(tr!("Home"))).child(div().px(px(6.)).child(meta_text(tr!(
        "What the first page shows. They are drawn in the order they were turned on, and a rail with nothing in it is hidden."
    ))));

    let media: Vec<Provider> = providers().into_iter().filter(|provider| provider.media).collect();
    if !media.is_empty() {
        let rows: Vec<AnyElement> = media
            .iter()
            .map(|provider| {
                let id = format!("continue:{}", provider.id);
                let label = tr!("Continue watching · {provider}", provider = provider.name.clone());
                let label: SharedString = SharedString::from(label);
                toggle_row(format!("watch-home:{id}"), label, Some(tr!("What you have started there.")), on(&id))
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |d, _, _, cx| set_home_rail(d, &id, None, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        column = column.child(caption(tr!("Continue watching"))).child(card(rows));
    }

    for provider in providers().into_iter().filter(|provider| provider.collections) {
        let rails: Vec<AnyElement> = d
            .watch
            .collections
            .iter()
            .filter(|(source, _)| *source == provider.id)
            .map(|(_, rail)| {
                let label = format!("{} · {}", crate::fork::watch::rails::title(rail), provider.name);
                let label: SharedString = SharedString::from(label);
                toggle_row(
                    format!("watch-home:{}", rail.id),
                    label,
                    Some(tr!("Recommended and ranked by {provider}.", provider = provider.name.clone()).as_str()),
                    on(&rail.id),
                )
                .on_click(cx.listener({
                    let id = rail.id.clone();
                    move |d, _, _, cx| set_home_rail(d, &id, None, cx)
                }))
                .into_any_element()
            })
            .collect();
        if !rails.is_empty() {
            column = column.child(caption(provider.name.clone())).child(card(rails));
        }
    }

    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

/// Settings → Watch → Recap: what was watched, over the chosen window.
pub fn render_recap(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    recap::render(d, window, cx)
}

/// Settings → Watch → Recap, as rows.
pub(crate) fn recap_rows(d: &SettingsPage) -> Vec<Row> {
    recap::rows(d)
}

/// The Recap page's choice.
pub(crate) fn recap_apply(d: &mut SettingsPage, outcome: Outcome, window: &mut Window, cx: &mut Context<SettingsPage>) {
    recap::apply(d, outcome, window, cx)
}

/// Settings → Watch → Home, as rows: the same rails as a list.
pub(crate) fn home_rows(d: &SettingsPage) -> Vec<Row> {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let on = |id: &str| settings.home_rails.iter().any(|rail| rail == id);
    let mut rows = vec![Row::header(tr!("Home"))];
    for provider in providers().into_iter().filter(|provider| provider.media) {
        let id = format!("continue:{}", provider.id);
        let label = tr!("Continue watching · {provider}", provider = provider.name);
        rows.push(Row::toggle(SharedString::from(format!("watch-home:{id}")), SharedString::from(label), on(&id)));
    }
    for (source, rail) in &d.watch.collections {
        let name = heronite_watch::collections::source_name(source);
        let label = format!("{} · {name}", crate::fork::watch::rails::title(rail));
        rows.push(Row::toggle(SharedString::from(format!("watch-home:{}", rail.id)), SharedString::from(label), on(&rail.id)));
    }
    rows.push(Row::info("watch-home-note", tr!("Order"), tr!("Rails are drawn in the order they were turned on.")));
    rows
}

/// The Home page's toggles.
pub(crate) fn home_apply(d: &mut SettingsPage, outcome: Outcome, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    if let Outcome::Changed(id, Value::Toggle(on)) = outcome {
        if let Some(rail) = id.strip_prefix("watch-home:") {
            set_home_rail(d, rail, Some(on), cx);
        }
    }
}

/// Turns a rail on or off. The list is also the drawing order, so a rail turned
/// on joins at the end.
fn set_home_rail(d: &mut SettingsPage, id: &str, on: Option<bool>, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    let want = on.unwrap_or(!settings.home_rails.iter().any(|rail| rail == id));
    toggled(&mut settings.home_rails, id, want);
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    // The first page's rails are read with the library.
    cx.emit(crate::pages::PageEvent::LibraryChanged);
    cx.notify();
}

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

/// One provider as Settings shows it: what it is, and what it can do. Generated
/// from the registries, so a provider this build gains needs no Settings code.
#[derive(Debug)]
struct Provider {
    id: String,
    name: String,
    /// Owns titles, streams, progress and the person's account.
    media: bool,
    /// May be browsed by the Movies and Series tabs.
    catalog: bool,
    /// Describes a title on its page.
    metadata: bool,
    /// Declares rails for the first page.
    collections: bool,
}

/// Every provider Watch has, the streaming services first. An id that is more
/// than one kind of source is one provider.
fn providers() -> Vec<Provider> {
    let mut out: Vec<Provider> = Vec::new();
    let mut add = |id: &str, name: &str, set: fn(&mut Provider)| match out.iter_mut().find(|provider| provider.id == id) {
        Some(provider) => set(provider),
        None => {
            let mut provider =
                Provider { id: id.to_owned(), name: name.to_owned(), media: false, catalog: false, metadata: false, collections: false };
            set(&mut provider);
            out.push(provider);
        }
    };
    for provider in heronite_watch::registry::providers() {
        add(provider.id(), provider.name(), |entry| entry.media = true);
    }
    for source in heronite_watch::catalog::sources() {
        let descriptor = source.descriptor();
        add(descriptor.id, descriptor.name, |entry| entry.catalog = true);
    }
    for source in heronite_watch::metadata::sources() {
        let descriptor = source.descriptor();
        add(descriptor.id, descriptor.name, |entry| entry.metadata = true);
    }
    for source in heronite_watch::collections::sources() {
        let descriptor = source.descriptor();
        add(descriptor.id, descriptor.name, |entry| entry.collections = true);
    }
    // The streaming services first: they hold the person's account, and the
    // sources only fill in what those provide.
    out.sort_by_key(|provider| !provider.media);
    out
}

/// What a provider gives, in one line under its name.
fn capabilities(provider: &Provider) -> String {
    let mut parts = Vec::new();
    if provider.media {
        parts.push(tr!("Titles and streams"));
    }
    if provider.catalog {
        parts.push(tr!("Catalogs"));
    }
    if provider.metadata {
        parts.push(tr!("Metadata"));
    }
    if provider.collections {
        parts.push(tr!("Rails"));
    }
    parts.join(" \u{b7} ")
}

/// Settings → Watch → Providers: one card per provider and nothing else. What
/// each provider offers the screens is set on the Home page.
pub fn render_providers(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let all = providers();
    let (media, sources): (Vec<Provider>, Vec<Provider>) = all.into_iter().partition(|provider| provider.media);

    let mut column = div().flex().flex_col().gap(px(14.));
    if !media.is_empty() {
        column = column
            .child(caption(tr!("Streaming services")))
            .child(div().px(px(6.)).child(meta_text(tr!("They own the titles, the streams and where you stopped."))));
        for provider in &media {
            column = column.child(provider_card(d, provider, &settings, cx));
        }
    }
    if !sources.is_empty() {
        column = column.child(caption(tr!("Sources"))).child(div().px(px(6.)).child(meta_text(tr!(
            "What the tabs browse and what describes a title. Metadata sources are asked in this order: the first answer for a field wins."
        ))));
        for provider in &sources {
            column = column.child(provider_card(d, provider, &settings, cx));
        }
    }
    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

/// One provider's card: its name and what it gives, then its account or key and
/// the one switch that turns it on.
fn provider_card(
    d: &mut SettingsPage,
    provider: &Provider,
    settings: &heronite_watch::settings::Settings,
    cx: &mut Context<SettingsPage>,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    if provider.catalog {
        let on = settings.catalog_sources.iter().any(|id| id == &provider.id);
        let id = provider.id.clone();
        rows.push(
            toggle_row(
                format!("watch-catalog-{}", provider.id),
                SharedString::from(tr!("Browse its catalogs")),
                Some(tr!("The Movies and Series tabs may page through what it lists.")),
                on,
            )
            .on_click(cx.listener(move |d, _, _, cx| set_catalog_source(d, &id, !on, cx)))
            .into_any_element(),
        );
    }
    if provider.metadata {
        let on = settings.metadata_sources.iter().any(|id| id == &provider.id);
        let id = provider.id.clone();
        rows.push(
            toggle_row(
                format!("watch-metadata-{}", provider.id),
                SharedString::from(tr!("Describe titles")),
                Some(tr!("Its answer fills the film page; what another source already said stays.")),
                on,
            )
            .on_click(cx.listener(move |d, _, _, cx| set_metadata_source(d, &id, !on, cx)))
            .into_any_element(),
        );
    }
    if provider.media && provider.id == "stremio" {
        rows.push(stremio_account(d, cx));
        let settings = heronite_watch::settings::read(&d.ctx.app);
        rows.push(
            toggle_row(
                "watch-service-stop",
                SharedString::from(tr!("Stop the service after watching")),
                Some(tr!("Frees its memory; it is started again when you play a torrent.")),
                settings.stop_service_when_done,
            )
            .on_click(cx.listener(|d, _, _, cx| toggle_service(d, cx)))
            .into_any_element(),
        );
    }
    if provider.id == "tmdb" {
        rows.push(tmdb_card(d, cx));
    }
    card_box()
        .py(px(12.))
        .px(px(14.))
        .gap(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(14.)).child(provider.name.clone()))
                .child(meta_text(capabilities(provider))),
        )
        .children(rows)
        .into_any_element()
}

/// The Stremio account: linked or not, the code to link it, and the actions.
fn stremio_account(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
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
    let mut account = div().child(
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
                    .child(div().text_size(px(13.5)).child(if linked { tr!("Linked") } else { tr!("Not linked") }))
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
        account = account.child(detail);
    }
    account.into_any_element()
}

/// The TMDB credential: one Connect button, the approval, or who is signed in.
fn tmdb_card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let browse = tr!("Browsing uses this account's key. Signing out keeps the key, so the catalogs keep working.");
    let (title, desc) = match (&d.watch.tmdb_username, &d.watch.tmdb_pending) {
        (Some(name), _) if name.is_empty() => (tr!("Signed in to TMDB").to_string(), browse),
        (Some(name), _) => (tr!("Signed in as {name}", name = name.clone()).to_string(), browse),
        (None, Some(_)) => (
            tr!("Approve the sign-in in your browser, then come back.").to_string(),
            tr!("TMDB opened in your browser: there is nothing to type here."),
        ),
        (None, None) if d.watch.tmdb_ready => {
            (tr!("TMDB").to_string(), tr!("Connect and approve it on your phone: your watchlist and favourites then follow the account."))
        }
        (None, None) => (tr!("TMDB").to_string(), tr!("This build carries no TMDB API key: it is compiled with HEARTHDECK_TMDB_API_KEY.")),
    };
    let mut card = div().flex().flex_col().gap(px(10.)).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(div().flex().flex_col().gap(px(3.)).child(div().text_size(px(13.5)).child(title)).child(meta_text(desc)))
            .child(tmdb_buttons(d, cx)),
    );
    // Waiting for the approval: the same page as a QR, so a phone approves it
    // without the host's browser.
    if let Some(token) = d.watch.tmdb_pending.as_deref() {
        let url = heronite_watch::tmdb_sign_in::approve_url(token);
        let mut row = div().flex().items_center().gap(px(16.));
        if let Some(code) = qr::qr_code(&url, 3.0) {
            row = row.child(code);
        }
        row = row.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(meta_text(tr!("Scan it with a phone to approve, or open it here.")))
                .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(url)),
        );
        card = card.child(row);
    }
    card.into_any_element()
}

fn tmdb_buttons(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let mut buttons = div().flex().items_center().gap(px(8.));
    if d.watch.tmdb_username.is_some() {
        buttons = buttons.child(action_button(
            "watch-tmdb-sign-out",
            ButtonKind::Ghost,
            tr!("Sign out"),
            true,
            cx.listener(|d, _, _, cx| sign_out_tmdb(d, cx)),
        ));
    } else if d.watch.tmdb_pending.is_some() {
        buttons = buttons.child(action_button(
            "watch-tmdb-again",
            ButtonKind::Glass,
            tr!("Open the page again"),
            true,
            cx.listener(|d, _, _, cx| {
                if let Some(token) = d.watch.tmdb_pending.clone() {
                    open_approval(d, &token);
                }
                cx.notify();
            }),
        ));
        buttons = buttons.child(action_button(
            "watch-tmdb-approved",
            ButtonKind::Primary,
            tr!("I approved it"),
            true,
            cx.listener(|d, _, _, cx| check_tmdb_sign_in(d, cx)),
        ));
    } else {
        buttons = buttons.child(action_button(
            "watch-tmdb-connect",
            ButtonKind::Primary,
            tr!("Connect TMDB"),
            d.watch.tmdb_ready,
            cx.listener(|d, _, window, cx| connect_tmdb(d, window, cx)),
        ));
    }
    if d.watch.tmdb_username.is_some() {
        buttons = buttons.child(action_button(
            "watch-tmdb-forget",
            ButtonKind::Ghost,
            tr!("Forget TMDB"),
            true,
            cx.listener(|d, _, _, cx| clear_tmdb_key(d, cx)),
        ));
    }
    buttons
}

/// The one way in: ask for the key when there is none (the keyboard, or the
/// on-screen one in console mode), then send the person to TMDB's approval page.
fn connect_tmdb(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    if !d.watch.tmdb_ready {
        d.watch.notice = Some(tr!("This build carries no TMDB API key: it is compiled with HEARTHDECK_TMDB_API_KEY.").into());
        cx.notify();
        return;
    }
    start_tmdb_sign_in(d, cx);
}

/// Shows `url` as a QR code over the settings, for a phone to scan: console mode
/// has no other way to draw one, and a phone approves the link without the host's
/// browser.
fn open_qr(d: &mut SettingsPage, title: &str, url: &str, window: &mut Window, cx: &mut Context<SettingsPage>) {
    let overlay = cx.new(|_| qr_overlay::QrOverlay::new(title, url));
    let sub = cx.subscribe_in(&overlay, window, |d, _, event: &qr_overlay::QrEvent, _, cx| {
        if *event == qr_overlay::QrEvent::Close {
            d.layer_mut().close();
            cx.notify();
        }
    });
    d.layer_mut().open(&overlay, sub);
    cx.notify();
}

/// Opens TMDB's approval page for `token`; a browser that will not open is a
/// notice, not a failure of the sign-in.
fn open_approval(d: &mut SettingsPage, token: &str) {
    let url = heronite_watch::tmdb_sign_in::approve_url(token);
    if let Err(error) = heronite_platform::shell::open(&url) {
        d.watch.notice = Some(tr!("Could not open the browser: {error}", error = error.to_string()).into());
    }
}

/// Asks TMDB for a request token and sends the person to its page.
fn start_tmdb_sign_in(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    if !d.watch.tmdb_ready {
        d.watch.notice = Some(tr!("Add the TMDB API key first.").into());
        cx.notify();
        return;
    }
    match heronite_watch::tmdb_sign_in::start(&d.ctx.app) {
        Ok(token) => {
            d.watch.tmdb_pending = Some(token.clone());
            open_approval(d, &token);
        }
        Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
    }
    cx.notify();
}

/// Exchanges the approved token for a session; pending while TMDB has not seen
/// the approval.
fn check_tmdb_sign_in(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    let Some(token) = d.watch.tmdb_pending.clone() else { return };
    match heronite_watch::tmdb_sign_in::finish(&d.ctx.app, &token) {
        Ok(heronite_watch::tmdb_sign_in::Step::Approved(session)) => {
            d.watch.tmdb_pending = None;
            d.watch.tmdb_username = Some(SharedString::from(session.username));
            // The account's own rails exist only while signed in.
            d.watch.collections = heronite_watch::collections::descriptors(&d.ctx.app);
            cx.emit(crate::pages::PageEvent::LibraryChanged);
        }
        Ok(heronite_watch::tmdb_sign_in::Step::Pending) => {
            d.watch.notice = Some(tr!("TMDB has not seen the approval yet: approve it, then press again.").into());
        }
        Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
    }
    cx.notify();
}

/// Forgets the session (the key stays).
fn sign_out_tmdb(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    match heronite_watch::tmdb_sign_in::sign_out(&d.ctx.app) {
        Ok(()) => {
            d.watch.tmdb_username = None;
            d.watch.tmdb_pending = None;
            d.watch.collections = heronite_watch::collections::descriptors(&d.ctx.app);
            cx.emit(crate::pages::PageEvent::LibraryChanged);
        }
        Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
    }
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

fn clear_tmdb_key(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if let Err(error) = heronite_watch::store::clear_tmdb_key(&d.ctx.app) {
        log::warn!("clearing the TMDB key: {error:#}");
    }
    d.watch.tmdb_username = None;
    d.watch.tmdb_pending = None;
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
pub(crate) fn console_rows(_d: &SettingsPage) -> Vec<Row> {
    vec![
        Row::info("watch-theme", tr!("Watch theme"), tr!("Edited from ☰ → Customize.")),
        Row::action("watch-reset", tr!("Reset Watch theme")),
    ]
}

/// Settings → Watch → Providers, as rows: a header per provider, its account or
/// key, and the switches that turn it on.
pub(crate) fn provider_rows(d: &SettingsPage) -> Vec<Row> {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let all = providers();
    let (media, sources): (Vec<Provider>, Vec<Provider>) = all.into_iter().partition(|provider| provider.media);
    let mut rows = Vec::new();
    for (group, providers) in [(tr!("Streaming services"), media), (tr!("Sources"), sources)] {
        if providers.is_empty() {
            continue;
        }
        rows.push(Row::header(group));
        for provider in providers {
            rows.push(Row::header(provider.name.clone()));
            if provider.catalog {
                let on = settings.catalog_sources.iter().any(|id| id == &provider.id);
                rows.push(Row::toggle(SharedString::from(format!("watch-catalog:{}", provider.id)), tr!("Browse its catalogs"), on));
            }
            if provider.metadata {
                let on = settings.metadata_sources.iter().any(|id| id == &provider.id);
                rows.push(Row::toggle(SharedString::from(format!("watch-metadata:{}", provider.id)), tr!("Describe titles"), on));
            }
            if provider.media && provider.id == "stremio" {
                rows.push(Row::info("watch-account", tr!("Account"), if d.watch.linked { tr!("Linked") } else { tr!("Not linked") }));
                if d.watch.linked {
                    rows.push(Row::action("watch-unlink", tr!("Unlink")));
                } else if let Some(offer) = &d.watch.offer {
                    rows.push(Row::info("watch-code", tr!("Code {code}", code = offer.code.clone()), offer.link.clone()));
                    // A phone scans this; the console can only draw the code as text.
                    rows.push(Row::action("watch-qr", tr!("Show the code as a QR")));
                    rows.push(Row::action("watch-check", tr!("Check")));
                } else {
                    rows.push(Row::action("watch-link", tr!("Link with a code")));
                }
                rows.push(Row::toggle("watch-service-stop", tr!("Stop the service after watching"), settings.stop_service_when_done));
            }
            if provider.id == "tmdb" {
                if let Some(name) = &d.watch.tmdb_username {
                    let who = if name.is_empty() {
                        tr!("Signed in to TMDB").to_string()
                    } else {
                        tr!("Signed in as {name}", name = name.clone()).to_string()
                    };
                    rows.push(Row::info("watch-tmdb-account", tr!("Account"), who));
                    rows.push(Row::action("watch-tmdb-sign-out", tr!("Sign out")));
                } else if d.watch.tmdb_pending.is_some() {
                    rows.push(Row::info("watch-tmdb-account", tr!("Account"), tr!("Waiting for the approval")));
                    rows.push(Row::action("watch-tmdb-qr", tr!("Approve with your phone (QR)")));
                    rows.push(Row::action("watch-tmdb-again", tr!("Open the page again")));
                    rows.push(Row::action("watch-tmdb-approved", tr!("I approved it")));
                } else {
                    rows.push(Row::action("watch-tmdb-connect", tr!("Connect TMDB")));
                }
                if d.watch.tmdb_username.is_some() {
                    rows.push(Row::action("watch-tmdb-forget", tr!("Forget TMDB")));
                }
            }
        }
    }
    rows
}

/// The Providers page's rows: the switches, the account and the TMDB flow.
pub(crate) fn provider_apply(d: &mut SettingsPage, outcome: Outcome, window: &mut Window, cx: &mut Context<SettingsPage>) {
    match outcome {
        Outcome::Activate(id) => match id.as_ref() {
            "watch-link" => start_link(d, cx),
            "watch-check" => check_link(d, cx),
            "watch-unlink" => unlink(d, cx),
            "watch-tmdb-connect" => connect_tmdb(d, window, cx),
            "watch-qr" => {
                if let Some(offer) = d.watch.offer.clone() {
                    open_qr(d, tr!("Link Stremio"), &offer.link, window, cx);
                }
            }
            "watch-tmdb-qr" => {
                if let Some(token) = d.watch.tmdb_pending.clone() {
                    let url = heronite_watch::tmdb_sign_in::approve_url(&token);
                    open_qr(d, tr!("Approve TMDB"), &url, window, cx);
                }
            }
            "watch-tmdb-forget" => clear_tmdb_key(d, cx),
            "watch-tmdb-again" => {
                if let Some(token) = d.watch.tmdb_pending.clone() {
                    open_approval(d, &token);
                }
                cx.notify();
            }
            "watch-tmdb-approved" => check_tmdb_sign_in(d, cx),
            "watch-tmdb-sign-out" => sign_out_tmdb(d, cx),
            _ => {}
        },
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-catalog:") => {
            let source = id.trim_start_matches("watch-catalog:").to_owned();
            set_catalog_source(d, &source, on, cx);
        }
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-metadata:") => {
            let source = id.trim_start_matches("watch-metadata:").to_owned();
            set_metadata_source(d, &source, on, cx);
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

/// The Watch page's own action.
pub(crate) fn console_apply(d: &mut SettingsPage, outcome: Outcome, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    if let Outcome::Activate(id) = outcome {
        if id.as_ref() == "watch-reset" {
            themes::update_for(&d.ctx.app, Space::Watch, |prefs| {
                prefs.theme = watch_default_theme();
                prefs.based_on = crate::fork::watch::prefs::base_id().into();
                prefs.last_tab = 0;
            });
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    #[test]
    fn every_registered_provider_is_one_card_and_the_streaming_ones_come_first() {
        let all = providers();
        assert!(!all.is_empty(), "the registries always declare something");
        assert!(all[0].media, "a streaming service leads: it holds the account");
        let find = |id: &str| all.iter().find(|provider| provider.id == id).unwrap_or_else(|| panic!("{id} is missing: {all:?}"));
        let stremio = find("stremio");
        assert!(stremio.media && !stremio.catalog && !stremio.metadata);
        let tmdb = find("tmdb");
        assert!(tmdb.catalog && tmdb.metadata && tmdb.collections && !tmdb.media, "one provider, every capability");
        let cinemeta = find("cinemeta");
        assert!(cinemeta.catalog && cinemeta.metadata, "a source that is no account at all");
        let ids: Vec<&str> = all.iter().map(|provider| provider.id.as_str()).collect();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "a provider that is several kinds is one card: {ids:?}");
        assert!(capabilities(tmdb).contains(tr!("Catalogs")), "the card says what it gives");
    }

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
