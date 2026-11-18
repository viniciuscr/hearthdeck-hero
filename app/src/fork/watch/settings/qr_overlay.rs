//! A pairing code and its QR, as an overlay: a phone scans it off the screen.
//!
//! The same element the desktop card draws (`qr::qr_code`, divs of modules) in a
//! panel over the settings rows, because a couch is further from the screen than
//! a desk. The code sits at the top, where it is easiest to read out; the wait
//! for the approval sits under the code, where the eyes already are.

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
    /// What the person types on the phone, when the flow has a code: Stremio's
    /// four letters, Simkl's eight. `None` for a flow that only has a page
    /// (TMDB's approval).
    code: Option<SharedString>,
    url: String,
}

impl QrOverlay {
    pub fn new(title: &str, code: Option<&str>, url: &str) -> Self {
        let code = code.map(str::trim).filter(|code| !code.is_empty()).map(SharedString::from);
        Self { title: title.to_string().into(), code, url: url.to_owned() }
    }
}

impl EventEmitter<QrEvent> for QrOverlay {}

impl Render for QrOverlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Seven pixels a module: a four-inch code on a TV across the room.
        let code = qr::qr_code(&self.url, 7.0);
        let body: AnyElement = match code {
            Some(code) => code.into_any_element(),
            None => div().child(kit::body_text(tr!("That address is too long to be a QR code."))).into_any_element(),
        };
        let pairing = self.code.as_ref().map(|code| {
            div().flex().flex_col().items_center().gap(u(2.)).child(kit::kicker(tr!("Pairing code"))).child(kit::title(code.clone(), 40.))
        });
        div().absolute().inset_0().child(kit::scrim()).child(
            div().absolute().inset_0().flex().items_center().justify_center().child(kit::panel_in(
                "watch-qr-panel",
                div()
                    .max_w(u(900.))
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
                    .children(pairing)
                    .child(body)
                    .child(kit::body_text(tr!("Waiting for the approval: this closes on its own once you approve it.")))
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
