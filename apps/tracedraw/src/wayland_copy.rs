//! What a copy offers other programs on a Wayland clipboard (#2288).
//!
//! On X11 TraceDraw owns the clipboard itself (`x11_copy.rs`); on a
//! Wayland desktop the copy went there as SVG text alone, so a
//! presentation or a word processor pasted the SVG source as text. Here
//! the copy is offered through the compositor's data control protocol,
//! the one the clipboard library already uses for text, as
//! `image/svg+xml`, as `image/png` and as text (the same SVG, which
//! TraceDraw's own paste and other editors read).
//!
//! The protocol takes the data whole when it is offered, so the picture
//! cannot wait for a program to ask for it as on X11: the SVG and the
//! text are offered at once, and the picture is drawn on a thread of its
//! own and offered with them when it is ready, unless another copy was
//! made meanwhile. A compositor without the protocol (GNOME) leaves the
//! copy to the caller, which then offers it on the X server the desktop
//! runs for X11 programs, whose clipboard the desktop shares.

use std::sync::atomic::{AtomicU64, Ordering};
use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};

use crate::x11_copy::DrawPng;

/// The copies made so far: a picture drawn for an older copy is not
/// offered over a newer one.
static COPIES: AtomicU64 = AtomicU64::new(0);

/// What is offered for `svg` and, when drawn, its picture: the vector
/// first, then the picture, then text.
fn sources(svg: &[u8], png: Option<&[u8]>) -> Vec<MimeSource> {
    let mut out = vec![MimeSource {
        source: Source::Bytes(svg.into()),
        mime_type: MimeType::Specific("image/svg+xml".into()),
    }];
    if let Some(png) = png {
        out.push(MimeSource {
            source: Source::Bytes(png.into()),
            mime_type: MimeType::Specific("image/png".into()),
        });
    }
    out.push(MimeSource {
        source: Source::Bytes(svg.into()),
        mime_type: MimeType::Text,
    });
    out
}

fn put(sources: Vec<MimeSource>) -> bool {
    match Options::new().copy_multi(sources) {
        Ok(()) => true,
        Err(e) => {
            log::debug!("Wayland clipboard: {e}");
            false
        }
    }
}

/// Offer a copy on the Wayland clipboard: `svg` as SVG and as text at
/// once, and the picture `png` draws as soon as it is drawn. On a
/// desktop that is not Wayland, or whose compositor does not let the
/// clipboard be set so, the picture's drawing is handed back.
pub fn offer(svg: String, png: DrawPng) -> Result<(), DrawPng> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(png);
    }
    let svg = svg.into_bytes();
    if !put(sources(&svg, None)) {
        return Err(png);
    }
    let copy = COPIES.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new()
        .name("copy picture".into())
        .spawn(move || {
            let Some(png) = png() else {
                return;
            };
            if is_latest(copy) {
                put(sources(&svg, Some(&png)));
            }
        });
    if let Err(e) = spawned {
        log::debug!("copy picture: {e}");
    }
    Ok(())
}

/// Whether `copy` is still the last copy made here.
fn is_latest(copy: u64) -> bool {
    COPIES.load(Ordering::SeqCst) == copy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The copy is offered as SVG, then as a picture once it is drawn,
    /// then as text, so a program that takes pictures before text pastes
    /// a picture, not the SVG source (#2288).
    #[test]
    fn a_copy_offers_the_vector_the_picture_and_text() {
        let kinds =
            |s: &[MimeSource]| -> Vec<MimeType> { s.iter().map(|m| m.mime_type.clone()).collect() };
        let first = sources(b"<svg/>", None);
        assert_eq!(
            kinds(&first),
            [MimeType::Specific("image/svg+xml".into()), MimeType::Text]
        );
        let drawn = sources(b"<svg/>", Some(b"\x89PNG"));
        assert_eq!(
            kinds(&drawn),
            [
                MimeType::Specific("image/svg+xml".into()),
                MimeType::Specific("image/png".into()),
                MimeType::Text
            ]
        );
        assert_eq!(drawn[1].source, Source::Bytes(b"\x89PNG"[..].into()));
        assert_eq!(drawn[2].source, Source::Bytes(b"<svg/>"[..].into()));
    }

    /// A picture drawn for a copy is not offered once another copy was
    /// made.
    #[test]
    fn an_older_copys_picture_is_not_offered() {
        let copy = COPIES.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(is_latest(copy));
        COPIES.fetch_add(1, Ordering::SeqCst);
        assert!(!is_latest(copy));
    }

    /// Off a Wayland desktop the picture's drawing comes back to the
    /// caller, undrawn.
    #[test]
    fn off_wayland_the_copy_is_handed_back() {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return;
        }
        let drawn = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d = drawn.clone();
        let back = offer(
            "<svg/>".into(),
            Box::new(move || {
                d.store(true, Ordering::SeqCst);
                None
            }),
        );
        let Err(png) = back else {
            panic!("offered off Wayland");
        };
        assert!(!drawn.load(Ordering::SeqCst));
        assert!(png().is_none());
        assert!(drawn.load(Ordering::SeqCst));
    }
}
