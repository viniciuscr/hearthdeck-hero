//! Command palette commands of the Wine Manager (see `pages/commands.rs`):
//! check the source on screen and install the newest build of a source.

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_wine::{CheckMode, Repository, Source, WineVersionInfo};

use super::WineManagerPage;
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

impl WineManagerPage {
    /// The newest installable build of `source`.
    fn latest(&self, source: &Source) -> Option<&WineVersionInfo> {
        self.releases.iter().filter(|r| r.kind == source.kind() && r.downloadable()).max_by(|a, b| a.date.cmp(&b.date))
    }

    fn install_target(&self, id: &str) -> Option<Source> {
        id.strip_prefix("install:").and_then(|k| self.sources.iter().find(|s| s.kind() == k).cloned())
    }

    /// Runs the command kept while the cache was read or its source checked.
    pub(super) fn run_palette_pending(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.palette_pending.clone() else { return };
        if !self.loaded || self.install_target(&id).is_some_and(|src| self.manager.checking(&src)) {
            return;
        }
        self.palette_pending = None;
        self.run_command(&id, false, cx);
    }

    /// `first`: not a retry after the source was checked.
    fn run_command(&mut self, id: &str, first: bool, cx: &mut Context<Self>) {
        if id == "refresh" {
            let active = self.source.clone();
            self.check(&active, CheckMode::Manual);
        } else if let Some(src) = self.install_target(id) {
            if !self.loaded {
                self.palette_pending = Some(id.to_string());
                return;
            }
            if self.source != src {
                self.select_source(src.clone());
            } else {
                self.check(&src, CheckMode::Auto);
            }
            match self.latest(&src).cloned() {
                Some(r) if r.installed() => self.toast(tr!("{version} is installed already", version = r.version), false, cx),
                Some(r) => {
                    self.manager.start_install(r);
                }
                None if first && self.manager.checking(&src) => self.palette_pending = Some(id.to_string()),
                None => self.toast(tr!("No {kind} release found: refresh the list", kind = src.kind()), true, cx),
            }
        }
        cx.notify();
    }
}

impl PaletteCommands for WineManagerPage {
    fn palette_commands(page: Option<&Self>, _ctx: &CommandCtx, _cx: &App) -> Vec<PageCommand> {
        let busy = page.is_some_and(|p| p.checking());
        let mut v = vec![PageCommand::new(
            "refresh",
            tr!("Refresh Wine / Proton releases"),
            tr!("Check the source on screen for new builds"),
            Icon::Refresh,
        )
        .keywords(tr!("proton wine runner update list"))
        .disabled(busy.then_some(tr!("Fetching releases already")))];
        let fallback: Vec<Source> = Repository::offered().iter().copied().map(Source::Builtin).collect();
        let sources: &[Source] = page.map(|p| p.sources.as_slice()).unwrap_or(&fallback);
        for src in sources {
            let kind = src.kind();
            let mut c = PageCommand::new(
                format!("install:{kind}"),
                tr!("Install the latest {kind}", kind),
                tr!("Newest build of this source"),
                Icon::Download,
            )
            .keywords(tr!("proton wine runner download compatibility"));
            if let Some(p) = page.filter(|p| p.loaded) {
                match p.latest(src) {
                    Some(r) if r.installed() => c = c.disabled(Some(tr!("{version} is installed", version = r.version))),
                    Some(r) if p.job(&r.version).is_some() => c = c.disabled(Some(tr!("Installing {version}", version = r.version))),
                    Some(r) => c.subtitle = format!("{} \u{b7} {}", r.version, r.date),
                    None => {}
                }
            }
            v.push(c);
        }
        v
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(id, true, cx);
    }

    fn close_modal(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let closed = self.confirm_remove.take().is_some() || self.add_source.close_modal() || self.prefixes.close_question();
        cx.notify();
        closed
    }
}
