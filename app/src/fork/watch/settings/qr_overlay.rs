//! A QR code as an overlay, for console mode: a phone scans it off the screen.
//!
//! The desktop draws the code beside the account card; the console only draws
//! text, so the code is written as half-block characters (two modules per
//! character) and shown in a panel over the settings rows. Every line is the
//! same width and the quiet zone is part of it, which is what a camera needs.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::qr;
use crate::console::kit::{self, u};
use crate::console::{hint, Btn, Hint, Pad, PadTarget};
use crate::theme;

/// Why the overlay can close itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QrEvent {
    Close,
}

pub struct QrOverlay {
    title: SharedString,
    url: String,
    /// The code as text, two modules per character.
    lines: Vec<String>,
}

impl QrOverlay {
    /// The code of `url`; an address too long for a QR code is a panel that says
    /// so rather than an empty box.
    pub fn new(title: &str, url: &str) -> Self {
        Self { title: title.to_string().into(), url: url.to_owned(), lines: qr::qr_text(url).unwrap_or_default() }
    }
}

impl EventEmitter<QrEvent> for QrOverlay {}

impl Render for QrOverlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut lines = div().flex().flex_col().gap(px(0.)).text_size(u(13.)).text_color(rgb(0xffffff));
        for line in &self.lines {
            lines = lines.child(div().whitespace_nowrap().child(SharedString::from(line.clone())));
        }
        let body = if self.lines.is_empty() {
            div().child(kit::body_text(tr!("That address is too long to be a QR code.")))
        } else {
            // Bare `div`, no layout class: monospaced by the font stack so the
            // blocks line up, on white because a camera needs the contrast.
            div().p(u(16.)).rounded(u(12.)).bg(rgb(0xffffff)).child(div().font_family("monospace").child(lines))
        };
        div().absolute().inset_0().child(kit::scrim()).child(
            div().absolute().inset_0().flex().items_center().justify_center().child(kit::panel_in(
                "watch-qr-panel",
                div()
                    .max_w(u(760.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(u(12.))
                    .p(u(24.))
                    .rounded(u(24.))
                    .bg(theme::console_sidebar())
                    .border_1()
                    .border_color(kit::ink(0.1))
                    .shadow(kit::shadow(0.5, 18., 48.))
                    .child(kit::title(self.title.clone(), 24.))
                    .child(body)
                    .child(kit::body_text(tr!("Scan it with a phone: the app follows when you come back.")))
                    .child(div().text_size(u(12.)).text_color(theme::text_muted()).child(SharedString::from(self.url.clone()))),
            )),
        )
    }
}

impl PadTarget for QrOverlay {
    fn on_pad(&mut self, pad: Pad, _: &mut Window, cx: &mut Context<Self>) -> bool {
        match pad {
            // Any of the way out closes it: the code is not interactive.
            Pad::A | Pad::B | Pad::Start => cx.emit(QrEvent::Close),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn hints(&self, _: &App) -> Vec<Hint> {
        vec![hint(Btn::B, tr!("Close"))]
    }
}
