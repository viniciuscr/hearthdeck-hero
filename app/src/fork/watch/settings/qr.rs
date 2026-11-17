//! A QR code drawn as divs: one row per line of modules, each run of dark
//! modules one div. Always black on white with the standard quiet zone, whatever
//! the theme: phone cameras read that best.
//!
//! `fast_qr` does the encoding; the drawing is ours so the code is a real
//! element (it scales with the window and prints crisply), not a bitmap.

use fast_qr::{QRBuilder, QRCode, ECL};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// The light margin around the code, in modules (the standard's minimum).
const QUIET_ZONE: usize = 4;

/// The code of `text`. `None` when the text does not fit a QR code.
fn encode(text: &str) -> Option<QRCode> {
    // Medium recovery, the level a screen shows comfortably: strong enough to
    // survive a glare, small enough to stay scannable from a couch.
    QRBuilder::new(text.as_bytes()).ecl(ECL::M).build().ok()
}

/// The runs of one line of modules: `(dark, length)`.
fn runs(qr: &QRCode, y: usize) -> Vec<(bool, usize)> {
    let mut out: Vec<(bool, usize)> = Vec::new();
    for module in &qr[y] {
        let dark = module.value();
        match out.last_mut() {
            Some((d, n)) if *d == dark => *n += 1,
            _ => out.push((dark, 1)),
        }
    }
    out
}

/// The code of `text`, `module` pixels per module (a whole number keeps the
/// modules sharp).
pub fn qr_code(text: &str, module: f32) -> Option<Div> {
    let qr = encode(text)?;
    let side = (qr.size + 2 * QUIET_ZONE) as f32 * module;
    let black = rgb(0x000000);
    let lines = (0..qr.size).map(|y| {
        div()
            .flex()
            .h(px(module))
            .children(runs(&qr, y).into_iter().map(|(dark, n)| div().h_full().w(px(n as f32 * module)).when(dark, |d| d.bg(black))))
    });
    Some(
        div()
            .flex_none()
            .size(px(side))
            .p(px(QUIET_ZONE as f32 * module))
            .rounded(px(10.))
            .bg(rgb(0xffffff))
            .child(div().flex().flex_col().children(lines)),
    )
}

#[cfg(test)]
mod tests {
    // Not `super::*` in the drawing tests: GPUI's own `test` attribute would
    // shadow the standard one.
    use super::{encode, runs};

    #[test]
    fn runs_cover_each_line() {
        let qr = encode("https://s.team/q/1/8406597215213412810").expect("a link fits");
        for y in 0..qr.size {
            let line = runs(&qr, y);
            assert_eq!(line.iter().map(|(_, n)| n).sum::<usize>(), qr.size);
            assert!(line.windows(2).all(|w| w[0].0 != w[1].0), "runs alternate");
        }
    }
}
