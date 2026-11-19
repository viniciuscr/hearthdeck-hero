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
use crate::ui::fields::{meta_text, select_field};

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
    /// Whether this build can talk to Simkl: the app's client id is stored (the
    /// bootstrap writes a file that is adopted here) or compiled in. The person
    /// is never asked for it.
    pub simkl_ready: bool,
    /// Whether a Simkl account is signed in on this machine.
    pub simkl_signed_in: bool,
    /// The device code waiting for the person to approve it on their phone.
    pub simkl_pending: Option<heronite_watch::simkl_sign_in::DeviceCode>,
    /// The trackers the person turned on, and what the outbox still owes: what
    /// the Simkl card shows under its account.
    pub trackers: Vec<String>,
    pub tracker_pending: usize,
    pub tracker_error: Option<SharedString>,
    /// Titles a tracker refused for good: shown beside the sync state, never as
    /// it (they do not go away when a drain succeeds).
    pub tracker_unmatched: Vec<String>,
    /// Whether **any** provider's history is pulled in automatically, and the last
    /// pull's sentence ("Brought in 3 titles · 2 hours ago"). One sentence: the
    /// rows below say which provider did what.
    pub import_any: bool,
    pub import_state: Option<SharedString>,
    /// Whether this visit already asked the pull to catch up: the page draws many
    /// times, and the cursor's own schedule decides whether a tick does anything.
    pub imports_ticked: bool,
    /// Whether this visit already asked the trackers to catch up: the page
    /// renders many times, and a drain per frame would be a drain per frame.
    pub trackers_drained: bool,
    /// The open place menu of a metadata source (desktop), so only one is open.
    pub metadata_open: Option<SharedString>,
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
        // A self-built copy may carry the application key as a file instead of
        // compiling it in; it is taken once, here, where Settings is built.
        heronite_watch::tmdb_sign_in::adopt_key_file(&ctx.app);
        let tmdb_ready = heronite_watch::tmdb_sign_in::configured(&ctx.app);
        let tmdb_username = heronite_watch::tmdb_sign_in::session(&ctx.app).map(|session| SharedString::from(session.username));
        // The Simkl client id, adopted the same way: the bootstrap writes it, the
        // store keeps it, and the person only ever approves a code.
        heronite_watch::simkl_sign_in::adopt_client_id_file(&ctx.app);
        let simkl_ready = heronite_watch::simkl_sign_in::configured(&ctx.app);
        let simkl_signed_in = heronite_watch::simkl_sign_in::session(&ctx.app).is_some();
        let pending = heronite_watch::trackers::pending(&ctx.app);
        let collections = heronite_watch::collections::descriptors(&ctx.app);
        Self {
            linked,
            busy: false,
            offer: None,
            notice: None,
            tmdb_ready,
            tmdb_username,
            tmdb_pending: None,
            simkl_ready,
            simkl_signed_in,
            simkl_pending: None,
            trackers: heronite_watch::trackers::enabled(&ctx.app),
            tracker_pending: pending.count,
            tracker_error: pending.last_error.map(SharedString::from),
            tracker_unmatched: pending.unmatched,
            import_any: !heronite_watch::import::enabled(&ctx.app).is_empty(),
            import_state: import_sentence(&ctx.app),
            imports_ticked: false,
            trackers_drained: false,
            metadata_open: None,
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

/// Puts a metadata source in `place` (0 is asked first) or takes it out
/// (`None`): the list is the priority, so one value says both.
fn set_metadata_place(d: &mut SettingsPage, source: &str, place: Option<usize>, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    placed(&mut settings.metadata_sources, source, place);
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    cx.notify();
}

/// The priority list after `source` is put in `place`: it leaves its old place,
/// the others close the gap, and it lands where asked (the end when the place is
/// past it). `None` takes it out. Out of the page so it can be tested.
fn placed(list: &mut Vec<String>, source: &str, place: Option<usize>) {
    list.retain(|id| id != source);
    if let Some(place) = place {
        let place = place.min(list.len());
        list.insert(place, source.to_owned());
    }
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
    /// Owns the person's account and nothing else (yet): its card is the
    /// sign-in.
    account: bool,
}

/// Every provider Watch has, the streaming services first. An id that is more
/// than one kind of source is one provider.
fn providers() -> Vec<Provider> {
    let mut out: Vec<Provider> = Vec::new();
    let mut add = |id: &str, name: &str, set: fn(&mut Provider)| match out.iter_mut().find(|provider| provider.id == id) {
        Some(provider) => set(provider),
        None => {
            let mut provider = Provider {
                id: id.to_owned(),
                name: name.to_owned(),
                media: false,
                catalog: false,
                metadata: false,
                collections: false,
                account: false,
            };
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
    // The providers that are only an account so far: they serve no titles and no
    // catalogs, so no registry declares them, and their card is the sign-in.
    for &(id, name) in heronite_watch::registry::ACCOUNTS {
        add(id, name, |entry| entry.account = true);
    }
    // The streaming services first: they hold the person's account, and the
    // sources only fill in what those provide.
    out.sort_by_key(|provider| !provider.media);
    out
}

/// Whether this provider's card is an **account**: the streaming services (they
/// hold the person's library), the ones the accounts registry lists, and TMDB
/// (its catalogs work with the app's key, but the person can sign in).
///
/// What a provider *feeds* the screens is not here: that is Settings → Watch →
/// Sources, so this page is only ever "who am I signed in as".
fn has_account(provider: &Provider) -> bool {
    provider.media || provider.account || provider.id == "tmdb"
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
    if provider.account {
        parts.push(tr!("Account"));
    }
    parts.join(" \u{b7} ")
}

/// Settings → Watch → **Accounts**: one card per provider Watch is signed in to,
/// and nothing else. What each provider feeds the screens lives on Sources.
pub fn render_accounts(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let accounts: Vec<Provider> = providers().into_iter().filter(has_account).collect();

    let mut column = div().flex().flex_col().gap(px(14.)).child(caption(tr!("Accounts"))).child(div().px(px(6.)).child(meta_text(tr!(
        "Who Watch is signed in to, and what that account does. What each provider feeds the screens is on Sources."
    ))));
    for provider in &accounts {
        column = column.child(provider_card(d, provider, cx));
    }
    if accounts.is_empty() {
        column = column
            .child(div().px(px(6.)).child(meta_text(tr!("Nothing to sign in here yet: this build ships no provider with an account."))));
    }
    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

/// Settings → Watch → **Sources**: what feeds the screens, by capability.
/// Streams decide who may play, catalogs what the tabs page, and metadata what
/// describes a title — in the order the sources are asked, because the first
/// answer for a field wins.
/// Settings → Watch → **Sources**: one switch per thing that feeds the screens,
/// grouped by what it feeds. Metadata is ordered — the place decides which source
/// answers a field first — and the trackers are switched on here too.
pub fn render_sources(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    // The outbox catches up here, where the trackers live: what a session left
    // waiting is sent once, not on every frame the page draws.
    if !d.watch.trackers_drained {
        d.watch.trackers_drained = true;
        if !d.watch.trackers.is_empty() {
            drain_trackers(d, cx);
        }
    }
    // And the pull catches up, on its own schedule: a visit that arrives before
    // the interval costs nothing.
    if !d.watch.imports_ticked {
        d.watch.imports_ticked = true;
        if d.watch.import_any {
            tick_imports(d, cx);
        }
    }
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let mut column = div().flex().flex_col().gap(px(14.));

    let streams = heronite_watch::registry::providers();
    if !streams.is_empty() {
        let mut card = card_box().py(px(12.)).px(px(14.)).gap(px(6.));
        for provider in streams {
            let on = settings.stream_providers.iter().any(|id| id == provider.id());
            let id = provider.id().to_owned();
            card = card.child(
                toggle_row(format!("watch-stream-{id}"), SharedString::from(provider.name()), None, on)
                    .on_click(cx.listener(move |d, _, _, cx| set_stream_source(d, &id, !on, cx))),
            );
        }
        card = card.child(
            toggle_row(
                "watch-service-stop",
                SharedString::from(tr!("Stop the service after watching")),
                Some(tr!("Frees its memory; it is started again when you play a torrent.")),
                settings.stop_service_when_done,
            )
            .on_click(cx.listener(|d, _, _, cx| toggle_service(d, cx))),
        );
        column = column
            .child(caption(tr!("Streams")))
            .child(sources_help(tr!(
                "Who may play a title. A provider switched off here keeps its account and its rails; only asking it for a stream is refused."
            )))
            .child(card);
    }

    let catalogs = heronite_watch::catalog::sources();
    if !catalogs.is_empty() {
        let mut card = card_box().py(px(12.)).px(px(14.)).gap(px(6.));
        for source in catalogs {
            let descriptor = source.descriptor();
            let on = settings.catalog_sources.iter().any(|id| id == descriptor.id);
            let id = descriptor.id.to_owned();
            card = card.child(
                toggle_row(format!("watch-catalog-{id}"), SharedString::from(descriptor.name), None, on)
                    .on_click(cx.listener(move |d, _, _, cx| set_catalog_source(d, &id, !on, cx))),
            );
        }
        column = column
            .child(caption(tr!("Catalogs")))
            .child(sources_help(tr!("What the Movies and Series tabs may page through.")))
            .child(card);
    }

    let metadata = heronite_watch::metadata::sources();
    if !metadata.is_empty() {
        let mut card = card_box().py(px(12.)).px(px(14.)).gap(px(6.));
        for source in metadata {
            let descriptor = source.descriptor();
            let place = settings.metadata_sources.iter().position(|id| id == descriptor.id);
            card = card.child(metadata_place_row(d, descriptor.id, descriptor.name, place, metadata.len(), cx));
        }
        column = column
            .child(caption(tr!("Metadata")))
            .child(sources_help(tr!(
                "What describes a title. The place decides whether a source is asked and in which order; the first answer for a field wins."
            )))
            .child(card);
    }

    let trackers = heronite_watch::trackers::available(&d.ctx.app);
    if !trackers.is_empty() {
        let mut card = card_box().py(px(12.)).px(px(14.)).gap(px(6.));
        for id in trackers {
            let on = d.watch.trackers.iter().any(|known| known == &id);
            let desc = tracker_state(d);
            let id = id.clone();
            card = card.child(
                toggle_row(format!("watch-tracker-{id}"), SharedString::from(tracker_name(&id)), Some(desc.as_str()), on)
                    .on_click(cx.listener(move |d, _, _, cx| set_tracker(d, &id, !on, cx))),
            );
        }
        column = column
            .child(caption(tr!("Trackers")))
            .child(sources_help(tr!(
                "Where what you watch is mirrored. Switching one on sends the history already on this machine, then only what happens from then on."
            )))
            .child(card);
    }

    if let Some(notice) = &d.watch.notice {
        column = column.child(div().text_size(px(12.5)).text_color(theme::warning()).child(notice.clone()));
    }
    column.into_any_element()
}

/// One line under a sources group.
fn sources_help(text: &str) -> Div {
    div().px(px(6.)).child(meta_text(text))
}

/// A metadata source's place in the priority list: `Off`, or `1`..`n`. One
/// control per source — the list is the order, so there are no move buttons.
fn metadata_place_row(
    d: &mut SettingsPage,
    id: &'static str,
    name: &str,
    place: Option<usize>,
    total: usize,
    cx: &mut Context<SettingsPage>,
) -> Div {
    let key = SharedString::from(format!("watch-metadata-place-{id}"));
    let open = d.watch.metadata_open.as_ref() == Some(&key);
    let shown: SharedString = match place {
        Some(index) => SharedString::from(format!("{}", index + 1)),
        None => SharedString::from(tr!("Off")),
    };
    let mut options: Vec<(SharedString, bool)> = vec![(SharedString::from(tr!("Off")), place.is_none())];
    for index in 0..total {
        options.push((SharedString::from(format!("{}", index + 1)), place == Some(index)));
    }
    let (toggle_key, source) = (key, id.to_owned());
    select_field(
        &format!("watch-metadata-{id}"),
        Some(name),
        shown,
        false,
        open,
        options,
        cx,
        move |d: &mut SettingsPage, _, cx| {
            let open = d.watch.metadata_open.as_ref() == Some(&toggle_key);
            d.watch.metadata_open = (!open).then(|| toggle_key.clone());
            cx.notify();
        },
        move |d: &mut SettingsPage, index, _, cx| {
            d.watch.metadata_open = None;
            set_metadata_place(d, &source, index.checked_sub(1), cx);
        },
    )
}

/// The state line under a tracker: what is waiting or the last fault, and — in
/// the same line, never a row of its own — the titles it could not match.
fn tracker_state(d: &SettingsPage) -> String {
    let state = match (&d.watch.tracker_error, d.watch.tracker_pending) {
        (Some(error), _) => error.to_string(),
        (None, 0) => tr!("Everything watched here is in Simkl.").to_string(),
        (None, 1) => tr!("1 watch event waiting for Simkl.").to_string(),
        (None, pending) => tr!("{n} watch events waiting for Simkl.", n = pending).to_string(),
    };
    match unmatched_line(d) {
        Some(unmatched) => format!("{state} {unmatched}"),
        None => state,
    }
}

/// How a tracker is called on a screen: the registry's own name.
fn tracker_name(id: &str) -> &'static str {
    heronite_watch::trackers::name(id)
}

/// One account's card: its name and what it gives, then its sign-in and the
/// switches that belong to that account. What it feeds the screens is on Sources.
fn provider_card(d: &mut SettingsPage, provider: &Provider, cx: &mut Context<SettingsPage>) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    let tmdb = provider.id == "tmdb";
    if provider.media && provider.id == "stremio" {
        rows.push(stremio_account(d, cx));
    }
    if provider.id == "tmdb" {
        rows.push(tmdb_card(d, cx));
    }
    if provider.id == "simkl" {
        rows.push(simkl_card(d, cx));
    }
    let card = card_box()
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
        .into_any_element();
    if !tmdb {
        return card;
    }
    // TMDB's terms require the credit wherever its data is shown. A wrapped line
    // under its card, not a row: a row clips the sentence.
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(card)
        .child(div().max_w(px(460.)).child(meta_text(if d.watch.tmdb_ready {
            tr!("This product uses the TMDB API but is not endorsed or certified by TMDB.")
        } else {
            tr!("TMDB needs this app's key: the bootstrap sets it (or ~/.config/hearthdeck/tmdb-api-key).")
        })))
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
                .child(meta_text(tr!("Scan the code with the Stremio app, or open {link}", link = offer.link.clone())))
                .child(meta_text(tr!("Waiting for the approval: this card follows on its own."))),
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
        (None, None) => (
            tr!("TMDB").to_string(),
            tr!("TMDB needs this app's key, which the bootstrap sets (or a file in ~/.config/hearthdeck/tmdb-api-key)."),
        ),
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
    } else {
        buttons = buttons.child(action_button(
            "watch-tmdb-connect",
            ButtonKind::Primary,
            tr!("Connect TMDB"),
            d.watch.tmdb_ready,
            cx.listener(|d, _, _, cx| connect_tmdb(d, cx)),
        ));
    }
    buttons
}

/// The one way in: the app's key has to be there (the bootstrap sets it), then
/// TMDB hands out a token and the browser and the pairing panel take over.
fn connect_tmdb(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    if !d.watch.tmdb_ready {
        d.watch.notice =
            Some(tr!("TMDB needs this app's key, which the bootstrap sets (or a file in ~/.config/hearthdeck/tmdb-api-key).").into());
        cx.notify();
        return;
    }
    start_tmdb_sign_in(d, cx);
}

/// The pairing panel the one button opens: the code at the top, the QR under it
/// and the wait under that. Console mode draws the layer; the desktop card has no
/// layer to draw, so it keeps its own inline code and QR and this returns `None`.
///
/// The panel is returned so the poll that finishes the sign-in can take it away
/// itself — and only it (`Layer::close_with`), never a keyboard opened meanwhile.
fn open_pairing(
    d: &mut SettingsPage,
    title: &str,
    code: Option<&str>,
    url: &str,
    cx: &mut Context<SettingsPage>,
) -> Option<Entity<qr_overlay::QrOverlay>> {
    if !crate::console::active() {
        return None;
    }
    let overlay = cx.new(|_| qr_overlay::QrOverlay::new(title, code, url));
    let sub = cx.subscribe(&overlay, |d, _, event: &qr_overlay::QrEvent, cx| {
        if *event == qr_overlay::QrEvent::Close {
            d.layer_mut().close();
            cx.notify();
        }
    });
    d.layer_mut().open(&overlay, sub);
    cx.notify();
    Some(overlay)
}

/// Takes the pairing panel away once the approval lands (or the code dies).
fn close_pairing(d: &mut SettingsPage, overlay: Option<&Entity<qr_overlay::QrOverlay>>, cx: &mut Context<SettingsPage>) {
    let Some(overlay) = overlay else { return };
    d.layer_mut().close_with(overlay);
    cx.notify();
}

/// Opens TMDB's approval page for `token` on a desk; a browser that will not
/// open is a notice, not a failure of the sign-in.
///
/// A console only shows the page as a QR ([`open_pairing`]): there is a
/// browser on the host to launch, and opening one would take the TV away from
/// the panel the person is looking at.
fn open_approval(d: &mut SettingsPage, token: &str) {
    let url = heronite_watch::tmdb_sign_in::approve_url(token);
    if let Err(error) = heronite_platform::shell::open(&url) {
        log::warn!("opening {url}: {error}");
        d.watch.notice = Some(tr!("Could not open the browser, but the QR code works: {error}", error = error.to_string()).into());
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
    // Asking TMDB for a token is a network call: off the UI thread, or a slow
    // answer looks exactly like a dead button.
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let started = cx.background_spawn(async move { heronite_watch::tmdb_sign_in::start(&app) }).await;
        this.update(cx, |d, cx| match started {
            Ok(token) => {
                let url = heronite_watch::tmdb_sign_in::approve_url(&token);
                d.watch.tmdb_pending = Some(token.clone());
                // The TV gets the page as a QR, and nothing else: a browser
                // opening on the host would take the screen away from the panel.
                // A desk has no panel over the page, so it opens the browser.
                let overlay = open_pairing(d, tr!("Approve TMDB"), None, &url, cx);
                if !crate::console::active() {
                    open_approval(d, &token);
                }
                // TMDB only says "approved" through step 3, and a phone approves on
                // a different device: ask again every few seconds instead of waiting
                // for a button. `finish` answers `Pending` until the person is done.
                wait_for_approval(d, token, overlay, cx);
            }
            Err(error) => {
                log::warn!("TMDB sign-in did not start: {error:#}");
                d.watch.notice = Some(format!("{error:#}").into());
            }
        })
        .ok();
    })
    .detach();
    cx.notify();
}

/// Polls step 3 until the token is approved or ten minutes pass. The phone
/// approves, the app notices on its own, and the panel goes away with it.
///
/// A pending token is a step; a bad key is not, and neither is a network fault
/// worth ending the wait over — the person is mid-scan, so keep asking.
fn wait_for_approval(d: &mut SettingsPage, token: String, overlay: Option<Entity<qr_overlay::QrOverlay>>, cx: &mut Context<SettingsPage>) {
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        for _ in 0..120 {
            cx.background_spawn(async { std::thread::sleep(std::time::Duration::from_secs(5)) }).await;
            // Another press may have started over.
            let still_waiting = this.update(cx, |d, _| d.watch.tmdb_pending.as_deref() == Some(token.as_str())).unwrap_or(false);
            if !still_waiting {
                return;
            }
            let (app, asked) = (app.clone(), token.clone());
            let step = cx.background_spawn(async move { heronite_watch::tmdb_sign_in::finish(&app, &asked) }).await;
            match step {
                Ok(heronite_watch::tmdb_sign_in::Step::Approved(session)) => {
                    this.update(cx, |d, cx| {
                        if d.watch.tmdb_pending.as_deref() != Some(token.as_str()) {
                            return;
                        }
                        d.watch.tmdb_pending = None;
                        d.watch.tmdb_username = Some(SharedString::from(session.username));
                        d.watch.collections = heronite_watch::collections::descriptors(&d.ctx.app);
                        d.watch.notice = None;
                        close_pairing(d, overlay.as_ref(), cx);
                        cx.emit(crate::pages::PageEvent::LibraryChanged);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                // Not approved yet: keep asking.
                Ok(heronite_watch::tmdb_sign_in::Step::Pending) => {}
                Err(error) if heronite_watch::tmdb_sign_in::key_rejected(&error) => {
                    // The key was reset: asking again cannot help.
                    log::warn!("TMDB sign-in: {error:#}");
                    this.update(cx, |d, cx| {
                        d.watch.tmdb_pending = None;
                        d.watch.notice = Some(format!("{error:#}").into());
                        close_pairing(d, overlay.as_ref(), cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                Err(error) => log::debug!("TMDB sign-in: {error:#}"),
            }
        }
        this.update(cx, |d, cx| close_pairing(d, overlay.as_ref(), cx)).ok();
    })
    .detach();
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

/// The Simkl credential: the code to approve while waiting, or who is signed in.
fn simkl_card(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> AnyElement {
    let (title, desc) = match (&d.watch.simkl_pending, d.watch.simkl_signed_in) {
        (Some(_), _) => (
            tr!("Approve the sign-in on your phone.").to_string(),
            tr!("Simkl is waiting for the code: this card follows when it is approved."),
        ),
        (None, true) => (tr!("Signed in to Simkl").to_string(), tr!("What you watch follows the account across your devices.")),
        (None, false) if d.watch.simkl_ready => {
            (tr!("Simkl").to_string(), tr!("Connect and approve it on your phone: Simkl keeps what you watch in step."))
        }
        (None, false) => (
            tr!("Simkl").to_string(),
            tr!("Simkl needs this app's client id, which the bootstrap sets (or a file in ~/.config/hearthdeck/simkl-client-id)."),
        ),
    };
    let mut card = div().flex().flex_col().gap(px(10.)).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(div().flex().flex_col().gap(px(3.)).child(div().text_size(px(13.5)).child(title)).child(meta_text(desc)))
            .child(simkl_buttons(d, cx)),
    );
    // Waiting for the approval: the same page as a QR, so a phone approves it
    // without a keyboard anywhere near the TV.
    if let Some(code) = &d.watch.simkl_pending {
        let mut row = div().flex().items_center().gap(px(16.));
        if let Some(qr) = qr::qr_code(&code.verification_url, 3.0) {
            row = row.child(qr);
        }
        row = row.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().text_size(px(13.)).child(tr!("Code {code}", code = code.user_code.clone())))
                .child(meta_text(tr!("Scan it with a phone to approve, or open {link}", link = code.verification_url.clone()))),
        );
        card = card.child(row);
    }
    card.into_any_element()
}

fn simkl_buttons(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Div {
    let mut buttons = div().flex().items_center().gap(px(8.));
    if d.watch.simkl_signed_in {
        buttons = buttons.child(action_button(
            "watch-simkl-sign-out",
            ButtonKind::Ghost,
            tr!("Sign out"),
            true,
            cx.listener(|d, _, _, cx| sign_out_simkl(d, cx)),
        ));
    } else {
        buttons = buttons.child(action_button(
            "watch-simkl-connect",
            ButtonKind::Primary,
            tr!("Connect Simkl"),
            d.watch.simkl_ready,
            cx.listener(|d, _, _, cx| connect_simkl(d, cx)),
        ));
    }
    buttons
}

/// Turns a tracker on or off. On is also the first sync: the finished history
/// already on this machine goes over once, then only what happens afterwards.
fn set_tracker(d: &mut SettingsPage, id: &str, on: bool, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    settings.trackers.retain(|tracker| tracker != id);
    if on {
        settings.trackers.push(id.to_owned());
    }
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    d.watch.trackers = heronite_watch::trackers::enabled(&d.ctx.app);
    if on {
        // Switching it on *is* the first sync: the history already on this
        // machine goes over once (with its own timestamps), and only what happens
        // from then on is sent afterwards.
        let app = d.ctx.app.clone();
        let switched = id.to_owned();
        cx.spawn(async move |this, cx| {
            let sent = cx.background_spawn(async move { heronite_watch::trackers::backfill_for(&app, &switched) }).await;
            this.update(cx, |d, cx| {
                match sent {
                    Ok(count) if count > 0 => d.watch.notice = Some(tr!("Sent {n} watch sessions.", n = count).into()),
                    Ok(_) => {}
                    Err(error) => {
                        log::warn!("trackers: {error:#}");
                        d.watch.notice = Some(format!("{error:#}").into());
                    }
                }
                refresh_trackers(d);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    cx.notify();
}

/// Hands the outbox to the trackers, off the UI thread, then shows what is left.
/// The sentence the import row shows: what the last pull did. Whichever provider
/// is enabled — one row per provider already carries its own switch.
fn import_sentence(app: &heronite_data::paths::AppPaths) -> Option<SharedString> {
    let provider = heronite_watch::import::enabled(app).into_iter().next()?;
    let last = heronite_watch::import::status(app, provider)?;
    Some(match &last.error {
        Some(error) => tr!("Could not bring anything in: {error}", error = error.clone()).into(),
        None if last.unchanged || last.watched + last.positions == 0 => tr!("Nothing new the last time it asked.").into(),
        None => tr!("Brought in {n} titles.", n = last.watched + last.positions).into(),
    })
}

/// Asks the pull to catch up in the background: the cursor's `next_at` decides
/// whether it does anything, so this is safe on every visit.
fn tick_imports(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let ran = cx.background_spawn(async move { heronite_watch::import::tick(&app) }).await;
        this.update(cx, |d, cx| {
            if let Some((provider, Err(error))) = ran.first() {
                log::warn!("import {provider}: {error:#}");
            }
            d.watch.import_state = import_sentence(&d.ctx.app);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Turning the pull on runs it once, right away; turning it off leaves what came
/// in, because imported marks are ordinary history.
fn set_import(d: &mut SettingsPage, provider: &str, on: bool, cx: &mut Context<SettingsPage>) {
    if let Err(error) = heronite_watch::import::set_enabled(&d.ctx.app, provider, on) {
        log::warn!("saving the Watch settings: {error:#}");
    }
    d.watch.import_any = !heronite_watch::import::enabled(&d.ctx.app).is_empty();
    if !on {
        d.watch.import_state = None;
        cx.notify();
        return;
    }
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let ran = cx.background_spawn(async move { heronite_watch::import::tick(&app) }).await;
        this.update(cx, |d, cx| {
            if let Some((_, Ok(report))) = ran.into_iter().next() {
                d.watch.notice = Some(tr!("Brought in {n} titles.", n = report.watched + report.positions).into());
            }
            d.watch.import_state = import_sentence(&d.ctx.app);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn drain_trackers(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let drained = cx.background_spawn(async move { heronite_watch::trackers::drain(&app) }).await;
        this.update(cx, |d, cx| {
            match drained {
                Ok(report) if report.delivered > 0 => {
                    d.watch.notice = Some(tr!("Sent {n} watch events.", n = report.delivered).into());
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("trackers: {error:#}");
                    d.watch.notice = Some(format!("{error:#}").into());
                }
            }
            refresh_trackers(d);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Re-reads what the outbox owes, for the card.
fn refresh_trackers(d: &mut SettingsPage) {
    let pending = heronite_watch::trackers::pending(&d.ctx.app);
    d.watch.trackers = heronite_watch::trackers::enabled(&d.ctx.app);
    d.watch.tracker_pending = pending.count;
    d.watch.tracker_error = pending.last_error.map(SharedString::from);
    d.watch.tracker_unmatched = pending.unmatched;
}

/// The titles a tracker could not match, in one sentence for the card.
fn unmatched_line(d: &SettingsPage) -> Option<String> {
    match d.watch.tracker_unmatched.as_slice() {
        [] => None,
        [one] => Some(tr!("Could not match: {title}", title = one.clone())),
        [first, ..] => {
            Some(tr!("Could not match {n} titles, such as {title}.", n = d.watch.tracker_unmatched.len(), title = first.clone()))
        }
    }
}

/// The one way in: the app's client id must be there (the bootstrap sets it),
/// then Simkl hands out a code and the phone approves it.
fn connect_simkl(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    if !d.watch.simkl_ready {
        d.watch.notice = Some(
            tr!("Simkl needs this app's client id, which the bootstrap sets (or a file in ~/.config/hearthdeck/simkl-client-id).").into(),
        );
        cx.notify();
        return;
    }
    start_simkl_sign_in(d, cx);
}

/// Asks Simkl for a code to show. Asking is a network call, so it happens off
/// the UI thread or a slow answer looks exactly like a dead button.
fn start_simkl_sign_in(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let started = cx.background_spawn(async move { heronite_watch::simkl_sign_in::start(&app) }).await;
        this.update(cx, |d, cx| match started {
            Ok(code) => {
                d.watch.simkl_pending = Some(code.clone());
                d.watch.notice = None;
                // The pairing panel: the code to type, the QR to scan, and the
                // wait under it. One button did all three.
                let overlay = open_pairing(d, tr!("Connect Simkl"), Some(&code.user_code), &code.verification_url, cx);
                // The phone approves on another device: ask again every interval
                // instead of waiting for a button.
                wait_for_simkl(d, code, overlay, cx);
            }
            Err(error) => {
                log::warn!("Simkl sign-in did not start: {error:#}");
                d.watch.notice = Some(format!("{error:#}").into());
            }
        })
        .ok();
    })
    .detach();
    cx.notify();
}

/// Polls until the code is approved, the code expires, or another attempt
/// replaces this one. Simkl records no refusal, so the code's own lifetime is
/// what ends the wait — and the panel goes away with either end.
fn wait_for_simkl(
    d: &mut SettingsPage,
    code: heronite_watch::simkl_sign_in::DeviceCode,
    overlay: Option<Entity<qr_overlay::QrOverlay>>,
    cx: &mut Context<SettingsPage>,
) {
    let app = d.ctx.app.clone();
    cx.spawn(async move |this, cx| {
        let mut interval = code.interval_secs.max(1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(code.expires_in);
        loop {
            // Another press may have started over, or the card may be gone.
            let waiting = this
                .update(cx, |d, _| {
                    d.watch.simkl_pending.as_ref().map(|pending| pending.device_code.as_str()) == Some(code.device_code.as_str())
                })
                .unwrap_or(false);
            if !waiting {
                return;
            }
            if std::time::Instant::now() >= deadline {
                this.update(cx, |d, cx| {
                    d.watch.simkl_pending = None;
                    d.watch.notice = Some(tr!("Simkl expired the code: connect again.").into());
                    close_pairing(d, overlay.as_ref(), cx);
                    cx.notify();
                })
                .ok();
                return;
            }
            cx.background_spawn(async move { std::thread::sleep(std::time::Duration::from_secs(interval)) }).await;
            let (app, asked) = (app.clone(), code.clone());
            let step = cx.background_spawn(async move { heronite_watch::simkl_sign_in::poll(&app, &asked) }).await;
            match step {
                Ok(heronite_watch::simkl_sign_in::Step::Approved(_)) => {
                    this.update(cx, |d, cx| {
                        if d.watch.simkl_pending.as_ref().map(|pending| pending.device_code.as_str()) != Some(code.device_code.as_str()) {
                            return;
                        }
                        d.watch.simkl_pending = None;
                        d.watch.simkl_signed_in = true;
                        d.watch.notice = None;
                        close_pairing(d, overlay.as_ref(), cx);
                        // A signed-in tracker gets the outbox at once.
                        refresh_trackers(d);
                        drain_trackers(d, cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                // Polled too fast: Simkl wants a longer wait, not another attempt.
                Ok(heronite_watch::simkl_sign_in::Step::SlowDown) => interval += heronite_watch::simkl_sign_in::SLOW_DOWN,
                // Not approved yet: keep asking.
                Ok(heronite_watch::simkl_sign_in::Step::Pending) => {}
                // A refused client id will not get better by asking again; a
                // network fault might, and the person is mid-approval.
                Err(error) if heronite_watch::simkl_sign_in::key_rejected(&error) => {
                    log::warn!("Simkl sign-in: {error:#}");
                    this.update(cx, |d, cx| {
                        d.watch.simkl_pending = None;
                        d.watch.notice = Some(format!("{error:#}").into());
                        close_pairing(d, overlay.as_ref(), cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                Err(error) => log::debug!("Simkl sign-in: {error:#}"),
            }
        }
    })
    .detach();
}

/// Forgets the session (the app's client id stays).
fn sign_out_simkl(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    d.watch.notice = None;
    match heronite_watch::simkl_sign_in::sign_out(&d.ctx.app) {
        Ok(()) => {
            d.watch.simkl_signed_in = false;
            d.watch.simkl_pending = None;
            // The refusals belonged to the account that was signed in.
            if let Err(error) = heronite_watch::trackers::clear_notices(&d.ctx.app) {
                log::warn!("clearing the tracker notices: {error:#}");
            }
            refresh_trackers(d);
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

/// Turns a media provider's streams on or off. Its account and rails stay either
/// way: only asking it for a stream is refused (`playback::enabled`).
fn set_stream_source(d: &mut SettingsPage, provider: &str, on: bool, cx: &mut Context<SettingsPage>) {
    let mut settings = heronite_watch::settings::read(&d.ctx.app);
    settings.stream_providers.retain(|id| id != provider);
    if on {
        settings.stream_providers.push(provider.to_owned());
    }
    if let Err(error) = heronite_watch::settings::write(&d.ctx.app, &settings) {
        log::warn!("saving the Watch settings: {error:#}");
    }
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
                Ok(offer) => {
                    d.watch.offer = Some(offer.clone());
                    // The one button opened the code, the QR and the wait.
                    let overlay = open_pairing(d, tr!("Link Stremio"), Some(&offer.code), &offer.link, cx);
                    // The phone scans the code: the app notices on its own.
                    wait_for_link(offer, overlay, cx);
                }
                Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Polls the link code every five seconds until the account is linked, another
/// flow replaces this one, or ten minutes pass — the person scans on a phone and
/// dismisses the code, so asking again is the app's job, not a button's. The
/// pairing panel goes away the moment the account is linked.
fn wait_for_link(offer: LinkOffer, overlay: Option<Entity<qr_overlay::QrOverlay>>, cx: &mut Context<SettingsPage>) {
    let link = Endpoints::default().link;
    let code = offer.code;
    cx.spawn(async move |this, cx| {
        for _ in 0..120 {
            cx.background_spawn(async { std::thread::sleep(std::time::Duration::from_secs(5)) }).await;
            let waiting = this
                .update(cx, |d, _| d.watch.offer.as_ref().map(|offer| offer.code.as_str()) == Some(code.as_str()) && !d.watch.linked)
                .unwrap_or(false);
            if !waiting {
                return;
            }
            let (link, asked) = (link.clone(), code.clone());
            let result = cx.background_spawn(async move { read_link(&UreqTransport::new(), &link, &asked) }).await;
            match result {
                Ok(LinkPoll::Linked(key)) => {
                    this.update(cx, |d, cx| {
                        if d.watch.offer.as_ref().map(|offer| offer.code.as_str()) != Some(code.as_str()) {
                            return;
                        }
                        match heronite_watch::store::write_auth(&d.ctx.app, &key) {
                            Ok(()) => {
                                d.watch.linked = true;
                                d.watch.offer = None;
                                d.watch.notice = None;
                                close_pairing(d, overlay.as_ref(), cx);
                            }
                            Err(error) => d.watch.notice = Some(format!("{error:#}").into()),
                        }
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                // Not scanned yet. A network fault is not the end either: the
                // person may be mid-scan, and the code lives for ten minutes.
                Ok(LinkPoll::Pending) => {}
                Err(error) => log::debug!("Stremio link: {error:#}"),
            }
        }
        // Ten minutes gone: the code died with them, so take the panel away.
        this.update(cx, |d, cx| close_pairing(d, overlay.as_ref(), cx)).ok();
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

/// Settings → Watch → **Accounts**, as rows: a header per provider Watch is
/// signed in to, then its sign-in and the switches that belong to that account.
pub(crate) fn account_rows(d: &SettingsPage) -> Vec<Row> {
    let accounts: Vec<Provider> = providers().into_iter().filter(has_account).collect();
    let mut rows = Vec::new();
    if accounts.is_empty() {
        rows.push(Row::header(tr!("Accounts")));
        rows.push(Row::info("watch-accounts-empty", tr!("Accounts"), tr!("This build ships no provider with an account.")));
        return rows;
    }
    rows.push(Row::header(tr!("Accounts")));
    {
        for provider in accounts {
            rows.push(Row::header(provider.name.clone()));
            if provider.media && provider.id == "stremio" {
                rows.push(Row::info("watch-account", tr!("Account"), if d.watch.linked { tr!("Linked") } else { tr!("Not linked") }));
                if d.watch.linked {
                    rows.push(Row::action("watch-unlink", tr!("Unlink")));
                } else {
                    rows.push(Row::action("watch-link", tr!("Link with a code")));
                }
            }
            if provider.id == "tmdb" {
                if let Some(name) = &d.watch.tmdb_username {
                    let who = if name.is_empty() {
                        tr!("Signed in to TMDB").to_string()
                    } else {
                        tr!("Signed in as {name}", name = name.clone()).to_string()
                    };
                    rows.push(
                        Row::info("watch-tmdb-account", tr!("Account"), who)
                            .desc(tr!("This product uses the TMDB API but is not endorsed or certified by TMDB.")),
                    );
                    rows.push(Row::action("watch-tmdb-sign-out", tr!("Sign out")));
                } else {
                    // One row: the button opens the approval panel and follows it.
                    // The hint under a focused row is where a sentence fits — and
                    // where a failure has to show, since a console row cannot.
                    let hint: String = match &d.watch.notice {
                        Some(notice) => notice.to_string(),
                        None if d.watch.tmdb_pending.is_some() => tr!("Waiting for the approval: the code is on screen.").to_string(),
                        None if d.watch.tmdb_ready => {
                            tr!("This product uses the TMDB API but is not endorsed or certified by TMDB.").to_string()
                        }
                        None => tr!("TMDB needs this app's key: the bootstrap sets it (or ~/.config/hearthdeck/tmdb-api-key).").to_string(),
                    };
                    rows.push(Row::action("watch-tmdb-connect", tr!("Connect TMDB")).desc(hint));
                }
            }
            if provider.id == "simkl" {
                if d.watch.simkl_signed_in {
                    rows.push(
                        Row::info("watch-simkl-account", tr!("Account"), tr!("Signed in"))
                            .desc(tr!("What you watch follows the account across your devices.")),
                    );
                    rows.push(Row::action("watch-simkl-sign-out", tr!("Sign out")));
                } else {
                    // One row, like TMDB's: the panel carries the code and the
                    // wait, so the menu needs no rows of its own for them.
                    let hint: String = match &d.watch.notice {
                        Some(notice) => notice.to_string(),
                        None if d.watch.simkl_pending.is_some() => tr!("Waiting for the approval: the code is on screen.").to_string(),
                        None if d.watch.simkl_ready => {
                            tr!("Simkl keeps what you watch in step across your devices.").to_string()
                        }
                        None => tr!(
                            "Simkl needs this app's client id, which the bootstrap sets (or a file in ~/.config/hearthdeck/simkl-client-id)."
                        )
                        .to_string(),
                    };
                    rows.push(Row::action("watch-simkl-connect", tr!("Connect Simkl")).desc(hint));
                }
            }
        }
    }
    rows
}

/// The Accounts page's rows: sign-ins and sign-outs, and nothing else.
pub(crate) fn account_apply(d: &mut SettingsPage, outcome: Outcome, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    if let Outcome::Activate(id) = outcome {
        match id.as_ref() {
            "watch-link" => start_link(d, cx),
            "watch-unlink" => unlink(d, cx),
            "watch-tmdb-connect" => connect_tmdb(d, cx),
            "watch-tmdb-sign-out" => sign_out_tmdb(d, cx),
            "watch-simkl-connect" => connect_simkl(d, cx),
            "watch-simkl-sign-out" => sign_out_simkl(d, cx),
            _ => {}
        }
    }
}

/// Settings → Watch → **Sources**, as rows: one switch per stream, catalog and
/// tracker, and a metadata source's place.
pub(crate) fn source_rows(d: &SettingsPage) -> Vec<Row> {
    let settings = heronite_watch::settings::read(&d.ctx.app);
    let mut rows = Vec::new();

    let streams = heronite_watch::registry::providers();
    if !streams.is_empty() {
        rows.push(Row::header(tr!("Streams")));
        for provider in streams {
            let on = settings.stream_providers.iter().any(|id| id == provider.id());
            rows.push(Row::toggle(SharedString::from(format!("watch-stream:{}", provider.id())), SharedString::from(provider.name()), on));
        }
        rows.push(Row::toggle("watch-service-stop", tr!("Stop the service after watching"), settings.stop_service_when_done));
    }

    let catalogs = heronite_watch::catalog::sources();
    if !catalogs.is_empty() {
        rows.push(Row::header(tr!("Catalogs")));
        for source in catalogs {
            let descriptor = source.descriptor();
            let on = settings.catalog_sources.iter().any(|id| id == descriptor.id);
            rows.push(Row::toggle(SharedString::from(format!("watch-catalog:{}", descriptor.id)), SharedString::from(descriptor.name), on));
        }
    }

    // The place is the order: Off, or 1..n among the metadata sources.
    let metadata = heronite_watch::metadata::sources();
    if !metadata.is_empty() {
        rows.push(Row::header(tr!("Metadata")));
        for source in metadata {
            let descriptor = source.descriptor();
            let place = settings.metadata_sources.iter().position(|id| id == descriptor.id);
            let mut options: Vec<SharedString> = vec![SharedString::from(tr!("Off"))];
            for index in 0..metadata.len() {
                options.push(SharedString::from(format!("{}", index + 1)));
            }
            let selected = place.map_or(0, |index| index + 1);
            rows.push(Row::choice(
                SharedString::from(format!("watch-metadata-place:{}", descriptor.id)),
                SharedString::from(descriptor.name),
                options,
                selected,
            ));
        }
    }

    let trackers = heronite_watch::trackers::available(&d.ctx.app);
    if !trackers.is_empty() {
        rows.push(Row::header(tr!("Trackers")));
        for id in trackers {
            let on = d.watch.trackers.iter().any(|known| known == &id);
            rows.push(
                Row::toggle(
                    SharedString::from(format!("watch-tracker:{id}")),
                    tr!("Send what I watch to {provider}", provider = tracker_name(&id)),
                    on,
                )
                .desc(tracker_state(d)),
            );
            // The other direction, a separate switch: reading never queues a
            // backfill of what was watched here.
            let pulling = heronite_watch::import::enabled(&d.ctx.app).iter().any(|pulled| *pulled == id);
            rows.push(
                Row::toggle(
                    SharedString::from(format!("watch-import:{id}")),
                    tr!("Bring in what I watched in {provider}", provider = tracker_name(&id)),
                    pulling,
                )
                .desc(d.watch.import_state.clone().unwrap_or_else(|| tr!("Automatic while Watch is open.").into())),
            );
        }
    }
    rows
}

/// A press on the Sources page: the switches, and a source's place.
pub(crate) fn source_apply(d: &mut SettingsPage, outcome: Outcome, _window: &mut Window, cx: &mut Context<SettingsPage>) {
    match outcome {
        Outcome::Changed(id, Value::Choice { index, .. }) if id.starts_with("watch-metadata-place:") => {
            let source = id.trim_start_matches("watch-metadata-place:").to_owned();
            set_metadata_place(d, &source, index.checked_sub(1), cx);
        }
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-stream:") => {
            let source = id.trim_start_matches("watch-stream:").to_owned();
            set_stream_source(d, &source, on, cx);
        }
        Outcome::Changed(id, Value::Toggle(_)) if id.as_ref() == "watch-service-stop" => toggle_service(d, cx),
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-catalog:") => {
            let source = id.trim_start_matches("watch-catalog:").to_owned();
            set_catalog_source(d, &source, on, cx);
        }
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-tracker:") => {
            let tracker = id.trim_start_matches("watch-tracker:").to_owned();
            set_tracker(d, &tracker, on, cx);
        }
        Outcome::Changed(id, Value::Toggle(on)) if id.starts_with("watch-import:") => {
            let provider = id.trim_start_matches("watch-import:").to_owned();
            set_import(d, &provider, on, cx);
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
        let simkl = find("simkl");
        assert!(simkl.account && simkl.metadata && !simkl.media && !simkl.catalog, "a tracker that can also describe a title");
        assert_eq!(
            capabilities(simkl),
            format!("{} \u{b7} {}", tr!("Metadata"), tr!("Account")),
            "it describes titles and holds an account"
        );
        let ids: Vec<&str> = all.iter().map(|provider| provider.id.as_str()).collect();
        // The Accounts page: the streaming service, TMDB's session and Simkl's
        // account. Cinemeta has none, so it never shows a card there.
        let accounts: Vec<&str> = all.iter().filter(|provider| has_account(provider)).map(|provider| provider.id.as_str()).collect();
        assert_eq!(accounts, ["stremio", "tmdb", "simkl"], "the media service leads, and a source without an account is absent");
        // The Sources page takes its lists straight from the registries.
        let streams: Vec<&str> = heronite_watch::registry::providers().iter().map(|provider| provider.id()).collect();
        assert_eq!(streams, ["stremio"], "one streaming service to switch");
        assert!(heronite_watch::catalog::sources().iter().any(|source| source.descriptor().id == "cinemeta"));
        assert!(
            heronite_watch::metadata::sources().iter().any(|source| source.descriptor().id == "simkl"),
            "Simkl describes titles, opt-in"
        );
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "a provider that is several kinds is one card: {ids:?}");
        assert!(capabilities(tmdb).contains(tr!("Catalogs")), "the card says what it gives");
    }

    #[test]
    fn a_metadata_source_takes_the_place_it_is_given() {
        let mut list = vec!["tmdb".to_owned(), "cinemeta".to_owned()];
        placed(&mut list, "simkl", Some(0));
        assert_eq!(list, ["simkl", "tmdb", "cinemeta"], "a new source lands where it was put");
        placed(&mut list, "cinemeta", Some(1));
        assert_eq!(list, ["simkl", "cinemeta", "tmdb"], "moving up closes the gap it left");
        placed(&mut list, "simkl", Some(9));
        assert_eq!(list, ["cinemeta", "tmdb", "simkl"], "a place past the end is the end");
        placed(&mut list, "tmdb", None);
        assert_eq!(list, ["cinemeta", "simkl"], "off is off");
        placed(&mut list, "unknown", Some(0));
        assert_eq!(list, ["unknown", "cinemeta", "simkl"], "a source this build has not is still placed");
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
