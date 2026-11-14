//! Effects worked out on the graphics card (Options > General, "Blur
//! shadows on the graphics card"). The renderer itself only draws on the
//! CPU; the application may hand it a [`BlurAccelerator`] that blurs on
//! the graphics card. It is asked only for renders shown on screen
//! (`RenderOptions::draft`), never for exports, printing or Convert to
//! Bitmap, and it must give exactly the pixels the CPU gives, so the
//! screen and the exported file always match. When it declines (no card,
//! a layer too large for it, a lost device) the CPU does the work.

use std::cell::Cell;
use std::sync::{Arc, RwLock};

/// Blurs on the graphics card.
pub trait BlurAccelerator: Send + Sync {
    /// Box blur premultiplied RGBA `data` (`w` x `h` pixels, 4 bytes each)
    /// in place: `passes` times a horizontal then a vertical pass of
    /// `radius`, each pixel the average (rounded down) of the
    /// `2 * radius + 1` pixels about it with the edge pixels repeated, as
    /// [`crate::box_blur_passes`] does. False when it was not done; `data`
    /// is then unchanged.
    fn box_blur(&self, data: &mut [u8], w: u32, h: u32, radius: u32, passes: u32) -> bool;
}

static ACCELERATOR: RwLock<Option<Arc<dyn BlurAccelerator>>> = RwLock::new(None);

/// Layers smaller than this many pixels are blurred on the CPU: sending
/// them to the card and back takes longer than blurring them.
pub const MIN_PIXELS: u64 = 256 * 256;

/// Hand the renderer a graphics card blur, or take it back (`None`).
pub fn set_blur_accelerator(a: Option<Arc<dyn BlurAccelerator>>) {
    match ACCELERATOR.write() {
        Ok(mut g) => *g = a,
        Err(p) => *p.into_inner() = a,
    }
}

/// Whether a graphics card blur is handed over.
pub fn has_blur_accelerator() -> bool {
    ACCELERATOR.read().map(|g| g.is_some()).unwrap_or(false)
}

thread_local! {
    /// Whether the shadow this thread is drawing goes to the screen.
    static ON_SCREEN: Cell<bool> = const { Cell::new(false) };
}

/// Marks the shadows drawn on this thread as shown on screen until it is
/// dropped.
pub(crate) struct OnScreen(bool);

impl Drop for OnScreen {
    fn drop(&mut self) {
        ON_SCREEN.with(|c| c.set(self.0));
    }
}

/// Shadows drawn on this thread until the guard drops go to the screen
/// when `on_screen`.
pub(crate) fn on_screen(on_screen: bool) -> OnScreen {
    OnScreen(ON_SCREEN.with(|c| c.replace(on_screen)))
}

/// Box blur `pm` with `passes` passes of `radius`: on the graphics card
/// when one is handed over, the layer is shown on screen and large
/// enough, else on the CPU.
pub(crate) fn box_blur(pm: &mut tiny_skia::Pixmap, radius: i32, passes: usize) {
    let (w, h) = (pm.width(), pm.height());
    let r = radius.max(1) as u32;
    if ON_SCREEN.with(|c| c.get()) && (w as u64) * (h as u64) >= MIN_PIXELS {
        let a = ACCELERATOR.read().ok().and_then(|g| g.clone());
        if let Some(a) = a {
            if a.box_blur(pm.data_mut(), w, h, r, passes as u32) {
                return;
            }
        }
    }
    crate::box_blur_passes(pm, radius, passes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts the blurs the test's own thread asks for (other tests may
    /// render at the same time) and does them on the CPU.
    struct Counting(AtomicUsize, std::thread::ThreadId);

    impl BlurAccelerator for Counting {
        fn box_blur(&self, data: &mut [u8], w: u32, h: u32, radius: u32, passes: u32) -> bool {
            if std::thread::current().id() == self.1 {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            let mut pm = tiny_skia::Pixmap::new(w, h).expect("pixmap");
            pm.data_mut().copy_from_slice(data);
            crate::box_blur_passes(&mut pm, radius as i32, passes as usize);
            data.copy_from_slice(pm.data());
            true
        }
    }

    /// The card is asked only on screen and only for large layers; the
    /// guard puts back what was there before.
    #[test]
    fn the_card_blurs_only_large_layers_shown_on_screen() {
        let c = Arc::new(Counting(AtomicUsize::new(0), std::thread::current().id()));
        set_blur_accelerator(Some(c.clone()));
        let mut big = tiny_skia::Pixmap::new(300, 300).expect("pixmap");
        let mut small = tiny_skia::Pixmap::new(40, 40).expect("pixmap");
        box_blur(&mut big, 4, 3);
        assert_eq!(c.0.load(Ordering::SeqCst), 0, "an export stays on the CPU");
        {
            let _g = on_screen(true);
            box_blur(&mut small, 4, 3);
            assert_eq!(
                c.0.load(Ordering::SeqCst),
                0,
                "a small layer stays on the CPU"
            );
            box_blur(&mut big, 4, 3);
            assert_eq!(c.0.load(Ordering::SeqCst), 1);
            {
                let _inner = on_screen(false);
                box_blur(&mut big, 4, 3);
                assert_eq!(c.0.load(Ordering::SeqCst), 1);
            }
            box_blur(&mut big, 4, 3);
            assert_eq!(c.0.load(Ordering::SeqCst), 2);
        }
        box_blur(&mut big, 4, 3);
        assert_eq!(c.0.load(Ordering::SeqCst), 2);
        set_blur_accelerator(None);
    }
}
