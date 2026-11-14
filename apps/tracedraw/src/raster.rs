//! Keeps the rasterized page as an egui texture, re-rendering only when the
//! document, view, canvas size or drag preview changed. While the view
//! moves (a pan, a scroll, a zoom) the last render is shown shifted or
//! scaled into place and the page is rendered anew once the view has
//! rested for a moment, so a page of thousands of objects pans smoothly.
//! A page that takes long to draw is drawn on a thread of its own: the
//! window keeps answering, showing the last picture moved into place,
//! until the new one is ready (#345).
//!
//! A zoom on the main canvas does not wait for the view to rest: the
//! last picture, scaled, looks blurry, so the page is rendered for the
//! new zoom on a thread at once. A zoom step that comes while that render
//! runs does not restart it: its picture is taken when ready (closer to
//! the zoom shown than the one before) and the next render starts at once
//! for the latest zoom, so a burst of wheel notches shows a sharper
//! picture every render and the last one about a render after the last
//! step.

use crate::view::View;
use egui::{ColorImage, Context, Rect, TextureHandle, TextureId, TextureOptions};
use tracedraw_core::{geometry::Affine, Document, PageId, ShapeId};
use tracedraw_render::{render_page, Preview, RenderOptions, ViewTransform};
#[cfg(not(target_arch = "wasm32"))]
use tracedraw_render::{
    render_page_cancellable, render_page_on_one_thread, render_page_progressive,
};

/// How long the view must rest, seconds, before a moved view is
/// rendered anew instead of showing the last render moved into place.
pub const SETTLE_SECONDS: f64 = 0.12;

/// A render that took longer than this, seconds, makes the next ones
/// run on a thread of their own.
const SLOW_RENDER: f64 = 0.03;

/// How long the page's picture must have been the one asked for, seconds,
/// before the whole page is drawn small for the backdrop: while the
/// drawing or the view keeps changing no backdrop is drawn.
#[cfg(not(target_arch = "wasm32"))]
pub const BACKDROP_REST: f64 = 0.4;

/// How long until the backdrop is tried again when the pointer was
/// moving in the frame it came due, seconds.
#[cfg(not(target_arch = "wasm32"))]
const BACKDROP_IDLE: f64 = 0.1;

/// Largest side of the backdrop, pixels.
#[cfg(not(target_arch = "wasm32"))]
const BACKDROP_MAX: f64 = 2048.0;

/// What a backdrop shows: the drawing at a revision, drawn one way.
#[cfg(not(target_arch = "wasm32"))]
#[derive(PartialEq, Clone, Debug)]
struct BackKey {
    revision: u64,
    page: PageId,
    /// The part of the page it covers, thousandths of a millimetre.
    area: [i64; 4],
    size: (u32, u32),
    wireframe: bool,
    simulate_overprints: bool,
    complex_effects: bool,
    proof: bool,
}

/// The whole page drawn small (see [`Raster::backdrop`]).
#[cfg(not(target_arch = "wasm32"))]
struct Backdrop {
    texture: TextureHandle,
    key: BackKey,
    page_rect: tracedraw_core::geometry::Rect,
}

/// A backdrop being drawn on its thread.
#[cfg(not(target_arch = "wasm32"))]
struct BackJob {
    key: BackKey,
    page_rect: tracedraw_core::geometry::Rect,
    rx: std::sync::mpsc::Receiver<Option<ColorImage>>,
}

/// A render running on its own thread: what it is for, and where its
/// picture will come from.
#[cfg(not(target_arch = "wasm32"))]
struct Job {
    key: Key,
    page_rect: Option<tracedraw_core::geometry::Rect>,
    rx: std::sync::mpsc::Receiver<Option<(ColorImage, f64)>>,
    /// Set to make the render give up: the view zoomed on before it
    /// was ready (#2522).
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The main canvas's picture a band of rows at a time as the bands
    /// come done (#2389): the first row of each and its pixels.
    bands: Option<std::sync::mpsc::Receiver<(u32, ColorImage)>>,
    /// The bands come so far, on a see-through texture the picture's
    /// size, shown over the last picture until the whole one is ready.
    incoming: Option<TextureHandle>,
    /// The picture's size, pixels.
    size: (u32, u32),
}

/// Turns to render on a thread: one page job runs at a time in the whole
/// program (each holds a copy of the drawing and decodes on band threads
/// of its own; Multipage view used to start one per slow page at once,
/// and a job made stale kept running beside the next one). The page being
/// edited goes first: when it had to wait, the turn is kept for it for a
/// moment after the running job ends, and other pages wait.
#[cfg(not(target_arch = "wasm32"))]
struct Turns(std::sync::Mutex<TurnState>);

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct TurnState {
    busy: bool,
    /// The current page asked while a job ran.
    current_waits: bool,
    /// Until when a freed turn is kept for the current page.
    kept_until: Option<web_time::Instant>,
    /// Turns given to other pages, for tests.
    #[cfg(test)]
    to_others: usize,
}

/// How long a freed turn waits for the current page that asked for it.
#[cfg(not(target_arch = "wasm32"))]
const KEEP_FOR_CURRENT: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(not(target_arch = "wasm32"))]
impl Turns {
    const fn new() -> Self {
        Turns(std::sync::Mutex::new(TurnState {
            busy: false,
            current_waits: false,
            kept_until: None,
            #[cfg(test)]
            to_others: 0,
        }))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, TurnState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The turn for the current page (`current`) or another one, unless a
    /// job holds it or it is kept for the current page.
    fn take(&'static self, current: bool) -> Option<Turn> {
        let mut st = self.state();
        if st.busy {
            st.current_waits |= current;
            return None;
        }
        if !current && st.kept_until.is_some_and(|t| web_time::Instant::now() < t) {
            return None;
        }
        st.busy = true;
        #[cfg(test)]
        if !current {
            st.to_others += 1;
        }
        if current {
            st.current_waits = false;
            st.kept_until = None;
        }
        Some(Turn(self))
    }

    /// How long until a refused page should ask again, or `None` when a
    /// job holds the turn (its end repaints the window). A turn that is
    /// only kept for the current page has no job to wake the window, so
    /// whoever was refused must ask for a repaint of its own (#1384).
    fn retry_after(&self) -> Option<std::time::Duration> {
        let st = self.state();
        if st.busy {
            return None;
        }
        Some(st.kept_until.map_or(std::time::Duration::ZERO, |t| {
            t.saturating_duration_since(web_time::Instant::now())
        }))
    }
}

/// A job's turn, given back when it is dropped.
#[cfg(not(target_arch = "wasm32"))]
struct Turn(&'static Turns);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Turn {
    fn drop(&mut self) {
        let mut st = self.0.state();
        st.busy = false;
        if std::mem::take(&mut st.current_waits) {
            st.kept_until = Some(web_time::Instant::now() + KEEP_FOR_CURRENT);
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
static RENDER_TURNS: Turns = Turns::new();

/// The program's turns, which every page of its window shares. Under
/// test each test thread has turns of its own: the tests run at the same
/// time, each with a window of its own, and one test's job would hold
/// back another's page, whose picture then came at a moment the test
/// could not foresee (a render starting in the middle of a drag).
#[cfg(not(target_arch = "wasm32"))]
fn program_turns() -> &'static Turns {
    #[cfg(test)]
    {
        thread_local! {
            static TURNS: &'static Turns = Box::leak(Box::new(Turns::new()));
        }
        TURNS.with(|t| *t)
    }
    #[cfg(not(test))]
    {
        &RENDER_TURNS
    }
}

#[derive(Default)]
pub struct Raster {
    texture: Option<TextureHandle>,
    key: Option<Key>,
    /// The part of the page the texture shows, page mm.
    page_rect: Option<tracedraw_core::geometry::Rect>,
    /// While the view moves: the view last asked for and when (egui
    /// time) it will be rendered if it stays.
    settle: Option<(Key, f64)>,
    /// Proofed colours on a grid of screen colours (View > Proof Colors),
    /// made when first needed and dropped with the texture (the colour
    /// settings changed).
    proof: Option<std::sync::Arc<ProofLut>>,
    /// How long the last render took, seconds.
    last_seconds: f64,
    /// The render running on its own thread, if any.
    #[cfg(not(target_arch = "wasm32"))]
    job: Option<Job>,
    /// A picture whose canvas follows the zoom (a page beside the current
    /// one in the Multipage View): the last render stays moved and scaled
    /// into place while the view moves even though the canvas changed
    /// size, instead of rendering anew every frame, and once there is a
    /// picture the next ones are drawn on a thread of their own
    /// (#902, #903).
    pub follows_zoom: bool,
    /// No render this frame (the frame's time for them is spent): the
    /// last picture shows where its part of the page now is, and the
    /// render waits for a later frame (#903).
    pub defer: bool,
    /// The drawing as given, shared, for a render on its own thread to
    /// take instead of a copy of its own (no threads in the browser).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub snapshot: Option<std::sync::Arc<Document>>,
    /// What the page's objects cover (page mm), with the revision it was
    /// measured at: a page beside the current one shows its objects off
    /// the page too, and measuring them every frame would cost time
    /// (#2055).
    pub content: Option<(u64, Option<tracedraw_core::geometry::Rect>)>,
    /// The copy of the drawing the last render on a thread took, with
    /// its revision: the next renders of the same drawing (every step of
    /// a zoom) share it instead of copying the drawing again on the
    /// window's thread (a copy of a large drawing takes a frame's time).
    #[cfg(not(target_arch = "wasm32"))]
    kept: Option<(u64, std::sync::Arc<Document>)>,
    /// Renders started so far, on this thread or another.
    renders: u64,
    /// Since when (egui time) the picture has been the one asked for,
    /// frame after frame: nothing moved and nothing changed.
    #[cfg(not(target_arch = "wasm32"))]
    rested: Option<f64>,
    /// The whole page drawn small, shown where the picture does not
    /// reach while the view moves (#2048).
    #[cfg(not(target_arch = "wasm32"))]
    backdrop: Option<Backdrop>,
    /// The backdrop being drawn on its thread, if any.
    #[cfg(not(target_arch = "wasm32"))]
    backdrop_job: Option<BackJob>,
    /// Backdrops started so far.
    #[cfg(not(target_arch = "wasm32"))]
    backdrops: u64,
    /// Turns other than the program's, for tests.
    #[cfg(not(target_arch = "wasm32"))]
    turns: Option<&'static Turns>,
    /// Every render counts as slow, for tests.
    #[cfg(test)]
    always_slow: bool,
    /// A lock a render on its thread takes before drawing, for tests that
    /// hold a render while the view moves on.
    #[cfg(test)]
    hold: Option<std::sync::Arc<std::sync::Mutex<()>>>,
}

/// Screen colours as they would print, on a `N` x `N` x `N` grid of the
/// screen's red, green and blue; colours between are interpolated.
struct ProofLut {
    table: Vec<[f32; 3]>,
}

impl ProofLut {
    const N: usize = 17;

    fn new() -> Self {
        let n = Self::N;
        let step = 1.0 / (n - 1) as f32;
        let mut table = Vec::with_capacity(n * n * n);
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    table.push(tracedraw_core::color::proof_rgb([
                        r as f32 * step,
                        g as f32 * step,
                        b as f32 * step,
                    ]));
                }
            }
        }
        ProofLut { table }
    }

    /// The proofed colour of `rgb` (0 to 1), interpolated between the
    /// eight grid colours around it.
    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = Self::N;
        let scale = (n - 1) as f32;
        let mut i0 = [0usize; 3];
        let mut t = [0.0f32; 3];
        for k in 0..3 {
            let x = (rgb[k].clamp(0.0, 1.0)) * scale;
            let i = (x.floor() as usize).min(n - 2);
            i0[k] = i;
            t[k] = x - i as f32;
        }
        let at = |r: usize, g: usize, b: usize| self.table[(r * n + g) * n + b];
        let mut out = [0.0f32; 3];
        for (dr, wr) in [(0, 1.0 - t[0]), (1, t[0])] {
            for (dg, wg) in [(0, 1.0 - t[1]), (1, t[1])] {
                for (db, wb) in [(0, 1.0 - t[2]), (1, t[2])] {
                    let w = wr * wg * wb;
                    if w == 0.0 {
                        continue;
                    }
                    let c = at(i0[0] + dr, i0[1] + dg, i0[2] + db);
                    for k in 0..3 {
                        out[k] += c[k] * w;
                    }
                }
            }
        }
        out
    }

    /// A premultiplied pixel, proofed.
    fn pixel(&self, r: u8, g: u8, b: u8, a: u8) -> egui::Color32 {
        if a == 0 {
            return egui::Color32::TRANSPARENT;
        }
        let af = a as f32 / 255.0;
        let un = |v: u8| (v as f32 / 255.0 / af).min(1.0);
        let p = self.apply([un(r), un(g), un(b)]);
        let q = |v: f32| (v.clamp(0.0, 1.0) * af * 255.0).round() as u8;
        egui::Color32::from_rgba_premultiplied(q(p[0]), q(p[1]), q(p[2]), a)
    }
}

/// A rendered pixmap as an egui image, proofed through `lut` when given.
fn to_image(pixmap: &tiny_skia::Pixmap, lut: Option<&ProofLut>) -> ColorImage {
    let (w, h) = (pixmap.width(), pixmap.height());
    let pixels: Vec<egui::Color32> = pixmap
        .pixels()
        .iter()
        .map(|p| match lut {
            Some(lut) => lut.pixel(p.red(), p.green(), p.blue(), p.alpha()),
            None => egui::Color32::from_rgba_premultiplied(p.red(), p.green(), p.blue(), p.alpha()),
        })
        .collect();
    ColorImage {
        size: [w as usize, h as usize],
        source_size: egui::vec2(w as f32, h as f32),
        pixels,
    }
}

#[derive(PartialEq, Clone, Debug)]
struct Key {
    revision: u64,
    page: PageId,
    zoom: f32,
    /// The page origin relative to the canvas: what the render uses. A
    /// canvas that moves (a bar docked elsewhere) without changing size
    /// still needs a new render when the view stays put.
    offset: egui::Vec2,
    size: (u32, u32),
    preview: Option<(Vec<ShapeId>, [f64; 6])>,
    wireframe: bool,
    simulate_overprints: bool,
    complex_effects: bool,
    proof: bool,
    focus: Option<(Vec<ShapeId>, u32)>,
    /// The Pixels view: the document pixels drawn (dpi, first column, top
    /// row).
    pixels: Option<(u64, i64, i64)>,
}

/// The document pixels that cover a screen rectangle: `w` by `h` pixels
/// whose top-left one is column `ix0`, row `iy1` (rows counted up from the
/// page's bottom edge, the row above `iy1` excluded), and where they land
/// on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelWindow {
    pub ix0: i64,
    pub iy1: i64,
    pub w: u32,
    pub h: u32,
    pub screen: Rect,
}

/// Largest side of the Pixels view texture.
const MAX_PIXELS: i64 = 8192;

/// The document pixels at `dpi` behind `rect`, when one of them spans at
/// least one physical screen pixel (`ppp` per point); below that the
/// normal render already shows them.
pub fn pixel_window(view: &View, rect: Rect, dpi: f64, ppp: f32) -> Option<PixelWindow> {
    if !dpi.is_finite() || dpi <= 0.0 || !view.zoom.is_finite() || view.zoom <= 0.0 {
        return None;
    }
    let k = dpi / 25.4;
    if (view.zoom * ppp) as f64 / k < 1.0 {
        return None;
    }
    let tl = view.to_page(rect.left_top());
    let br = view.to_page(rect.right_bottom());
    let ix0 = (tl.x * k).floor() as i64;
    let ix1 = (br.x * k).ceil() as i64;
    let iy0 = (br.y * k).floor() as i64;
    let iy1 = (tl.y * k).ceil() as i64;
    let (w, h) = (ix1 - ix0, iy1 - iy0);
    if w <= 0 || h <= 0 || w > MAX_PIXELS || h > MAX_PIXELS {
        return None;
    }
    let a = view.to_screen(tracedraw_core::geometry::Point::new(
        ix0 as f64 / k,
        iy1 as f64 / k,
    ));
    let b = view.to_screen(tracedraw_core::geometry::Point::new(
        ix1 as f64 / k,
        iy0 as f64 / k,
    ));
    Some(PixelWindow {
        ix0,
        iy1,
        w: w as u32,
        h: h as u32,
        screen: Rect::from_min_max(a, b),
    })
}

impl Raster {
    /// Drop the cached texture so the next frame re-renders (and the
    /// proofed colours, which the colour settings decide).
    pub fn invalidate(&mut self) {
        self.key = None;
        self.proof = None;
        self.settle = None;
        // A render under way is for what no longer holds.
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.job = None;
            self.rested = None;
            self.backdrop = None;
            self.backdrop_job = None;
            // Freed on a thread of its own, as a render frees its copy.
            if let Some((_, d)) = self.kept.take() {
                std::thread::spawn(move || drop(d));
            }
        }
    }

    /// True once a picture has been rendered.
    pub fn has_picture(&self) -> bool {
        self.texture.is_some()
    }

    /// How many renders this picture has started.
    /// The part of the page the picture shows, page mm.
    #[cfg(test)]
    pub fn shown_part(&self) -> Option<tracedraw_core::geometry::Rect> {
        self.page_rect
    }

    pub fn render_count(&self) -> u64 {
        self.renders
    }

    /// True while a picture is on its way: a render running on its
    /// thread, or a moved view waiting to rest before it is rendered.
    #[cfg(test)]
    pub fn drawing(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        if self.job.is_some() {
            return true;
        }
        self.settle.is_some()
    }

    /// The zoom the picture shown was rendered at, for tests.
    #[cfg(test)]
    pub fn picture_zoom(&self) -> Option<f32> {
        self.texture.as_ref()?;
        self.key.as_ref().map(|k| k.zoom)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn texture(
        &mut self,
        ctx: &Context,
        doc: &Document,
        page: PageId,
        view: &View,
        rect: Rect,
        preview: Option<(Vec<ShapeId>, Affine)>,
        revision: u64,
        wireframe: bool,
        simulate_overprints: bool,
        complex_effects: bool,
        proof: bool,
        focus: Option<(Vec<ShapeId>, f32)>,
        pixel_dpi: Option<f64>,
    ) -> Option<(TextureId, Rect)> {
        let ppp = ctx.pixels_per_point();
        let window = pixel_dpi.and_then(|dpi| pixel_window(view, rect, dpi, ppp));
        let (w, h) = match window {
            Some(win) => (win.w, win.h),
            None => (
                (rect.width() * ppp).round().max(1.0) as u32,
                (rect.height() * ppp).round().max(1.0) as u32,
            ),
        };
        let screen = window.map_or(rect, |win| win.screen);
        let key = Key {
            revision,
            page,
            zoom: view.zoom,
            offset: view.origin - rect.min,
            size: (w, h),
            preview: preview.as_ref().map(|(s, t)| (s.clone(), t.as_coeffs())),
            wireframe,
            simulate_overprints,
            complex_effects,
            proof,
            focus: focus
                .as_ref()
                .map(|(ids, v)| (ids.clone(), (v * 1000.0) as u32)),
            pixels: window
                .zip(pixel_dpi)
                .map(|(win, dpi)| ((dpi * 1000.0).round() as u64, win.ix0, win.iy1)),
        };
        // Rested only while the picture stays the one asked for.
        #[cfg(not(target_arch = "wasm32"))]
        let rested = self.rested.take();
        // A render on its thread: taken when ready; while it runs for the
        // view asked for (or the drawing changes faster than it draws), the
        // last picture shows, moved into place.
        #[cfg(not(target_arch = "wasm32"))]
        if self.poll_job(ctx) {
            // A render for a zoom the view has already left: nobody will
            // see its picture, and the one for this zoom would wait for
            // all of it, so a large drawing stayed blurry for two whole
            // renders after each zoom step (#2522). It gives up at its
            // next object; the scaled picture shows meanwhile.
            if let Some(job) = &self.job {
                if job.key != key
                    && job.key.same_but_view(&key, self.follows_zoom)
                    && self.zooms_at_once(&job.key, &key)
                {
                    job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let running_for_this = self.job.as_ref().is_some_and(|j| j.key == key);
            if running_for_this || window.is_none() {
                return self.shown_meanwhile(view, screen);
            }
        }
        if self.key.as_ref() == Some(&key) {
            if let Some(t) = &self.texture {
                self.settle = None;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.rested = rested.or_else(|| Some(ctx.input(|i| i.time)));
                }
                return Some((t.id(), screen));
            }
        }
        // The view moved (pan, scroll, zoom) and nothing else changed:
        // show the last render moved into place until the view rests. A
        // pan shows the old picture as sharp as it was; a zoom scales it,
        // blurry, so the main canvas renders the new zoom at once on a
        // thread (one render at a time: one still running for an earlier
        // step is waited for above, then the latest zoom goes next).
        let mut zoom_now = false;
        if window.is_none() {
            if let (Some(old), Some(t), Some(page_rect)) =
                (self.key.as_ref(), self.texture.as_ref(), self.page_rect)
            {
                let moved = old.same_but_view(&key, self.follows_zoom);
                zoom_now = moved && self.zooms_at_once(old, &key);
                if moved && !zoom_now {
                    let now = ctx.input(|i| i.time);
                    let due = match &self.settle {
                        Some((k, due)) if *k == key => *due,
                        _ => now + SETTLE_SECONDS,
                    };
                    if now < due {
                        self.settle = Some((key, due));
                        ctx.request_repaint_after(std::time::Duration::from_secs_f64(due - now));
                        return Some((t.id(), view.rect_to_screen(page_rect)));
                    }
                }
            }
        }
        // The browser draws on its one thread: never set there.
        #[cfg(target_arch = "wasm32")]
        let _ = zoom_now;
        if self.defer {
            ctx.request_repaint();
            let t = self.texture.as_ref()?;
            let page_rect = self.page_rect?;
            return Some((t.id(), view.rect_to_screen(page_rect)));
        }
        self.settle = None;
        let vt = match (window, pixel_dpi) {
            // One render pixel per document pixel, the window's top-left
            // one at the corner.
            (Some(win), Some(dpi)) => ViewTransform {
                zoom: dpi / 25.4,
                origin_x: -(win.ix0 as f64),
                origin_y: win.iy1 as f64,
            },
            _ => ViewTransform {
                zoom: (view.zoom * ppp) as f64,
                origin_x: ((view.origin.x - rect.min.x) * ppp) as f64,
                origin_y: ((view.origin.y - rect.min.y) * ppp) as f64,
            },
        };
        let opts = RenderOptions {
            width: w,
            height: h,
            view: vt,
            preview: preview.map(|(shapes, transform)| Preview { shapes, transform }),
            wireframe,
            simulate_overprints,
            complex_effects,
            anti_alias: true,
            draft: true,
            focus,
        };
        let lut = proof.then(|| {
            self.proof
                .get_or_insert_with(|| std::sync::Arc::new(ProofLut::new()))
                .clone()
        });
        let page_rect = (window.is_none()).then(|| {
            tracedraw_core::geometry::Rect::new(
                -vt.origin_x / vt.zoom,
                (vt.origin_y - h as f64) / vt.zoom,
                (w as f64 - vt.origin_x) / vt.zoom,
                vt.origin_y / vt.zoom,
            )
        });
        // A slow page, with a picture to show meanwhile: drawn on a
        // thread of its own, the window answering all along. A page
        // beside the current one always is: many of them come due in the
        // same moment, once a zoom rests (#903).
        #[cfg(test)]
        if self.always_slow {
            self.last_seconds = 1.0;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if window.is_none()
            && (self.last_seconds > SLOW_RENDER || self.follows_zoom || zoom_now)
            && self.texture.is_some()
        {
            // Another page's job (or a stale one) is running: the last
            // picture shows meanwhile, and the job holding the turn
            // repaints when it ends. A page beside the current one
            // yields to it.
            let turns = self.turns.unwrap_or_else(program_turns);
            let Some(turn) = turns.take(!self.follows_zoom) else {
                if let Some(wait) = turns.retry_after() {
                    ctx.request_repaint_after(wait);
                }
                return self.shown_meanwhile(view, screen);
            };
            let (doc, stale) = match self.snapshot.take() {
                Some(shared) => (shared, None),
                None => self.copy_of(doc, key.revision),
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            #[cfg(test)]
            let hold = self.hold.clone();
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let gives_up = cancel.clone();
            // The main canvas's picture comes a band at a time, the middle
            // of the window first, laid over the last one as it comes
            // (#2389); the pages beside it come whole.
            let (bands_tx, bands) = if self.follows_zoom {
                (None, None)
            } else {
                let (t, r) = std::sync::mpsc::channel();
                (Some(t), Some(r))
            };
            std::thread::spawn(move || {
                #[cfg(test)]
                let _held = hold.as_ref().map(|m| m.lock());
                let t = web_time::Instant::now();
                let image = match &bands_tx {
                    Some(bands_tx) => {
                        let band_wake = wake.clone();
                        let lut = lut.as_deref();
                        let band = move |y0: u32, rows: &tiny_skia::Pixmap| {
                            if bands_tx.send((y0, to_image(rows, lut))).is_ok() {
                                band_wake.request_repaint();
                            }
                        };
                        render_page_progressive(&doc, page, &opts, &gives_up, h / 2, &band)
                    }
                    None => render_page_cancellable(&doc, page, &opts, &gives_up),
                }
                .map(|pm| to_image(&pm, lut.as_deref()));
                let _ = tx.send(image.map(|i| (i, t.elapsed().as_secs_f64())));
                drop(doc);
                drop(stale);
                drop(turn);
                wake.request_repaint();
            });
            self.renders += 1;
            self.job = Some(Job {
                key,
                page_rect,
                rx,
                cancel,
                bands,
                incoming: None,
                size: (w, h),
            });
            return self.shown_meanwhile(view, screen);
        }
        let t = web_time::Instant::now();
        self.renders += 1;
        let pixmap = render_page(doc, page, &opts)?;
        let image = to_image(&pixmap, lut.as_deref());
        self.last_seconds = t.elapsed().as_secs_f64();
        // Document pixels are blown up as blocks, the way they will look.
        let filter = if window.is_some() {
            TextureOptions::NEAREST
        } else {
            TextureOptions::LINEAR
        };
        match &mut self.texture {
            Some(t) => t.set(image, filter),
            None => self.texture = Some(ctx.load_texture("page", image, filter)),
        }
        self.key = Some(key);
        self.page_rect = page_rect;
        self.texture.as_ref().map(|t| (t.id(), screen))
    }

    /// The whole page drawn small, for the parts of the canvas the page's
    /// picture does not reach while the view moves (#2048). The picture
    /// covers the canvas only, the part of the page on screen when it was
    /// rendered: zoomed out, or panned, it is shown scaled or shifted into
    /// place until the new one is ready, and the rest of the page showed
    /// as blank paper meanwhile, a white flash on a dark design at every
    /// zoom step. The backdrop is `area` (page mm: the page and what is drawn
    /// beside it, see `App::backdrop_area`) fitted into
    /// the canvas `rect`, at most [`BACKDROP_MAX`] pixels a side: one more
    /// texture no larger than the picture's own, whatever the zoom.
    ///
    /// It is drawn on one thread of its own (not in bands on every core
    /// as a picture somebody waits for is), once per change of the
    /// drawing, after the picture has been the one asked for during
    /// [`BACKDROP_REST`] seconds (so never while the drawing changes or
    /// the view moves) and once the pointer is still with no button or
    /// key held: the drawing is copied for the thread in that frame,
    /// which on a large drawing takes a frame's time nobody then sees.
    /// It takes no render turn: a zoom that starts meanwhile renders at
    /// once all the same. Returns the backdrop of
    /// the drawing as it is now and where it shows on screen, or `None`
    /// while there is none (a backdrop of an earlier state is never
    /// shown: it would bring back what was deleted).
    ///
    /// Call it after [`Raster::texture`], every frame.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    pub fn backdrop(
        &mut self,
        ctx: &Context,
        doc: &Document,
        page: PageId,
        area: tracedraw_core::geometry::Rect,
        view: &View,
        rect: Rect,
        revision: u64,
        wireframe: bool,
        simulate_overprints: bool,
        complex_effects: bool,
        proof: bool,
    ) -> Option<(TextureId, Rect)> {
        // The one on its thread: taken when ready.
        if let Some(job) = &self.backdrop_job {
            match job.rx.try_recv() {
                Ok(image) => {
                    let job = self.backdrop_job.take()?;
                    if let Some(image) = image {
                        let texture = match self.backdrop.take() {
                            Some(mut b) => {
                                b.texture.set(image, TextureOptions::LINEAR);
                                b.texture
                            }
                            None => {
                                ctx.load_texture("page backdrop", image, TextureOptions::LINEAR)
                            }
                        };
                        self.backdrop = Some(Backdrop {
                            texture,
                            key: job.key,
                            page_rect: job.page_rect,
                        });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.backdrop_job = None,
            }
        }
        let ppp = ctx.pixels_per_point();
        let (aw, ah) = (area.width(), area.height());
        if !(aw.is_finite() && ah.is_finite() && aw > 0.0 && ah > 0.0) {
            return None;
        }
        // The page fitted into the canvas, as Zoom To Page shows it.
        let scale = ((rect.width() * ppp) as f64 / aw)
            .min((rect.height() * ppp) as f64 / ah)
            .min(BACKDROP_MAX / aw.max(ah));
        if !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        let w = (aw * scale).ceil().max(1.0) as u32;
        let h = (ah * scale).ceil().max(1.0) as u32;
        let q = |v: f64| (v * 1000.0).round() as i64;
        let key = BackKey {
            revision,
            page,
            area: [q(area.x0), q(area.y0), q(area.x1), q(area.y1)],
            size: (w, h),
            wireframe,
            simulate_overprints,
            complex_effects,
            proof,
        };
        let have = self.backdrop.as_ref().is_some_and(|b| b.key == key);
        if !have && self.backdrop_job.is_none() {
            let now = ctx.input(|i| i.time);
            // Not in a frame the hand is moving in: the copy of the
            // drawing made below may take that frame's time. (The
            // pointer's speed reads still for the first frames of a
            // move, so a move in this very frame counts too.)
            let idle = ctx.input(|i| {
                i.pointer.is_still()
                    && i.pointer.time_since_last_movement() > 0.0
                    && !i.pointer.any_down()
                    && i.keys_down.is_empty()
            });
            match self.rested {
                Some(since) if now - since >= BACKDROP_REST && !idle => {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(BACKDROP_IDLE));
                }
                Some(since) if now - since >= BACKDROP_REST => {
                    let opts = RenderOptions {
                        width: w,
                        height: h,
                        view: ViewTransform {
                            zoom: scale,
                            origin_x: -area.x0 * scale,
                            origin_y: area.y1 * scale,
                        },
                        preview: None,
                        wireframe,
                        simulate_overprints,
                        complex_effects,
                        anti_alias: true,
                        draft: true,
                        focus: None,
                    };
                    let page_rect = tracedraw_core::geometry::Rect::new(
                        area.x0,
                        area.y1 - h as f64 / scale,
                        area.x0 + w as f64 / scale,
                        area.y1,
                    );
                    let lut = proof.then(|| {
                        self.proof
                            .get_or_insert_with(|| std::sync::Arc::new(ProofLut::new()))
                            .clone()
                    });
                    // The copy a zoom's renders share: made here, at rest,
                    // it is there for the first step of the next zoom.
                    let (doc, stale) = self.copy_of(doc, revision);
                    let (tx, rx) = std::sync::mpsc::channel();
                    let wake = ctx.clone();
                    std::thread::spawn(move || {
                        let image = render_page_on_one_thread(&doc, page, &opts)
                            .map(|pm| to_image(&pm, lut.as_deref()));
                        let _ = tx.send(image);
                        drop(doc);
                        drop(stale);
                        wake.request_repaint();
                    });
                    self.backdrops += 1;
                    self.backdrop_job = Some(BackJob { key, page_rect, rx });
                    return None;
                }
                Some(since) => {
                    let wait = (BACKDROP_REST - (now - since)).max(0.0);
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
                }
                None => {}
            }
        }
        let b = self.backdrop.as_ref().filter(|b| b.key == key)?;
        Some((b.texture.id(), view.rect_to_screen(b.page_rect)))
    }

    /// True once the backdrop of the drawing at `revision` is there.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn has_backdrop(&self, revision: u64) -> bool {
        self.backdrop
            .as_ref()
            .is_some_and(|b| b.key.revision == revision)
    }

    /// True while a backdrop is being drawn on its thread, for tests.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn backdrop_drawing(&self) -> bool {
        self.backdrop_job.is_some()
    }

    /// The backdrop's texture, for tests.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub fn backdrop_texture(&self) -> Option<TextureId> {
        self.backdrop.as_ref().map(|b| b.texture.id())
    }

    /// The drawing at `revision` for a render on a thread: the copy kept
    /// from the last render on a thread when it is of the same revision,
    /// or a new copy, kept for the next renders of the same drawing (the
    /// steps of a zoom, every picture of a drag). Also the copy no longer
    /// kept, for the render's thread to free (freeing a large drawing
    /// takes time the window's thread does not have). A drag on a large
    /// drawing copied all of it again for every picture of the drag, on
    /// the window's thread, so the object stalled before it followed the
    /// pointer (#2522).
    #[cfg(not(target_arch = "wasm32"))]
    fn copy_of(
        &mut self,
        doc: &Document,
        revision: u64,
    ) -> (std::sync::Arc<Document>, Option<std::sync::Arc<Document>>) {
        match &self.kept {
            Some((r, d)) if *r == revision => (d.clone(), None),
            _ => {
                let d = std::sync::Arc::new(doc.clone());
                let keep = Some((revision, d.clone()));
                let stale = std::mem::replace(&mut self.kept, keep);
                (d, stale.map(|(_, d)| d))
            }
        }
    }

    /// The copy of the drawing kept for renders on a thread, with its
    /// revision, for other pictures of the same drawing to share.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn kept_copy(&self) -> Option<(u64, std::sync::Arc<Document>)> {
        self.kept.clone()
    }

    /// True when the view asked for (`new`) differs from the picture's
    /// (`old`) by its zoom and the picture is the main canvas's: rendered
    /// at once on a thread instead of shown scaled until the view rests.
    /// The pages beside the current one in the Multipage View keep waiting
    /// (many come due at once, #903), and the browser has no threads.
    fn zooms_at_once(&self, old: &Key, new: &Key) -> bool {
        cfg!(not(target_arch = "wasm32")) && !self.follows_zoom && old.zoom != new.zoom
    }

    /// The bands of the picture being drawn on its thread that are done,
    /// on a see-through texture, and where it shows on screen: drawn
    /// over the last picture (shown scaled or moved into place
    /// meanwhile), so the window sharpens a band at a time instead of
    /// all at once at the end (#2389). `None` while no band has come, or
    /// for a render that gave up.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn incoming(&self, view: &View) -> Option<(TextureId, Rect)> {
        let job = self.job.as_ref()?;
        if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        Some((
            job.incoming.as_ref()?.id(),
            view.rect_to_screen(job.page_rect?),
        ))
    }

    /// The last picture, where its part of the page now is on screen
    /// (while a new one is drawn on its thread).
    #[cfg(not(target_arch = "wasm32"))]
    fn shown_meanwhile(&self, view: &View, screen: Rect) -> Option<(TextureId, Rect)> {
        let t = self.texture.as_ref()?;
        Some((
            t.id(),
            self.page_rect.map_or(screen, |r| view.rect_to_screen(r)),
        ))
    }

    /// A render on its thread: its picture taken when ready (`true` when
    /// it is still running).
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_job(&mut self, ctx: &Context) -> bool {
        let Some(job) = &mut self.job else {
            return false;
        };
        job.take_bands(ctx);
        match job.rx.try_recv() {
            Ok(result) => {
                let job = self.job.take().expect("job");
                if let Some((image, seconds)) = result {
                    self.last_seconds = seconds;
                    match &mut self.texture {
                        Some(t) => t.set(image, TextureOptions::LINEAR),
                        None => {
                            self.texture =
                                Some(ctx.load_texture("page", image, TextureOptions::LINEAR))
                        }
                    }
                    self.key = Some(job.key);
                    self.page_rect = job.page_rect;
                }
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.job = None;
                false
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Job {
    /// The bands that came since the last frame, onto the texture of the
    /// bands so far.
    fn take_bands(&mut self, ctx: &Context) {
        let Some(rx) = &self.bands else {
            return;
        };
        let (w, h) = (self.size.0 as usize, self.size.1 as usize);
        // A texture over the graphics card's largest side cannot be
        // made: such a picture comes all at once at the end, as before.
        let side = ctx.input(|i| i.max_texture_side);
        if w > side || h > side {
            self.bands = None;
            return;
        }
        while let Ok((y0, rows)) = rx.try_recv() {
            let y0 = y0 as usize;
            if rows.size[0] != w || y0 + rows.size[1] > h {
                continue;
            }
            let t = self.incoming.get_or_insert_with(|| {
                ctx.load_texture(
                    "page-bands",
                    ColorImage::filled([w, h], egui::Color32::TRANSPARENT),
                    TextureOptions::LINEAR,
                )
            });
            t.set_partial([0, y0], rows, TextureOptions::LINEAR);
        }
    }
}

impl Key {
    /// True when `other` shows the same drawing the same way, only from
    /// another place or at another zoom (the canvas the same size, unless
    /// `any_size`).
    fn same_but_view(&self, other: &Key, any_size: bool) -> bool {
        self.revision == other.revision
            && self.page == other.page
            && (any_size || self.size == other.size)
            && self.preview == other.preview
            && self.wireframe == other.wireframe
            && self.simulate_overprints == other.simulate_overprints
            && self.complex_effects == other.complex_effects
            && self.proof == other.proof
            && self.focus == other.focus
            && self.pixels.is_none()
            && other.pixels.is_none()
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    /// One page job at a time, and the current page first: another page
    /// that asks first every frame waits while the turn is kept for the
    /// current page.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn one_render_job_runs_at_a_time_and_the_current_page_goes_first() {
        static TURNS: Turns = Turns::new();
        let job = TURNS.take(false).expect("free");
        assert!(TURNS.take(false).is_none());
        // The current page asks while the job runs, and so waits.
        assert!(TURNS.take(true).is_none());
        std::thread::spawn(move || drop(job)).join().unwrap();
        // Freed: another page asking first does not get it.
        assert!(TURNS.take(false).is_none());
        let current = TURNS.take(true).expect("kept for the current page");
        drop(current);
        // Nobody waits: whoever asks.
        assert!(TURNS.take(false).is_some());
    }

    /// Tests running at the same time do not share the program's turns:
    /// a job of another test holding its turn leaves this test's pages
    /// free to start theirs, while the pages of one test (one window)
    /// still take turns.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_test_has_render_turns_of_its_own() {
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let other = std::thread::spawn(move || {
            let turn = program_turns().take(true).expect("free");
            held_tx.send(()).expect("sent");
            let _ = done_rx.recv();
            drop(turn);
        });
        held_rx.recv().expect("held");
        let mine = program_turns()
            .take(true)
            .expect("not held by the other test");
        assert!(
            program_turns().take(false).is_none(),
            "one job at a time in this test"
        );
        drop(mine);
        done_tx.send(()).expect("sent");
        other.join().expect("joined");
    }

    /// Two slow pages drawn every frame, the other page asked first and
    /// the drawing changing each frame: the current page's render starts
    /// within one release of the turn.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_current_page_gets_the_next_turn() {
        static TURNS: Turns = Turns::new();
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        let view = View::default();
        let mut current = Raster {
            turns: Some(&TURNS),
            ..Default::default()
        };
        let mut other = Raster {
            turns: Some(&TURNS),
            follows_zoom: true,
            ..Default::default()
        };
        let draw = |r: &mut Raster, revision: u64| {
            let page = doc.pages[0].id;
            let input = egui::RawInput {
                time: Some(revision as f64),
                ..Default::default()
            };
            let mut full = ctx.run_ui(input, |ui| {
                r.texture(
                    ui.ctx(),
                    &doc,
                    page,
                    &view,
                    rect,
                    None,
                    revision,
                    false,
                    false,
                    false,
                    false,
                    None,
                    None,
                );
            });
            full.textures_delta.clear();
        };
        draw(&mut other, 0);
        draw(&mut current, 0);
        other.always_slow = true;
        current.always_slow = true;
        // The other page takes the turn first.
        draw(&mut other, 1);
        assert!(other.job.is_some());
        // From the first time the current page waits, no other page may
        // take a turn before it.
        let mut given = None;
        for revision in 2..2000 {
            draw(&mut other, revision);
            draw(&mut current, revision);
            if current.job.is_some() {
                break;
            }
            let st = TURNS.state();
            if st.current_waits && given.is_none() {
                given = Some(st.to_others);
            }
            if let Some(n) = given {
                assert_eq!(st.to_others, n, "another page went first");
            }
            drop(st);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(given.is_some(), "the current page had to wait");
        assert!(current.job.is_some(), "the current page got a turn");
    }
    use tracedraw_core::geometry::Size;

    /// A page beside the current one that is refused its turn only because
    /// the turn is kept for the current page, with no job running to wake
    /// the window when it ends: a repaint is asked for when the turn is
    /// free again, or the page keeps its old, blurry picture until the next
    /// pointer move or key press (#1384).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_page_refused_a_kept_turn_asks_for_a_repaint() {
        static TURNS: Turns = Turns::new();
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let page = doc.pages[0].id;
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        let view = View::default();
        let mut other = Raster {
            turns: Some(&TURNS),
            follows_zoom: true,
            ..Default::default()
        };
        // One frame at egui time `time`; how long egui may wait before the
        // next one.
        let draw = |r: &mut Raster, revision: u64, time: f64| {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut full = ctx.run_ui(input, |ui| {
                r.texture(
                    ui.ctx(),
                    &doc,
                    page,
                    &view,
                    rect,
                    None,
                    revision,
                    false,
                    false,
                    false,
                    false,
                    None,
                    None,
                );
            });
            full.textures_delta.clear();
            full.viewport_output
                .get(&egui::ViewportId::ROOT)
                .map_or(std::time::Duration::MAX, |v| v.repaint_delay)
        };
        draw(&mut other, 0, 0.0);
        assert!(other.has_picture());
        // The frame after a texture is loaded repaints once on its own;
        // measure from the next one.
        draw(&mut other, 0, 0.5);
        // A job holds the turn and the current page asks for it meanwhile;
        // when the job ends the turn is kept for the current page.
        let job = TURNS.take(false).expect("free");
        assert!(TURNS.take(true).is_none());
        drop(job);
        // The drawing changed: the page wants a new render and is refused.
        let delay = draw(&mut other, 1, 1.0);
        assert!(other.job.is_none(), "the turn is kept for the current page");
        assert_eq!(other.render_count(), 1);
        assert!(
            delay <= KEEP_FOR_CURRENT,
            "no repaint asked for: waits {delay:?}"
        );
    }

    #[test]
    fn proofed_pixels_follow_the_grid_of_proofed_colours() {
        let lut = ProofLut::new();
        // On the grid, the proofed colour itself.
        let blue = lut.apply([0.0, 0.0, 1.0]);
        let want = tracedraw_core::color::proof_rgb([0.0, 0.0, 1.0]);
        for (a, b) in blue.iter().zip(want) {
            assert!((a - b).abs() < 1e-5);
        }
        // Between, close to the colour proofed directly.
        let c = [0.31, 0.62, 0.18];
        let (got, want) = (lut.apply(c), tracedraw_core::color::proof_rgb(c));
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 0.03, "{got:?} {want:?}");
        }
        // Transparent stays transparent; half-covered pixels keep their
        // coverage.
        assert_eq!(lut.pixel(0, 0, 0, 0), egui::Color32::TRANSPARENT);
        let half = lut.pixel(0, 0, 128, 128);
        assert_eq!(half.a(), 128);
        assert!(half.b() <= 128);
    }

    /// One frame at egui time `time`: the page texture for `view` on the
    /// canvas `rect`.
    fn pass(
        ctx: &Context,
        raster: &mut Raster,
        doc: &Document,
        view: &View,
        rect: Rect,
        time: f64,
    ) -> (TextureId, Rect) {
        let page = doc.pages[0].id;
        let mut out = None;
        let input = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        let mut full = ctx.run_ui(input, |ui| {
            out = raster.texture(
                ui.ctx(),
                doc,
                page,
                view,
                rect,
                None,
                1,
                false,
                false,
                false,
                false,
                None,
                None,
            );
        });
        full.textures_delta.clear();
        out.expect("texture")
    }

    /// A pan shows the last render moved into place at once, and renders
    /// the page anew once the view has rested; a zoom renders at once (on
    /// its thread), the old picture scaled into place meanwhile.
    #[test]
    fn a_moving_view_shows_the_last_render_until_it_rests() {
        static TURNS: Turns = Turns::new();
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        let mut raster = Raster {
            turns: Some(&TURNS),
            ..Default::default()
        };
        let view = View::default();
        let (first, screen) = pass(&ctx, &mut raster, &doc, &view, rect, 0.0);
        assert_eq!(screen, rect);
        let key = raster.key.clone();
        // Panned 10 points right: the same texture, 10 points to the right,
        // nothing rendered.
        let mut moved = view;
        moved.pan(egui::vec2(10.0, 0.0));
        let (tex, screen) = pass(&ctx, &mut raster, &doc, &moved, rect, 0.01);
        assert_eq!(tex, first);
        assert_eq!(raster.key, key, "no new render yet");
        assert!((screen.min.x - 10.0).abs() < 1e-3, "{screen:?}");
        assert!(raster.settle.is_some());
        // The view rests: rendered anew for where it is now.
        let (_, screen) = pass(
            &ctx,
            &mut raster,
            &doc,
            &moved,
            rect,
            0.01 + SETTLE_SECONDS + 0.01,
        );
        assert_ne!(raster.key, key);
        assert_eq!(screen, rect);
        assert!(raster.settle.is_none());
        // Moving on at once re-arms the wait from the latest move.
        let mut again = moved;
        again.pan(egui::vec2(0.0, 5.0));
        let before = raster.key.clone();
        pass(&ctx, &mut raster, &doc, &again, rect, 1.0);
        pass(
            &ctx,
            &mut raster,
            &doc,
            &again,
            rect,
            1.0 + SETTLE_SECONDS / 2.0,
        );
        assert_eq!(raster.key, before);
        pass(
            &ctx,
            &mut raster,
            &doc,
            &again,
            rect,
            1.0 + SETTLE_SECONDS + 0.01,
        );
        assert_ne!(raster.key, before);
        // Zoomed: no wait for the view to rest. The render starts in the
        // same frame, on its thread, the old texture scaled about the page
        // meanwhile.
        let shown = raster.texture.as_ref().map(|t| t.id());
        let renders = raster.render_count();
        let mut zoomed = again;
        zoomed.zoom *= 2.0;
        let (tex, screen) = pass(&ctx, &mut raster, &doc, &zoomed, rect, 2.0);
        assert_eq!(Some(tex), shown);
        assert!((screen.width() - 640.0).abs() < 1e-3, "{screen:?}");
        assert_eq!(raster.render_count(), renders + 1, "rendered at once");
        assert!(raster.settle.is_none(), "no wait for the view to rest");
        assert!(raster.job.is_some(), "on its thread");
        let mut done = false;
        for i in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            let (_, screen) = pass(
                &ctx,
                &mut raster,
                &doc,
                &zoomed,
                rect,
                2.0 + 0.001 * i as f64,
            );
            if raster.picture_zoom() == Some(zoomed.zoom) {
                assert_eq!(screen, rect);
                done = true;
                break;
            }
        }
        assert!(done, "the zoomed picture came well before the view rested");
        assert_eq!(raster.render_count(), renders + 1);
    }

    /// A slow page is drawn on a thread of its own: the frame that asks
    /// for it returns at once with the last picture moved into place, and
    /// the new picture is taken once it is ready (#345).
    #[test]
    fn a_slow_page_is_drawn_on_its_own_thread() {
        // Turns of its own: the program's are shared with the other tests
        // running at the same time, whose jobs could keep it waiting.
        static TURNS: Turns = Turns::new();
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        let mut raster = Raster {
            turns: Some(&TURNS),
            ..Default::default()
        };
        let view = View::default();
        let (first, _) = pass(&ctx, &mut raster, &doc, &view, rect, 0.0);
        let key = raster.key.clone();
        // Taken for slow: the next render runs on a thread. Panned, so the
        // render waits for the view to rest first (a zoom would not).
        raster.last_seconds = 1.0;
        let mut zoomed = view;
        zoomed.pan(egui::vec2(40.0, 0.0));
        let t = SETTLE_SECONDS + 0.01;
        pass(&ctx, &mut raster, &doc, &zoomed, rect, 0.0);
        let (mut tex, mut screen) = pass(&ctx, &mut raster, &doc, &zoomed, rect, t);
        for _ in 0..500 {
            if raster.job.is_some() {
                break;
            }
            assert_eq!(tex, first, "the old picture shows while waiting");
            std::thread::sleep(std::time::Duration::from_millis(10));
            (tex, screen) = pass(&ctx, &mut raster, &doc, &zoomed, rect, t);
        }
        assert_eq!(tex, first);
        assert!(raster.job.is_some(), "drawn on its thread");
        assert_eq!(raster.key, key, "the old picture shows meanwhile");
        assert!((screen.min.x - 40.0).abs() < 1e-3, "{screen:?}");
        // Ready a moment later: taken, for the view asked for.
        let mut done = false;
        for i in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let (_, screen) = pass(&ctx, &mut raster, &doc, &zoomed, rect, t + 0.01 * i as f64);
            if raster.job.is_none() && raster.key != key {
                assert_eq!(screen, rect);
                done = true;
                break;
            }
        }
        assert!(done, "the picture from the thread was taken");
        assert!(raster.last_seconds < 1.0, "its time measured");
    }

    /// The whole page is drawn small once the picture has rested, on a
    /// thread and without counting as a render of the picture; it shows
    /// where the page is on screen at any zoom, only for the drawing as
    /// it is now, and is not drawn while the view keeps moving (#2048).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_whole_page_is_drawn_small_once_the_picture_rests() {
        static TURNS: Turns = Turns::new();
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let page = doc.pages[0].id;
        let area = tracedraw_core::geometry::Rect::new(0.0, 0.0, 100.0, 50.0);
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        let mut raster = Raster {
            turns: Some(&TURNS),
            ..Default::default()
        };
        // Where the pointer moves to in the next frame, if it does.
        let pointer = std::cell::Cell::new(None::<egui::Pos2>);
        // One frame: the picture, then the backdrop.
        let frame = |raster: &mut Raster, view: &View, revision: u64, time: f64| {
            let mut out = (None, None);
            let input = egui::RawInput {
                time: Some(time),
                events: pointer
                    .take()
                    .map(egui::Event::PointerMoved)
                    .into_iter()
                    .collect(),
                ..Default::default()
            };
            let mut full = ctx.run_ui(input, |ui| {
                let main = raster.texture(
                    ui.ctx(),
                    &doc,
                    page,
                    view,
                    rect,
                    None,
                    revision,
                    false,
                    false,
                    false,
                    false,
                    None,
                    None,
                );
                let back = raster.backdrop(
                    ui.ctx(),
                    &doc,
                    page,
                    area,
                    view,
                    rect,
                    revision,
                    false,
                    false,
                    false,
                    false,
                );
                out = (main, back);
            });
            full.textures_delta.clear();
            out
        };
        // Frames at rest until the backdrop of `revision` is there.
        let wait = |raster: &mut Raster, view: &View, revision: u64, from: f64| {
            for i in 0..400 {
                let (_, back) = frame(raster, view, revision, from + 0.001 * i as f64);
                if let Some(b) = back {
                    return b;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("the backdrop never came");
        };
        // Zoomed in on the middle of the page.
        let mut view = View::default();
        view.zoom = 16.0;
        // Moving every frame, for longer than the rest: no backdrop.
        let mut t = 0.0;
        for _ in 0..20 {
            view.pan(egui::vec2(1.0, 0.0));
            let (main, back) = frame(&mut raster, &view, 1, t);
            assert!(main.is_some());
            assert!(back.is_none());
            t += BACKDROP_REST / 10.0;
        }
        assert_eq!(raster.backdrops, 0, "none while the view moves");
        // At rest: not before the rest is over, then once.
        frame(&mut raster, &view, 1, 10.0);
        frame(&mut raster, &view, 1, 10.0 + SETTLE_SECONDS + 0.01);
        let rested = 10.0 + SETTLE_SECONDS + 0.02;
        frame(&mut raster, &view, 1, rested);
        frame(&mut raster, &view, 1, rested + BACKDROP_REST / 2.0);
        assert_eq!(raster.backdrops, 0, "not before the picture has rested");
        // Rested, but the pointer is moving over the page: not in such a
        // frame (the drawing is copied for the thread when it starts).
        for i in 0..10 {
            pointer.set(Some(egui::pos2(20.0 + 9.0 * i as f32, 30.0)));
            let t = rested + BACKDROP_REST + 0.01 + 0.016 * i as f64;
            frame(&mut raster, &view, 1, t);
        }
        assert_eq!(raster.backdrops, 0, "not while the pointer moves");
        let rested = rested + 1.0;
        let renders = raster.render_count();
        let (_, back) = frame(&mut raster, &view, 1, rested + BACKDROP_REST + 0.01);
        assert!(back.is_none(), "on its thread");
        assert_eq!(raster.backdrops, 1);
        let (tex, at) = wait(&mut raster, &view, 1, rested + BACKDROP_REST + 0.02);
        assert_eq!(raster.backdrops, 1);
        assert_eq!(raster.render_count(), renders, "no render of the picture");
        // The page, where it is on screen.
        let page_at = view.rect_to_screen(area);
        assert!((at.min - page_at.min).length() < 1e-2, "{at:?} {page_at:?}");
        assert!((at.max - page_at.max).length() < 1e-2, "{at:?} {page_at:?}");
        // Zoomed far out: the picture, scaled into place, covers a small
        // part of the page; the backdrop covers the page.
        let mut out = view;
        out.zoom_at(rect.center(), 1.0 / 16.0);
        let (main, back) = frame(&mut raster, &out, 1, 20.0);
        let (_, main_at) = main.expect("the last picture");
        let (back_tex, back_at) = back.expect("the backdrop");
        assert_eq!(back_tex, tex);
        let page_at = out.rect_to_screen(area);
        assert!(main_at.width() < page_at.width() / 4.0, "{main_at:?}");
        assert!((back_at.min - page_at.min).length() < 1e-2, "{back_at:?}");
        assert!((back_at.max - page_at.max).length() < 1e-2, "{back_at:?}");
        // The drawing changed: the old backdrop is not shown, and the new
        // one waits for the picture to rest again.
        let (_, back) = frame(&mut raster, &out, 2, 30.0);
        assert!(back.is_none(), "never the backdrop of an earlier state");
        assert_eq!(raster.backdrops, 1);
        for i in 0..400 {
            frame(&mut raster, &out, 2, 30.0 + 0.001 * i as f64);
            if !raster.drawing() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frame(&mut raster, &out, 2, 31.0);
        frame(&mut raster, &out, 2, 31.0 + BACKDROP_REST + 0.01);
        wait(&mut raster, &out, 2, 31.0 + BACKDROP_REST + 0.02);
        assert_eq!(raster.backdrops, 2);
        assert!(raster.has_backdrop(2));
        // Dropped with the picture.
        raster.invalidate();
        assert!(!raster.has_backdrop(2));
    }

    /// A burst of zoom steps while a render runs: no render is started
    /// for each step. The running one, for a zoom the view has left, gives
    /// up (its picture would only be shown scaled again, and the latest
    /// zoom's render waited for all of it, #2522), and the next render
    /// starts as soon as it has for the latest zoom only, covering only
    /// the canvas, the part of the page on screen.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_latest_zoom_wins_without_restarting_renders() {
        static TURNS: Turns = Turns::new();
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        let hold = std::sync::Arc::new(std::sync::Mutex::new(()));
        let mut raster = Raster {
            turns: Some(&TURNS),
            hold: Some(hold.clone()),
            ..Default::default()
        };
        let mut view = View::default();
        pass(&ctx, &mut raster, &doc, &view, rect, 0.0);
        let renders = raster.render_count();
        // The first step starts a render, held on its thread.
        let held = hold.lock().expect("lock");
        let at = egui::pos2(160.0, 100.0);
        view.zoom_at(at, 1.15);
        let first_step = view.zoom;
        pass(&ctx, &mut raster, &doc, &view, rect, 0.01);
        assert_eq!(raster.render_count(), renders + 1);
        let copy = raster.kept_copy().expect("the drawing copied").1;
        // Nineteen more steps while it runs: nothing restarted, nothing
        // started beside it.
        for i in 0..19 {
            view.zoom_at(at, 1.15f32.powf(0.25));
            pass(
                &ctx,
                &mut raster,
                &doc,
                &view,
                rect,
                0.02 + 0.016 * i as f64,
            );
            assert_eq!(raster.render_count(), renders + 1, "restarted");
            assert_eq!(raster.job.as_ref().map(|j| j.key.zoom), Some(first_step));
            assert!(
                raster
                    .job
                    .as_ref()
                    .is_some_and(|j| j.cancel.load(std::sync::atomic::Ordering::Relaxed)),
                "the first step's render told to give up"
            );
        }
        let before = raster.picture_zoom();
        drop(held);
        // It gave up, and the latest zoom's render started at once.
        let mut next = None;
        for i in 0..400 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            pass(&ctx, &mut raster, &doc, &view, rect, 1.0 + 0.001 * i as f64);
            if raster.render_count() > renders + 1 {
                next = Some(i);
                break;
            }
        }
        assert!(next.is_some(), "the latest zoom was never rendered");
        assert_eq!(
            raster.picture_zoom(),
            before,
            "no picture of the first step"
        );
        assert_eq!(raster.job.as_ref().map(|j| j.key.zoom), Some(view.zoom));
        assert_eq!(raster.render_count(), renders + 2);
        // The second render shares the first one's copy of the drawing.
        let again = raster.kept_copy().expect("kept").1;
        assert!(std::sync::Arc::ptr_eq(&copy, &again), "copied again");
        for i in 0..400 {
            if raster.picture_zoom() == Some(view.zoom) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            pass(&ctx, &mut raster, &doc, &view, rect, 2.0 + 0.001 * i as f64);
        }
        assert_eq!(raster.picture_zoom(), Some(view.zoom));
        assert_eq!(
            raster.render_count(),
            renders + 2,
            "two renders for twenty steps"
        );
        // The picture is the canvas's size and shows only the part of the
        // page on screen.
        assert_eq!(raster.texture.as_ref().map(|t| t.size()), Some([320, 200]));
        let shown = raster.page_rect.expect("page rect");
        let want = view.visible_page_rect(rect);
        for (a, b) in [
            (shown.x0, want.x0),
            (shown.y0, want.y0),
            (shown.x1, want.x1),
            (shown.y1, want.y1),
        ] {
            assert!((a - b).abs() < 1e-3, "{shown:?} {want:?}");
        }
    }

    /// A page beside the current one in the Multipage View is rendered
    /// on a canvas the size of its paper, which changes with the zoom:
    /// while the view moves, the last picture is scaled into place, the
    /// page's corner and size following the zoom, and nothing is rendered
    /// until the view rests (#902, #903).
    #[test]
    fn a_picture_that_follows_the_zoom_keeps_its_place_while_resized() {
        // Turns of its own, as above: a turn kept for another test's
        // current page would defer this page's render.
        static TURNS: Turns = Turns::new();
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let mut raster = Raster {
            follows_zoom: true,
            turns: Some(&TURNS),
            ..Default::default()
        };
        let view = View {
            zoom: 2.0,
            origin: egui::pos2(10.0, 110.0),
        };
        let paper =
            |v: &View| v.rect_to_screen(tracedraw_core::geometry::Rect::new(0.0, 0.0, 100.0, 50.0));
        let (first, at) = pass(&ctx, &mut raster, &doc, &view, paper(&view), 0.0);
        assert_eq!(at, paper(&view));
        let renders = raster.render_count();
        // Zoomed out about a point: the paper is smaller, the canvas with
        // it. The same picture, exactly on the paper.
        let mut zoomed = view;
        zoomed.zoom_at(egui::pos2(300.0, 300.0), 0.8);
        let (tex, at) = pass(&ctx, &mut raster, &doc, &zoomed, paper(&zoomed), 0.02);
        assert_eq!(tex, first);
        assert_eq!(raster.render_count(), renders, "nothing rendered");
        let want = paper(&zoomed);
        assert!((at.min - want.min).length() < 1e-3, "{at:?} {want:?}");
        assert!((at.max - want.max).length() < 1e-3, "{at:?} {want:?}");
        // At rest: rendered for the new size.
        pass(
            &ctx,
            &mut raster,
            &doc,
            &zoomed,
            paper(&zoomed),
            0.02 + SETTLE_SECONDS + 0.01,
        );
        assert_eq!(raster.render_count(), renders + 1);
        // The main canvas keeps rendering when its size changes.
        let mut main = Raster {
            turns: Some(&TURNS),
            ..Default::default()
        };
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        pass(&ctx, &mut main, &doc, &view, rect, 0.0);
        let wider = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(330.0, 200.0));
        pass(&ctx, &mut main, &doc, &view, wider, 0.01);
        assert_eq!(main.render_count(), 2);
    }

    /// A deferred render shows the last picture where its part of the
    /// page now is and renders nothing; the next frame without the
    /// deferral renders.
    #[test]
    fn a_deferred_render_waits_for_a_later_frame() {
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let page = doc.pages[0].id;
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 200.0));
        let view = View::default();
        let mut raster = Raster {
            defer: true,
            ..Default::default()
        };
        let at_revision = |raster: &mut Raster, revision: u64| {
            raster.texture(
                &ctx, &doc, page, &view, rect, None, revision, false, false, false, false, None,
                None,
            )
        };
        // Nothing to show yet: nothing shown, nothing rendered.
        assert!(at_revision(&mut raster, 1).is_none());
        assert_eq!(raster.render_count(), 0);
        raster.defer = false;
        let (first, _) = at_revision(&mut raster, 1).expect("rendered");
        // The drawing changed: deferred, the old picture stays.
        raster.defer = true;
        let (tex, at) = at_revision(&mut raster, 2).expect("old picture");
        assert_eq!(tex, first);
        assert_eq!(at, rect);
        assert_eq!(raster.render_count(), 1);
        raster.defer = false;
        at_revision(&mut raster, 2).expect("rendered");
        assert_eq!(raster.render_count(), 2);
    }

    #[test]
    fn a_canvas_that_moves_renders_again() {
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let view = View::default();
        let mut raster = Raster::default();
        let at = |x: f32, y: f32| Rect::from_min_size(egui::pos2(x, y), egui::vec2(320.0, 200.0));
        pass(&ctx, &mut raster, &doc, &view, at(40.0, 160.0), 0.0);
        let first = raster.key.clone();
        // Same canvas, same view: the cached texture.
        pass(&ctx, &mut raster, &doc, &view, at(40.0, 160.0), 0.01);
        assert_eq!(raster.key, first);
        // The canvas moves up by a bar's height and keeps its size: the
        // page sits elsewhere inside it. The last render is shown where
        // the page now is, and the page is rendered anew once the view
        // rests, with the new offset.
        let (_, screen) = pass(&ctx, &mut raster, &doc, &view, at(40.0, 107.0), 0.02);
        assert_eq!(
            screen,
            view.rect_to_screen(raster.page_rect.expect("page rect"))
        );
        pass(
            &ctx,
            &mut raster,
            &doc,
            &view,
            at(40.0, 107.0),
            0.02 + SETTLE_SECONDS + 0.01,
        );
        assert_ne!(raster.key, first);
        assert_eq!(
            raster.key.as_ref().map(|k| k.offset),
            Some(view.origin - egui::pos2(40.0, 107.0))
        );
    }

    #[test]
    fn the_pixels_view_draws_whole_document_pixels_as_blocks() {
        // 72 dpi: a document pixel is 0.3528 mm; at 10 screen pixels per
        // mm it covers 3.5 screen pixels.
        let view = View {
            zoom: 10.0,
            origin: egui::pos2(0.0, 500.0),
        };
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 500.0));
        let win = pixel_window(&view, rect, 72.0, 1.0).expect("window");
        assert_eq!(win.ix0, 0);
        // 50 mm of page height: 141.7 pixels, rounded up to whole ones.
        assert_eq!(win.iy1, 142);
        assert_eq!(win.w, 114);
        assert_eq!(win.h, 142);
        // Whole pixels: the blocks start on the page corner and may run
        // past the canvas a little.
        assert!(win.screen.min.x <= rect.min.x && win.screen.max.x >= rect.max.x);
        assert!(win.screen.min.y <= rect.min.y);
        assert!((win.screen.max.y - 500.0).abs() < 1e-3);
        // Zoomed out below one screen pixel per document pixel: the normal
        // render.
        let far = View { zoom: 1.0, ..view };
        assert_eq!(pixel_window(&far, rect, 72.0, 1.0), None);
        assert_eq!(pixel_window(&view, rect, 0.0, 1.0), None);
        assert_eq!(pixel_window(&view, rect, f64::NAN, 1.0), None);
        // A huge window (very high resolution) falls back too.
        assert_eq!(pixel_window(&view, rect, 1.0e6, 1.0), None);
    }

    #[test]
    fn the_pixels_view_texture_is_one_texel_per_document_pixel() {
        let ctx = Context::default();
        let doc = Document::new("t", Size::new(100.0, 50.0));
        let page = doc.pages[0].id;
        let view = View {
            zoom: 40.0,
            origin: egui::pos2(0.0, 400.0),
        };
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        let mut raster = Raster::default();
        let (_, at) = raster
            .texture(
                &ctx,
                &doc,
                page,
                &view,
                rect,
                None,
                1,
                false,
                false,
                false,
                false,
                None,
                Some(96.0),
            )
            .expect("texture");
        let size = raster.texture.as_ref().map(|t| t.size());
        let win = pixel_window(&view, rect, 96.0, 1.0).expect("window");
        assert_eq!(size, Some([win.w as usize, win.h as usize]));
        assert_eq!(at, win.screen);
        // Without the Pixels view: screen-sized, painted on the canvas.
        let (_, at) = raster
            .texture(
                &ctx, &doc, page, &view, rect, None, 1, false, false, false, false, None, None,
            )
            .expect("texture");
        assert_eq!(at, rect);
        assert_eq!(raster.texture.as_ref().map(|t| t.size()), Some([400, 400]));
    }

    /// The browser has no clock behind std::time: asking it the time
    /// panics, and the page froze on the first drawing it showed. Code
    /// outside tests asks web_time instead (mcp_link is not in the web
    /// build).
    #[test]
    fn the_web_build_never_asks_std_for_the_time() {
        fn scan(dir: &std::path::Path, found: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).expect("src").flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if p.is_dir() {
                    scan(&p, found);
                    continue;
                }
                if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "mcp_link.rs" {
                    continue;
                }
                let text = std::fs::read_to_string(&p).expect("source");
                for (i, line) in text.lines().enumerate() {
                    if line.trim_start().starts_with("#[cfg(test)]") {
                        break;
                    }
                    if line.contains(concat!("std::time::", "Instant::now"))
                        || line.contains(concat!("std::time::", "SystemTime::now"))
                    {
                        found.push(format!("{}:{}", p.display(), i + 1));
                    }
                }
            }
        }
        let mut found = Vec::new();
        scan(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut found,
        );
        assert!(found.is_empty(), "use web_time: {found:?}");
    }
}
