//! "Add source" panel and custom source actions in the Wine Manager:
//! probe a GitHub repository's releases, pick a name and optional archive
//! variant, save it to `store/wine-sources.json`, or remove a custom source.

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_wine::{CustomSource, ProbeResult, Source};

use super::WineManagerPage;
use crate::theme;
use crate::ui::controls::{button_sm, card_box, close_button, ButtonKind};
use crate::ui::fields;

pub(super) struct AddSourceState {
    pub(super) open: bool,
    pub(super) url: Entity<InputState>,
    pub(super) name: Entity<InputState>,
    pub(super) probing: bool,
    /// Bumped when starting a probe, editing the URL, or closing the form so
    /// stale in-flight probe replies are discarded.
    pub(super) probe_seq: u64,
    pub(super) saving: bool,
    pub(super) probe: Option<ProbeResult>,
    pub(super) variant: usize,
    pub(super) error: Option<String>,
    /// Suggested name waiting to be written into `name` once on the next render.
    pub(super) pending_name: Option<String>,
    /// Custom source `kind` whose removal awaits confirmation.
    pub(super) confirm_remove: Option<String>,
}

impl AddSourceState {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            open: false,
            url: fields::new_input(window, cx, "", "https://github.com/owner/repo"),
            name: fields::new_input(window, cx, "", tr!("Source name")),
            probing: false,
            probe_seq: 0,
            saving: false,
            probe: None,
            variant: 0,
            error: None,
            pending_name: None,
            confirm_remove: None,
        }
    }

    /// Invalidates any in-flight probe and clears its preview/error.
    pub(super) fn invalidate_probe(&mut self) {
        self.probe_seq = self.probe_seq.wrapping_add(1);
        self.probing = false;
        self.probe = None;
        self.error = None;
        self.pending_name = None;
    }

    /// Esc: closes a pending removal confirmation or the Add Source panel.
    pub(super) fn close_modal(&mut self) -> bool {
        if self.confirm_remove.take().is_some() {
            return true;
        }
        if self.open {
            self.open = false;
            self.saving = false;
            self.invalidate_probe();
            return true;
        }
        false
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl WineManagerPage {
    pub(super) fn open_add_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_source.open = true;
        self.add_source.saving = false;
        self.add_source.invalidate_probe();
        self.add_source.variant = 0;
        self.add_source.confirm_remove = None;
        self.show_prefixes = false;
        self.add_source.url.update(cx, |s, cx| s.set_value("", window, cx));
        self.add_source.name.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    pub(super) fn close_add_source(&mut self, cx: &mut Context<Self>) {
        self.add_source.open = false;
        self.add_source.saving = false;
        self.add_source.invalidate_probe();
        cx.notify();
    }

    /// Inspects the GitHub URL in the form, validating that it publishes
    /// compatible `.tar.*` releases and discovering variants.
    pub(super) fn probe_custom_source(&mut self, cx: &mut Context<Self>) {
        if self.add_source.probing || self.add_source.saving {
            return;
        }
        if self.offline {
            self.add_source.error = Some(tr!("You are offline.").to_owned());
            cx.notify();
            return;
        }
        let input = self.add_source.url.read(cx).value().trim().to_owned();
        if input.is_empty() {
            self.add_source.error = Some(tr!("Enter a GitHub repository URL or owner/repo.").to_owned());
            cx.notify();
            return;
        }
        self.add_source.invalidate_probe();
        self.add_source.probing = true;
        self.add_source.variant = 0;
        let seq = self.add_source.probe_seq;
        cx.notify();

        let data_paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    heronite_wine::custom::probe_github_source(&data_paths, &heronite_wine::checks::GitHub, &input, now_secs())
                })
                .await;
            this.update(cx, |this, cx| {
                if this.add_source.probe_seq != seq || !this.add_source.open {
                    return;
                }
                this.add_source.probing = false;
                match res {
                    Ok(probe) => {
                        this.add_source.pending_name = Some(probe.suggested_name.clone());
                        this.add_source.probe = Some(probe);
                        this.add_source.variant = 0;
                        this.add_source.error = None;
                    }
                    Err(e) => {
                        this.add_source.error = Some(format!("{e:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Persists the probed custom source and seeds its release cache.
    pub(super) fn save_custom_source(&mut self, cx: &mut Context<Self>) {
        if self.add_source.saving {
            return;
        }
        let Some(probe) = self.add_source.probe.clone() else { return };
        let typed_name = self.add_source.name.read(cx).value().trim().to_owned();
        let kind = if typed_name.is_empty() { probe.suggested_name.clone() } else { typed_name };
        let asset_filter = probe.variants.get(self.add_source.variant).and_then(|v| v.filter.clone());
        let custom = CustomSource { kind, github: probe.github.clone(), asset_filter };
        let raw = probe.raw_releases.clone();
        let data_paths = self.ctx.data_paths.clone();
        self.add_source.saving = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let added = heronite_wine::custom::add_custom_source(&data_paths, custom, Some(&raw))?;
                    let sources = Source::offered(&data_paths);
                    let releases = heronite_wine::store::read_releases(&data_paths);
                    anyhow::Ok((added, sources, releases))
                })
                .await;
            this.update(cx, |this, cx| {
                this.add_source.saving = false;
                match res {
                    Ok((added, sources, releases)) => {
                        let name = added.kind.clone();
                        this.sources = sources;
                        this.releases = releases;
                        this.source = Source::Custom(added);
                        this.show_prefixes = false;
                        this.add_source.open = false;
                        this.add_source.invalidate_probe();
                        this.toast(tr!("{name} added", name), false, cx);
                    }
                    Err(e) => {
                        this.add_source.error = Some(format!("{e:#}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Console mode shortcut: probes and adds a GitHub repository in one step.
    pub(super) fn add_custom_source_quick(&mut self, input: String, cx: &mut Context<Self>) {
        if self.offline {
            self.toast(tr!("You are offline."), true, cx);
            return;
        }
        let data_paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    let probe =
                        heronite_wine::custom::probe_github_source(&data_paths, &heronite_wine::checks::GitHub, &input, now_secs())?;
                    let custom = CustomSource { kind: probe.suggested_name, github: probe.github, asset_filter: None };
                    let added = heronite_wine::custom::add_custom_source(&data_paths, custom, Some(&probe.raw_releases))?;
                    let sources = Source::offered(&data_paths);
                    let releases = heronite_wine::store::read_releases(&data_paths);
                    anyhow::Ok((added, sources, releases))
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok((added, sources, releases)) => {
                        let name = added.kind.clone();
                        this.sources = sources;
                        this.releases = releases;
                        this.source = Source::Custom(added);
                        this.show_prefixes = false;
                        this.toast(tr!("{name} added", name), false, cx);
                    }
                    Err(e) => this.toast(format!("{e:#}"), true, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Removes a saved custom source (keeps already-installed builds on disk).
    pub(super) fn remove_custom_source(&mut self, kind: String, cx: &mut Context<Self>) {
        self.add_source.confirm_remove = None;
        let data_paths = self.ctx.data_paths.clone();
        cx.spawn(async move |this, cx| {
            let k = kind.clone();
            let res = cx
                .background_spawn(async move {
                    heronite_wine::custom::remove_custom_source(&data_paths, &k)?;
                    let sources = Source::offered(&data_paths);
                    let releases = heronite_wine::store::read_releases(&data_paths);
                    anyhow::Ok((sources, releases))
                })
                .await;
            this.update(cx, |this, cx| {
                match res {
                    Ok((sources, releases)) => {
                        this.sources = sources;
                        this.releases = releases;
                        this.limited.remove(&kind);
                        if this.source.kind() == kind {
                            this.source = this.sources[0].clone();
                        }
                        this.toast(tr!("{name} removed", name = kind), false, cx);
                    }
                    Err(e) => this.toast(format!("{e:#}"), true, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_add_source(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if let Some(suggested) = self.add_source.pending_name.take() {
            self.add_source.name.update(cx, |s, cx| s.set_value(suggested, window, cx));
        }
        let st = &self.add_source;
        let probing = st.probing;
        let check_label = if probing { tr!("Checking\u{2026}") } else { tr!("Check") };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(theme::W_BOLD)
                    .text_color(theme::text_primary())
                    .child(tr!("Add GitHub Proton source")),
            )
            .child(close_button("wine-add-source-close").on_click(cx.listener(|this, _, _, cx| this.close_add_source(cx))));

        let url_row =
            div().flex().items_center().gap(px(10.)).child(div().flex_1().min_w_0().child(fields::input_box(&st.url, probing))).child(
                button_sm("wine-add-source-check", if st.probe.is_some() { ButtonKind::Glass } else { ButtonKind::Primary }, check_label)
                    .when(probing, |b| b.opacity(0.6).cursor_not_allowed())
                    .when(!probing, |b| b.on_click(cx.listener(|this, _, _, cx| this.probe_custom_source(cx)))),
            );

        let mut panel = card_box()
            .py(px(14.))
            .gap(px(10.))
            .child(header)
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(theme::text_muted())
                    .child(tr!("Paste a GitHub repository URL or owner/repo. Only add sources you trust.")),
            )
            .child(url_row);

        if let Some(err) = &st.error {
            panel = panel.child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::danger_text()).child(err.clone()));
        }

        if let Some(probe) = &st.probe {
            let selected = probe.variants.get(st.variant).or_else(|| probe.variants.first());
            let found = tr!("Found {count} releases \u{b7} latest: {tag}", count = probe.release_count, tag = probe.newest_tag);
            panel = panel
                .child(div().h(px(1.)).w_full().bg(theme::row_separator()))
                .child(div().text_size(px(13.)).text_color(theme::success()).child(found))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().w(px(90.)).flex_none().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Name")))
                        .child(div().flex_1().max_w(px(340.)).child(fields::input_box(&st.name, false))),
                );

            if probe.variants.len() > 1 {
                let mut chips = div().flex().flex_wrap().items_center().gap(px(6.));
                for (i, v) in probe.variants.iter().enumerate() {
                    let active = i == st.variant;
                    chips = chips.child(
                        div()
                            .id(SharedString::from(format!("wine-add-variant-{i}")))
                            .px(px(10.))
                            .py(px(5.))
                            .rounded_full()
                            .text_size(px(12.))
                            .cursor_pointer()
                            .border_1()
                            .when(active, |d| d.bg(theme::accent_tint()).border_color(theme::accent()).text_color(theme::text_bright()))
                            .when(!active, |d| {
                                d.bg(theme::button_fill())
                                    .border_color(theme::button_border())
                                    .text_color(theme::text_muted())
                                    .hover(|s| s.bg(theme::button_fill_hover()).text_color(theme::text_primary()))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.add_source.variant = i;
                                cx.notify();
                            }))
                            .child(v.label.clone()),
                    );
                }
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().w(px(90.)).flex_none().text_size(px(13.)).text_color(theme::text_muted()).child(tr!("Variant")))
                        .child(chips),
                );
            }

            if let Some(v) = selected {
                panel = panel.child(
                    div()
                        .text_size(px(12.))
                        .font_family(theme::FONT_MONO)
                        .text_color(theme::text_dim())
                        .child(tr!("Archive: {file}", file = v.example_file)),
                );
            }

            let saving = st.saving;
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(2.))
                    .child(
                        button_sm("wine-add-source-save", ButtonKind::Primary, tr!("Add source"))
                            .when(saving, |b| b.opacity(0.6).cursor_not_allowed())
                            .when(!saving, |b| {
                                b.on_click(cx.listener(|this, _, _, cx| {
                                    this.save_custom_source(cx);
                                }))
                            }),
                    )
                    .child(button_sm("wine-add-source-cancel", ButtonKind::Ghost, tr!("Cancel")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close_add_source(cx);
                        },
                    ))),
            );
        }

        panel
    }
}
