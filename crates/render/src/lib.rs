//! CPU rasterizer. Draws a page region into an RGBA pixmap with tiny-skia:
//! correct non-convex fills, fountain fills, outline widths, caps, joins,
//! dashes, hairlines and anti-aliasing. The GPU renderer, when it exists,
//! must match this one.

#![allow(clippy::field_reassign_with_default)]
pub mod accel;
mod image_cache;

use tiny_skia::{
    Color as SkColor, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint,
    Path as SkPath, PathBuilder, Pixmap, RadialGradient, SpreadMode, Stroke as SkStroke,
    StrokeDash, Transform,
};
use tracedraw_core::geometry::Shape as _;
use tracedraw_core::{
    document::{Shape, ShapeKind},
    geometry::{Affine, BezPath, PathEl, Point, Rect},
    Color, Document, Fill, LineCap as LCap, LineJoin as LJoin, PageId, ShapeId, Stroke,
};

/// Page (mm, Y up) to pixel mapping for one render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    /// Pixels per millimetre.
    pub zoom: f64,
    /// Pixel position of the page origin (bottom-left corner) inside the target.
    pub origin_x: f64,
    pub origin_y: f64,
}

impl ViewTransform {
    pub fn affine(&self) -> Affine {
        Affine::new([
            self.zoom,
            0.0,
            0.0,
            -self.zoom,
            self.origin_x,
            self.origin_y,
        ])
    }
}

/// Objects to draw displaced by a transform (live drag preview).
#[derive(Debug, Clone, Default)]
pub struct Preview {
    pub shapes: Vec<ShapeId>,
    pub transform: Affine,
}

/// Runs bitmap effects over straight (not premultiplied) RGBA pixels of
/// `width` x `height`, in place. The renderer has no bitmap effects of its
/// own: the application that has them registers this for the Bitmap
/// Effects lens; without it the lens shows what is beneath unchanged.
pub type PixelEffects = fn(&[tracedraw_core::BitmapEffect], u32, u32, &mut Vec<u8>);

static PIXEL_EFFECTS: std::sync::OnceLock<PixelEffects> = std::sync::OnceLock::new();

/// Register the bitmap effects (once; later calls are ignored).
pub fn set_pixel_effects(f: PixelEffects) {
    let _ = PIXEL_EFFECTS.set(f);
}

#[derive(Clone)]
pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
    pub view: ViewTransform,
    pub preview: Option<Preview>,
    /// Draw outlines only (the Wireframe view).
    pub wireframe: bool,
    /// Show overprints (View > Simulate Overprints): a fill marked
    /// `overprint_fill` and an outline marked `overprint_outline` are
    /// multiplied over what lies beneath instead of knocking it out.
    pub simulate_overprints: bool,
    /// Rasterise lenses and non-uniform transparency ("Rasterize complex
    /// effects" in Enhanced view). When false
    /// those objects are drawn plainly, which is much faster.
    pub complex_effects: bool,
    /// Smooth edges (Convert to Bitmap's Anti-aliasing); off gives hard
    /// pixel edges.
    pub anti_alias: bool,
    /// On screen: large procedural fills (textures, conical and square
    /// gradients) may be worked out on a coarser grid, and drop shadows
    /// may be blurred on the graphics card ([`accel`], same pixels).
    /// Exports keep every pixel and draw everything on the CPU.
    pub draft: bool,
    /// Focus mode: these objects drawn as they are and everything else
    /// faded to the given visibility (0 to 1).
    pub focus: Option<(Vec<tracedraw_core::ShapeId>, f32)>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            width: 1,
            height: 1,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 0.0,
            },
            preview: None,
            wireframe: false,
            simulate_overprints: false,
            complex_effects: true,
            anti_alias: true,
            draft: false,
            focus: None,
        }
    }
}

/// Per-renderer behaviour flags, copied into every nested renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Flags {
    wireframe: bool,
    simulate_overprints: bool,
    complex_effects: bool,
    /// Fill open subpaths (the document's "Fill open curves").
    fill_open_curves: bool,
    anti_alias: bool,
    draft: bool,
    /// Drawing the warped outline of a text: filled with the nonzero rule
    /// as the text is, so letters that overlap leave no hole (#647).
    text_outline: bool,
}

impl Flags {
    fn from_options(opts: &RenderOptions) -> Self {
        Flags {
            wireframe: opts.wireframe,
            simulate_overprints: opts.simulate_overprints,
            complex_effects: opts.complex_effects,
            fill_open_curves: false,
            anti_alias: opts.anti_alias,
            draft: opts.draft,
            text_outline: false,
        }
    }
}

impl Default for Flags {
    fn default() -> Self {
        Flags {
            wireframe: false,
            simulate_overprints: false,
            complex_effects: true,
            fill_open_curves: false,
            anti_alias: true,
            draft: false,
            text_outline: false,
        }
    }
}

/// How `kind` is filled: text, and the warped outline of a text
/// (`text_outline`), with the nonzero rule, so overlapping strokes and
/// letters stay filled (#647, #800); other curves with even-odd.
fn fill_rule(kind: &ShapeKind, text_outline: bool) -> FillRule {
    match kind {
        ShapeKind::Text { .. } => FillRule::Winding,
        _ if text_outline => FillRule::Winding,
        _ => FillRule::EvenOdd,
    }
}

/// How `shape` is filled: as [`fill_rule`] says for its kind, or with the
/// nonzero rule when the object asks for it (#1919, #1211).
fn shape_fill_rule(shape: &Shape, text_outline: bool) -> FillRule {
    match shape.fill_rule {
        tracedraw_core::shaping::FillRule::NonZero => FillRule::Winding,
        tracedraw_core::shaping::FillRule::EvenOdd => fill_rule(&shape.kind, text_outline),
    }
}

/// The subpaths of `path` that are closed: ended with a close command, or
/// ending where they start. Open curves are not filled unless the
/// document asks for it.
pub fn closed_subpaths(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut cur: Vec<PathEl> = Vec::new();
    let flush = |cur: &mut Vec<PathEl>, out: &mut BezPath| {
        if cur.is_empty() {
            return;
        }
        let start = match cur.first() {
            Some(PathEl::MoveTo(p)) => Some(*p),
            _ => None,
        };
        let end = match cur.last() {
            Some(PathEl::LineTo(p))
            | Some(PathEl::QuadTo(_, p))
            | Some(PathEl::CurveTo(_, _, p)) => Some(*p),
            _ => None,
        };
        let closed = cur.iter().any(|e| matches!(e, PathEl::ClosePath))
            || matches!((start, end), (Some(a), Some(b)) if (a - b).hypot() < 1e-6);
        if closed {
            for e in cur.drain(..) {
                out.push(e);
            }
        }
        cur.clear();
    };
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            flush(&mut cur, &mut out);
        }
        cur.push(*el);
    }
    flush(&mut cur, &mut out);
    out
}

/// The most pixels a page is drawn as at once: 1 G pixels (4 GB of RGBA),
/// more than any export or print asks for. A page whose size is not a
/// number (left by a division by zero somewhere, or read from a damaged
/// file) asks for an image as tall as a number holds, and asking the
/// allocator for it takes the program down, which cannot be caught; such
/// an image is refused instead (`None`).
pub const MAX_PIXELS: u64 = 1 << 30;

/// Render the objects of one page onto a transparent pixmap, `None` for
/// an image of more than [`MAX_PIXELS`].
pub fn render_page(doc: &Document, page: PageId, opts: &RenderOptions) -> Option<Pixmap> {
    render_page_until(doc, page, opts, None)
}

/// [`render_page`] that gives up once `cancel` is set (checked before
/// each object): `None` then, as soon as the object being drawn is
/// done. The canvas sets it when the view has zoomed on before the
/// picture was ready, so the picture for the new zoom starts at once
/// instead of after a whole picture nobody will see.
pub fn render_page_cancellable(
    doc: &Document,
    page: PageId,
    opts: &RenderOptions,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<Pixmap> {
    render_page_until(doc, page, opts, Some(cancel))
}

/// [`render_page_cancellable`] that hands over the picture a band of
/// rows at a time as the bands come done, `band(y0, rows)` with the rows
/// `y0..y0 + rows.height()` exactly as they are in the whole picture,
/// the band holding row `first` first and the others outward from it
/// (#2389). A zoom on a large drawing showed the last picture scaled,
/// blurry, until the whole new one was ready: the canvas now lays each
/// band over it as it comes, sharp where it is done, the middle of the
/// window first, as CorelDRAW fills its window in while it draws. The
/// bands are a few times as many as the cores, so the first comes in a
/// fraction of the time; each is drawn with the rows of margin shadow
/// blurs need. Drawn whole (no bands handed over) where bands cannot be
/// used: a small image, one core, a lens, blurs too wide.
#[cfg(not(target_arch = "wasm32"))]
pub fn render_page_progressive(
    doc: &Document,
    page: PageId,
    opts: &RenderOptions,
    cancel: &std::sync::atomic::AtomicBool,
    first: u32,
    band: &(dyn Fn(u32, &Pixmap) + Sync),
) -> Option<Pixmap> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (w, h) = (opts.width.max(1), opts.height.max(1));
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return None;
    }
    let cancelled = || cancel.load(Ordering::Relaxed);
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(16) as u32;
    let (lens, blur_mm) = page_reach(doc, page);
    let bands = if cores < 2 || lens || (w as u64) * (h as u64) < 250_000 {
        None
    } else {
        // Bands of at least 64 rows, about three for each core.
        let count = (cores * PROGRESS_BANDS_PER_CORE).min(h / 64).max(cores);
        band_layout(h, count, blur_margin(blur_mm, opts.view.zoom))
    };
    let Some(mut bands) = bands.filter(|b| b.len() > 1) else {
        return render_page_cancellable(doc, page, opts, cancel);
    };
    let middle = |b: &(u32, u32, u32, u32)| (b.0 + b.1) / 2;
    bands.sort_by_key(|b| middle(b).abs_diff(first));
    let next = AtomicUsize::new(0);
    let done: std::sync::Mutex<Vec<(u32, Pixmap)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..cores.min(bands.len() as u32) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(y0, y1, ry0, ry1)) = bands.get(i) else {
                    return;
                };
                if cancelled() {
                    return;
                }
                let mut o = opts.clone();
                o.height = ry1 - ry0;
                o.view.origin_y = opts.view.origin_y - ry0 as f64;
                let Some(part) = render_page_whole(doc, page, &o, Some(cancel)) else {
                    return;
                };
                if cancelled() {
                    return;
                }
                let Some(mut rows) = Pixmap::new(w, y1 - y0) else {
                    return;
                };
                let row = w as usize * 4;
                let from = (y0 - ry0) as usize * row;
                let len = (y1 - y0) as usize * row;
                let Some(src) = part.data().get(from..from + len) else {
                    return;
                };
                rows.data_mut().copy_from_slice(src);
                band(y0, &rows);
                if let Ok(mut d) = done.lock() {
                    d.push((y0, rows));
                }
            });
        }
    });
    let done = done.into_inner().ok()?;
    if cancelled() || done.len() != bands.len() {
        return None;
    }
    let mut out = Pixmap::new(w, h)?;
    let row = w as usize * 4;
    for (y0, rows) in done {
        let d = y0 as usize * row;
        out.data_mut()[d..d + rows.data().len()].copy_from_slice(rows.data());
    }
    Some(out)
}

/// How many bands for each core a picture handed over a band at a time
/// is drawn in ([`render_page_progressive`]).
#[cfg(not(target_arch = "wasm32"))]
const PROGRESS_BANDS_PER_CORE: u32 = 3;

fn render_page_until(
    doc: &Document,
    page: PageId,
    opts: &RenderOptions,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Option<Pixmap> {
    if (opts.width.max(1) as u64) * (opts.height.max(1) as u64) > MAX_PIXELS {
        return None;
    }
    let cancelled = || cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed));
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(p) = render_page_in_bands(doc, page, opts, cancel) {
        return (!cancelled()).then_some(p);
    }
    let p = render_page_whole(doc, page, opts, cancel)?;
    (!cancelled()).then_some(p)
}

/// [`render_page`] on the calling thread alone, never in bands on all
/// the cores: for a picture nobody is waiting for (the whole page drawn
/// small behind the canvas's picture, #2048), so that the other cores
/// stay free for the frames meanwhile.
pub fn render_page_on_one_thread(
    doc: &Document,
    page: PageId,
    opts: &RenderOptions,
) -> Option<Pixmap> {
    if (opts.width.max(1) as u64) * (opts.height.max(1) as u64) > MAX_PIXELS {
        return None;
    }
    render_page_whole(doc, page, opts, None)
}

/// Rows `y0..y1` of the image `opts` describes, the same as those rows of
/// the whole image drawn at once: drawn with rows of margin above and
/// below as far as shadow blurs reach (every row when the page has a
/// lens, which may show what lies anywhere beneath it), so a large page
/// can be drawn a strip at a time without the whole of it in memory
/// (#1404). Strips should start on rows that are multiples of 16, as the
/// coarse grid of procedural fills does.
pub fn render_page_rows(
    doc: &Document,
    page: PageId,
    opts: &RenderOptions,
    y0: u32,
    y1: u32,
) -> Option<Pixmap> {
    let (w, h) = (opts.width.max(1), opts.height.max(1));
    let (y0, y1) = (y0.min(h), y1.min(h));
    if y1 <= y0 {
        return None;
    }
    let (lens, blur_mm) = page_reach(doc, page);
    let margin = if lens {
        h
    } else {
        blur_margin(blur_mm, opts.view.zoom)
    };
    let (ry0, ry1) = (y0.saturating_sub(margin), y1.saturating_add(margin).min(h));
    let mut o = opts.clone();
    o.width = w;
    o.height = ry1 - ry0;
    o.view.origin_y = opts.view.origin_y - ry0 as f64;
    let part = render_page(doc, page, &o)?;
    let mut out = Pixmap::new(w, y1 - y0)?;
    let row = w as usize * 4;
    let from = (y0 - ry0) as usize * row;
    let len = (y1 - y0) as usize * row;
    out.data_mut()
        .copy_from_slice(part.data().get(from..from + len)?);
    Some(out)
}

/// The reduced level of a picture to draw it from, given `m` from its
/// pixels to the screen: each level halves its sides, and the one
/// chosen is drawn at between half and the whole of its pixels' size
/// (level 0 when the picture is drawn at half its size or more).
fn bitmap_level(m: Affine) -> u32 {
    let c = m.as_coeffs();
    let scale = c[0].hypot(c[1]).max(c[2].hypot(c[3]));
    if !scale.is_finite() || scale <= 0.0 || scale >= 0.5 {
        return 0;
    }
    ((1.0 / scale).log2().floor().max(0.0) as u32).min(image_cache::MAX_LEVEL)
}

/// A picture's pixels-to-screen map `m` (a `w` x `h` picture) made to
/// cover whole screen pixels when it is upright (not turned or
/// slanted): its edges rounded to the nearest pixel boundary, so two
/// pictures that meet on the page meet on the same pixel boundary. The
/// area to fill comes with it, `None` when the picture is narrower than
/// a pixel after rounding. `None` for a turned or slanted picture.
fn snap_upright(m: [f64; 6], w: u32, h: u32) -> Option<([f64; 6], Option<tiny_skia::Rect>)> {
    let upright = m[1].abs() < 1e-9 && m[2].abs() < 1e-9;
    if !upright || !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let (xa, xb) = (m[4], m[4] + m[0] * w);
    let (ya, yb) = (m[5], m[5] + m[3] * h);
    let (x0, x1) = (xa.min(xb).round(), xa.max(xb).round());
    let (y0, y1) = (ya.min(yb).round(), ya.max(yb).round());
    // Each side goes to the rounded edge it came from (a mirrored
    // picture keeps its mirroring).
    let (a, e) = if m[0] >= 0.0 {
        ((x1 - x0) / w, x0)
    } else {
        (-(x1 - x0) / w, x1)
    };
    let (d, f) = if m[3] >= 0.0 {
        ((y1 - y0) / h, y0)
    } else {
        (-(y1 - y0) / h, y1)
    };
    let area = tiny_skia::Rect::from_ltrb(x0 as f32, y0 as f32, x1 as f32, y1 as f32)
        .filter(|_| x1 > x0 && y1 > y0);
    Some(([a, 0.0, 0.0, d, e, f], area))
}

/// What `s` reaches past its own pixels: sets `lens` when it or an
/// object in it has a lens, and grows `blur` to its widest drop shadow
/// blur (mm).
fn reach_walk(s: &Shape, lens: &mut bool, blur: &mut f64) {
    if let Some(sh) = &s.shadow {
        *blur = blur.max(sh.blur.abs());
    }
    if s.effects
        .iter()
        .any(|e| matches!(e, tracedraw_core::live::Effect::Lens(_)))
    {
        *lens = true;
    }
    match &s.kind {
        ShapeKind::Group { children } => {
            for c in children {
                reach_walk(c, lens, blur);
            }
        }
        ShapeKind::ClipFrame { frame, contents } => {
            reach_walk(frame, lens, blur);
            for c in contents {
                reach_walk(c, lens, blur);
            }
        }
        _ => {}
    }
}

/// What on the page reaches past its own pixels: a lens (it samples what
/// lies around it) and the widest drop shadow blur, mm.
fn page_reach(doc: &Document, page: PageId) -> (bool, f64) {
    let (mut lens, mut blur) = (false, 0.0f64);
    if let Ok(layers) = doc.layers_drawn_for_page(page) {
        for l in layers {
            for s in &l.shapes {
                reach_walk(s, &mut lens, &mut blur);
            }
        }
    }
    (lens, blur)
}

/// A large render split into horizontal bands drawn at once on the
/// machine's cores, then put together (#345: one core took half a second
/// for 5000 objects). Each band is drawn with rows of margin above and
/// below, as far as the widest shadow blur reaches, so blurred edges meet; the
/// bands start on rows that are multiples of 16, as the coarse grid of
/// procedural fills does. `None` (draw it whole) for a small image, a
/// single core, a lens on the page, or blurs too wide for the bands.
#[cfg(not(target_arch = "wasm32"))]
fn render_page_in_bands(
    doc: &Document,
    page: PageId,
    opts: &RenderOptions,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Option<Pixmap> {
    let (w, h) = (opts.width.max(1), opts.height.max(1));
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(16) as u32;
    if cores < 2 || (w as u64) * (h as u64) < 250_000 || h < BAND_ALIGN * 4 {
        return None;
    }
    let (lens, blur_mm) = page_reach(doc, page);
    if lens {
        return None;
    }
    let bands = band_layout(h, cores, blur_margin(blur_mm, opts.view.zoom))?;
    let parts: Vec<Option<Pixmap>> = std::thread::scope(|sc| {
        let handles: Vec<_> = bands
            .iter()
            .map(|&(_, _, ry0, ry1)| {
                let mut o = opts.clone();
                o.height = ry1 - ry0;
                o.view.origin_y = opts.view.origin_y - ry0 as f64;
                sc.spawn(move || render_page_whole(doc, page, &o, cancel))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect()
    });
    let mut out = Pixmap::new(w, h)?;
    let row = w as usize * 4;
    for (&(y0, y1, ry0, _), part) in bands.iter().zip(parts) {
        let part = part?;
        let src = part.data();
        let dst = out.data_mut();
        for y in y0..y1 {
            let s = (y - ry0) as usize * row;
            let d = y as usize * row;
            dst[d..d + row].copy_from_slice(&src[s..s + row]);
        }
    }
    Some(out)
}

/// Bands and strips start on rows that are multiples of this, as the
/// coarse grid of procedural fills does.
const BAND_ALIGN: u32 = 16;

/// The rows of margin a strip of a page needs above and below for the
/// widest drop shadow blur `blur_mm` to meet across its edges at `zoom`:
/// a shadow is blurred with three box passes of its radius (or one of
/// 1.7 times it), so it reaches three radii past its silhouette. The
/// arithmetic saturates, so a blur that is not a number (read from a
/// damaged file) gives the whole range, not an overflow.
fn blur_margin(blur_mm: f64, zoom: f64) -> u32 {
    ((blur_mm * zoom).round() as u32)
        .saturating_mul(3)
        .saturating_add(4)
        .div_ceil(BAND_ALIGN)
        .saturating_mul(BAND_ALIGN)
}

/// The bands of an image `h` rows high drawn on `cores` cores, each as
/// `(y0, y1, ry0, ry1)`: its own rows `y0..y1` and the rows `ry0..ry1`
/// it is drawn with, `margin` rows above and below. `None` when the
/// margin is wider than two bands (the bands would draw more than the
/// whole). The arithmetic saturates, so an image as tall as a number
/// holds (a page whose size is not a number, read from a damaged file)
/// gives bands that end at its last row, not an overflow.
#[cfg(not(target_arch = "wasm32"))]
fn band_layout(h: u32, cores: u32, margin: u32) -> Option<Vec<(u32, u32, u32, u32)>> {
    let band = h
        .div_ceil(cores.max(1))
        .div_ceil(BAND_ALIGN)
        .saturating_mul(BAND_ALIGN);
    if margin > band.saturating_mul(2) {
        return None;
    }
    Some(
        (0..cores)
            .map(|i| i.saturating_mul(band))
            .take_while(|y0| *y0 < h)
            .map(|y0| {
                let y1 = y0.saturating_add(band).min(h);
                (
                    y0,
                    y1,
                    y0.saturating_sub(margin),
                    y1.saturating_add(margin).min(h),
                )
            })
            .collect(),
    )
}

fn render_page_whole(
    doc: &Document,
    page: PageId,
    opts: &RenderOptions,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(opts.width.max(1), opts.height.max(1))?;
    let page = doc.page(page).ok()?;
    let screen = opts.view.affine();
    let mut flags = Flags::from_options(opts);
    flags.fill_open_curves = doc.metadata.fill_open_curves;
    let mut r = Renderer {
        pixmap: &mut pixmap,
        screen,
        zoom: opts.view.zoom,
        flags,
        symbols: &doc.symbols,
        preview: opts.preview.as_ref(),
        origin: (0, 0),
        cancel,
    };
    // Focus at 0 % visibility hides everything but the objects in focus:
    // the page and its layers are not drawn at all, as their faded
    // pixels would all be clear (#981).
    let rest_hidden = rest_hidden(opts);
    if let Some(bg) = page.background.as_ref().filter(|_| !rest_hidden) {
        let mut bg_shape = Shape::new(
            tracedraw_core::ShapeId(0),
            ShapeKind::Rect {
                rect: page.rect(),
                radius: 0.0,
                corners: None,
            },
        );
        bg_shape.fill = bg.clone();
        bg_shape.stroke = None;
        r.draw_shape(&bg_shape, Affine::IDENTITY);
    }
    let visible = visible_page_rect(opts);
    let layers = doc.layers_drawn_for_page(page.id).ok()?;
    for layer in layers {
        if !layer.visible || rest_hidden {
            continue;
        }
        for shape in &layer.shapes {
            if r.out_of_sight(shape, &visible) {
                continue;
            }
            let extra = r.child_transform(shape, Affine::IDENTITY);
            r.draw_shape(shape, extra);
        }
    }
    // Focus mode: everything fades, then the objects in focus are drawn
    // again over it as they are.
    if let Some((ids, visibility)) = &opts.focus {
        let v = visibility.clamp(0.0, 1.0);
        let faded = if rest_hidden {
            &mut [][..]
        } else {
            r.pixmap.pixels_mut()
        };
        for px in faded {
            let c = |x: u8| (x as f32 * v).round() as u8;
            if let Some(faded) = tiny_skia::PremultipliedColorU8::from_rgba(
                c(px.red()),
                c(px.green()),
                c(px.blue()),
                c(px.alpha()),
            ) {
                *px = faded;
            }
        }
        for id in ids {
            if let (Some(shape), Some(parent)) = (doc.find_shape(*id), doc.parent_transform(*id)) {
                let extra = r.child_transform(shape, parent);
                r.draw_shape(shape, extra);
            }
        }
    }
    Some(pixmap)
}

/// Whether focus mode hides everything outside the focus: a visibility
/// of 0 (or one that is not a number, read from damaged settings).
fn rest_hidden(opts: &RenderOptions) -> bool {
    opts.focus
        .as_ref()
        .is_some_and(|(_, v)| v.is_nan() || *v <= 0.0)
}

/// The part of the page the target shows, page millimetres.
fn visible_page_rect(opts: &RenderOptions) -> Rect {
    let inv = opts.view.affine().inverse();
    let a = inv * Point::new(0.0, 0.0);
    let b = inv * Point::new(opts.width as f64, opts.height as f64);
    Rect::from_points(a, b)
}

/// How far an outline may reach past the path it follows, in widths:
/// mitres, arrowheads and calligraphic nibs all stay within this (the
/// nodes of a variable width are counted on their own).
const OUTLINE_REACH: f64 = 12.0;

/// Paths drawn on this thread, for tests.
#[cfg(test)]
thread_local! {
    static PATHS_DRAWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Renderer<'a> {
    pixmap: &'a mut Pixmap,
    screen: Affine,
    zoom: f64,
    flags: Flags,
    symbols: &'a [tracedraw_core::Symbol],
    /// Objects drawn displaced (the live drag preview), on a layer or
    /// inside a group or a clip frame.
    preview: Option<&'a Preview>,
    /// Where `pixmap` lies on the target it is laid on, in pixels: a
    /// semi-transparent object's layer covers only what it reaches, and
    /// pattern tiles stay where they fall on the target (#1335).
    origin: (i32, i32),
    /// Set when the picture is no longer wanted (the view zoomed on
    /// before it was ready): every object from then on is skipped.
    cancel: Option<&'a std::sync::atomic::AtomicBool>,
}

impl Renderer<'_> {
    /// A renderer with the same settings drawing into another pixmap.
    fn sub<'b>(&self, pixmap: &'b mut Pixmap) -> Renderer<'b>
    where
        Self: 'b,
    {
        Renderer {
            pixmap,
            screen: self.screen,
            zoom: self.zoom,
            flags: self.flags,
            symbols: self.symbols,
            preview: self.preview,
            origin: self.origin,
            cancel: self.cancel,
        }
    }

    /// The transform `child` is drawn with inside `transform`: the live
    /// drag preview's first when the child is being dragged.
    /// True when nothing of `shape` can show in `visible` (page mm), so
    /// it need not be drawn: its bounds, grown by what its outline may
    /// reach, lie wholly outside. Objects whose extent is not their
    /// bounds (effects, shadows, symbols, the drag preview) are always
    /// drawn.
    fn out_of_sight(&self, shape: &Shape, visible: &Rect) -> bool {
        self.out_of_sight_in(shape, Affine::IDENTITY, visible)
    }

    /// [`Self::out_of_sight`] for `shape` inside a group or clip frame
    /// whose transform to the page is `parent`: the objects of a group
    /// and the frame of a clip frame lie in its space, so a moved clip
    /// frame (the pages placed on a printed sheet) is tested where it
    /// draws (#848).
    fn out_of_sight_in(&self, shape: &Shape, parent: Affine, visible: &Rect) -> bool {
        if !shape.visible {
            return true;
        }
        if !shape.effects.is_empty()
            || shape.shadow.is_some()
            || matches!(shape.kind, ShapeKind::SymbolInstance { .. })
            || self.preview.is_some_and(|p| p.shapes.contains(&shape.id))
        {
            return false;
        }
        // A group or a clip frame: its own outline reaches nowhere, its
        // objects' outlines do. Out of sight when every object is (a clip
        // frame draws nothing past its frame): the group's bounds count
        // no outline, so its strips used to cut a wide or variable
        // outline inside it flat (#626).
        match &shape.kind {
            ShapeKind::Group { children } => {
                let t = parent * shape.transform;
                return children.iter().all(|c| self.out_of_sight_in(c, t, visible));
            }
            ShapeKind::ClipFrame { frame, .. } => {
                return self.out_of_sight_in(frame, parent * shape.transform, visible)
            }
            _ => {}
        }
        let Some(b) = Self::reach(shape, parent) else {
            return false;
        };
        b.x1 < visible.x0 || b.x0 > visible.x1 || b.y1 < visible.y0 || b.y0 > visible.y1
    }

    /// The part of the page (mm) this renderer's pixmap shows, `None`
    /// when its transform cannot be undone (nothing is left out then).
    fn sight(&self) -> Option<Rect> {
        let c = self.screen.as_coeffs();
        let det = c[0] * c[3] - c[1] * c[2];
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let px = Rect::new(
            0.0,
            0.0,
            self.pixmap.width() as f64,
            self.pixmap.height() as f64,
        );
        let r = self.screen.inverse().transform_rect_bbox(px);
        [r.x0, r.y0, r.x1, r.y1]
            .iter()
            .all(|v| v.is_finite())
            .then_some(r)
    }

    /// Where a plain object (no group or clip frame) inside `parent` can
    /// paint, page mm: its bounds grown by what its outline may reach.
    /// `None` when its bounds are not finite. Effects, shadows, symbols
    /// and the drag preview reach further; callers leave those out.
    fn reach(shape: &Shape, parent: Affine) -> Option<Rect> {
        let b = if parent == Affine::IDENTITY {
            shape.bounds()
        } else {
            parent.transform_rect_bbox(shape.bounds())
        };
        if !b.x0.is_finite() || !b.y0.is_finite() || !b.x1.is_finite() || !b.y1.is_finite() {
            return None;
        }
        let scale = {
            let c = (parent * shape.transform).as_coeffs();
            (c[0] * c[0] + c[1] * c[1])
                .sqrt()
                .max((c[2] * c[2] + c[3] * c[3]).sqrt())
                .max(1.0)
        };
        let width = shape.stroke.as_ref().map_or(0.0, |s| s.width.abs()) * scale;
        // A Variable Length node may reach any distance from the path,
        // past what a width covers: counted on its own, or the band was
        // left out of the strips of the page it reaches into (#626).
        let reach = shape.stroke.as_ref().map_or(0.0, |s| {
            s.widths
                .iter()
                .map(|n| n.left.abs().max(n.right.abs()))
                .filter(|r| r.is_finite())
                .fold(0.0, f64::max)
        }) * scale;
        let margin = (width * OUTLINE_REACH).max(reach) + 1.0;
        Some(b.inflate(margin, margin))
    }

    /// The pixels of this renderer's target that a semi-transparent
    /// object inside `parent` can paint, as x, y, width and height: its
    /// scratch layer needs no more (#1335). A layer the size of the whole
    /// target for each such object made a page of 2000 of them take
    /// seconds to draw. The whole target for an object whose reach is
    /// not its bounds (see [`Self::out_of_sight`]).
    fn layer_area(&self, shape: &Shape, parent: Affine) -> (u32, u32, u32, u32) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let whole = (0, 0, w, h);
        if !shape.effects.is_empty()
            || shape.shadow.is_some()
            || matches!(
                shape.kind,
                ShapeKind::SymbolInstance { .. }
                    | ShapeKind::Group { .. }
                    | ShapeKind::ClipFrame { .. }
            )
            || self.preview.is_some()
        {
            return whole;
        }
        let Some(page) = Self::reach(shape, parent) else {
            return whole;
        };
        let r = self.screen.transform_rect_bbox(page);
        // One pixel more each way for the edge's anti-aliasing.
        let clamp = |v: f64, max: u32| (v.max(0.0).min(max as f64)) as u32;
        let (x0, y0) = (clamp(r.x0.floor() - 1.0, w), clamp(r.y0.floor() - 1.0, h));
        let (x1, y1) = (clamp(r.x1.ceil() + 1.0, w), clamp(r.y1.ceil() + 1.0, h));
        (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
    }

    fn child_transform(&self, child: &Shape, transform: Affine) -> Affine {
        match self.preview {
            Some(p) if p.shapes.contains(&child.id) => p.transform * transform,
            _ => transform,
        }
    }

    /// Blend mode for a fill or outline: multiply when it overprints and
    /// overprint simulation is on, otherwise ordinary source-over.
    fn blend_for(&self, overprint: bool) -> tiny_skia::BlendMode {
        if overprint && self.flags.simulate_overprints {
            tiny_skia::BlendMode::Multiply
        } else {
            tiny_skia::BlendMode::SourceOver
        }
    }

    fn draw_shape(&mut self, shape: &Shape, parent: Affine) {
        if self
            .cancel
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        {
            return;
        }
        // Bitmap effects on a vector object: its picture in its place.
        if let Some(pic) = shape.drawn_as().filter(|_| !self.flags.wireframe) {
            return self.draw_shape(&pic, parent);
        }
        if !shape.visible {
            return;
        }
        // Character backgrounds go under the text, in its transform.
        if matches!(shape.kind, ShapeKind::Text { .. }) && !self.flags.wireframe {
            for (rect, fill) in tracedraw_core::document::text_backgrounds(&shape.kind) {
                let mut bg = Shape::new(
                    shape.id,
                    ShapeKind::Rect {
                        rect,
                        radius: 0.0,
                        corners: None,
                    },
                );
                bg.transform = shape.transform;
                bg.fill = fill;
                bg.stroke = None;
                bg.opacity = shape.opacity;
                self.draw_shape(&bg, parent);
            }
        }
        // Live effects: geometry warps, extra objects below and above, and
        // the pixel effects (lens, non-uniform transparency).
        if !shape.effects.is_empty() {
            let ev = tracedraw_core::live::evaluate(shape);
            // An object whose effects draw objects of their own (the body
            // of an extrusion, a contour's steps) casts the shadow of all
            // of it, beneath all of it. The shadow was the object's
            // alone, drawn over what its effects put behind it (#2215).
            let whole = casts_whole(&ev);
            if whole && !self.flags.wireframe && shape.opacity > 0.0 {
                if let Some(sh) = shape.shadow.as_ref().filter(|sh| sh.opacity > 0.0) {
                    self.draw_group_shadow(sh, shape, parent);
                }
            }
            for b in &ev.below {
                let mut b = b.clone();
                b.effects.clear();
                if whole {
                    b.shadow = None;
                }
                self.draw_shape(&b, parent);
            }
            let lens = shape.effects.iter().find_map(|e| match e {
                tracedraw_core::live::Effect::Lens(l) => Some(l.clone()),
                _ => None,
            });
            let transparency = shape.effects.iter().find_map(|e| match e {
                tracedraw_core::live::Effect::Transparency {
                    mask,
                    merge,
                    target,
                } => Some((mask.clone(), *merge, *target)),
                _ => None,
            });
            let mut main = ev.main.clone();
            main.effects.clear();
            if whole {
                main.shadow = None;
            }
            let was_text = self.flags.text_outline;
            self.flags.text_outline |= matches!(shape.kind, ShapeKind::Text { .. });
            if !self.flags.complex_effects || self.flags.wireframe {
                // Complex effects off: the object is drawn as it is, with
                // no pixel work for lenses or transparency masks.
                self.draw_shape(&main, parent);
            } else if let Some(l) = lens {
                self.draw_lens(&main, parent, &l);
            } else if let Some((mask, merge, target)) = transparency {
                // The transparency's target: the fill alone (0), the
                // outline alone (1) or the whole object (2). The part it
                // leaves out is drawn as it is (live test of 0.41: the
                // target was ignored).
                // An outline set behind the fill is drawn first.
                for (part, faded) in tracedraw_core::live::transparency_parts(&main, target) {
                    if faded {
                        self.draw_with_mask(&part, parent, &mask, merge);
                    } else {
                        self.draw_shape(&part, parent);
                    }
                }
            } else {
                self.draw_shape(&main, parent);
            }
            self.flags.text_outline = was_text;
            // An inner shadow, over the object and cut to its shape
            // (#2473).
            if self.flags.complex_effects && !self.flags.wireframe && main.opacity > 0.0 {
                for e in &shape.effects {
                    if let tracedraw_core::live::Effect::InnerShadow {
                        offset,
                        color,
                        opacity,
                        blur,
                        merge,
                    } = e
                    {
                        self.draw_inner_shadow(
                            &main,
                            parent,
                            InnerShadow {
                                offset: *offset,
                                color: *color,
                                opacity: *opacity,
                                blur: *blur,
                                merge: *merge,
                            },
                        );
                    }
                }
            }
            for a in &ev.above {
                let mut a = a.clone();
                // A mirrored copy keeps its transparency or lens (#2121).
                a.effects.retain(|e| {
                    matches!(
                        e,
                        tracedraw_core::live::Effect::Lens(_)
                            | tracedraw_core::live::Effect::Transparency { .. }
                    )
                });
                if whole {
                    a.shadow = None;
                }
                self.draw_shape(&a, parent);
            }
            return;
        }
        // Characters with their own fill or outline: one text a look.
        if let Some(parts) = shape.text_parts() {
            for p in &parts {
                self.draw_shape(p, parent);
            }
            return;
        }
        let transform = parent * shape.transform;
        // A group's or a clip frame's own shadow, beneath all it holds.
        if matches!(
            shape.kind,
            ShapeKind::Group { .. } | ShapeKind::ClipFrame { .. }
        ) && !self.flags.wireframe
            && shape.opacity > 0.0
        {
            if let Some(sh) = shape.shadow.as_ref().filter(|sh| sh.opacity > 0.0) {
                self.draw_group_shadow(sh, shape, parent);
            }
        }
        if let ShapeKind::Group { children } = &shape.kind {
            // A group's uniform transparency goes on each member, as if
            // set on each: members that overlap show through each other,
            // as files keep it (on the objects of a group) and as Ungroup
            // leaves it (#1386).
            if shape.opacity < 1.0 && !self.flags.wireframe {
                if shape.opacity <= 0.0 {
                    return;
                }
                let sight = self.sight();
                for c in children {
                    if sight.is_some_and(|v| self.out_of_sight_in(c, transform, &v)) {
                        continue;
                    }
                    let mut c = c.clone();
                    c.absorb_opacity(shape.opacity);
                    let t = self.child_transform(&c, transform);
                    self.draw_shape(&c, t);
                }
                return;
            }
            // Only the members that can show in this picture: zoomed in on
            // a large group (a whole design grouped, as files often keep
            // it), every member off screen was drawn all the same, at the
            // zoom's detail, so each zoom in took far longer to draw than
            // the page fitted in the window (discussion #2389).
            let sight = self.sight();
            for c in children {
                if sight.is_some_and(|v| self.out_of_sight_in(c, transform, &v)) {
                    continue;
                }
                let t = self.child_transform(c, transform);
                self.draw_shape(c, t);
            }
            return;
        }
        if matches!(
            shape.kind,
            ShapeKind::Table(_) | ShapeKind::SymbolInstance { .. }
        ) {
            for c in shape.expand(self.symbols) {
                self.draw_shape(&c, transform);
            }
            return;
        }
        if let (true, ShapeKind::ClipFrame { frame, contents }) =
            (self.flags.wireframe, &shape.kind)
        {
            // Wireframe: the frame, and of the contents what lies inside
            // it, as the other views cut them (#2146). They were drawn
            // whole, so the parts the frame hides showed all round it.
            self.draw_wireframe_contents(frame, contents, transform);
            self.draw_shape(frame, transform);
            return;
        }
        if let (ShapeKind::ClipFrame { frame, contents }, Some(om)) =
            (&shape.kind, shape.opacity_mask)
        {
            // Opacity mask: the frame drawn alone gives, by its brightness,
            // how much of the contents shows; the frame itself is not drawn.
            let (w, h) = (self.pixmap.width(), self.pixmap.height());
            if let (Some(mut frame_px), Some(mut layer)) = (Pixmap::new(w, h), Pixmap::new(w, h)) {
                {
                    let mut fill_only = (**frame).clone();
                    fill_only.stroke = None;
                    let mut sub = self.sub(&mut frame_px);
                    sub.draw_shape(&fill_only, transform);
                }
                if om.invert {
                    // Black shows: white where the frame is black, and
                    // nothing outside the frame.
                    for p in frame_px.pixels_mut() {
                        let c = p.demultiply();
                        let a = c.alpha();
                        let inv = |v: u8| 255 - v;
                        *p = tiny_skia::ColorU8::from_rgba(
                            inv(c.red()),
                            inv(c.green()),
                            inv(c.blue()),
                            a,
                        )
                        .premultiply();
                    }
                }
                let mask =
                    tiny_skia::Mask::from_pixmap(frame_px.as_ref(), tiny_skia::MaskType::Luminance);
                {
                    let mut sub = self.sub(&mut layer);
                    for c in contents {
                        let t = sub.child_transform(c, transform);
                        sub.draw_shape(c, t);
                    }
                }
                let pp = tiny_skia::PixmapPaint {
                    opacity: shape.opacity.clamp(0.0, 1.0) as f32,
                    ..Default::default()
                };
                self.pixmap.draw_pixmap(
                    0,
                    0,
                    layer.as_ref(),
                    &pp,
                    Transform::identity(),
                    Some(&mask),
                );
            }
            return;
        }
        if let ShapeKind::ClipFrame { frame, contents } = &shape.kind {
            // A shadow the frame had of its own (given before it became a
            // frame, or while editing inside it) goes once beneath it all.
            // It went with both the fill and the outline drawn below, so
            // it came twice, the second time over the contents.
            if let Some(sh) = frame.shadow.as_ref().filter(|sh| sh.opacity > 0.0) {
                if frame.visible && frame.opacity > 0.0 && shape.opacity > 0.0 {
                    let page_path = (transform * frame.transform) * frame.local_path();
                    self.draw_object_shadow(sh, frame, transform, &page_path);
                }
            }
            // Frame's fill first, contents clipped to the frame, then the frame's outline.
            let mut frame_fill_only = (**frame).clone();
            frame_fill_only.stroke = None;
            frame_fill_only.shadow = None;
            frame_fill_only.opacity = shape.opacity;
            self.draw_clip_frame_inside(shape, &frame_fill_only, contents, transform);
            if frame.stroke.is_some() {
                let mut outline_only = (**frame).clone();
                outline_only.fill = Fill::None;
                outline_only.shadow = None;
                outline_only.opacity = shape.opacity;
                self.draw_shape(&outline_only, transform);
            }
            return;
        }
        // Further fills and outlines: the parts, bottom first.
        if let Some(parts) = shape.layers() {
            for p in &parts {
                self.draw_shape(p, parent);
            }
            return;
        }
        let local = shape.local_path();
        if local.elements().is_empty() {
            return;
        }
        #[cfg(test)]
        PATHS_DRAWN.with(|n| n.set(n.get() + 1));
        let page_path = transform * local;
        let Some(sk) = to_sk_path(&(self.screen * page_path.clone())) else {
            return;
        };
        if self.flags.wireframe {
            // Wireframe: every object as its bare path, the same black
            // hairline whatever its outline's colour or width, its fill,
            // opacity or effects; bitmaps as grey boxes. The hairline
            // took the outline's colour, so an object with a pale outline
            // stood out from the others in a colour of its own, and one
            // with a white outline did not show on the page at all
            // (#2041).
            if let ShapeKind::Bitmap { rect, .. } = &shape.kind {
                let mut paint = Paint::default();
                paint.anti_alias = self.flags.anti_alias;
                paint.set_color_rgba8(208, 208, 208, 255);
                self.pixmap
                    .fill_path(&sk, &paint, FillRule::Winding, Transform::identity(), None);
                // Corner to corner, the mark of a picture left out
                // (#2146): a grey box alone read as an empty frame.
                let mut cross = tracedraw_core::BezPath::new();
                cross.move_to((rect.x0, rect.y0));
                cross.line_to((rect.x1, rect.y1));
                cross.move_to((rect.x0, rect.y1));
                cross.line_to((rect.x1, rect.y0));
                if let Some(cross) = to_sk_path(&(self.screen * (transform * cross))) {
                    self.hairline(&cross, tracedraw_core::Color::BLACK);
                }
            }
            self.hairline(&sk, tracedraw_core::Color::BLACK);
            return;
        }
        let opacity = shape.opacity.clamp(0.0, 1.0) as f32;
        if opacity <= 0.0 {
            return;
        }

        // Drop shadow: blurred silhouette behind the object. A picture's
        // silhouette is what its pixels cover, not its box, so a picture
        // with a transparent background casts the shadow of what it
        // shows (#2027).
        if let Some(sh) = &shape.shadow {
            if !self.flags.wireframe && sh.opacity > 0.0 {
                self.draw_object_shadow(sh, shape, parent, &page_path);
            }
        }

        // Semi-transparent objects render into a scratch layer first so fill
        // and outline do not double up where they overlap.
        // The layer covers only what the object can reach (#1335).
        if opacity < 1.0 {
            let (x, y, w, h) = self.layer_area(shape, parent);
            if w == 0 || h == 0 {
                return;
            }
            if let Some(mut layer) = Pixmap::new(w, h) {
                let mut sub = self.sub(&mut layer);
                sub.screen = Affine::translate((-(x as f64), -(y as f64))) * self.screen;
                sub.origin = (self.origin.0 + x as i32, self.origin.1 + y as i32);
                let mut opaque = shape.clone();
                opaque.opacity = 1.0;
                sub.draw_shape(&opaque, parent);
                let paint = tiny_skia::PixmapPaint {
                    opacity,
                    ..Default::default()
                };
                self.pixmap.draw_pixmap(
                    x as i32,
                    y as i32,
                    layer.as_ref(),
                    &paint,
                    Transform::identity(),
                    None,
                );
            }
            return;
        }

        if let ShapeKind::Bitmap {
            rect,
            width_px,
            height_px,
            png,
            fx: _,
        } = &shape.kind
        {
            if !self.flags.wireframe {
                // Image pixel space (y down) to local rect (y up) to screen.
                let (w0, h0) = ((*width_px).max(1) as f64, (*height_px).max(1) as f64);
                let full = self.screen
                    * transform
                    * Affine::new([
                        rect.width() / w0,
                        0.0,
                        0.0,
                        -rect.height() / h0,
                        rect.x0,
                        rect.y1,
                    ]);
                // Drawn much smaller than its pixels: from a reduced level
                // of the picture near the size drawn (#853).
                let level = bitmap_level(full);
                if let Some(img) = image_cache::decode_level(png, level) {
                    let m = if level == 0 {
                        full.as_coeffs()
                    } else {
                        let (lw, lh) = (img.width().max(1) as f64, img.height().max(1) as f64);
                        let img_to_local = Affine::new([
                            rect.width() / lw,
                            0.0,
                            0.0,
                            -rect.height() / lh,
                            rect.x0,
                            rect.y1,
                        ]);
                        (self.screen * transform * img_to_local).as_coeffs()
                    };
                    let mut paint = Paint::default();
                    paint.anti_alias = self.flags.anti_alias;
                    paint.blend_mode = self.blend_for(shape.overprint_fill);
                    // Upright: on whole pixels, so pictures laid side by
                    // side meet without a pale seam where both edges were
                    // half covered (#853).
                    let snapped = snap_upright(m, img.width(), img.height());
                    let (m, area) = match snapped {
                        Some((m, r)) => (m, Some(r)),
                        None => (m, None),
                    };
                    let t = Transform::from_row(
                        m[0] as f32,
                        m[1] as f32,
                        m[2] as f32,
                        m[3] as f32,
                        m[4] as f32,
                        m[5] as f32,
                    );
                    paint.shader = tiny_skia::Pattern::new(
                        Pixmap::as_ref(&img),
                        SpreadMode::Pad,
                        tiny_skia::FilterQuality::Bilinear,
                        1.0,
                        t,
                    );
                    match area {
                        Some(None) => {}
                        Some(Some(r)) => {
                            paint.anti_alias = false;
                            self.pixmap
                                .fill_rect(r, &paint, Transform::identity(), None);
                        }
                        None => self.pixmap.fill_path(
                            &sk,
                            &paint,
                            FillRule::Winding,
                            Transform::identity(),
                            None,
                        ),
                    }
                }
            }
            if let Some(stroke) = &shape.stroke {
                let (mut paint, sk_stroke) = self.stroke_paint(stroke, transform);
                paint.blend_mode = self.blend_for(shape.overprint_outline);
                self.pixmap
                    .stroke_path(&sk, &paint, &sk_stroke, Transform::identity(), None);
            } else if self.flags.wireframe {
                let mut paint = Paint::default();
                paint.set_color_rgba8(0, 0, 0, 255);
                let s = SkStroke {
                    width: 0.0,
                    ..Default::default()
                };
                self.pixmap
                    .stroke_path(&sk, &paint, &s, Transform::identity(), None);
            }
            return;
        }

        // An outline set behind the fill: the outline first, then the
        // fill over it (half the outline's width shows outside).
        if shape.stroke.as_ref().is_some_and(|st| st.behind_fill)
            && !matches!(shape.fill, Fill::None)
        {
            let mut outline = shape.clone();
            outline.fill = Fill::None;
            if let Some(st) = outline.stroke.as_mut() {
                st.behind_fill = false;
            }
            let mut fill = shape.clone();
            fill.stroke = None;
            fill.shadow = None;
            outline.shadow = None;
            self.draw_shape(&outline, parent);
            self.draw_shape(&fill, parent);
            return;
        }

        if !self.flags.wireframe {
            let rule = shape_fill_rule(shape, self.flags.text_outline);
            let blend = self.blend_for(shape.overprint_fill);
            if let Fill::Mesh(m) = &shape.fill {
                // Mesh nodes are local: bring them to page space first.
                self.fill_mesh(&m.transformed(transform), &sk, rule, blend);
            } else if !self.flags.fill_open_curves
                // An ellipse made an arc (not a pie) is an open curve
                // too: not filled while Fill open curves is off.
                && shape.kind.is_open_curve()
                && !matches!(shape.fill, Fill::None)
            {
                // Only the closed subpaths of a curve get the fill.
                let closed = closed_subpaths(&page_path);
                if let Some(sk_fill) = to_sk_path(&(self.screen * closed.clone())) {
                    self.draw_fill_of(&shape.fill, &closed, &sk_fill, rule, blend, transform);
                }
            } else {
                self.draw_fill_of(&shape.fill, &page_path, &sk, rule, blend, transform);
            }
        }
        if let Some(stroke) = &shape.stroke {
            let (mut paint, sk_stroke) = self.stroke_paint(stroke, transform);
            paint.blend_mode = self.blend_for(shape.overprint_outline);
            if let Some(brush) = stroke.painterly.as_ref().filter(|_| !self.flags.wireframe) {
                // Painterly brush: its marks, each with its own opacity.
                let c = transform.as_coeffs();
                let scale = if stroke.scale_with_object {
                    ((c[0] * c[3] - c[1] * c[2]).abs()).sqrt().max(1e-6)
                } else {
                    1.0
                };
                let [r, g, b] = stroke.color.to_rgb8();
                for mark in
                    tracedraw_core::painterly::marks(&page_path, stroke.width * scale, brush)
                {
                    let Some(mp) = to_sk_path(&(self.screen * mark.path)) else {
                        continue;
                    };
                    paint.set_color_rgba8(r, g, b, (mark.alpha * 255.0).round() as u8);
                    self.pixmap.fill_path(
                        &mp,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
                return;
            }
            let object_scale = if stroke.scale_with_object {
                let c = transform.as_coeffs();
                ((c[0] * c[3] - c[1] * c[2]).abs()).sqrt().max(1e-6)
            } else {
                1.0
            };
            if let Some(band) =
                tracedraw_core::shaping::outline_band(&page_path, stroke, object_scale)
            {
                // Calligraphic nib or variable width: fill the band instead
                // of stroking, with the outline's own fill when it has one.
                if let Some(bp) = to_sk_path(&(self.screen * band.clone())) {
                    match outline_paint(stroke, transform) {
                        Some(fill) => {
                            use tracedraw_core::geometry::Shape as _;
                            let b = bbox(&band).to_path(0.01);
                            self.draw_fill(&fill, &b, &bp, FillRule::Winding);
                        }
                        None => self.pixmap.fill_path(
                            &bp,
                            &paint,
                            FillRule::Winding,
                            Transform::identity(),
                            None,
                        ),
                    }
                }
            } else if let Some(fill) =
                outline_paint(stroke, transform).filter(|_| sk_stroke.width > 0.0)
            {
                // A filled outline: the stroke's area painted with the fill,
                // spread over the outline's bounds.
                if let Some(band) = sk.stroke(&sk_stroke, 1.0) {
                    use tracedraw_core::geometry::Shape as _;
                    let pad = (sk_stroke.width as f64 / self.zoom.max(1e-9)) / 2.0;
                    let b = bbox(&page_path).inflate(pad, pad);
                    self.draw_fill(&fill, &b.to_path(0.01), &band, FillRule::Winding);
                }
            } else {
                self.pixmap
                    .stroke_path(&sk, &paint, &sk_stroke, Transform::identity(), None);
            }
            for head in tracedraw_core::style::arrowhead_paths(&page_path, stroke) {
                if let Some(hp) = to_sk_path(&(self.screen * head)) {
                    self.pixmap.fill_path(
                        &hp,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
        } else if self.flags.wireframe {
            let mut paint = Paint::default();
            paint.set_color_rgba8(0, 0, 0, 255);
            paint.anti_alias = self.flags.anti_alias;
            let s = SkStroke {
                width: 0.0,
                ..Default::default()
            };
            self.pixmap
                .stroke_path(&sk, &paint, &s, Transform::identity(), None);
        }
    }

    /// The drop shadow `sh` of `shape` drawn inside `parent`, whose
    /// page-space path is `page_path`, without the object: a picture's
    /// from its pixels, a group's or clip frame's from what it shows,
    /// any other object's from its path.
    fn draw_object_shadow(
        &mut self,
        sh: &tracedraw_core::Shadow,
        shape: &Shape,
        parent: Affine,
        page_path: &BezPath,
    ) {
        let drawn = match &shape.kind {
            ShapeKind::Group { .. } | ShapeKind::ClipFrame { .. } => {
                self.draw_group_shadow(sh, shape, parent);
                true
            }
            _ if page_path.elements().is_empty() => true,
            ShapeKind::Bitmap {
                rect,
                width_px,
                height_px,
                png,
                ..
            } => self.draw_bitmap_shadow(
                sh,
                page_path,
                parent * shape.transform,
                *rect,
                (*width_px, *height_px),
                png,
            ),
            _ => false,
        };
        if !drawn {
            let rule = shape_fill_rule(shape, self.flags.text_outline);
            self.draw_drop_shadow(sh, shape, parent, page_path, rule);
        }
    }

    /// The inside of a clip frame `clip` drawn inside `parent`: the
    /// frame's fill (`fill_only`, its outline and shadow taken off) and
    /// the contents over it, cut to the frame. Both are put together
    /// first and then cut by the frame's coverage in one step, so an edge
    /// pixel the frame half covers takes half of what the frame shows
    /// and half of what lies beneath. The fill was drawn with its own
    /// soft edge and the contents then blended over it by the same
    /// coverage again, which left a thin line of the fill's colour (or
    /// of a shadow beneath) all round a frame that did not sit on whole
    /// pixels.
    fn draw_clip_frame_inside(
        &mut self,
        clip: &Shape,
        fill_only: &Shape,
        contents: &[Shape],
        parent: Affine,
    ) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let frame_path = self.screen * (parent * fill_only.page_path());
        let Some(sk) = to_sk_path(&frame_path) else {
            // No frame to cut to: its fill alone, as it is.
            self.draw_shape(fill_only, parent);
            return;
        };
        let b = sk.bounds();
        let x0 = (b.left().floor().max(0.0) as usize).min(w as usize);
        let x1 = (b.right().ceil().max(0.0) as usize).min(w as usize);
        let y0 = (b.top().floor().max(0.0) as usize).min(h as usize);
        let y1 = (b.bottom().ceil().max(0.0) as usize).min(h as usize);
        // The contents are put together on a layer covering only the
        // frame's box (with room for their shadows' blur), not the whole
        // canvas: a canvas-sized copy and mask for every clip frame made
        // a large drawing of many of them slow to draw at every zoom
        // (#2522), and the contents outside the box are then left out as
        // off screen. A lens samples what lies anywhere around it, so
        // with one inside the layer stays the whole canvas.
        let (mut lens, mut blur) = (false, 0.0f64);
        for c in contents {
            reach_walk(c, &mut lens, &mut blur);
        }
        let (lx0, ly0, lx1, ly1) = if lens {
            (0, 0, w as usize, h as usize)
        } else {
            let m = blur_margin(blur, self.zoom) as usize;
            let align = BAND_ALIGN as usize;
            (
                x0.saturating_sub(m) / align * align,
                y0.saturating_sub(m) / align * align,
                x1.saturating_add(m).min(w as usize),
                y1.saturating_add(m).min(h as usize),
            )
        };
        let (rw, rh) = (lx1.saturating_sub(lx0), ly1.saturating_sub(ly0));
        let mask = (|| {
            let mut mask = tiny_skia::Mask::new(rw as u32, rh as u32)?;
            let rule = shape_fill_rule(fill_only, false);
            mask.fill_path(
                &sk,
                rule,
                true,
                Transform::from_translate(-(lx0 as f32), -(ly0 as f32)),
            );
            Some(mask)
        })();
        let Some(mask) = mask else {
            // Nothing of the frame on the canvas: its fill alone, as it is.
            self.draw_shape(fill_only, parent);
            return;
        };
        // The fill on a layer of its own, only as large as it reaches.
        let (fx, fy, fw, fh) = self.layer_area(fill_only, parent);
        let fill_px = Pixmap::new(fw, fh).map(|mut px| {
            let mut sub = self.sub(&mut px);
            sub.screen = Affine::translate((-f64::from(fx), -f64::from(fy))) * self.screen;
            sub.origin = (self.origin.0 + fx as i32, self.origin.1 + fy as i32);
            sub.draw_shape(fill_only, parent);
            px
        });
        let (bw, w) = (x1.saturating_sub(x0), w as usize);
        // What lies beneath inside the frame's box, before the fill.
        let mut beneath = Vec::with_capacity(bw * (y1.saturating_sub(y0)) * 4);
        for y in y0..y1 {
            if let Some(row) = self.pixmap.data().get((y * w + x0) * 4..(y * w + x1) * 4) {
                beneath.extend_from_slice(row);
            }
        }
        if beneath.len() != bw * (y1.saturating_sub(y0)) * 4 {
            return;
        }
        // The fill as drawn, edge and all: where the frame does not reach
        // (the outlines of a group used as a frame) it stays so.
        if let Some(px) = &fill_px {
            self.pixmap.draw_pixmap(
                fx as i32,
                fy as i32,
                px.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                Transform::identity(),
                None,
            );
        }
        // The contents are drawn over a copy of what lies beneath, so
        // their merge modes (Multiply on a photo, say) work on it as
        // outside a frame; inside the frame the copy then takes the place
        // of the page (#13). Inside the frame the copy holds the fill
        // without its soft edge (the frame's coverage gives the edge
        // once, at the end). The copy is of the layer's box only.
        let Some(mut layer) = Pixmap::new(rw as u32, rh as u32) else {
            return;
        };
        {
            let src = self.pixmap.data();
            let dst = layer.data_mut();
            for y in ly0..ly1 {
                let (s0, d0) = ((y * w + lx0) * 4, (y - ly0) * rw * 4);
                if let (Some(s), Some(d)) = (src.get(s0..s0 + rw * 4), dst.get_mut(d0..d0 + rw * 4))
                {
                    d.copy_from_slice(s);
                }
            }
        }
        // Index of canvas pixel (x, y) in the layer and its mask.
        let at = |x: usize, y: usize| (y - ly0) * rw + (x - lx0);
        let m = mask.data();
        let mut under = beneath.clone();
        {
            let fill = fill_px.as_ref().map(|p| (p.data(), p.width() as usize));
            let dst = layer.data_mut();
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = at(x, y);
                    let k = m.get(i).copied().unwrap_or(0);
                    if k == 0 {
                        continue;
                    }
                    let j = ((y - y0) * bw + (x - x0)) * 4;
                    let f = fill.and_then(|(d, fw)| {
                        let (lx, ly) = (x.checked_sub(fx as usize)?, y.checked_sub(fy as usize)?);
                        if lx >= fw {
                            return None;
                        }
                        d.get((ly * fw + lx) * 4..(ly * fw + lx) * 4 + 4)
                    });
                    let mut f = match f {
                        Some(f) => [f[0] as f32, f[1] as f32, f[2] as f32, f[3] as f32],
                        None => [0.0; 4],
                    };
                    if k < 255 {
                        let s = 255.0 / k as f32;
                        f[3] = (f[3] * s).min(255.0);
                        for c in 0..3 {
                            f[c] = (f[c] * s).min(f[3]);
                        }
                    }
                    let keep = 1.0 - f[3] / 255.0;
                    for c in 0..4 {
                        let v = (f[c] + under[j + c] as f32 * keep)
                            .round()
                            .clamp(0.0, 255.0);
                        under[j + c] = v as u8;
                        if let Some(d) = dst.get_mut(i * 4 + c) {
                            *d = v as u8;
                        }
                    }
                }
            }
        }
        {
            let mut sub = self.sub(&mut layer);
            sub.screen = Affine::translate((-(lx0 as f64), -(ly0 as f64))) * self.screen;
            sub.origin = (self.origin.0 + lx0 as i32, self.origin.1 + ly0 as i32);
            let sight = sub.sight();
            for c in contents {
                if sight.is_some_and(|v| sub.out_of_sight_in(c, parent, &v)) {
                    continue;
                }
                let t = sub.child_transform(c, parent);
                sub.draw_shape(c, t);
            }
        }
        let opacity = clip.opacity.clamp(0.0, 1.0) as f32;
        let src = layer.data();
        let dst = self.pixmap.data_mut();
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * w + x;
                let li = at(x, y);
                let k = m.get(li).copied().unwrap_or(0) as f32 / 255.0;
                if k <= 0.0 {
                    continue;
                }
                let j = ((y - y0) * bw + (x - x0)) * 4;
                for c in 0..4 {
                    let (Some(s), Some(d)) = (src.get(li * 4 + c), dst.get_mut(i * 4 + c)) else {
                        continue;
                    };
                    let u = under[j + c] as f32;
                    let inside = u + (*s as f32 - u) * opacity;
                    let bg = beneath[j + c] as f32;
                    *d = (bg + (inside - bg) * k).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    /// A drop shadow beneath an object whose page-space path is
    /// `page_path`: its silhouette moved (flat) or sheared (perspective),
    /// filled with the shadow colour, feathered, faded along a
    /// perspective shadow and merged with what is beneath. `rule` is the
    /// rule the object is filled by, so its holes are the shadow's.
    /// `shape` is the object, drawn inside `parent`: the shadow is of
    /// what it shows, its fill and its outline, and its layer reaches
    /// round all its outline may cover.
    fn draw_drop_shadow(
        &mut self,
        sh: &tracedraw_core::Shadow,
        shape: &Shape,
        parent: Affine,
        page_path: &BezPath,
        rule: FillRule,
    ) {
        let b = page_path.bounding_box();
        let extent = Self::reach(shape, parent).map_or(b, |r| r.union(b));
        let object = parent * shape.transform;
        let Some((layer_at, mut layer, screen)) = self.shadow_target_over(sh, b, extent) else {
            return;
        };
        let _screen = accel::on_screen(self.flags.draft);
        if !shadow_layer(
            &mut layer,
            sh,
            Shown {
                shape,
                path: page_path,
                object,
                rule,
            },
            screen,
            self.zoom,
            self.flags.anti_alias,
        ) {
            return;
        }
        self.merge_shadow(sh, &layer, layer_at);
    }

    /// The drop shadow of a picture: its silhouette is the picture's own
    /// alpha, placed as the picture is and moved as the shadow asks, so a
    /// transparent background casts no shadow (#2027). False when the
    /// picture cannot be decoded; the caller then shadows its box.
    fn draw_bitmap_shadow(
        &mut self,
        sh: &tracedraw_core::Shadow,
        page_path: &BezPath,
        transform: Affine,
        rect: Rect,
        (width_px, height_px): (u32, u32),
        png: &[u8],
    ) -> bool {
        let Some(img) = image_cache::decode_level(png, 0) else {
            return false;
        };
        let b = page_path.bounding_box();
        let Some((layer_at, mut layer, screen)) = self.shadow_target(sh, b) else {
            return true;
        };
        bitmap_silhouette(
            &mut layer,
            sh,
            b,
            screen,
            transform,
            rect,
            (width_px, height_px),
            &img,
        );
        let _screen = accel::on_screen(self.flags.draft);
        shade_shadow(&mut layer, sh, b, screen, self.zoom);
        self.merge_shadow(sh, &layer, layer_at);
        true
    }

    /// The layer a drop shadow measured on box `b` is painted on: only
    /// the pixels of the target it can reach (see [`shadow_pixels`]),
    /// with where it sits on the target and the page-to-layer map. A
    /// layer the size of the whole target made every frame of a shadow
    /// drag blur and shade the whole window (#935).
    fn shadow_target(
        &self,
        sh: &tracedraw_core::Shadow,
        b: Rect,
    ) -> Option<((i32, i32), Pixmap, Affine)> {
        self.shadow_target_over(sh, b, b)
    }

    /// [`Self::shadow_target`] for a silhouette that covers `extent`.
    fn shadow_target_over(
        &self,
        sh: &tracedraw_core::Shadow,
        b: Rect,
        extent: Rect,
    ) -> Option<((i32, i32), Pixmap, Affine)> {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let (x, y, lw, lh) = shadow_pixels_over(sh, b, extent, self.screen, self.zoom, w, h)?;
        let layer = Pixmap::new(lw, lh)?;
        let screen = Affine::translate((-f64::from(x), -f64::from(y))) * self.screen;
        Some(((x, y), layer, screen))
    }

    /// The drop shadow of a group or a clip frame drawn inside `parent`:
    /// the silhouette of what it shows (see [`group_shadow_layer`]). These
    /// returned before their own shadow was drawn, so a shadow given to a
    /// clip frame or a group never showed (#873).
    fn draw_group_shadow(&mut self, sh: &tracedraw_core::Shadow, shape: &Shape, parent: Affine) {
        let Some((b, extent)) = cast_boxes(shape, parent) else {
            return;
        };
        let Some((layer_at, mut layer, screen)) = self.shadow_target_over(sh, b, extent) else {
            return;
        };
        let _screen = accel::on_screen(self.flags.draft);
        group_shadow_layer(
            &mut layer,
            sh,
            shape,
            parent,
            b,
            screen,
            self.zoom,
            self.flags.anti_alias,
        );
        self.merge_shadow(sh, &layer, layer_at);
    }

    /// The inner shadow `ish` of `shape` drawn inside `parent`: what lies
    /// outside the object, moved by the shadow's offset, blurred by its
    /// feathering and kept where the object covers, in the shadow's
    /// colour and opacity, merged over what the object drew (#2473).
    fn draw_inner_shadow(&mut self, shape: &Shape, parent: Affine, ish: InnerShadow) {
        let opacity = if ish.opacity.is_finite() {
            ish.opacity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if opacity <= 0.0 {
            return;
        }
        let page_path = parent * shape.page_path();
        let Some(object) = to_sk_path(&(self.screen * page_path.clone())) else {
            return;
        };
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let b = object.bounds();
        let clamp = |v: f32, max: u32| (v.max(0.0).min(max as f32)) as u32;
        let (x0, y0) = (
            clamp(b.left().floor() - 1.0, w),
            clamp(b.top().floor() - 1.0, h),
        );
        let (x1, y1) = (
            clamp(b.right().ceil() + 1.0, w),
            clamp(b.bottom().ceil() + 1.0, h),
        );
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let blur = if ish.blur.is_finite() {
            ish.blur.max(0.0)
        } else {
            0.0
        };
        let radius = (blur * self.zoom).round().min(64.0) as i32;
        // Room round the object for the blur to draw on what lies outside.
        let pad = radius.max(0) as u32 * 3 + 2;
        let (lw, lh) = (x1 - x0 + 2 * pad, y1 - y0 + 2 * pad);
        let Some(mut layer) = Pixmap::new(lw, lh) else {
            return;
        };
        let to_layer = Affine::translate((
            f64::from(pad) - f64::from(x0),
            f64::from(pad) - f64::from(y0),
        ));
        let rule = shape_fill_rule(shape, self.flags.text_outline);
        // Everything outside the object, moved by the offset, casts.
        layer.fill(tiny_skia::Color::WHITE);
        let moved = to_layer
            * self.screen
            * (Affine::translate((ish.offset.x, ish.offset.y)) * page_path.clone());
        if let Some(sk) = to_sk_path(&moved) {
            let mut clear = Paint::default();
            clear.anti_alias = self.flags.anti_alias;
            clear.blend_mode = tiny_skia::BlendMode::Clear;
            layer.fill_path(&sk, &clear, rule, Transform::identity(), None);
        }
        if radius > 0 {
            accel::box_blur(&mut layer, radius, 3);
        }
        // Kept where the object covers.
        let Some(mut mask) = tiny_skia::Mask::new(lw, lh) else {
            return;
        };
        if let Some(sk) = to_sk_path(&(to_layer * (self.screen * page_path))) {
            mask.fill_path(&sk, rule, self.flags.anti_alias, Transform::identity());
        }
        let [cr, cg, cb] = ish.color.to_rgb8();
        for (px, m) in layer.pixels_mut().iter_mut().zip(mask.data()) {
            let a = px.alpha() as f64 / 255.0 * f64::from(*m) / 255.0 * opacity;
            *px =
                tiny_skia::ColorU8::from_rgba(cr, cg, cb, (a * 255.0).round() as u8).premultiply();
        }
        // Back to the picture's size and place.
        let Some(mut out) = Pixmap::new(x1 - x0, y1 - y0) else {
            return;
        };
        out.draw_pixmap(
            -(pad as i32),
            -(pad as i32),
            layer.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            Transform::identity(),
            None,
        );
        composite_layer(self.pixmap, &out, (x0 as i32, y0 as i32), ish.merge);
    }

    /// Merge a finished shadow layer, its top left at `at` on the
    /// target, with what lies beneath.
    fn merge_shadow(&mut self, sh: &tracedraw_core::Shadow, layer: &Pixmap, at: (i32, i32)) {
        composite_layer(self.pixmap, layer, at, sh.merge);
    }

    /// The contents of a clip frame in the Wireframe view: their
    /// hairlines on a layer the size of the frame, cut to the frame.
    /// A frame with no area to cut to shows them whole.
    fn draw_wireframe_contents(&mut self, frame: &Shape, contents: &[Shape], parent: Affine) {
        if contents.is_empty() {
            return;
        }
        let (x, y, w, h) = self.layer_area(frame, parent);
        let to_layer = Affine::translate((-f64::from(x), -f64::from(y))) * self.screen;
        let cut = to_sk_path(&(to_layer * (parent * frame.page_path()))).and_then(|sk| {
            let mut mask = tiny_skia::Mask::new(w, h)?;
            mask.fill_path(
                &sk,
                shape_fill_rule(frame, false),
                self.flags.anti_alias,
                Transform::identity(),
            );
            Some(mask)
        });
        let (Some(mask), Some(mut layer)) = (cut, Pixmap::new(w, h)) else {
            // Nothing of the frame in sight, or no outline to cut to.
            if w > 0 && h > 0 {
                for c in contents {
                    let t = self.child_transform(c, parent);
                    self.draw_shape(c, t);
                }
            }
            return;
        };
        {
            let mut sub = self.sub(&mut layer);
            sub.screen = to_layer;
            sub.origin = (self.origin.0 + x as i32, self.origin.1 + y as i32);
            for c in contents {
                let t = sub.child_transform(c, parent);
                sub.draw_shape(c, t);
            }
        }
        layer.apply_mask(&mask);
        self.pixmap.draw_pixmap(
            x as i32,
            y as i32,
            layer.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    /// A one pixel line along `sk` (the Wireframe view).
    fn hairline(&mut self, sk: &tiny_skia::Path, color: tracedraw_core::Color) {
        let mut paint = Paint::default();
        let [r, g, b] = color.to_rgb8();
        paint.set_color_rgba8(r, g, b, 255);
        paint.anti_alias = self.flags.anti_alias;
        let s = SkStroke {
            width: 0.0,
            ..Default::default()
        };
        self.pixmap
            .stroke_path(sk, &paint, &s, Transform::identity(), None);
    }

    /// Lens: transform the pixels already drawn beneath the shape's area.
    fn draw_lens(&mut self, shape: &Shape, parent: Affine, lens: &tracedraw_core::live::Lens) {
        use tracedraw_core::live::Lens;
        let transform = parent * shape.transform;
        let path = self.screen * (transform * shape.local_path());
        let Some(sk) = to_sk_path(&path) else {
            return;
        };
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Some(mut mask) = tiny_skia::Mask::new(w, h) else {
            return;
        };
        let rule = shape_fill_rule(shape, false);
        mask.fill_path(&sk, rule, true, Transform::identity());
        let source = self.pixmap.clone();
        let b = path.bounding_box();
        let center = b.center();
        let (x0, y0) = (b.x0.max(0.0) as u32, b.y0.max(0.0) as u32);
        let (x1, y1) = ((b.x1.ceil() as u32).min(w), (b.y1.ceil() as u32).min(h));
        let mdata = mask.data();
        let src_px = |x: i64, y: i64| -> [f32; 3] {
            if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                return [1.0, 1.0, 1.0];
            }
            let p = source.pixel(x as u32, y as u32).map(|p| p.demultiply());
            match p {
                Some(p) if p.alpha() > 0 => {
                    let a = p.alpha() as f32 / 255.0;
                    // Composite over white (the page).
                    [
                        p.red() as f32 / 255.0 * a + (1.0 - a),
                        p.green() as f32 / 255.0 * a + (1.0 - a),
                        p.blue() as f32 / 255.0 * a + (1.0 - a),
                    ]
                }
                _ => [1.0, 1.0, 1.0],
            }
        };
        // Bitmap Effects lens: the effects run over what lies beneath (with
        // a margin, so blurs and the like see past the edge), then the
        // result shows inside the lens.
        let filtered: Option<(u32, u32, u32, Vec<u8>)> = match lens {
            Lens::Effects { effects } => PIXEL_EFFECTS.get().and_then(|run| {
                let m = 24;
                let (rx0, ry0) = (x0.saturating_sub(m), y0.saturating_sub(m));
                let (rx1, ry1) = ((x1 + m).min(w), (y1 + m).min(h));
                let (rw, rh) = (rx1.checked_sub(rx0)?, ry1.checked_sub(ry0)?);
                if rw == 0 || rh == 0 {
                    return None;
                }
                let mut buf = Vec::with_capacity((rw * rh * 4) as usize);
                for y in ry0..ry1 {
                    for x in rx0..rx1 {
                        let c = src_px(x as i64, y as i64);
                        buf.extend_from_slice(&[
                            (c[0] * 255.0).round() as u8,
                            (c[1] * 255.0).round() as u8,
                            (c[2] * 255.0).round() as u8,
                            255,
                        ]);
                    }
                }
                run(effects, rw, rh, &mut buf);
                (buf.len() == (rw * rh * 4) as usize).then_some((rx0, ry0, rw, buf))
            }),
            _ => None,
        };
        let pixels = self.pixmap.pixels_mut();
        for y in y0..y1 {
            for x in x0..x1 {
                let m = mdata[(y * w + x) as usize];
                if m == 0 {
                    continue;
                }
                let c = match lens {
                    Lens::Effects { .. } => match &filtered {
                        Some((rx0, ry0, rw, buf)) => {
                            let i = (((y - ry0) * rw + (x - rx0)) * 4) as usize;
                            match buf.get(i..i + 3) {
                                Some(p) => [
                                    p[0] as f32 / 255.0,
                                    p[1] as f32 / 255.0,
                                    p[2] as f32 / 255.0,
                                ],
                                None => src_px(x as i64, y as i64),
                            }
                        }
                        None => src_px(x as i64, y as i64),
                    },
                    Lens::FishEye { rate } => {
                        let r = (*rate / 1000.0).clamp(-0.95, 10.0);
                        let dx = x as f64 - center.x;
                        let dy = y as f64 - center.y;
                        let radius = (b.width().min(b.height()) / 2.0).max(1.0);
                        let d = (dx * dx + dy * dy).sqrt() / radius;
                        let k = if d > 0.0 { d.powf(1.0 + r) / d } else { 1.0 };
                        src_px(
                            (center.x + dx * k).round() as i64,
                            (center.y + dy * k).round() as i64,
                        )
                    }
                    Lens::Magnify { amount } => {
                        let a = amount.max(1.0);
                        let dx = (x as f64 - center.x) / a;
                        let dy = (y as f64 - center.y) / a;
                        src_px(
                            (center.x + dx).round() as i64,
                            (center.y + dy).round() as i64,
                        )
                    }
                    Lens::Wireframe { fill, .. } => {
                        let [r, g, bl] = fill.to_rgb8();
                        [r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0]
                    }
                    other => other.map_color(src_px(x as i64, y as i64)),
                };
                let mf = m as f32 / 255.0;
                let old = src_px(x as i64, y as i64);
                let out = [
                    old[0] + (c[0] - old[0]) * mf,
                    old[1] + (c[1] - old[1]) * mf,
                    old[2] + (c[2] - old[2]) * mf,
                ];
                if let Some(px) = tiny_skia::ColorU8::from_rgba(
                    (out[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                    (out[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                    (out[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                    255,
                )
                .premultiply()
                .into()
                {
                    pixels[(y * w + x) as usize] = px;
                }
            }
        }
        // The lens object's own outline.
        if let Some(stroke) = &shape.stroke {
            let mut outline = shape.clone();
            outline.fill = Fill::None;
            outline.stroke = Some(stroke.clone());
            self.draw_shape(&outline, parent);
        }
    }

    /// Non-uniform transparency: render the shape to a layer, multiply its
    /// alpha by the luminance of the mask fill, composite with a blend mode.
    fn draw_with_mask(
        &mut self,
        shape: &Shape,
        parent: Affine,
        mask_fill: &Fill,
        merge: tracedraw_core::live::MergeMode,
    ) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let (Some(mut layer), Some(mut mask_pm)) = (Pixmap::new(w, h), Pixmap::new(w, h)) else {
            return;
        };
        {
            let mut sub = self.sub(&mut layer);
            sub.draw_shape(shape, parent);
        }
        let transform = parent * shape.transform;
        let page_path = transform * shape.local_path();
        let bounds = page_path.bounding_box();
        let rect_path = tracedraw_core::geometry::rect_path(bounds, 0.0);
        if let Some(sk) = to_sk_path(&(self.screen * rect_path.clone())) {
            let mut sub = self.sub(&mut mask_pm);
            sub.flags.wireframe = false;
            match mask_fill {
                // A shaped mask follows the object, over its whole bounds.
                Fill::Fountain(f) if f.kind == tracedraw_core::FountainKind::Shaped => {
                    if let Some(shape_sk) = to_sk_path(&(self.screen * page_path.clone())) {
                        let rule = shape_fill_rule(shape, false);
                        sub.fill_shaped(f, &shape_sk, &sk, rule);
                    }
                }
                // A pattern mask set to transform with its object follows it.
                _ => sub.draw_fill_of(
                    mask_fill,
                    &rect_path,
                    &sk,
                    FillRule::Winding,
                    tiny_skia::BlendMode::SourceOver,
                    transform,
                ),
            }
        }
        let lum = |p: &tiny_skia::PremultipliedColorU8| {
            let d = p.demultiply();
            if d.alpha() == 0 {
                255
            } else {
                ((d.red() as u32 * 54 + d.green() as u32 * 183 + d.blue() as u32 * 19) / 256) as u8
            }
        };
        let mask_px = mask_pm.pixels();
        // The mask is drawn over the object's box; past it (where the
        // outer half of an outline lies) it goes on as it is at the box's
        // edge, the nearest pixel wholly inside the box. Taken as no
        // transparency there, the outline of a rounded rectangle showed
        // opaque outside the box and faded inside it, so most of it
        // seemed to stay and its corners to vanish (live test of 0.41).
        let sb = (self.screen * rect_path).bounding_box();
        let inner = |lo: f64, hi: f64, n: u32| -> Option<(usize, usize)> {
            let (a, b) = (lo.ceil().max(0.0), (hi.floor() - 1.0).min(n as f64 - 1.0));
            (a.is_finite() && b.is_finite() && a <= b).then_some((a as usize, b as usize))
        };
        let mdata: Vec<u8> = match (inner(sb.x0, sb.x1, w), inner(sb.y0, sb.y1, h)) {
            (Some((x0, x1)), Some((y0, y1))) => (0..h as usize)
                .flat_map(|y| (0..w as usize).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let (cx, cy) = (x.clamp(x0, x1), y.clamp(y0, y1));
                    mask_px.get(cy * w as usize + cx).map_or(255, lum)
                })
                .collect(),
            _ => mask_px.iter().map(lum).collect(),
        };
        for (px, m) in layer.pixels_mut().iter_mut().zip(mdata) {
            let d = px.demultiply();
            let a = (d.alpha() as u32 * m as u32 / 255) as u8;
            if let Some(np) = tiny_skia::ColorU8::from_rgba(d.red(), d.green(), d.blue(), a)
                .premultiply()
                .into()
            {
                *px = np;
            }
        }
        composite_layer(self.pixmap, &layer, (0, 0), merge);
    }

    /// Fill a screen-space path with any fill type.
    fn draw_fill(&mut self, fill: &Fill, page_path: &BezPath, sk: &SkPath, rule: FillRule) {
        self.draw_fill_blended(fill, page_path, sk, rule, tiny_skia::BlendMode::SourceOver);
    }

    /// `draw_fill` with a blend mode. Solid fills blend directly; the other
    /// fill types render into a scratch layer that is then composited with
    /// the mode, so every fill type can overprint.
    fn draw_fill_blended(
        &mut self,
        fill: &Fill,
        page_path: &BezPath,
        sk: &SkPath,
        rule: FillRule,
        blend: tiny_skia::BlendMode,
    ) {
        self.draw_fill_of(fill, page_path, sk, rule, blend, Affine::IDENTITY);
    }

    /// `draw_fill_blended` for the fill of an object drawn with
    /// `object` (its transform with its parents'): a pattern set to
    /// transform with its object follows it.
    fn draw_fill_of(
        &mut self,
        fill: &Fill,
        page_path: &BezPath,
        sk: &SkPath,
        rule: FillRule,
        blend: tiny_skia::BlendMode,
        object: Affine,
    ) {
        if blend != tiny_skia::BlendMode::SourceOver && !matches!(fill, Fill::Solid(_)) {
            let (w, h) = (self.pixmap.width(), self.pixmap.height());
            let Some(mut layer) = Pixmap::new(w, h) else {
                return;
            };
            {
                let mut sub = self.sub(&mut layer);
                sub.draw_fill_of(
                    fill,
                    page_path,
                    sk,
                    rule,
                    tiny_skia::BlendMode::SourceOver,
                    object,
                );
            }
            let pp = tiny_skia::PixmapPaint {
                blend_mode: blend,
                ..Default::default()
            };
            self.pixmap
                .draw_pixmap(0, 0, layer.as_ref(), &pp, Transform::identity(), None);
            return;
        }
        let mut paint = Paint::default();
        paint.anti_alias = self.flags.anti_alias;
        paint.blend_mode = blend;
        match fill {
            Fill::None => {}
            Fill::Solid(c) => {
                paint.set_color(sk_color(*c));
                self.pixmap
                    .fill_path(sk, &paint, rule, Transform::identity(), None);
            }
            Fill::Fountain(f) => {
                let b = bbox(page_path);
                // The edge pad, repeats, mirror and inversion are worked
                // into one run of stops the shader pads beyond.
                let stops: Vec<GradientStop> = f
                    .resolved_stops()
                    .iter()
                    .map(|s| GradientStop::new(s.pos as f32, sk_color_alpha(s.color, s.opacity)))
                    .collect();
                // Nodes placed freely with the Interactive Fill tool: the
                // fill's own frame (start, end, skew handle) places it.
                if let Some(frame) = f.frame(b) {
                    let m = (self.screen * frame).as_coeffs();
                    let t = Transform::from_row(
                        m[0] as f32,
                        m[1] as f32,
                        m[2] as f32,
                        m[3] as f32,
                        m[4] as f32,
                        m[5] as f32,
                    );
                    let origin = tiny_skia::Point::from_xy(0.0, 0.0);
                    let shader = match f.kind {
                        tracedraw_core::FountainKind::Linear => LinearGradient::new(
                            origin,
                            tiny_skia::Point::from_xy(1.0, 0.0),
                            stops,
                            SpreadMode::Pad,
                            t,
                        ),
                        tracedraw_core::FountainKind::Radial => {
                            RadialGradient::new(origin, 0.0, origin, 1.0, stops, SpreadMode::Pad, t)
                        }
                        _ => None,
                    };
                    if let Some(sh) = shader {
                        paint.shader = sh;
                        self.pixmap
                            .fill_path(sk, &paint, rule, Transform::identity(), None);
                    } else if matches!(
                        f.kind,
                        tracedraw_core::FountainKind::Conical
                            | tracedraw_core::FountainKind::Square
                    ) {
                        let f2 = f.clone();
                        // The blends once; per pixel only the parameter,
                        // its band and a bisection in the stops.
                        let base = f.blended_stops();
                        let key = format!("{f:?}");
                        self.fill_procedural(page_path, sk, rule, &key, move |u, v| {
                            let p = Point::new(b.x0 + u * b.width(), b.y0 + v * b.height());
                            let t = f2.param_at(b, p).unwrap_or(0.0);
                            let s = f2.sample_blended(&base, t);
                            (s.color, s.opacity)
                        });
                    }
                    return;
                }
                match f.kind {
                    tracedraw_core::FountainKind::Shaped => {
                        self.fill_shaped(f, sk, sk, rule);
                    }
                    tracedraw_core::FountainKind::Linear => {
                        let a = f.angle.to_radians();
                        let (cx, cy) = (b.center().x, b.center().y);
                        let half = (b.width() * a.cos().abs() + b.height() * a.sin().abs()) / 2.0;
                        let p0 = self.screen * Point::new(cx - half * a.cos(), cy - half * a.sin());
                        let p1 = self.screen * Point::new(cx + half * a.cos(), cy + half * a.sin());
                        if let Some(sh) = LinearGradient::new(
                            tiny_skia::Point::from_xy(p0.x as f32, p0.y as f32),
                            tiny_skia::Point::from_xy(p1.x as f32, p1.y as f32),
                            stops,
                            SpreadMode::Pad,
                            Transform::identity(),
                        ) {
                            paint.shader = sh;
                            self.pixmap
                                .fill_path(sk, &paint, rule, Transform::identity(), None);
                        }
                    }
                    tracedraw_core::FountainKind::Radial => {
                        let c = Point::new(
                            b.center().x + f.offset.x * b.width() / 2.0,
                            b.center().y + f.offset.y * b.height() / 2.0,
                        );
                        let radius = (b.width().max(b.height()) / 2.0) * std::f64::consts::SQRT_2;
                        let sc = self.screen * c;
                        if let Some(sh) = RadialGradient::new(
                            tiny_skia::Point::from_xy(sc.x as f32, sc.y as f32),
                            0.0,
                            tiny_skia::Point::from_xy(sc.x as f32, sc.y as f32),
                            (radius * self.zoom) as f32,
                            stops,
                            SpreadMode::Pad,
                            Transform::identity(),
                        ) {
                            paint.shader = sh;
                            self.pixmap
                                .fill_path(sk, &paint, rule, Transform::identity(), None);
                        }
                    }
                    tracedraw_core::FountainKind::Conical
                    | tracedraw_core::FountainKind::Square => {
                        let f2 = f.clone();
                        let base = f.blended_stops();
                        let key = format!("{f:?}");
                        self.fill_procedural(page_path, sk, rule, &key, move |u, v| {
                            let dx = u - 0.5 - f2.offset.x / 2.0;
                            let dy = v - 0.5 - f2.offset.y / 2.0;
                            let t = match f2.kind {
                                tracedraw_core::FountainKind::Conical => {
                                    let a = dy.atan2(dx) - f2.angle.to_radians();
                                    let a =
                                        a.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
                                    if a < 0.5 {
                                        a * 2.0
                                    } else {
                                        2.0 - a * 2.0
                                    }
                                }
                                _ => (dx.abs().max(dy.abs()) * 2.0).min(1.0),
                            };
                            let s = f2.sample_blended(&base, t);
                            (s.color, s.opacity)
                        });
                    }
                }
            }
            Fill::Pattern(p) => self.fill_pattern(p, sk, rule, object),
            Fill::Procedural(p) => self.fill_procedural_tile(p, sk, rule),
            Fill::Mesh(m) => self.fill_mesh(m, sk, rule, tiny_skia::BlendMode::SourceOver),
            Fill::Texture(t) => {
                let b = bbox(page_path);
                let t2 = t.clone();
                let key = format!("{t:?}");
                // The end colours once (a CMYK colour's conversion is not
                // cheap), then blended per pixel as `lerp_color` does.
                let ca = t.color_a.to_rgb8();
                let cb = t.color_b.to_rgb8();
                self.fill_procedural(page_path, sk, rule, &key, move |u, v| {
                    let x = u * b.width() / t2.scale.max(0.1);
                    let y = v * b.height() / t2.scale.max(0.1);
                    let n = texture_value(t2.kind, x, y, t2.seed) as f32;
                    let l = |x: u8, y: u8| {
                        (x as f32 + (y as f32 - x as f32) * n)
                            .round()
                            .clamp(0.0, 255.0) as u8
                    };
                    (
                        Color::rgb8(l(ca[0], cb[0]), l(ca[1], cb[1]), l(ca[2], cb[2])),
                        1.0,
                    )
                });
            }
        }
    }

    /// Mesh fill (nodes in page space): each cell is the Coons patch
    /// bounded by its four grid lines (cubic curves where the nodes carry
    /// handles), cut into small triangles whose colour and opacity are
    /// mixed pixel by pixel from the cell's four nodes (bilinear), or
    /// with Smooth colour from the sixteen around it (Catmull-Rom across
    /// the grid), so they flow through the nodes without creases. The
    /// picture is worked out in the screen's scale and turn without its
    /// pan and kept (`procedural_cache`), so panning, a selection change
    /// or another object dragged do not compute it again; it is drawn
    /// through the path with bilinear sampling.
    fn fill_mesh(
        &mut self,
        m: &tracedraw_core::Mesh,
        sk: &SkPath,
        rule: FillRule,
        blend: tiny_skia::BlendMode,
    ) {
        let [a, b, c, d, tx, ty] = self.screen.as_coeffs();
        let lm = m.transformed(Affine::new([a, b, c, d, 0.0, 0.0]));
        let Some(bounds) = mesh_bounds(&lm) else {
            return;
        };
        if !(bounds.width().is_finite() && bounds.height().is_finite()) {
            return;
        }
        // A mesh far larger than the canvas is worked out for the part on
        // it only, and not kept.
        let (cw, ch) = (self.pixmap.width() as f64, self.pixmap.height() as f64);
        let huge = bounds.width() * bounds.height() > MESH_RASTER_PIXELS as f64;
        let area = if huge {
            let visible = bounds.intersect(Rect::new(-tx, -ty, cw - tx, ch - ty));
            if visible.is_zero_area() {
                return;
            }
            visible
        } else {
            bounds
        };
        // On screen a large picture is worked out on a coarser grid (one
        // picture pixel for `step` screen pixels).
        let step = if self.flags.draft {
            procedural_step(area.width().ceil() as u32, area.height().ceil() as u32)
        } else {
            1
        };
        // The picture starts two of its pixels before the mesh's corner,
        // wherever that falls, so a moved mesh is the same picture (the
        // pattern's transform carries the fraction of a pixel); the
        // outermost pixels stay clear (the edge is grown by one pixel
        // only, and the ring is cleared besides), so the path shows
        // nothing outside the grid, at every step.
        let pad = 2.0 * step as f64;
        let x0 = area.x0 - pad;
        let y0 = area.y0 - pad;
        let w = ((area.width() + 2.0 * pad) / step as f64).ceil().max(1.0) as u32 + 1;
        let h = ((area.height() + 2.0 * pad) / step as f64).ceil().max(1.0) as u32 + 1;
        let key = if huge {
            None
        } else {
            use std::hash::{Hash, Hasher};
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            let q = |p: Point| {
                (
                    ((p.x - x0) * 64.0).round() as i64,
                    ((p.y - y0) * 64.0).round() as i64,
                )
            };
            for n in &lm.nodes {
                q(n.pos).hash(&mut hs);
                n.color.to_rgb8().hash(&mut hs);
                n.alpha.to_bits().hash(&mut hs);
                if let Some(hd) = n.handles {
                    for p in hd {
                        q(p).hash(&mut hs);
                    }
                }
            }
            (lm.rows, lm.cols, lm.smooth).hash(&mut hs);
            Some(format!("mesh|{:x}|{w},{h},{step}", hs.finish()))
        };
        let compute = || {
            let mut pm = Pixmap::new(w, h)?;
            let to_raster = Affine::scale(1.0 / step as f64) * Affine::translate((-x0, -y0));
            rasterise_mesh(&lm, to_raster, &mut pm);
            Some(pm)
        };
        let Some(pm) = (match key {
            Some(key) => procedural_cached(&key, compute),
            None => compute().map(std::sync::Arc::new),
        }) else {
            return;
        };
        let mut paint = Paint::default();
        paint.anti_alias = self.flags.anti_alias;
        paint.blend_mode = blend;
        paint.shader = tiny_skia::Pattern::new(
            (*pm).as_ref(),
            SpreadMode::Pad,
            tiny_skia::FilterQuality::Bilinear,
            1.0,
            Transform::from_translate((tx + x0) as f32, (ty + y0) as f32)
                .pre_scale(step as f32, step as f32),
        );
        self.pixmap
            .fill_path(sk, &paint, rule, Transform::identity(), None);
    }

    /// Fill with a per-pixel function over the path's bounding box
    /// (u, v in 0..1, v up). Only the part on the canvas is worked out;
    /// large areas are worked out on a coarser grid (the functions are
    /// smooth) and on several threads, and the last results are kept, so
    /// a drawing redrawn with the object in the same place (another object
    /// dragged, the selection changed) does not compute it again.
    /// Shaped fountain: the colour follows the distance from the edge of
    /// `mask` (screen space) towards its inside, 0 on the edge and 1 at
    /// the deepest point; `area` is painted with it. The distances are
    /// worked out on a grid of at most `SHAPED_GRID` pixels across the
    /// whole mask, so zooming and panning keep the colours in place.
    fn fill_shaped(
        &mut self,
        f: &tracedraw_core::Fountain,
        mask: &SkPath,
        area: &SkPath,
        rule: FillRule,
    ) {
        let b = mask.bounds();
        let (bw, bh) = (b.width(), b.height());
        if !(bw > 0.0 && bh > 0.0 && bw.is_finite() && bh.is_finite()) {
            return;
        }
        let scale = (SHAPED_GRID / bw.max(bh)).min(1.0);
        let gw = (bw * scale).ceil() as u32 + 2;
        let gh = (bh * scale).ceil() as u32 + 2;
        // Grid pixel (1, 1) is the mask's top-left corner.
        let to_grid = Transform::from_translate(1.0, 1.0)
            .pre_scale(scale, scale)
            .pre_translate(-b.left(), -b.top());
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for p in mask.points() {
                (((p.x - b.left()) * scale * 64.0).round() as i64).hash(&mut h);
                (((p.y - b.top()) * scale * 64.0).round() as i64).hash(&mut h);
            }
            mask.verbs().len().hash(&mut h);
            matches!(rule, FillRule::EvenOdd).hash(&mut h);
            format!("shaped|{f:?}|{:x}|{gw}x{gh}", h.finish())
        };
        let pm = match procedural_cache_get(&key) {
            Some(pm) => pm,
            None => {
                let Some(mut m) = Pixmap::new(gw, gh) else {
                    return;
                };
                let mut paint = Paint::default();
                paint.set_color_rgba8(0, 0, 0, 255);
                paint.anti_alias = false;
                m.fill_path(mask, &paint, rule, to_grid, None);
                let inside: Vec<bool> = m.pixels().iter().map(|p| p.alpha() >= 128).collect();
                let dist = edge_distance(&inside, gw as usize, gh as usize);
                let deepest = dist.iter().cloned().fold(0.0f64, f64::max).max(1e-9);
                let base = f.blended_stops();
                let Some(mut pm) = Pixmap::new(gw, gh) else {
                    return;
                };
                for (px, d) in pm.pixels_mut().iter_mut().zip(&dist) {
                    let s = f.sample_blended(&base, d / deepest);
                    let [r, g, bb] = s.color.to_rgb8();
                    let a = (s.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                    *px = tiny_skia::ColorU8::from_rgba(r, g, bb, a).premultiply();
                }
                let pm = std::sync::Arc::new(pm);
                procedural_cache_put(key, pm.clone());
                pm
            }
        };
        let Some(to_screen) = to_grid.invert() else {
            return;
        };
        let mut paint = Paint::default();
        paint.anti_alias = self.flags.anti_alias;
        paint.shader = tiny_skia::Pattern::new(
            (*pm).as_ref(),
            SpreadMode::Pad,
            tiny_skia::FilterQuality::Bilinear,
            1.0,
            to_screen,
        );
        self.pixmap
            .fill_path(area, &paint, rule, Transform::identity(), None);
    }

    fn fill_procedural(
        &mut self,
        page_path: &BezPath,
        sk: &SkPath,
        rule: FillRule,
        key: &str,
        f: impl Fn(f64, f64) -> (Color, f32) + Sync,
    ) {
        use tracedraw_core::geometry::Shape as _;
        let b = bbox(page_path);
        let sb = (self.screen * b.to_path(0.01)).bounding_box();
        if !(sb.width() > 0.0 && sb.height() > 0.0) {
            return;
        }
        // Zoomed in, the object may be far larger than the window; a pixel
        // of margin keeps the edges smooth.
        let vis = sb.intersect(tracedraw_core::Rect::new(
            -1.0,
            -1.0,
            self.pixmap.width() as f64 + 1.0,
            self.pixmap.height() as f64 + 1.0,
        ));
        if !(vis.width() > 0.0 && vis.height() > 0.0) {
            return;
        }
        let (x0, y0) = (vis.x0.floor(), vis.y0.floor());
        let full_w = ((vis.x1.ceil() - x0) as u32).clamp(1, 8192);
        let full_h = ((vis.y1.ceil() - y0) as u32).clamp(1, 8192);
        let step = if self.flags.draft {
            procedural_step(full_w, full_h)
        } else {
            1
        };
        let w = full_w.div_ceil(step).max(1);
        let h = full_h.div_ceil(step).max(1);
        let cache_key = format!(
            "{key}|{:.2},{:.2},{:.2},{:.2}|{x0},{y0},{w},{h},{step}",
            sb.x0, sb.y0, sb.x1, sb.y1
        );
        let pm = match procedural_cache_get(&cache_key) {
            Some(pm) => pm,
            None => {
                let Some(mut pm) = Pixmap::new(w, h) else {
                    return;
                };
                let (bw, bh) = (sb.width(), sb.height());
                let st = step as f64;
                let row = |y: u32, out: &mut [tiny_skia::PremultipliedColorU8]| {
                    for (x, px) in out.iter_mut().enumerate() {
                        let u = (x0 + (x as f64 + 0.5) * st - sb.x0) / bw;
                        let v = 1.0 - (y0 + (y as f64 + 0.5) * st - sb.y0) / bh;
                        let (c, o) = f(u, v);
                        let [r, g, bb] = c.to_rgb8();
                        let a = (o.clamp(0.0, 1.0) * 255.0).round() as u8;
                        *px = tiny_skia::ColorU8::from_rgba(r, g, bb, a).premultiply();
                    }
                };
                for_rows(pm.pixels_mut(), w as usize, &row);
                let pm = std::sync::Arc::new(pm);
                procedural_cache_put(cache_key, pm.clone());
                pm
            }
        };
        let mut paint = Paint::default();
        paint.anti_alias = self.flags.anti_alias;
        paint.shader = tiny_skia::Pattern::new(
            (*pm).as_ref(),
            SpreadMode::Pad,
            tiny_skia::FilterQuality::Bilinear,
            1.0,
            Transform::from_translate(x0 as f32, y0 as f32).pre_scale(step as f32, step as f32),
        );
        self.pixmap
            .fill_path(sk, &paint, rule, Transform::identity(), None);
    }

    /// A pattern fill: the block of tiles that repeats (one tile, or the
    /// mirrored and shifted tiles of the placement) as one pixmap, drawn
    /// once with a repeating shader whose transform carries the whole
    /// placement (tile size, offset, rotation, skew, the row or column
    /// lean) and, with Transform with object, the object's transform
    /// `object`. Tiles anchor to the page origin otherwise.
    fn fill_pattern(
        &mut self,
        p: &tracedraw_core::Pattern,
        sk: &SkPath,
        rule: FillRule,
        object: Affine,
    ) {
        let placement = p.placement().sanitized();
        let tile = p.tile_size();
        if !(tile.width.is_finite() && tile.height.is_finite())
            || tile.width <= 0.0
            || tile.height <= 0.0
        {
            return;
        }
        let to_page = if placement.transform_with_object {
            object * placement.place()
        } else {
            placement.place()
        };
        let unit_to_screen = self.screen * to_page * placement.cell(tile);
        let Some((pm, px_to_unit)) = self.pattern_cell(p, &placement, tile, unit_to_screen) else {
            return;
        };
        let m = (unit_to_screen * px_to_unit).as_coeffs();
        if !m.iter().all(|v| v.is_finite()) {
            return;
        }
        let t = Transform::from_row(
            m[0] as f32,
            m[1] as f32,
            m[2] as f32,
            m[3] as f32,
            m[4] as f32,
            m[5] as f32,
        );
        // A two-colour tile keeps its hard edges when its pixels line up
        // with the screen's; turned or skewed, it is smoothed like the
        // other kinds.
        let axis_aligned = m[1].abs() < 1e-9 && m[2].abs() < 1e-9;
        let quality = if matches!(p, tracedraw_core::Pattern::TwoColor { .. }) && axis_aligned {
            tiny_skia::FilterQuality::Nearest
        } else {
            tiny_skia::FilterQuality::Bilinear
        };
        let mut paint = Paint::default();
        paint.anti_alias = self.flags.anti_alias;
        paint.shader =
            tiny_skia::Pattern::new(pm.as_ref().as_ref(), SpreadMode::Repeat, quality, 1.0, t);
        self.pixmap
            .fill_path(sk, &paint, rule, Transform::identity(), None);
    }

    /// The pixels of the block that repeats for `p` placed by
    /// `placement` (tiles of `tile` mm), at the resolution
    /// `unit_to_screen` (the block's unit square to screen pixels) asks
    /// for, with the transform from those pixels to the unit square (Y
    /// up). A bitmap pattern without mirrored or shifted tiles is its own
    /// picture, decoded once and kept.
    fn pattern_cell(
        &self,
        p: &tracedraw_core::Pattern,
        placement: &tracedraw_core::style::PatternPlacement,
        tile: tracedraw_core::geometry::Size,
        unit_to_screen: Affine,
    ) -> Option<(std::sync::Arc<Pixmap>, Affine)> {
        use std::sync::Arc;
        // Pixels along the block's two edges on screen.
        let o = unit_to_screen * Point::new(0.0, 0.0);
        let ex = ((unit_to_screen * Point::new(1.0, 0.0)) - o).hypot();
        let ey = ((unit_to_screen * Point::new(0.0, 1.0)) - o).hypot();
        if !(ex.is_finite() && ey.is_finite()) {
            return None;
        }
        let size = |max: u32| -> (u32, u32) {
            (
                (ex.round() as u32).clamp(1, max),
                (ey.round() as u32).clamp(1, max),
            )
        };
        let px_to_unit = |tw: u32, th: u32| {
            Affine::new([
                1.0 / f64::from(tw),
                0.0,
                0.0,
                -1.0 / f64::from(th),
                0.0,
                1.0,
            ])
        };
        let cell = placement.cell(tile);
        let cell_inv = cell.inverse();
        match p {
            tracedraw_core::Pattern::TwoColor {
                tile: kind,
                front,
                back,
                ..
            } => {
                let (tw, th) = size(1024);
                let key =
                    format!("pat2|{kind:?}|{front:?}|{back:?}|{placement:?}|{tile:?}|{tw}x{th}");
                if let Some(pm) = procedural_cache_get(&key) {
                    return Some((pm, px_to_unit(tw, th)));
                }
                let mut pm = Pixmap::new(tw, th)?;
                let [fr, fg, fb] = front.to_rgb8();
                let [br, bg, bb] = back.to_rgb8();
                let fc = tiny_skia::ColorU8::from_rgba(fr, fg, fb, 255).premultiply();
                let bc = tiny_skia::ColorU8::from_rgba(br, bg, bb, 255).premultiply();
                let data = pm.pixels_mut();
                for y in 0..th {
                    for x in 0..tw {
                        let unit = Point::new(
                            (f64::from(x) + 0.5) / f64::from(tw),
                            1.0 - (f64::from(y) + 0.5) / f64::from(th),
                        );
                        let (u, v) = placement.tile_uv(cell * unit, tile);
                        if let Some(px) = data.get_mut((y * tw + x) as usize) {
                            *px = if kind.front(u, v) { fc } else { bc };
                        }
                    }
                }
                let pm = Arc::new(pm);
                procedural_cache_put(key, pm.clone());
                Some((pm, px_to_unit(tw, th)))
            }
            tracedraw_core::Pattern::Bitmap { png, .. } => {
                let img = image_cache::decode(png)?;
                let (iw, ih) = (img.width(), img.height());
                // The picture's pixels to one tile's millimetres.
                let content = Affine::new([
                    tile.width / f64::from(iw.max(1)),
                    0.0,
                    0.0,
                    -tile.height / f64::from(ih.max(1)),
                    0.0,
                    tile.height,
                ]);
                if !placement.needs_cell() {
                    // One tile is the block: the picture itself repeats.
                    return Some((img, cell_inv * content));
                }
                let (cols, rows) = placement.cell_tiles();
                // No sharper than the picture, no larger than the screen asks.
                let tw = ((ex.round() as u32).min(iw.saturating_mul(cols))).clamp(1, 4096);
                let th = ((ey.round() as u32).min(ih.saturating_mul(rows))).clamp(1, 4096);
                let key = format!(
                    "patb|{:x}|{placement:?}|{tile:?}|{tw}x{th}",
                    image_cache::fingerprint(png)
                );
                if let Some(pm) = procedural_cache_get(&key) {
                    return Some((pm, px_to_unit(tw, th)));
                }
                let mut pm = Pixmap::new(tw, th)?;
                let unit_to_px = px_to_unit(tw, th).inverse();
                let paint = tiny_skia::PixmapPaint {
                    quality: tiny_skia::FilterQuality::Bilinear,
                    ..Default::default()
                };
                let picture = Rect::new(0.0, 0.0, f64::from(iw), f64::from(ih));
                for copy in placement.tile_copies(tile) {
                    let t = unit_to_px * cell_inv * copy * content;
                    if !copy_shows(t, picture, tw, th) {
                        continue;
                    }
                    let c = t.as_coeffs();
                    pm.draw_pixmap(
                        0,
                        0,
                        img.as_ref().as_ref(),
                        &paint,
                        Transform::from_row(
                            c[0] as f32,
                            c[1] as f32,
                            c[2] as f32,
                            c[3] as f32,
                            c[4] as f32,
                            c[5] as f32,
                        ),
                        None,
                    );
                }
                let pm = Arc::new(pm);
                procedural_cache_put(key, pm.clone());
                Some((pm, px_to_unit(tw, th)))
            }
            tracedraw_core::Pattern::Vector {
                shapes, tile: own, ..
            } => {
                let (tw, th) = size(2048);
                // The tile's content stretched to the tile size placed.
                let content = Affine::scale_non_uniform(
                    tile.width / own.width.max(1e-9),
                    tile.height / own.height.max(1e-9),
                );
                let unit_to_px = px_to_unit(tw, th).inverse();
                let copies: Vec<Affine> = placement
                    .tile_copies(tile)
                    .into_iter()
                    .map(|copy| unit_to_px * cell_inv * copy * content)
                    .collect();
                let pm = self.render_vector_cell(
                    shapes,
                    Rect::new(0.0, 0.0, own.width, own.height),
                    &copies,
                    tw,
                    th,
                )?;
                Some((Arc::new(pm), px_to_unit(tw, th)))
            }
        }
    }

    /// A procedural fill: its tile (core builds the paths once per set of
    /// numbers) painted once at the current zoom, kept with the other
    /// procedural pixmaps, and repeated from the page origin, so the
    /// tiles stay in place when the object moves and match the exports.
    /// No per-pixel work: the tile is drawn as paths and the pattern
    /// shader repeats it.
    fn fill_procedural_tile(
        &mut self,
        p: &tracedraw_core::Procedural,
        sk: &SkPath,
        rule: FillRule,
    ) {
        let tile = tracedraw_core::procedural::procedural_tile(p);
        let (tw_mm, th_mm) = (tile.size.width, tile.size.height);
        if !(tw_mm.is_finite() && th_mm.is_finite()) || tw_mm <= 0.0 || th_mm <= 0.0 {
            return;
        }
        let tw = ((tw_mm * self.zoom).round() as u32).clamp(1, 2048);
        let th = ((th_mm * self.zoom).round() as u32).clamp(1, 2048);
        let key = format!("procedural|{p:?}|{tw}x{th}|{}", self.flags.anti_alias);
        let pm = match procedural_cache_get(&key) {
            Some(pm) => pm,
            None => {
                let Some(mut pm) = Pixmap::new(tw, th) else {
                    return;
                };
                if let Some(bg) = p.background {
                    pm.fill(sk_color(bg));
                }
                // Tile space (mm, Y up, origin bottom-left) to tile pixels.
                let to_px = Affine::new([
                    tw as f64 / tw_mm,
                    0.0,
                    0.0,
                    -(th as f64) / th_mm,
                    0.0,
                    th as f64,
                ]);
                let mut paint = Paint::default();
                paint.anti_alias = self.flags.anti_alias;
                paint.set_color(sk_color(p.foreground));
                for path in &tile.paths {
                    if let Some(sp) = to_sk_path(&(to_px * path)) {
                        pm.fill_path(&sp, &paint, FillRule::Winding, Transform::identity(), None);
                    }
                }
                let pm = std::sync::Arc::new(pm);
                procedural_cache_put(key, pm.clone());
                pm
            }
        };
        // The tiles start at the page origin.
        let o = self.screen * Point::new(0.0, 0.0);
        let shift = Transform::from_translate(
            o.x.rem_euclid(tw as f64) as f32,
            o.y.rem_euclid(th as f64) as f32,
        );
        let mut paint = Paint::default();
        paint.anti_alias = self.flags.anti_alias;
        paint.shader = tiny_skia::Pattern::new(
            pm.as_ref().as_ref(),
            SpreadMode::Repeat,
            tiny_skia::FilterQuality::Bilinear,
            1.0,
            shift,
        );
        self.pixmap
            .fill_path(sk, &paint, rule, Transform::identity(), None);
    }

    /// Rasterise a vector pattern's block of tiles: the tile's shapes
    /// drawn once per `copies` entry (tile mm, Y up, to block pixels, Y
    /// down) into a `tw` by `th` pixmap. The shapes go through the
    /// ordinary shape drawing, so nested groups, fills, outlines and text
    /// in the tile render exactly as on the page. The pixmap is a whole
    /// number of pixels, so the repeat is seamless; the tile's millimetre
    /// size is stretched to fit by at most half a pixel on each axis.
    fn render_vector_cell(
        &self,
        shapes: &[Shape],
        tile: Rect,
        copies: &[Affine],
        tw: u32,
        th: u32,
    ) -> Option<Pixmap> {
        let mut pm = Pixmap::new(tw, th)?;
        for screen in copies {
            if !copy_shows(*screen, tile, tw, th) {
                continue;
            }
            let c = screen.as_coeffs();
            // Pixels per millimetre of the content, for outline widths.
            let zoom = (c[0] * c[3] - c[1] * c[2]).abs().sqrt();
            if !(zoom.is_finite() && zoom > 0.0) {
                continue;
            }
            let mut sub = Renderer {
                pixmap: &mut pm,
                screen: *screen,
                zoom,
                flags: Flags {
                    wireframe: false,
                    ..self.flags
                },
                symbols: self.symbols,
                preview: None,
                origin: (0, 0),
                cancel: None,
            };
            for s in shapes {
                sub.draw_shape(s, Affine::IDENTITY);
            }
        }
        Some(pm)
    }

    fn stroke_paint(&self, stroke: &Stroke, transform: Affine) -> (Paint<'static>, SkStroke) {
        let mut paint = Paint::default();
        paint.anti_alias = self.flags.anti_alias;
        paint.set_color(sk_color(stroke.color));
        let hair = stroke.width <= Stroke::HAIRLINE + 1e-9;
        // "Scale with object" uses the object's own scale factor.
        let object_scale = if stroke.scale_with_object {
            let c = transform.as_coeffs();
            ((c[0] * c[3] - c[1] * c[2]).abs()).sqrt().max(1e-6)
        } else {
            1.0
        };
        let true_px = (stroke.width * object_scale * self.zoom) as f32;
        // An outline thinner than a pixel is drawn one pixel wide (a
        // hairline) rather than faded until it disappears, as previews of
        // such files show it.
        let width_px = if hair || true_px < 1.0 { 0.0 } else { true_px };
        // A dashed hairline keeps its dashes, a pixel per unit of the
        // pattern at least, as a thin outline's do. It was drawn solid, so
        // dashed seams of a `.cdr` drawing opened as plain lines (#2463).
        let dash = if stroke.dash.is_empty() {
            None
        } else {
            let unit = if hair {
                (Stroke::HAIRLINE * object_scale * self.zoom) as f32
            } else {
                true_px
            }
            .max(1.0);
            let arr: Vec<f32> = stroke
                .dash
                .iter()
                .map(|d| (*d as f32 * unit).max(0.01))
                .collect();
            let arr = if arr.len() % 2 == 1 {
                [arr.clone(), arr].concat()
            } else {
                arr
            };
            StrokeDash::new(arr, 0.0)
        };
        let sk = SkStroke {
            width: width_px,
            miter_limit: stroke.miter_ratio() as f32,
            line_cap: match stroke.cap {
                LCap::Butt => LineCap::Butt,
                LCap::Round => LineCap::Round,
                LCap::Square => LineCap::Square,
            },
            line_join: match stroke.join {
                LJoin::Miter => LineJoin::Miter,
                LJoin::Round => LineJoin::Round,
                LJoin::Bevel => LineJoin::Bevel,
            },
            dash,
        };
        (paint, sk)
    }
}

/// The silhouette of a picture on `layer`: its alpha as the coverage, in
/// white, placed as the picture is (`transform`, its `rect` of
/// `width_px` by `height_px` pixels) and moved as the shadow `sh`
/// measured on box `b` asks, so a transparent background casts no shadow
/// (#2027).
#[allow(clippy::too_many_arguments)]
fn bitmap_silhouette(
    layer: &mut Pixmap,
    sh: &tracedraw_core::Shadow,
    b: Rect,
    screen: Affine,
    transform: Affine,
    rect: Rect,
    (width_px, height_px): (u32, u32),
    img: &Pixmap,
) {
    let (w0, h0) = (width_px.max(1) as f64, height_px.max(1) as f64);
    let img_to_page = transform
        * Affine::new([
            rect.width() / w0,
            0.0,
            0.0,
            -rect.height() / h0,
            rect.x0,
            rect.y1,
        ]);
    let m = (screen * sh.silhouette_transform(b) * img_to_page).as_coeffs();
    let t = Transform::from_row(
        m[0] as f32,
        m[1] as f32,
        m[2] as f32,
        m[3] as f32,
        m[4] as f32,
        m[5] as f32,
    );
    let pp = tiny_skia::PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..Default::default()
    };
    layer.draw_pixmap(0, 0, Pixmap::as_ref(img), &pp, t, None);
    // The silhouette in white, the picture's alpha as its coverage.
    for px in layer.pixels_mut().iter_mut() {
        let a = px.alpha();
        *px = tiny_skia::ColorU8::from_rgba(255, 255, 255, a).premultiply();
    }
}

/// Paint the drop shadow `sh` of an object with page path `page_path` into
/// the empty `layer` (pixels = `screen` times page millimetres, `zoom`
/// pixels per millimetre), with its opacity and fade but not its merge
/// mode. The silhouette is what the object shows ([`paint_shown`]): its
/// fill by the rule the object is filled by, so a hole of the object is a
/// hole of its shadow (#2216), and its outline as drawn, so an object
/// with no fill casts the shadow of its outline alone and a wide or
/// variable outline casts its own shape. It was the filled path always:
/// a ring drawn with a variable outline cast a whole disc. False when
/// nothing was painted.
fn shadow_layer(
    layer: &mut Pixmap,
    sh: &tracedraw_core::Shadow,
    shown: Shown,
    screen: Affine,
    zoom: f64,
    anti_alias: bool,
) -> bool {
    let b = shown.path.bounding_box();
    if to_sk_path(shown.path).is_none() {
        return false;
    }
    paint_shown(
        layer,
        &shown,
        screen * sh.silhouette_transform(b),
        None,
        anti_alias,
    );
    shade_shadow(layer, sh, b, screen, zoom);
    true
}

/// An inner shadow's settings (see `Effect::InnerShadow`).
#[derive(Debug, Clone, Copy)]
struct InnerShadow {
    offset: tracedraw_core::geometry::Vec2,
    color: tracedraw_core::Color,
    opacity: f64,
    blur: f64,
    merge: tracedraw_core::live::MergeMode,
}

/// An object as its shadow sees it: `shape`, its outline `path` on the
/// page (its parent's space), `object` its transform there and `rule`
/// the rule its fill is drawn by.
struct Shown<'a> {
    shape: &'a Shape,
    path: &'a BezPath,
    object: Affine,
    rule: FillRule,
}

/// Paint in white, through `to_screen` (page to layer), what `shown`
/// shows: its fill unless it has none, and its outline as drawn (a band
/// for a variable, calligraphic or placed outline, the stroked line
/// otherwise, dashes and arrowheads with it; a hairline one pixel wide).
/// An object with neither fill nor outline is painted filled, as before.
fn paint_shown(
    layer: &mut Pixmap,
    shown: &Shown,
    to_screen: Affine,
    clip: Option<&tiny_skia::Mask>,
    anti_alias: bool,
) {
    let Some(sk) = to_sk_path(&(to_screen * shown.path.clone())) else {
        return;
    };
    let mut paint = Paint::default();
    paint.anti_alias = anti_alias;
    paint.set_color_rgba8(255, 255, 255, 255);
    let stroke = shown.shape.stroke.as_ref();
    let filled = !matches!(shown.shape.fill, Fill::None);
    if filled || stroke.is_none() {
        layer.fill_path(&sk, &paint, shown.rule, Transform::identity(), clip);
    }
    let Some(stroke) = stroke else {
        return;
    };
    let object_scale = if stroke.scale_with_object {
        let c = shown.object.as_coeffs();
        ((c[0] * c[3] - c[1] * c[2]).abs()).sqrt().max(1e-6)
    } else {
        1.0
    };
    if let Some(band) = tracedraw_core::shaping::outline_band(shown.path, stroke, object_scale) {
        if let Some(bp) = to_sk_path(&(to_screen * band)) {
            layer.fill_path(&bp, &paint, FillRule::Winding, Transform::identity(), clip);
        }
    } else {
        let c = to_screen.as_coeffs();
        let px_per_mm = ((c[0] * c[3] - c[1] * c[2]).abs()).sqrt();
        let hair = stroke.width <= Stroke::HAIRLINE + 1e-9 || !stroke.width.is_finite();
        let true_px = if hair {
            0.0
        } else {
            (stroke.width * object_scale * px_per_mm) as f32
        };
        let dash = if stroke.dash.is_empty() {
            None
        } else {
            // A dashed hairline keeps its dashes, as on the page (#2463).
            let unit = if hair {
                (Stroke::HAIRLINE * object_scale * px_per_mm) as f32
            } else {
                true_px
            }
            .max(1.0);
            let mut arr: Vec<f32> = stroke
                .dash
                .iter()
                .map(|d| (*d as f32 * unit).max(0.01))
                .collect();
            if arr.len() % 2 == 1 {
                arr = [arr.clone(), arr].concat();
            }
            StrokeDash::new(arr, 0.0)
        };
        let line = SkStroke {
            width: true_px.max(1.0),
            miter_limit: stroke.miter_ratio() as f32,
            line_cap: match stroke.cap {
                LCap::Butt => LineCap::Butt,
                LCap::Round => LineCap::Round,
                LCap::Square => LineCap::Square,
            },
            line_join: match stroke.join {
                LJoin::Miter => LineJoin::Miter,
                LJoin::Round => LineJoin::Round,
                LJoin::Bevel => LineJoin::Bevel,
            },
            dash,
        };
        layer.stroke_path(&sk, &paint, &line, Transform::identity(), clip);
    }
    for head in tracedraw_core::style::arrowhead_paths(shown.path, stroke) {
        if let Some(hp) = to_sk_path(&(to_screen * head)) {
            layer.fill_path(&hp, &paint, FillRule::Winding, Transform::identity(), clip);
        }
    }
}

/// [`shadow_layer`] for a group or a clip frame drawn inside `parent`:
/// the silhouette is what the object shows, every object of a group and
/// a clip frame's contents cut to its frame (#873), not the outline of
/// the whole. The shadow is measured on the object's box, as the Drop
/// Shadow tool measures it.
#[allow(clippy::too_many_arguments)]
fn group_shadow_layer(
    layer: &mut Pixmap,
    sh: &tracedraw_core::Shadow,
    shape: &Shape,
    parent: Affine,
    b: Rect,
    screen: Affine,
    zoom: f64,
    anti_alias: bool,
) {
    let to_screen = screen * sh.silhouette_transform(b) * parent;
    fill_silhouette(layer, shape, to_screen, None, anti_alias);
    shade_shadow(layer, sh, b, screen, zoom);
}

/// The box the shadow of a group or a clip frame drawn inside `parent`
/// is measured on; `None` when it is not finite.
fn group_shadow_box(shape: &Shape, parent: Affine) -> Option<Rect> {
    cast_boxes(shape, parent).map(|(b, _)| b)
}

/// The effects of an object draw objects of their own: its drop shadow
/// is cast by all of them together (a picture keeps the shadow of what
/// its pixels show).
fn casts_whole(ev: &tracedraw_core::live::Evaluated) -> bool {
    !(ev.below.is_empty() && ev.above.is_empty())
        && !matches!(ev.main.kind, ShapeKind::Bitmap { .. })
}

/// The box the drop shadow of `shape`, drawn inside `parent`, is measured
/// on, and the box all it casts the shadow from lies in. The first is the
/// box of its outline as drawn, an envelope or another warp taken in (of
/// its plain outline when its effects draw it among their own objects,
/// as a body turned in space does); the second reaches round what its
/// effects add. `None` when they are not finite.
fn cast_boxes(shape: &Shape, parent: Affine) -> Option<(Rect, Rect)> {
    let plain = || (parent * shape.page_path()).bounding_box();
    let (b, extent) = if shape.has_live_effects() {
        let drawn = (!shape.effects.is_empty())
            .then(|| tracedraw_core::live::evaluate(shape).main.page_path())
            .filter(|p| !p.elements().is_empty());
        let b = drawn.map_or_else(plain, |p| (parent * p).bounding_box());
        let all = parent.transform_rect_bbox(shape.visual_bounds());
        (b, if all.is_finite() { b.union(all) } else { b })
    } else {
        let b = plain();
        (b, b)
    };
    // What its outline covers casts the shadow too.
    let extent = match shape.kind {
        ShapeKind::Group { .. } | ShapeKind::ClipFrame { .. } => extent,
        _ if shape.stroke.is_none() => extent,
        _ => Renderer::reach(shape, parent).map_or(extent, |r| r.union(extent)),
    };
    (b.width().is_finite() && b.height().is_finite()).then_some((b, extent))
}

/// The pixels of a `w` x `h` target that the shadow `sh` of an object
/// measured on box `b` can reach through `screen` (`zoom` pixels per
/// millimetre): the moved or sheared box, widened by the three blur
/// passes and a pixel of smoothing, as left, top, width and height.
/// Pixels beyond stay clear on a layer of any size, so the shadow drawn
/// on a layer of only these is the same as on one the size of the target
/// (a blur clamps at the target's own edges, which this keeps). `None`
/// when it covers no pixel of the target.
#[cfg(test)]
fn shadow_pixels(
    sh: &tracedraw_core::Shadow,
    b: Rect,
    screen: Affine,
    zoom: f64,
    w: u32,
    h: u32,
) -> Option<(i32, i32, u32, u32)> {
    shadow_pixels_over(sh, b, b, screen, zoom, w, h)
}

/// [`shadow_pixels`] for a silhouette that covers `extent`, which the
/// shadow measured on `b` moves or shears.
fn shadow_pixels_over(
    sh: &tracedraw_core::Shadow,
    b: Rect,
    extent: Rect,
    screen: Affine,
    zoom: f64,
    w: u32,
    h: u32,
) -> Option<(i32, i32, u32, u32)> {
    let (w, h) = (f64::from(w), f64::from(h));
    let flat = sh.silhouette_transform(b).transform_rect_bbox(extent);
    let corners = [
        Point::new(flat.x0, flat.y0),
        Point::new(flat.x1, flat.y0),
        Point::new(flat.x0, flat.y1),
        Point::new(flat.x1, flat.y1),
    ]
    .map(|p| screen * p);
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in corners {
        (x0, x1) = (x0.min(c.x), x1.max(c.x));
        (y0, y1) = (y0.min(c.y), y1.max(c.y));
    }
    let blur = if sh.blur.is_finite() { sh.blur } else { 0.0 };
    let radius = (blur * zoom).round().max(0.0);
    // Three box passes reach three radii; Average's one pass of 1.7
    // radii less. Two more pixels for the smoothed edge and rounding.
    let reach = radius * 3.0 + 2.0;
    if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite() && reach.is_finite())
    {
        // Nothing to measure by: the whole target, as before.
        return Some((0, 0, w as u32, h as u32));
    }
    let left = (x0 - reach).floor().clamp(0.0, w);
    let right = (x1 + reach).ceil().clamp(0.0, w);
    let top = (y0 - reach).floor().clamp(0.0, h);
    let bottom = (y1 + reach).ceil().clamp(0.0, h);
    if right <= left || bottom <= top {
        return None;
    }
    Some((
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    ))
}

/// Fill in white what `shape` covers, its parent's space taken to the
/// layer by `to_screen`, inside `clip` when given: the objects of a group
/// each, a clip frame's filled frame and its contents only where its
/// frame is (nested frames cut further), hidden objects not at all.
/// Each is filled by its own fill rule, holes left open (#2216). An
/// object with live effects covers what they draw: its outline as they
/// warp it and the objects they add (#2215).
fn fill_silhouette(
    layer: &mut Pixmap,
    shape: &Shape,
    to_screen: Affine,
    clip: Option<&tiny_skia::Mask>,
    anti_alias: bool,
) {
    silhouette(layer, shape, to_screen, clip, anti_alias, false)
}

/// [`fill_silhouette`]; `text` when `shape` is drawn from a text, whose
/// warped outline fills as text does.
fn silhouette(
    layer: &mut Pixmap,
    shape: &Shape,
    to_screen: Affine,
    clip: Option<&tiny_skia::Mask>,
    anti_alias: bool,
    text: bool,
) {
    if !shape.visible {
        return;
    }
    if !shape.effects.is_empty() {
        let ev = tracedraw_core::live::evaluate(shape);
        let text = text || matches!(shape.kind, ShapeKind::Text { .. });
        for part in ev.below.iter().chain([&ev.main]).chain(&ev.above) {
            let mut part = part.clone();
            part.effects.clear();
            silhouette(layer, &part, to_screen, clip, anti_alias, text);
        }
        return;
    }
    let inner = to_screen * shape.transform;
    match &shape.kind {
        ShapeKind::Group { children } => {
            for c in children {
                silhouette(layer, c, inner, clip, anti_alias, text);
            }
        }
        ShapeKind::ClipFrame { frame, contents } => {
            let Some(sk_frame) = to_sk_path(&(inner * frame.page_path())) else {
                return;
            };
            // A filled frame shows whole: its fill casts the shadow, and
            // the contents, all inside it, add nothing. Painted over the
            // fill they thickened its soft edge, a darker line along a
            // sharp shadow's edge.
            if !matches!(frame.fill, Fill::None) && frame.visible {
                let mut paint = Paint::default();
                paint.anti_alias = anti_alias;
                paint.set_color_rgba8(255, 255, 255, 255);
                layer.fill_path(
                    &sk_frame,
                    &paint,
                    shape_fill_rule(frame, false),
                    Transform::identity(),
                    clip,
                );
                return;
            }
            let mask = match clip {
                Some(m) => {
                    let mut m = m.clone();
                    m.intersect_path(
                        &sk_frame,
                        FillRule::EvenOdd,
                        anti_alias,
                        Transform::identity(),
                    );
                    Some(m)
                }
                None => tiny_skia::Mask::new(layer.width(), layer.height()).map(|mut m| {
                    m.fill_path(
                        &sk_frame,
                        FillRule::EvenOdd,
                        anti_alias,
                        Transform::identity(),
                    );
                    m
                }),
            };
            let Some(mask) = mask else {
                return;
            };
            for c in contents {
                fill_silhouette(layer, c, inner, Some(&mask), anti_alias);
            }
        }
        _ => {
            let path = shape.page_path();
            let shown = Shown {
                shape,
                path: &path,
                object: shape.transform,
                rule: shape_fill_rule(shape, text),
            };
            paint_shown(layer, &shown, to_screen, clip, anti_alias);
        }
    }
}

/// Turn a layer holding a shadow's silhouette in white into the shadow:
/// feathered, faded along a perspective shadow, in its colour and
/// opacity. `b` is the box the shadow is measured on.
fn shade_shadow(
    layer: &mut Pixmap,
    sh: &tracedraw_core::Shadow,
    b: Rect,
    screen: Affine,
    zoom: f64,
) {
    use tracedraw_core::shadow::FeatherDirection;
    let blur = if sh.blur.is_finite() { sh.blur } else { 0.0 };
    let radius = (blur * zoom).round() as i32;
    let feathered = radius > 0;
    if feathered {
        match sh.feather_direction {
            // One even pass about as wide as the three passes together.
            FeatherDirection::Average => {
                accel::box_blur(layer, ((radius as f64) * 1.7).round().min(110.0) as i32, 1)
            }
            _ => accel::box_blur(layer, radius.min(64), 3),
        }
    }
    // The fade of a perspective shadow, in pixels.
    let fade = sh.fade_axis(b).map(|(a, e, f)| {
        let (a, e) = (screen * a, screen * e);
        let d = e - a;
        (a, d, d.hypot2().max(1e-9), f)
    });
    let opacity = sh.opacity.clamp(0.0, 1.0);
    let [cr, cg, cb] = sh.color.to_rgb8();
    let width = layer.width() as usize;
    for (i, px) in layer.pixels_mut().iter_mut().enumerate() {
        let cov = px.alpha();
        if cov == 0 {
            continue;
        }
        let mut a = cov as f64 / 255.0;
        if feathered {
            a = sh.feather_value(a);
        }
        if let Some((pa, d, len2, f)) = fade {
            let x = (i % width) as f64 + 0.5;
            let y = (i / width) as f64 + 0.5;
            let t = ((x - pa.x) * d.x + (y - pa.y) * d.y) / len2;
            a *= 1.0 - f * t.clamp(0.0, 1.0);
        }
        let a = (a * opacity).clamp(0.0, 1.0);
        let c = tiny_skia::ColorU8::from_rgba(cr, cg, cb, (a * 255.0).round() as u8);
        *px = c.premultiply();
    }
}

/// The drop shadow of `shape` alone as an image at `dpi`, with the page
/// rectangle it covers (Break Drop Shadow Apart). The shadow's opacity and
/// fade are in the pixels; its merge mode is not. `None` when the shape
/// has no shadow or it covers nothing.
pub fn render_shadow_image(shape: &Shape, dpi: f64) -> Option<(Rect, Pixmap)> {
    let sh = shape.shadow.as_ref()?;
    let path = shape.page_path();
    if path.elements().is_empty() || !(dpi.is_finite() && dpi > 0.0) {
        return None;
    }
    // Measured on the outline as drawn and reaching round all the
    // object's effects draw, as on the page (#2215, #2217).
    let (b, extent) = cast_boxes(shape, Affine::IDENTITY)?;
    let pad = if sh.blur.is_finite() {
        sh.blur.max(0.0) * 3.0
    } else {
        0.0
    };
    let area = sh
        .silhouette_transform(b)
        .transform_rect_bbox(extent)
        .inflate(pad, pad);
    let zoom = dpi / 25.4;
    let w = (area.width() * zoom).ceil();
    let h = (area.height() * zoom).ceil();
    if !(w >= 1.0 && h >= 1.0 && w * h <= 64.0e6) {
        return None;
    }
    let mut pm = Pixmap::new(w as u32, h as u32)?;
    // Page (y up) to the image (y down), the area's top left at 0, 0.
    let screen = Affine::new([zoom, 0.0, 0.0, -zoom, -area.x0 * zoom, area.y1 * zoom]);
    let drawn = if !shape.effects.is_empty() {
        group_shadow_layer(&mut pm, sh, shape, Affine::IDENTITY, b, screen, zoom, true);
        true
    } else if matches!(
        shape.kind,
        ShapeKind::Group { .. } | ShapeKind::ClipFrame { .. }
    ) {
        match group_shadow_box(shape, Affine::IDENTITY) {
            Some(b) => {
                group_shadow_layer(&mut pm, sh, shape, Affine::IDENTITY, b, screen, zoom, true);
                true
            }
            None => false,
        }
    } else if let Some(img) = match &shape.kind {
        ShapeKind::Bitmap { png, .. } => image_cache::decode_level(png, 0),
        _ => None,
    } {
        // A picture casts the shadow of what its pixels cover, as on the
        // page (#2027): a picture with a transparent background cast the
        // shadow of its whole box in PDF output (d#2596).
        if let ShapeKind::Bitmap {
            rect,
            width_px,
            height_px,
            ..
        } = &shape.kind
        {
            bitmap_silhouette(
                &mut pm,
                sh,
                b,
                screen,
                shape.transform,
                *rect,
                (*width_px, *height_px),
                &img,
            );
            shade_shadow(&mut pm, sh, b, screen, zoom);
        }
        true
    } else {
        let shown = Shown {
            shape,
            path: &path,
            object: shape.transform,
            rule: shape_fill_rule(shape, false),
        };
        shadow_layer(&mut pm, sh, shown, screen, zoom, true)
    };
    if !drawn {
        return None;
    }
    let exact = Rect::new(area.x0, area.y1 - h / zoom, area.x0 + w / zoom, area.y1);
    Some((exact, pm))
}

/// The blend mode a merge mode draws with. Subtract, Texturize and
/// Lightness have no blend of their own and get the nearest;
/// [`composite_layer`] draws them exactly.
pub fn merge_blend(merge: tracedraw_core::live::MergeMode) -> tiny_skia::BlendMode {
    use tiny_skia::BlendMode as B;
    use tracedraw_core::live::MergeMode as M;
    match merge {
        M::Normal | M::Red | M::Green | M::Blue => B::SourceOver,
        M::Add => B::Plus,
        M::Subtract | M::Multiply | M::Texturize => B::Multiply,
        M::Difference => B::Difference,
        M::Divide => B::Screen,
        M::IfLighter => B::Lighten,
        M::IfDarker => B::Darken,
        M::Hue => B::Hue,
        M::Saturation => B::Saturation,
        M::Color => B::Color,
        M::Lightness => B::Luminosity,
        M::Invert => B::Exclusion,
        M::And | M::Or | M::Xor => B::Xor,
        M::Behind => B::DestinationOver,
        M::Screen => B::Screen,
        M::Overlay => B::Overlay,
        M::SoftLight => B::SoftLight,
        M::HardLight => B::HardLight,
        M::ColorDodge => B::ColorDodge,
        M::ColorBurn => B::ColorBurn,
        M::Exclusion => B::Exclusion,
    }
}

/// Draw the premultiplied `layer`, its top left at `at`, over `target`
/// with `merge`: the blend of [`merge_blend`], except Subtract (the
/// backdrop and the source added, less white: `b + s - 1`, so white
/// leaves the backdrop as it is and darker source colours darken it,
/// #2512), Texturize (the backdrop multiplied by the source's lightness)
/// and Lightness (the source's HSL lightness with the backdrop's hue and
/// saturation), which are mixed pixel by pixel here.
pub fn composite_layer(
    target: &mut Pixmap,
    layer: &Pixmap,
    at: (i32, i32),
    merge: tracedraw_core::live::MergeMode,
) {
    use tracedraw_core::live::MergeMode as M;
    let mix: fn([f32; 3], [f32; 3]) -> [f32; 3] = match merge {
        M::Subtract => |cb, cs| {
            [
                (cb[0] + cs[0] - 1.0).max(0.0),
                (cb[1] + cs[1] - 1.0).max(0.0),
                (cb[2] + cs[2] - 1.0).max(0.0),
            ]
        },
        M::Texturize => |cb, cs| {
            let l = 0.2126 * cs[0] + 0.7152 * cs[1] + 0.0722 * cs[2];
            [cb[0] * l, cb[1] * l, cb[2] * l]
        },
        M::Lightness => |cb, cs| {
            let l = (cs[0].max(cs[1]).max(cs[2]) + cs[0].min(cs[1]).min(cs[2])) / 2.0;
            set_lightness(cb, l)
        },
        _ => {
            let pp = tiny_skia::PixmapPaint {
                blend_mode: merge_blend(merge),
                ..Default::default()
            };
            target.draw_pixmap(at.0, at.1, layer.as_ref(), &pp, Transform::identity(), None);
            return;
        }
    };
    let (tw, th) = (target.width() as i32, target.height() as i32);
    let (lw, lh) = (layer.width() as i32, layer.height() as i32);
    let x0 = at.0.max(0);
    let y0 = at.1.max(0);
    let x1 = (at.0 + lw).min(tw);
    let y1 = (at.1 + lh).min(th);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let src = layer.pixels();
    let dst = target.pixels_mut();
    for y in y0..y1 {
        let srow = ((y - at.1) * lw) as usize;
        let drow = (y * tw) as usize;
        for x in x0..x1 {
            let (Some(s), Some(d)) = (
                src.get(srow + (x - at.0) as usize),
                dst.get_mut(drow + x as usize),
            ) else {
                continue;
            };
            let sa = s.alpha();
            if sa == 0 {
                continue;
            }
            let (s, d8) = (s.demultiply(), d.demultiply());
            let (a_s, a_b) = (sa as f32 / 255.0, d8.alpha() as f32 / 255.0);
            let cs = [
                s.red() as f32 / 255.0,
                s.green() as f32 / 255.0,
                s.blue() as f32 / 255.0,
            ];
            let cb = [
                d8.red() as f32 / 255.0,
                d8.green() as f32 / 255.0,
                d8.blue() as f32 / 255.0,
            ];
            let b = mix(cb, cs);
            // The compositing formula: the source where only it is, the
            // backdrop where only it is, the mix where both are.
            let a_o = a_s + a_b - a_s * a_b;
            let mut out = [0u8; 3];
            for i in 0..3 {
                let c = cs[i] * a_s * (1.0 - a_b) + cb[i] * a_b * (1.0 - a_s) + a_s * a_b * b[i];
                out[i] = ((c / a_o.max(1e-6)).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            if let Some(np) =
                tiny_skia::ColorU8::from_rgba(out[0], out[1], out[2], (a_o * 255.0).round() as u8)
                    .premultiply()
                    .into()
            {
                *d = np;
            }
        }
    }
}

/// `c` with its HSL lightness set to `l`, its hue and saturation kept.
fn set_lightness(c: [f32; 3], l: f32) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let lb = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return [l, l, l];
    }
    let s = d / (1.0 - (2.0 * lb - 1.0).abs()).max(1e-6);
    let h = if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s.min(1.0);
    let x = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = l - chroma / 2.0;
    [r + m, g + m, b + m]
}

/// Three-pass box blur (approximates a Gaussian) on premultiplied RGBA.
pub fn box_blur(pm: &mut Pixmap, radius: i32) {
    box_blur_passes(pm, radius, 3);
}

/// Box blur with `passes` passes each way on premultiplied RGBA.
pub fn box_blur_passes(pm: &mut Pixmap, radius: i32, passes: usize) {
    let w = pm.width() as usize;
    let h = pm.height() as usize;
    if w == 0 || h == 0 {
        return;
    }
    let r = radius.max(1) as usize;
    let data = pm.data_mut();
    let mut tmp = vec![0u8; data.len()];
    for _ in 0..passes {
        blur_pass(data, &mut tmp, w, h, r, true);
        blur_pass(&tmp, data, w, h, r, false);
    }
}

fn blur_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    let idx = |o: usize, i: usize| -> usize {
        if horizontal {
            (o * w + i) * 4
        } else {
            (i * w + o) * 4
        }
    };
    let span = (2 * r + 1) as u32;
    for o in 0..outer {
        let mut acc = [0u32; 4];
        // Prime the window with the first pixel repeated, like edge clamping.
        for i in 0..=r {
            let p = idx(o, i.min(inner - 1));
            for c in 0..4 {
                acc[c] += src[p + c] as u32;
            }
        }
        for _ in 0..r {
            let p = idx(o, 0);
            for c in 0..4 {
                acc[c] += src[p + c] as u32;
            }
        }
        for i in 0..inner {
            let d = idx(o, i);
            for c in 0..4 {
                dst[d + c] = (acc[c] / span) as u8;
            }
            let add = idx(o, (i + r + 1).min(inner - 1));
            let sub = idx(o, i.saturating_sub(r));
            for c in 0..4 {
                acc[c] = acc[c] + src[add + c] as u32 - src[sub + c] as u32;
            }
        }
    }
}

/// An outline's own paint when it has one other than a plain colour (a
/// gradient, pattern, texture or mesh), ready to draw in page space: a
/// mesh's nodes are local to the object and go through `transform`.
fn outline_paint(stroke: &Stroke, transform: Affine) -> Option<Fill> {
    match stroke.fill.as_deref()? {
        Fill::None | Fill::Solid(_) => None,
        Fill::Mesh(m) => Some(Fill::Mesh(m.transformed(transform))),
        other => Some(other.clone()),
    }
}

/// Largest side, in pixels, of the grid a shaped fountain measures its
/// distances on.
const SHAPED_GRID: f32 = 384.0;

/// For each pixel of a `w` by `h` grid, the distance in pixels from its
/// centre to the edge of the region `inside` marks (0 outside): exact
/// Euclidean distance to the nearest outside pixel, less the half pixel
/// to the edge between them.
fn edge_distance(inside: &[bool], w: usize, h: usize) -> Vec<f64> {
    const INF: f64 = 1e20;
    let mut g: Vec<f64> = inside.iter().map(|&i| if i { INF } else { 0.0 }).collect();
    if g.len() != w * h || w == 0 || h == 0 {
        return vec![0.0; inside.len()];
    }
    let n = w.max(h);
    let (mut f, mut d) = (vec![0.0; n], vec![0.0; n]);
    let (mut v, mut z) = (vec![0usize; n], vec![0.0; n + 1]);
    for x in 0..w {
        for y in 0..h {
            f[y] = g[y * w + x];
        }
        distance_1d(&f[..h], &mut d[..h], &mut v, &mut z);
        for y in 0..h {
            g[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&g[y * w..(y + 1) * w]);
        distance_1d(&f[..w], &mut d[..w], &mut v, &mut z);
        g[y * w..(y + 1) * w].copy_from_slice(&d[..w]);
    }
    g.iter()
        .zip(inside)
        .map(|(s, &i)| if i { (s.sqrt() - 0.5).max(0.0) } else { 0.0 })
        .collect()
}

/// One pass of the squared distance transform: `d[q] = min over p of
/// (q - p)^2 + f[p]`, by the lower envelope of the parabolas.
fn distance_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let sq = |i: usize| (i * i) as f64;
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + sq(q)) - (f[p] + sq(p))) / (2.0 * (q as f64 - p as f64));
            // z[0] is minus infinity, so this stops at the first parabola.
            if k > 0 && s <= z[k] {
                k -= 1;
                continue;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
            break;
        }
    }
    let mut k = 0usize;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f64 - p as f64;
        *out = dq * dq + f[p];
    }
}

/// Cheap value noise in 0..1 for texture fills.
/// The grid of a procedural fill over `w` by `h` pixels: every pixel up
/// to about a third of a megapixel, then every second or third.
fn procedural_step(w: u32, h: u32) -> u32 {
    match w as u64 * h as u64 {
        0..=350_000 => 1,
        350_001..=1_400_000 => 2,
        _ => 3,
    }
}

/// Fill the rows of `px` (`w` pixels wide) with `row`, on several threads
/// where there are some.
fn for_rows(
    px: &mut [tiny_skia::PremultipliedColorU8],
    w: usize,
    row: &(impl Fn(u32, &mut [tiny_skia::PremultipliedColorU8]) + Sync),
) {
    if w == 0 {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8);
        let rows = px.len() / w;
        if threads > 1 && rows >= 64 {
            let per = rows.div_ceil(threads);
            std::thread::scope(|scope| {
                for (k, chunk) in px.chunks_mut(per * w).enumerate() {
                    scope.spawn(move || {
                        for (i, line) in chunk.chunks_mut(w).enumerate() {
                            row((k * per + i) as u32, line);
                        }
                    });
                }
            });
            return;
        }
    }
    for (y, line) in px.chunks_mut(w).enumerate() {
        row(y as u32, line);
    }
}

/// The largest mesh picture kept, in pixels; past it only the part on
/// the canvas is worked out, and not kept.
const MESH_RASTER_PIXELS: u32 = 8_000_000;
/// Triangles gathered before they are painted: bounds the memory a mesh
/// takes while it is drawn (96 bytes a triangle).
const MESH_TRIANGLE_BATCH: usize = 1 << 17;
/// The most pieces a cell's side is cut into.
const MESH_MAX_SUBDIVISIONS: usize = 256;
/// Pixels of a cell's side per piece.
const MESH_CHORD: f64 = 4.0;

/// The box round a mesh's nodes and handles.
fn mesh_bounds(m: &tracedraw_core::Mesh) -> Option<Rect> {
    m.nodes
        .iter()
        .flat_map(|n| std::iter::once(n.pos).chain(n.handles.into_iter().flatten()))
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .map(|p| Rect::from_points(p, p))
        .reduce(|a, b| a.union(b))
}

/// A corner of a mesh triangle: its place in the picture and its colour
/// and opacity (0 to 1).
#[derive(Clone, Copy)]
struct MeshVertex {
    x: f64,
    y: f64,
    rgba: [f32; 4],
}

/// How many pieces the sides of a cell are cut into, from its size in
/// pixels (`MESH_CHORD` pixels a piece, at most `MESH_MAX_SUBDIVISIONS`),
/// or `None` for a cell that misses the picture of `w` x `h` pixels:
/// `cell` is the box of its four side curves, grown by a few per cent
/// of the cell's size and a chord, as the patch inside can reach a
/// little past its sides.
fn mesh_cell_subdivisions(cell: Rect, size: f64, w: usize, h: usize) -> Option<usize> {
    let grow = size * 0.05 + MESH_CHORD;
    if !(cell.x1 + grow >= -1.0
        && cell.y1 + grow >= -1.0
        && cell.x0 - grow <= w as f64 + 1.0
        && cell.y0 - grow <= h as f64 + 1.0)
    {
        return None;
    }
    Some(((size / MESH_CHORD).ceil() as usize).clamp(1, MESH_MAX_SUBDIVISIONS))
}

/// The triangles of the mesh `m` (its nodes taken through `to_raster`,
/// which maps them to the pixels of a picture `w` x `h`), handed to
/// `paint` in batches of at most `MESH_TRIANGLE_BATCH`: every cell as a
/// grid of triangles small enough for the curves to look smooth, the
/// cells and pieces off the picture left out, so a mesh far larger than
/// the picture costs only what shows.
fn mesh_triangles(
    m: &tracedraw_core::Mesh,
    to_raster: Affine,
    w: usize,
    h: usize,
    mut paint: impl FnMut(&[[MeshVertex; 3]]),
) {
    use tracedraw_core::geometry::ParamCurve;
    let cols = m.cols as usize + 1;
    let rgb: Vec<[f32; 3]> = m.nodes.iter().map(|n| n.color.to_rgb_f32()).collect();
    // Smooth colour: each channel through the 4 x 4 nodes around the
    // cell with Catmull-Rom weights (rows and columns past the edge
    // repeat the edge).
    let weights = |t: f64| {
        let (t2, t3) = (t * t, t * t * t);
        [
            (-t3 + 2.0 * t2 - t) / 2.0,
            (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0,
            (-3.0 * t3 + 4.0 * t2 + t) / 2.0,
            (t3 - t2) / 2.0,
        ]
    };
    let smooth_at = |r: usize, c: usize, u: f64, v: f64| -> Option<[f32; 4]> {
        let (wu, wv) = (weights(u), weights(v));
        let mut acc = [0.0f64; 4];
        for (j, wj) in wv.iter().enumerate() {
            let rr = (r + j).saturating_sub(1).min(m.rows as usize);
            for (i, wi) in wu.iter().enumerate() {
                let cc = (c + i).saturating_sub(1).min(m.cols as usize);
                let k = rr * cols + cc;
                let (n, c3) = (m.nodes.get(k)?, rgb.get(k)?);
                let wgt = wi * wj;
                for ch in 0..3 {
                    acc[ch] += wgt * c3[ch] as f64;
                }
                acc[3] += wgt * n.alpha as f64;
            }
        }
        Some([
            acc[0].clamp(0.0, 1.0) as f32,
            acc[1].clamp(0.0, 1.0) as f32,
            acc[2].clamp(0.0, 1.0) as f32,
            acc[3].clamp(0.0, 1.0) as f32,
        ])
    };
    let bilinear = |r: usize, c: usize, u: f64, v: f64| -> Option<[f32; 4]> {
        let at = |rr: usize, cc: usize| {
            let k = rr * cols + cc;
            let n = m.nodes.get(k)?;
            let c3 = rgb.get(k)?;
            Some([c3[0], c3[1], c3[2], n.alpha])
        };
        let (a, b, d, e) = (at(r, c)?, at(r, c + 1)?, at(r + 1, c)?, at(r + 1, c + 1)?);
        let (u, v) = (u as f32, v as f32);
        let mut out = [0.0f32; 4];
        for (ch, o) in out.iter_mut().enumerate() {
            let top = a[ch] + (b[ch] - a[ch]) * u;
            let bottom = d[ch] + (e[ch] - d[ch]) * u;
            *o = top + (bottom - top) * v;
        }
        Some(out)
    };
    let mut tris: Vec<[MeshVertex; 3]> = Vec::new();
    let mut grid: Vec<MeshVertex> = Vec::new();
    let (mut tops, mut bottoms, mut lefts, mut rights) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for r in 0..m.rows {
        for c in 0..m.cols {
            // The cell's four sides, in pixels.
            let (Some(top), Some(bottom), Some(left), Some(right)) = (
                m.edge(r, c, true),
                m.edge(r + 1, c, true),
                m.edge(r, c, false),
                m.edge(r, c + 1, false),
            ) else {
                continue;
            };
            let (top, bottom, left, right) = (
                to_raster * top,
                to_raster * bottom,
                to_raster * left,
                to_raster * right,
            );
            let cell = [top, bottom, left, right]
                .iter()
                .map(|e| e.bounding_box())
                .reduce(|a, b| a.union(b))
                .unwrap_or_default();
            // Subdivisions from the cell's size in pixels: a chord every
            // few pixels keeps the curves and the colour smooth.
            let size = (top.p0 - top.p3)
                .hypot()
                .max((bottom.p0 - bottom.p3).hypot())
                .max((left.p0 - left.p3).hypot())
                .max((right.p0 - right.p3).hypot());
            let Some(k) = mesh_cell_subdivisions(cell, size, w, h) else {
                continue;
            };
            // The sides evaluated once each; the Coons patch mixes them.
            let steps = |curve: tracedraw_core::geometry::CubicBez, out: &mut Vec<Point>| {
                out.clear();
                out.extend((0..=k).map(|i| curve.eval(i as f64 / k as f64)));
            };
            steps(top, &mut tops);
            steps(bottom, &mut bottoms);
            steps(left, &mut lefts);
            steps(right, &mut rights);
            let (a, b, d, e) = (top.p0, top.p3, bottom.p0, bottom.p3);
            grid.clear();
            let mut ok = true;
            'cell: for j in 0..=k {
                let v = j as f64 / k as f64;
                for i in 0..=k {
                    let u = i as f64 / k as f64;
                    let colour = if m.smooth {
                        smooth_at(r as usize, c as usize, u, v)
                    } else {
                        bilinear(r as usize, c as usize, u, v)
                    };
                    let (Some(rgba), Some(&t), Some(&bo), Some(&l), Some(&ri)) = (
                        colour,
                        tops.get(i),
                        bottoms.get(i),
                        lefts.get(j),
                        rights.get(j),
                    ) else {
                        ok = false;
                        break 'cell;
                    };
                    let p = t.to_vec2() * (1.0 - v)
                        + bo.to_vec2() * v
                        + l.to_vec2() * (1.0 - u)
                        + ri.to_vec2() * u
                        - a.to_vec2() * ((1.0 - u) * (1.0 - v))
                        - b.to_vec2() * (u * (1.0 - v))
                        - d.to_vec2() * ((1.0 - u) * v)
                        - e.to_vec2() * (u * v);
                    grid.push(MeshVertex {
                        x: p.x,
                        y: p.y,
                        rgba,
                    });
                }
            }
            if !ok {
                continue;
            }
            for j in 0..k {
                for i in 0..k {
                    let at = |ii: usize, jj: usize| grid.get(jj * (k + 1) + ii).copied();
                    let (Some(v00), Some(v10), Some(v01), Some(v11)) =
                        (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1))
                    else {
                        continue;
                    };
                    // A piece off the picture is left out.
                    let q = [v00, v10, v01, v11];
                    if q.iter().all(|p| p.x < -1.0)
                        || q.iter().all(|p| p.x > w as f64 + 1.0)
                        || q.iter().all(|p| p.y < -1.0)
                        || q.iter().all(|p| p.y > h as f64 + 1.0)
                    {
                        continue;
                    }
                    tris.push([v00, v10, v11]);
                    tris.push([v00, v11, v01]);
                    if tris.len() >= MESH_TRIANGLE_BATCH {
                        paint(&tris);
                        tris.clear();
                    }
                }
            }
        }
    }
    if !tris.is_empty() {
        paint(&tris);
    }
}

/// Draw the mesh `m` (its nodes taken through `to_raster`, which maps
/// them to the picture's pixels) into `pm`: its triangles painted batch
/// by batch (see [`mesh_triangles`]), the colour mixed per pixel, large
/// batches on several threads, each painting a band of rows; then the
/// picture's edge is grown by a pixel so the path it is drawn through
/// finds colour under its smoothed edge, and the outermost ring of
/// pixels is cleared so nothing spreads outside the grid.
fn rasterise_mesh(m: &tracedraw_core::Mesh, to_raster: Affine, pm: &mut Pixmap) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let mut buf: Vec<[u8; 4]> = vec![[0; 4]; w * h];
    mesh_triangles(m, to_raster, w, h, |tris| {
        raster_triangles(&mut buf, w, tris)
    });
    for (i, px) in pm.pixels_mut().iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let ring = x == 0 || y == 0 || x + 1 == w || y + 1 == h;
        let mut c = if ring { [0; 4] } else { buf[i] };
        if c[3] == 0 && !ring {
            for n in [buf[i + 1], buf[i - 1], buf[i + w], buf[i - w]] {
                if n[3] > c[3] {
                    c = n;
                }
            }
        }
        *px = tiny_skia::ColorU8::from_rgba(c[0], c[1], c[2], c[3]).premultiply();
    }
}

/// Paint the triangles into `buf` (`w` pixels wide): a large batch has
/// the rows cut into bands, one thread each, every band painting the
/// triangles that reach it; a small one is painted on this thread.
fn raster_triangles(buf: &mut [[u8; 4]], w: usize, tris: &[[MeshVertex; 3]]) {
    if w == 0 || tris.is_empty() {
        return;
    }
    let rows = buf.len() / w;
    let paint_band = |band: &mut [[u8; 4]], y0: usize| {
        let band_rows = band.len() / w;
        for t in tris {
            raster_triangle(band, w, y0, band_rows, *t);
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8);
        if threads > 1 && rows >= 64 && tris.len() >= 4096 {
            let per = rows.div_ceil(threads);
            std::thread::scope(|scope| {
                for (k, band) in buf.chunks_mut(per * w).enumerate() {
                    scope.spawn(move || paint_band(band, k * per));
                }
            });
            return;
        }
    }
    paint_band(buf, 0);
}

/// Paint a triangle into the band `buf` of `band_rows` rows (`w` pixels
/// wide, starting at row `y0` of the picture), the colour of each pixel
/// mixed from the corners by where it lies; the edges reach half a pixel
/// out so neighbouring triangles leave no gaps between them.
fn raster_triangle(buf: &mut [[u8; 4]], w: usize, y0: usize, band_rows: usize, v: [MeshVertex; 3]) {
    let area = (v[1].x - v[0].x) * (v[2].y - v[0].y) - (v[2].x - v[0].x) * (v[1].y - v[0].y);
    if !area.is_finite() || area.abs() < 1e-12 {
        return;
    }
    const REACH: f64 = 0.55;
    let min_x = v.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let max_x = v.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let min_y = v.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_y = v.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    if !(min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite()) {
        return;
    }
    // The pixels whose centres can be within reach of the triangle, in
    // this band.
    let x_start = (min_x - REACH - 0.5).ceil().max(0.0) as usize;
    let x_end = (max_x + REACH - 0.5).floor().min(w as f64 - 1.0);
    let y_start = (min_y - REACH - 0.5).ceil().max(y0 as f64) as usize;
    let y_end = (max_y + REACH - 0.5)
        .floor()
        .min((y0 + band_rows) as f64 - 1.0);
    if x_end < x_start as f64 || y_end < y_start as f64 {
        return;
    }
    let (x_end, y_end) = (x_end as usize, y_end as usize);
    // Edge functions, signed by the triangle's turn, in pixels (divided
    // by the edge's length), and their change per pixel across.
    let sign = area.signum();
    let aa = area.abs();
    let edge_len = |i: usize, j: usize| ((v[j].x - v[i].x).hypot(v[j].y - v[i].y)).max(1e-9);
    let lens = [edge_len(1, 2), edge_len(2, 0), edge_len(0, 1)];
    let at = |px: f64, py: f64| {
        [
            ((v[1].x - px) * (v[2].y - py) - (v[2].x - px) * (v[1].y - py)) * sign,
            ((v[2].x - px) * (v[0].y - py) - (v[0].x - px) * (v[2].y - py)) * sign,
            ((v[0].x - px) * (v[1].y - py) - (v[1].x - px) * (v[0].y - py)) * sign,
        ]
    };
    let dx = [
        (v[1].y - v[2].y) * sign,
        (v[2].y - v[0].y) * sign,
        (v[0].y - v[1].y) * sign,
    ];
    let limits = [-REACH * lens[0], -REACH * lens[1], -REACH * lens[2]];
    for y in y_start..=y_end {
        let py = y as f64 + 0.5;
        let mut wgt = at(x_start as f64 + 0.5, py);
        let row = &mut buf[(y - y0) * w..(y - y0 + 1) * w];
        for out in &mut row[x_start..=x_end] {
            if wgt[0] >= limits[0] && wgt[1] >= limits[1] && wgt[2] >= limits[2] {
                let (b0, b1, b2) = (
                    (wgt[0] / aa).clamp(0.0, 1.0),
                    (wgt[1] / aa).clamp(0.0, 1.0),
                    (wgt[2] / aa).clamp(0.0, 1.0),
                );
                let sum = (b0 + b1 + b2).max(1e-9);
                let (b0, b1, b2) = ((b0 / sum) as f32, (b1 / sum) as f32, (b2 / sum) as f32);
                for (ch, o) in out.iter_mut().enumerate() {
                    let x = v[0].rgba[ch] * b0 + v[1].rgba[ch] * b1 + v[2].rgba[ch] * b2;
                    *o = (x.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
            wgt[0] += dx[0];
            wgt[1] += dx[1];
            wgt[2] += dx[2];
        }
    }
}

/// The last procedural fills worked out, newest last: up to this many,
/// and up to this many pixels together. Shared by every thread (the
/// application draws a slow page on a thread of its own), so a picture
/// worked out once serves every later render.
const PROCEDURAL_CACHE: usize = 256;
const PROCEDURAL_CACHE_PIXELS: usize = 24_000_000;

static PROCEDURAL: std::sync::Mutex<Vec<(String, std::sync::Arc<Pixmap>)>> =
    std::sync::Mutex::new(Vec::new());

fn procedural_cache_get(key: &str) -> Option<std::sync::Arc<Pixmap>> {
    let mut c = PROCEDURAL.lock().unwrap_or_else(|e| e.into_inner());
    let i = c.iter().position(|(k, _)| k == key)?;
    let hit = c.remove(i);
    let pm = hit.1.clone();
    c.push(hit);
    Some(pm)
}

/// The keys of pictures being worked out, so that threads asking for the
/// same picture at once (the bands of one render) wait for the first
/// instead of each working it out.
static IN_FLIGHT: std::sync::Mutex<Vec<(String, std::sync::Arc<std::sync::Mutex<()>>)>> =
    std::sync::Mutex::new(Vec::new());

/// The cached picture of `key`, or the one `compute` gives, kept under
/// that key. A second thread asking for a key being worked out waits for
/// it and takes the result.
fn procedural_cached(
    key: &str,
    compute: impl FnOnce() -> Option<Pixmap>,
) -> Option<std::sync::Arc<Pixmap>> {
    if let Some(pm) = procedural_cache_get(key) {
        return Some(pm);
    }
    let gate = {
        let mut f = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        match f.iter().find(|(k, _)| k == key) {
            Some((_, g)) => g.clone(),
            None => {
                let g = std::sync::Arc::new(std::sync::Mutex::new(()));
                f.push((key.to_string(), g.clone()));
                g
            }
        }
    };
    let held = gate.lock().unwrap_or_else(|e| e.into_inner());
    let pm = match procedural_cache_get(key) {
        Some(pm) => Some(pm),
        None => {
            let pm = compute().map(std::sync::Arc::new);
            if let Some(pm) = &pm {
                procedural_cache_put(key.to_string(), pm.clone());
            }
            pm
        }
    };
    {
        let mut f = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        f.retain(|(k, _)| k != key);
    }
    drop(held);
    pm
}

/// Forget every kept procedural picture (when the last drawing closes,
/// so the memory goes back).
pub fn clear_procedural_cache() {
    PROCEDURAL.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn procedural_cache_put(key: String, pm: std::sync::Arc<Pixmap>) {
    let mut c = PROCEDURAL.lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|(k, _)| *k != key);
    c.push((key, pm));
    let mut total: usize = c
        .iter()
        .map(|(_, p)| p.width() as usize * p.height() as usize)
        .sum();
    while c.len() > PROCEDURAL_CACHE || (total > PROCEDURAL_CACHE_PIXELS && c.len() > 1) {
        let (_, old) = c.remove(0);
        total -= old.width() as usize * old.height() as usize;
    }
}

/// The keys of the procedural fills the cache holds.
#[cfg(test)]
fn procedural_cache_keys() -> Vec<String> {
    PROCEDURAL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(k, _)| k.clone())
        .collect()
}

fn noise(x: f64, y: f64, seed: u32) -> f64 {
    fn hash(ix: i64, iy: i64, seed: u32) -> f64 {
        let mut h = (ix as u64).wrapping_mul(0x9E3779B97F4A7C15)
            ^ (iy as u64).wrapping_mul(0xC2B2AE3D27D4EB4F)
            ^ (seed as u64).wrapping_mul(0x165667B19E3779F9);
        h ^= h >> 31;
        h = h.wrapping_mul(0x9E3779B97F4A7C15);
        h ^= h >> 29;
        (h & 0xFFFF) as f64 / 65535.0
    }
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(ix, iy, seed);
    let b = hash(ix + 1, iy, seed);
    let c = hash(ix, iy + 1, seed);
    let d = hash(ix + 1, iy + 1, seed);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

fn fbm(x: f64, y: f64, seed: u32) -> f64 {
    let mut v = 0.0;
    let mut amp = 0.5;
    let mut f = 1.0;
    for _ in 0..5 {
        v += amp * noise(x * f, y * f, seed);
        amp *= 0.5;
        f *= 2.0;
    }
    v
}

pub fn texture_value(kind: tracedraw_core::TextureKind, x: f64, y: f64, seed: u32) -> f64 {
    match kind {
        tracedraw_core::TextureKind::Clouds => fbm(x, y, seed).clamp(0.0, 1.0),
        tracedraw_core::TextureKind::Marble => ((x + 4.0 * fbm(x, y, seed)) * 2.0).sin().abs(),
        tracedraw_core::TextureKind::Noise => noise(x * 8.0, y * 8.0, seed),
        tracedraw_core::TextureKind::Wood => {
            let r = ((x - 0.5).powi(2) + (y * 0.2).powi(2)).sqrt() * 6.0 + fbm(x, y, seed) * 1.5;
            r.fract()
        }
    }
}

fn bbox(p: &BezPath) -> Rect {
    use tracedraw_core::geometry::Shape as _;
    p.bounding_box()
}

pub fn sk_color(c: Color) -> SkColor {
    let [r, g, b] = c.to_rgb8();
    SkColor::from_rgba8(r, g, b, 255)
}

/// [`sk_color`] at an opacity (0 clear, 1 opaque).
pub fn sk_color_alpha(c: Color, opacity: f32) -> SkColor {
    let [r, g, b] = c.to_rgb8();
    SkColor::from_rgba8(r, g, b, (opacity.clamp(0.0, 1.0) * 255.0).round() as u8)
}

pub fn to_sk_path(path: &BezPath) -> Option<SkPath> {
    let mut pb = PathBuilder::new();
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(c, p) => pb.quad_to(c.x as f32, c.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(c1, c2, p) => pb.cubic_to(
                c1.x as f32,
                c1.y as f32,
                c2.x as f32,
                c2.y as f32,
                p.x as f32,
                p.y as f32,
            ),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

/// Rasterise a fill over `bounds` (page mm) at `dpi`, y up mapped to image
/// rows top-down. Used by exporters that have no native pattern or
/// texture support (PDF, SVG fallback).
pub fn render_fill_image(fill: &Fill, bounds: Rect, dpi: f64) -> Option<Pixmap> {
    let zoom = dpi / 25.4;
    let w = (bounds.width() * zoom).ceil().max(1.0) as u32;
    let h = (bounds.height() * zoom).ceil().max(1.0) as u32;
    if w > 8192 || h > 8192 {
        return None;
    }
    let mut pixmap = Pixmap::new(w, h)?;
    let view = ViewTransform {
        zoom,
        origin_x: -bounds.x0 * zoom,
        origin_y: bounds.y1 * zoom,
    };
    let screen = view.affine();
    let page_path = tracedraw_core::geometry::rect_path(bounds, 0.0);
    let sk = to_sk_path(&(screen * page_path.clone()))?;
    let mut r = Renderer {
        pixmap: &mut pixmap,
        screen,
        zoom,
        flags: Flags::default(),
        symbols: &[],
        preview: None,
        origin: (0, 0),
        cancel: None,
    };
    r.draw_fill(fill, &page_path, &sk, FillRule::Winding);
    Some(pixmap)
}

/// [`render_fill_image`] over the bounds of `path` (page space, mm) for
/// a fill that depends on the object's outline: a shaped fountain
/// measures its distances from the edge of `path` (even-odd) and covers
/// the whole rectangle, so an exporter can clip it with the path. Other
/// fills render as [`render_fill_image`] does.
pub fn render_fill_image_in(fill: &Fill, path: &BezPath, dpi: f64) -> Option<Pixmap> {
    let bounds = bbox(path);
    let Fill::Fountain(f) = fill else {
        return render_fill_image(fill, bounds, dpi);
    };
    if f.kind != tracedraw_core::FountainKind::Shaped {
        return render_fill_image(fill, bounds, dpi);
    }
    let zoom = dpi / 25.4;
    let w = (bounds.width() * zoom).ceil().max(1.0) as u32;
    let h = (bounds.height() * zoom).ceil().max(1.0) as u32;
    if w > 8192 || h > 8192 {
        return None;
    }
    let mut pixmap = Pixmap::new(w, h)?;
    let screen = ViewTransform {
        zoom,
        origin_x: -bounds.x0 * zoom,
        origin_y: bounds.y1 * zoom,
    }
    .affine();
    let rect = to_sk_path(&(screen * tracedraw_core::geometry::rect_path(bounds, 0.0)))?;
    let mask = to_sk_path(&(screen * path.clone()))?;
    let mut r = Renderer {
        pixmap: &mut pixmap,
        screen,
        zoom,
        flags: Flags::default(),
        symbols: &[],
        preview: None,
        origin: (0, 0),
        cancel: None,
    };
    r.fill_shaped(f, &mask, &rect, FillRule::EvenOdd);
    Some(pixmap)
}

/// Whether a tile copy drawn with `t` (tile mm to block pixels) reaches a
/// `tw` by `th` pixmap at all: its corners' box meets the pixmap.
fn copy_shows(t: Affine, tile: Rect, tw: u32, th: u32) -> bool {
    let b = (t * tile.to_path(0.01)).bounding_box();
    b.x1 > 0.0 && b.y1 > 0.0 && b.x0 < f64::from(tw) && b.y0 < f64::from(th)
}

/// The block of tiles a pattern repeats (one tile, or the mirrored and
/// shifted tiles of its placement), rasterised at `dpi`, with the
/// transform from the pixmap's unit square (Y up) to page millimetres:
/// the placement's tile size, offset, rotation, skew and lean. Exporters
/// tile the picture with that transform (the object's transform in front
/// of it when the pattern transforms with its object).
pub fn render_pattern_cell(p: &tracedraw_core::Pattern, dpi: f64) -> Option<(Pixmap, Affine)> {
    let zoom = dpi / 25.4;
    if !(zoom.is_finite() && zoom > 0.0) {
        return None;
    }
    let placement = p.placement().sanitized();
    let tile = p.tile_size();
    if !(tile.width.is_finite() && tile.height.is_finite())
        || tile.width <= 0.0
        || tile.height <= 0.0
    {
        return None;
    }
    let unit_to_page = placement.transform(tile);
    let mut scratch = Pixmap::new(1, 1)?;
    let r = Renderer {
        pixmap: &mut scratch,
        screen: Affine::scale(zoom),
        zoom,
        flags: Flags::default(),
        symbols: &[],
        preview: None,
        origin: (0, 0),
        cancel: None,
    };
    let (pm, px_to_unit) =
        r.pattern_cell(p, &placement, tile, Affine::scale(zoom) * unit_to_page)?;
    // The picture's pixels (Y down) laid over the unit square: a decoded
    // bitmap is the block itself only when it covers the square exactly.
    let unit = px_to_unit.as_coeffs();
    let covers = (unit[0] * f64::from(pm.width()) - 1.0).abs() < 1e-6
        && (unit[3] * f64::from(pm.height()) + 1.0).abs() < 1e-6
        && unit[4].abs() < 1e-9
        && (unit[5] - 1.0).abs() < 1e-9;
    if covers {
        return Some(((*pm).clone(), unit_to_page));
    }
    // A bitmap tile stretched over the block: drawn into a pixmap of its own.
    let (tw, th) = (pm.width().clamp(1, 4096), pm.height().clamp(1, 4096));
    let mut out = Pixmap::new(tw, th)?;
    let t = Affine::new([f64::from(tw), 0.0, 0.0, -f64::from(th), 0.0, f64::from(th)]) * px_to_unit;
    let c = t.as_coeffs();
    out.draw_pixmap(
        0,
        0,
        pm.as_ref().as_ref(),
        &tiny_skia::PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        },
        Transform::from_row(
            c[0] as f32,
            c[1] as f32,
            c[2] as f32,
            c[3] as f32,
            c[4] as f32,
            c[5] as f32,
        ),
        None,
    );
    Some((out, unit_to_page))
}

/// Render a whole page to an RGBA image at `dpi`, white background (exports, thumbnails, tests).
pub fn render_page_image(doc: &Document, page: PageId, dpi: f64) -> Option<Pixmap> {
    render_page_image_with(doc, page, dpi, &RenderOptions::default())
}

/// `area` (page coordinates, mm) of `page` as an image at `dpi`, white
/// where nothing is drawn: the page with whatever lies around it on the
/// desktop, for the Navigator (#2053).
pub fn render_area_image(doc: &Document, page: PageId, area: Rect, dpi: f64) -> Option<Pixmap> {
    doc.page(page).ok()?;
    let zoom = dpi / 25.4;
    if !(area.width() > 0.0 && area.height() > 0.0) || !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    let w = (area.width() * zoom).ceil().clamp(1.0, 8192.0) as u32;
    let h = (area.height() * zoom).ceil().clamp(1.0, 8192.0) as u32;
    let view = ViewTransform {
        zoom,
        origin_x: -area.x0 * zoom,
        origin_y: h as f64 + area.y0 * zoom,
    };
    let objects = render_page(
        doc,
        page,
        &RenderOptions {
            width: w,
            height: h,
            view,
            ..RenderOptions::default()
        },
    )?;
    let mut out = Pixmap::new(w, h)?;
    out.fill(SkColor::WHITE);
    out.draw_pixmap(
        0,
        0,
        objects.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    Some(out)
}

/// `render_page_image` with the behaviour flags of `opts` (`wireframe`,
/// `simulate_overprints`, `complex_effects`); its size, view and preview
/// are replaced by the page's own.
pub fn render_page_image_with(
    doc: &Document,
    page: PageId,
    dpi: f64,
    opts: &RenderOptions,
) -> Option<Pixmap> {
    let p = doc.page(page).ok()?;
    let zoom = dpi / 25.4;
    let w = (p.size.width * zoom).ceil() as u32;
    let h = (p.size.height * zoom).ceil() as u32;
    let view = ViewTransform {
        zoom,
        origin_x: 0.0,
        origin_y: h as f64,
    };
    let mut objects = render_page(
        doc,
        page,
        &RenderOptions {
            width: w,
            height: h,
            view,
            preview: None,
            wireframe: opts.wireframe,
            simulate_overprints: opts.simulate_overprints,
            complex_effects: opts.complex_effects,
            anti_alias: opts.anti_alias,
            draft: false,
            focus: None,
        },
    )?;
    let mut out = Pixmap::new(w, h)?;
    out.fill(SkColor::WHITE);
    out.draw_pixmap(
        0,
        0,
        objects.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    let _ = &mut objects;
    Some(out)
}

#[cfg(test)]
mod tests {
    /// A text and the warped outline of one fill with the nonzero rule,
    /// so letters an envelope pushes together stay filled where they
    /// overlap (#647); other curves fill even-odd.
    #[test]
    fn warped_text_fills_like_text() {
        use super::fill_rule;
        let path = ShapeKind::Path {
            path: BezPath::new(),
            closed: true,
        };
        assert!(matches!(fill_rule(&path, false), FillRule::EvenOdd));
        assert!(matches!(fill_rule(&path, true), FillRule::Winding));
    }

    use super::*;
    use tracedraw_core::document::{Shape, ShapeKind};
    use tracedraw_core::{
        Fountain, FountainKind, Pattern, PatternTile, Stop, Texture, TextureKind,
    };

    fn px(pm: &Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
        let p = pm.pixel(x, y).unwrap().demultiply();
        (p.red(), p.green(), p.blue())
    }

    fn doc_with(shapes: Vec<Shape>) -> (Document, PageId) {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let page = doc.pages[0].id;
        let layer = doc.pages[0].layers[0].id;
        for s in shapes {
            doc.layer_mut(layer).unwrap().shapes.push(s);
        }
        (doc, page)
    }

    /// Two squares overlapping in one curve, both drawn the same way
    /// round, as the overlapping parts of traced lettering (#1919): with
    /// the curve's fill rule nonzero the overlap stays filled; even-odd
    /// (the default) leaves it a hole.
    #[test]
    fn a_curve_fills_its_overlaps_by_its_own_fill_rule() {
        use tracedraw_core::shaping::FillRule as Rule;
        let mut path = BezPath::new();
        for (x0, x1) in [(10.0, 60.0), (40.0, 90.0)] {
            path.move_to((x0, 10.0));
            path.line_to((x1, 10.0));
            path.line_to((x1, 90.0));
            path.line_to((x0, 90.0));
            path.close_path();
        }
        let curve = |rule: Rule| {
            let mut s = Shape::new(
                ShapeId(1),
                ShapeKind::Path {
                    path: path.clone(),
                    closed: true,
                },
            );
            s.fill = Fill::Solid(Color::BLACK);
            s.stroke = None;
            s.fill_rule = rule;
            s
        };
        for (rule, overlap) in [(Rule::NonZero, (0, 0, 0)), (Rule::EvenOdd, (255, 255, 255))] {
            let (doc, page) = doc_with(vec![curve(rule)]);
            let pm = render_page_image(&doc, page, 25.4).expect("page");
            assert_eq!(px(&pm, 50, 50), overlap, "{rule:?}");
            assert_eq!(px(&pm, 20, 50), (0, 0, 0), "{rule:?}");
        }
    }

    /// A page whose size is not a number (read from a damaged file, or
    /// left by a division by zero) asks for an image as tall as a number
    /// holds. Splitting it into bands for the machine's cores, and
    /// drawing a strip of it with its margin of rows, added past the end
    /// of the range and panicked; now the bands and the strip end at the
    /// image's last row, and drawing the page gives no picture, as a
    /// picture that large cannot be made, instead of taking the program
    /// down.
    #[test]
    fn an_image_as_tall_as_a_number_holds_does_not_overflow_its_bands() {
        use tracedraw_core::geometry::{Rect, Size};
        let s = rect(1, Rect::new(20.0, 20.0, 80.0, 80.0), Color::BLACK);
        let (mut doc, page) = doc_with(vec![s]);
        doc.pages[0].size = Size::new(100.0, f64::INFINITY);
        assert!(render_page_image(&doc, page, 25.4).is_none());
        // A strip at the end of such an image is still drawn.
        let opts = RenderOptions {
            width: 10,
            height: u32::MAX,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        let strip = render_page_rows(&doc, page, &opts, u32::MAX - 16, u32::MAX).expect("strip");
        assert_eq!((strip.width(), strip.height()), (10, 16));
        // The bands themselves, for every core count the renderer uses,
        // with a blur that is not a number as well.
        assert_eq!(blur_margin(f64::INFINITY, 1.0), u32::MAX);
        assert_eq!(blur_margin(f64::NAN, 1.0), 16);
        for cores in 2..=16 {
            for margin in [0, 16, u32::MAX] {
                if let Some(bands) = band_layout(u32::MAX, cores, margin) {
                    assert!(bands.iter().all(|b| b.0 < b.1 && b.2 <= b.0 && b.1 <= b.3));
                    assert_eq!(bands.last().map(|b| (b.1, b.3)), Some((u32::MAX, u32::MAX)));
                }
            }
        }
        let bands = band_layout(u32::MAX, 2, 16).expect("two bands");
        assert_eq!(bands.len(), 2);
        assert_eq!(bands[1].0, bands[0].1);
        assert!(band_layout(64, 2, u32::MAX).is_none());
    }

    /// A bevel drawn over a mid-grey square: the band on the light side is
    /// lighter than the fill and the one opposite darker, and with more
    /// levels the tone fades from the edge inwards (#1135).
    #[test]
    fn a_bevel_is_lit_on_one_side_and_fades_with_its_levels() {
        use tracedraw_core::geometry::Rect;
        use tracedraw_core::live::Effect;
        let bevelled = |style: u8, level: u8| {
            let mut s = rect(
                1,
                Rect::new(20.0, 20.0, 80.0, 80.0),
                Color::rgb8(128, 128, 128),
            );
            s.effects.push(Effect::Bevel {
                distance: 12.0,
                light_angle: 180.0,
                intensity: 100.0,
                style,
                shadow_color: Color::BLACK,
                light_color: Color::WHITE,
                level,
                emboss_color: None,
            });
            let (doc, page) = doc_with(vec![s]);
            let pm = render_page_image(&doc, page, 25.4).expect("render");
            if let Ok(dir) = std::env::var("TRACEDRAW_TEST_PNG") {
                let _ = pm.save_png(format!("{dir}/bevel-{style}-{level}.png"));
            }
            pm
        };
        // Light from the left (angle 180): the left band is lit, the right shaded.
        let flat = bevelled(0, 1);
        assert!(px(&flat, 22, 50).0 > 200, "{:?}", px(&flat, 22, 50));
        assert!(px(&flat, 78, 50).0 < 60, "{:?}", px(&flat, 78, 50));
        assert_eq!(px(&flat, 50, 50), (128, 128, 128));
        let soft = bevelled(0, 4);
        let (edge, inner) = (px(&soft, 21, 50).0, px(&soft, 30, 50).0);
        assert!(edge > inner && inner > 128, "{edge} {inner}");
        // Emboss lies outside the square.
        let emboss = bevelled(1, 1);
        assert!(px(&emboss, 12, 50).0 > 200, "{:?}", px(&emboss, 12, 50));
        assert_eq!(px(&emboss, 50, 50), (128, 128, 128));
    }

    /// A white-to-black gradient on a wide rectangle, turned half a turn,
    /// mirrored or turned a quarter turn through the engine, is drawn
    /// turned, mirrored and turned with it (#775).
    #[test]
    fn a_gradient_is_drawn_turned_and_mirrored_with_its_object() {
        use tracedraw_core::{Command, Engine};
        let mut s = rect(1, Rect::new(10.0, 40.0, 90.0, 60.0), Color::WHITE);
        s.fill = Fill::Fountain(Fountain::two(
            FountainKind::Linear,
            Color::WHITE,
            Color::BLACK,
            0.0,
        ));
        let (doc, page) = doc_with(vec![s]);
        let mut eng = Engine::new(doc);
        let about =
            |a: Affine| Affine::translate((50.0, 50.0)) * a * Affine::translate((-50.0, -50.0));
        let mut turn = |eng: &mut Engine, t: Affine| {
            eng.run(&Command::TransformShapes {
                shapes: vec![tracedraw_core::ShapeId(1)],
                transform: about(t),
            })
            .expect("turned");
            let pm = render_page_image(eng.document(), page, 25.4).expect("render");
            if let Ok(dir) = std::env::var("TRACEDRAW_TEST_PNG") {
                let n = eng.history_labels().0.len();
                let _ = pm.save_png(format!("{dir}/gradient-{n}.png"));
            }
            pm
        };
        let pm = render_page_image(eng.document(), page, 25.4).expect("render");
        // Light on the left, dark on the right.
        assert!(px(&pm, 15, 50).0 > 200 && px(&pm, 85, 50).0 < 55);
        let pm = turn(&mut eng, Affine::rotate(std::f64::consts::PI));
        assert!(px(&pm, 15, 50).0 < 55 && px(&pm, 85, 50).0 > 200);
        let pm = turn(&mut eng, Affine::scale_non_uniform(-1.0, 1.0));
        assert!(px(&pm, 15, 50).0 > 200 && px(&pm, 85, 50).0 < 55);
        // A quarter turn: light at the bottom, dark at the top of the
        // upright rectangle (the page's y grows upwards, the picture's
        // downwards).
        let pm = turn(&mut eng, Affine::rotate(std::f64::consts::FRAC_PI_2));
        assert!(px(&pm, 50, 85).0 > 200 && px(&pm, 50, 15).0 < 55);
    }

    fn rect(id: u64, r: Rect, fill: Color) -> Shape {
        let mut s = Shape::new(
            tracedraw_core::ShapeId(id),
            ShapeKind::Rect {
                rect: r,
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(fill);
        s.stroke = None;
        s
    }

    /// A semi-transparent object draws into a layer only as large as
    /// what it can reach (#1335), and the page comes out as with a layer
    /// the size of the window: plain, outlined with arrowheads, with a
    /// gradient, a pattern, a texture, a picture, across the window's
    /// edge, and wholly outside it.
    #[test]
    fn a_transparent_object_takes_a_small_layer_and_draws_the_same() {
        use tracedraw_core::style::{Arrowhead, Pattern, PatternTile};
        let opts = RenderOptions {
            width: 240,
            height: 180,
            view: ViewTransform {
                zoom: 3.0,
                origin_x: 10.0,
                origin_y: 170.0,
            },
            anti_alias: true,
            ..RenderOptions::default()
        };
        let screen = opts.view.affine();
        let under = rect(1, Rect::new(0.0, 0.0, 80.0, 60.0), Color::rgb8(30, 160, 60));
        let mut photo = Pixmap::new(6, 4).expect("pixmap");
        for (k, p) in photo.data_mut().chunks_mut(4).enumerate() {
            p.copy_from_slice(&[(k * 40) as u8, 90, (255 - k * 9) as u8, 255]);
        }
        let mut cases: Vec<Shape> = Vec::new();
        cases.push(rect(
            2,
            Rect::new(10.3, 12.7, 31.1, 29.9),
            Color::rgb8(200, 20, 40),
        ));
        let mut line = Shape::new(
            tracedraw_core::ShapeId(2),
            ShapeKind::Path {
                path: {
                    let mut p = BezPath::new();
                    p.move_to((12.0, 8.0));
                    p.curve_to((30.0, 50.0), (45.0, -5.0), (60.0, 30.0));
                    p
                },
                closed: false,
            },
        );
        let mut stroke = Stroke::new(Color::rgb8(20, 20, 200), 1.6);
        stroke.start_arrow = Arrowhead::Arrow;
        stroke.end_arrow = Arrowhead::Circle;
        line.stroke = Some(stroke);
        line.fill = Fill::None;
        cases.push(line);
        let mut graded = rect(2, Rect::new(20.0, 5.0, 50.0, 40.0), Color::BLACK);
        graded.fill = Fill::Fountain(tracedraw_core::Fountain::two(
            tracedraw_core::FountainKind::Linear,
            Color::rgb8(250, 200, 0),
            Color::rgb8(0, 40, 160),
            30.0,
        ));
        graded.stroke = Some(Stroke::new(Color::BLACK, 0.8));
        cases.push(graded);
        let mut patterned = rect(2, Rect::new(5.0, 30.0, 45.0, 55.0), Color::BLACK);
        patterned.fill = Fill::Pattern(Pattern::TwoColor {
            tile: PatternTile::DiagonalStripes,
            front: Color::rgb8(0, 0, 0),
            back: Color::rgb8(255, 255, 255),
            size_mm: 3.0,
            placement: Default::default(),
        });
        cases.push(patterned);
        let mut clouds = rect(2, Rect::new(8.0, 3.0, 38.0, 21.0), Color::BLACK);
        clouds.fill = Fill::Texture(tracedraw_core::style::Texture {
            kind: tracedraw_core::style::TextureKind::Clouds,
            color_a: Color::rgb8(0, 0, 0),
            color_b: Color::rgb8(255, 255, 255),
            scale: 10.0,
            seed: 7,
        });
        cases.push(clouds);
        let mut pic = Shape::new(
            tracedraw_core::ShapeId(2),
            ShapeKind::Bitmap {
                rect: Rect::new(33.0, 22.0, 51.0, 34.0),
                width_px: photo.width(),
                height_px: photo.height(),
                png: photo.encode_png().expect("png").into(),
                fx: None,
            },
        );
        pic.fill = Fill::None;
        pic.stroke = None;
        cases.push(pic);
        // Across the window's left edge, and wholly outside it.
        cases.push(rect(
            2,
            Rect::new(-20.0, 20.0, 15.0, 40.0),
            Color::rgb8(90, 0, 90),
        ));
        cases.push(rect(
            2,
            Rect::new(200.0, 20.0, 230.0, 40.0),
            Color::rgb8(90, 0, 90),
        ));
        for (k, mut top) in cases.into_iter().enumerate() {
            let (plain_doc, page) = doc_with(vec![under.clone()]);
            let plain = render_page(&plain_doc, page, &opts).expect("render");
            // The reference: the object drawn opaque on a layer the size
            // of the window, laid over the page at half strength.
            let mut full = Pixmap::new(opts.width, opts.height).expect("layer");
            {
                let mut r = Renderer {
                    pixmap: &mut full,
                    screen,
                    zoom: opts.view.zoom,
                    flags: Flags::from_options(&opts),
                    symbols: &[],
                    preview: None,
                    origin: (0, 0),
                    cancel: None,
                };
                r.draw_shape(&top, Affine::IDENTITY);
            }
            let mut expected = plain.clone();
            let pp = tiny_skia::PixmapPaint {
                opacity: 0.5,
                ..Default::default()
            };
            expected.draw_pixmap(0, 0, full.as_ref(), &pp, Transform::identity(), None);
            top.opacity = 0.5;
            let mut layer_px = Pixmap::new(opts.width, opts.height).expect("pixmap");
            let area = Renderer {
                pixmap: &mut layer_px,
                screen,
                zoom: opts.view.zoom,
                flags: Flags::from_options(&opts),
                symbols: &[],
                preview: None,
                origin: (0, 0),
                cancel: None,
            }
            .layer_area(&top, Affine::IDENTITY);
            let (doc, page) = doc_with(vec![under.clone(), top]);
            let drawn = render_page(&doc, page, &opts).expect("render");
            assert!(drawn.data() == expected.data(), "case {k}, area {area:?}");
            if k < 7 {
                assert!(drawn.data() != plain.data(), "case {k} shows");
            }
            // An outline's reach counts 12 widths (mitres, arrowheads),
            // so the outlined ones take more.
            let most = match k {
                1..=3 => opts.width * opts.height,
                _ => opts.width * opts.height / 4,
            };
            assert!(area.2 * area.3 <= most, "case {k}: a small layer, {area:?}");
            if k == 7 {
                assert_eq!(area.2 * area.3, 0, "outside: no layer");
            }
        }
    }

    /// A stand-in text engine: character `i` of the text is a square from
    /// (10 i, 0) to (10 i + 8, 8) mm; characters only laid out are left
    /// out.
    fn square_letters(req: &tracedraw_core::document::text_outline::TextRequest) -> BezPath {
        let mut p = BezPath::new();
        let mut i = 0usize;
        for sp in req.spans {
            for _ in sp.text.chars() {
                if !sp.layout_only {
                    let x = 10.0 * i as f64;
                    p.extend(Rect::new(x, 0.0, x + 8.0, 8.0).to_path(0.01));
                }
                i += 1;
            }
        }
        p
    }

    #[test]
    fn characters_are_drawn_in_their_own_colours() {
        tracedraw_core::document::text_outline::set(square_letters);
        let red = Color::rgb8(255, 0, 0);
        let blue = Color::rgb8(0, 0, 255);
        let mut b = tracedraw_core::TextSpan::new("b", "Any", 12.0);
        b.fill = Some(Fill::Solid(red));
        let mut c = tracedraw_core::TextSpan::new("c", "Any", 12.0);
        c.outline = Some(tracedraw_core::Stroke::new(blue, 3.0));
        let mut t = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Text {
                spans: vec![
                    tracedraw_core::TextSpan::new("a", "Any", 12.0),
                    b,
                    c,
                    tracedraw_core::TextSpan::new("d", "Any", 12.0),
                ],
                origin: tracedraw_core::Point::ZERO,
                frame: None,
                align: Default::default(),
                para: Default::default(),
                on_path: None,
            },
        );
        t.transform = Affine::translate((10.0, 10.0));
        t.fill = Fill::Solid(Color::BLACK);
        t.stroke = None;
        let (doc, page) = doc_with(vec![t]);
        let opts = RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        let pm = render_page(&doc, page, &opts).unwrap();
        // Letters at x 10, 20, 30, 40 mm; their middles at y 86 on screen.
        assert_eq!(px(&pm, 14, 86), (0, 0, 0));
        assert_eq!(px(&pm, 24, 86), (255, 0, 0));
        assert_eq!(px(&pm, 34, 86), (0, 0, 0));
        // The third letter's own outline, on its edge.
        assert_eq!(px(&pm, 30, 86), (0, 0, 255));
        assert_eq!(px(&pm, 44, 86), (0, 0, 0));
        // Between the letters nothing is drawn.
        assert_eq!(pm.pixel(19, 86).unwrap().alpha(), 0);
    }

    /// A further fill paints over the fill, a further outline over the
    /// outline.
    #[test]
    fn further_fills_and_outlines_paint_over_the_first() {
        let mut r = rect(1, Rect::new(10.0, 10.0, 90.0, 90.0), Color::rgb8(255, 0, 0));
        r.extra_fills = vec![Fill::Solid(Color::rgb8(0, 0, 255))];
        r.stroke = Some(tracedraw_core::Stroke::new(Color::BLACK, 4.0));
        r.extra_strokes = vec![tracedraw_core::Stroke::new(Color::rgb8(0, 255, 0), 2.0)];
        let (doc, page) = doc_with(vec![r]);
        let opts = RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        let pm = render_page(&doc, page, &opts).unwrap();
        assert_eq!(px(&pm, 50, 50), (0, 0, 255));
        // On the edge the thin green outline lies over the black one.
        assert_eq!(px(&pm, 50, 89), (0, 255, 0));
        // Just outside it, the black one shows.
        assert_eq!(px(&pm, 50, 91), (0, 0, 0));
    }

    /// A clip frame as an opacity mask: the contents show where the
    /// frame is light, not where it is dark, and nothing outside it; the
    /// frame is not drawn. Inverted, the other way round.
    #[test]
    fn opacity_masks_show_the_contents_by_the_frame_brightness() {
        let red = Color::rgb8(255, 0, 0);
        let contents = rect(1, Rect::new(0.0, 0.0, 100.0, 100.0), red);
        // The frame: white on the left half, black on the right, covering
        // the middle 80 mm.
        let mut frame = rect(2, Rect::new(10.0, 10.0, 90.0, 90.0), Color::WHITE);
        frame.fill = Fill::Fountain(Fountain {
            kind: FountainKind::Linear,
            stops: vec![
                Stop::new(0.0, Color::WHITE),
                Stop::new(0.499, Color::WHITE),
                Stop::new(0.501, Color::BLACK),
                Stop::new(1.0, Color::BLACK),
            ],
            ..Fountain::two(FountainKind::Linear, Color::WHITE, Color::BLACK, 0.0)
        });
        let mut clip = Shape::new(
            tracedraw_core::ShapeId(3),
            ShapeKind::ClipFrame {
                frame: Box::new(frame),
                contents: vec![contents],
            },
        );
        clip.opacity_mask = Some(tracedraw_core::document::OpacityMask { invert: false });
        let opts = RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        let render = |clip: &Shape| {
            let (doc, page) = doc_with(vec![clip.clone()]);
            render_page(&doc, page, &opts).unwrap()
        };
        let pm = render(&clip);
        // White part: the red shows fully; black part and outside: nothing.
        assert_eq!(pm.pixel(25, 50).unwrap().alpha(), 255);
        assert_eq!(px(&pm, 25, 50), (255, 0, 0));
        assert_eq!(pm.pixel(75, 50).unwrap().alpha(), 0);
        assert_eq!(pm.pixel(5, 50).unwrap().alpha(), 0);
        clip.opacity_mask = Some(tracedraw_core::document::OpacityMask { invert: true });
        let pm = render(&clip);
        assert_eq!(pm.pixel(25, 50).unwrap().alpha(), 0);
        assert_eq!(pm.pixel(75, 50).unwrap().alpha(), 255);
        assert_eq!(pm.pixel(5, 50).unwrap().alpha(), 0);
    }

    /// The drag preview moves an object inside a moved group by the
    /// page-space transform, and only that object.
    #[test]
    fn the_drag_preview_reaches_objects_inside_groups() {
        let red = Color::rgb8(255, 0, 0);
        let blue = Color::rgb8(0, 0, 255);
        let a = rect(1, Rect::new(0.0, 0.0, 10.0, 10.0), red);
        let b = rect(2, Rect::new(20.0, 0.0, 30.0, 10.0), blue);
        let mut g = Shape::new(
            tracedraw_core::ShapeId(3),
            ShapeKind::Group {
                children: vec![a, b],
            },
        );
        g.transform = Affine::translate((10.0, 10.0));
        let (doc, page) = doc_with(vec![g]);
        let mut opts = RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        // Without a preview: red at page (15, 15), screen (15, 85).
        let pm = render_page(&doc, page, &opts).unwrap();
        assert_eq!(px(&pm, 15, 85), (255, 0, 0));
        opts.preview = Some(Preview {
            shapes: vec![tracedraw_core::ShapeId(1)],
            transform: Affine::translate((0.0, 40.0)),
        });
        let pm = render_page(&doc, page, &opts).unwrap();
        // The red square moved up by 40 mm; the blue one stayed.
        assert_eq!(pm.pixel(15, 85).unwrap().alpha(), 0);
        assert_eq!(px(&pm, 15, 45), (255, 0, 0));
        assert_eq!(px(&pm, 35, 85), (0, 0, 255));
    }

    #[test]
    fn open_curves_are_not_filled_unless_the_document_says_so() {
        use tracedraw_core::geometry::Point as P;
        // A "U" (open) and a triangle (closed) in one curve.
        let mut path = BezPath::new();
        path.move_to(P::new(10.0, 90.0));
        path.line_to(P::new(10.0, 10.0));
        path.line_to(P::new(40.0, 10.0));
        path.line_to(P::new(40.0, 90.0));
        path.move_to(P::new(60.0, 10.0));
        path.line_to(P::new(90.0, 10.0));
        path.line_to(P::new(75.0, 60.0));
        path.close_path();
        let closed = closed_subpaths(&path);
        assert_eq!(closed.elements().len(), 4);
        let mut s = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Path {
                path,
                closed: false,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = None;
        let (mut doc, page) = doc_with(vec![s]);
        let opts = RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        let pm = render_page(&doc, page, &opts).unwrap();
        // Inside the U (page 25, 50 -> screen 25, 50): nothing.
        assert_eq!(pm.pixel(25, 50).unwrap().alpha(), 0);
        // Inside the triangle (page 75, 20 -> screen 75, 80): red.
        assert_eq!(px(&pm, 75, 80), (255, 0, 0));
        doc.metadata.fill_open_curves = true;
        let pm = render_page(&doc, page, &opts).unwrap();
        assert_eq!(px(&pm, 25, 50), (255, 0, 0));
    }

    /// A live test of 0.41: on a rounded rectangle with a transparency,
    /// part of the outline vanished and the target (fill, outline or
    /// all) made no difference to the outline. The mask stopped at the
    /// object's box, so the outer half of the outline stayed opaque while
    /// the inner half and the corners faded. Now the whole outline fades
    /// with the outline or the whole object as the target, and keeps its
    /// colour with the fill alone.
    #[test]
    fn a_transparency_fades_the_part_its_target_names_outline_and_all() {
        use tracedraw_core::live::{Effect, MergeMode};
        let shape = |target: u8| {
            let mut s = Shape::new(
                tracedraw_core::ShapeId(1),
                ShapeKind::Rect {
                    rect: Rect::new(20.0, 20.0, 80.0, 60.0),
                    radius: 8.0,
                    corners: None,
                },
            );
            s.fill = Fill::Solid(Color::rgb8(0, 0, 255));
            s.stroke = Some(tracedraw_core::Stroke::new(Color::BLACK, 4.0));
            s.effects.push(Effect::Transparency {
                mask: Fill::Solid(Color::Gray { v: 0.5 }),
                merge: MergeMode::Normal,
                target,
            });
            s
        };
        let opts = RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        let alpha = |pm: &Pixmap, (x, y): (f64, f64)| {
            pm.pixel(x as u32, (100.0 - y) as u32).unwrap().alpha()
        };
        let half = |a: u8| (100..=160).contains(&a);
        // Page points: the outline's outer half on the top side (outside
        // the box), the round corner's outline past the fill, the
        // outline's inner half (over the fill), the fill.
        let (outer, corner, inner, fill) = ((50.0, 61.5), (21.8, 58.2), (50.0, 58.5), (50.0, 40.0));
        let colour = |pm: &Pixmap, (x, y): (f64, f64)| px(pm, x as u32, (100.0 - y) as u32);
        for target in [1u8, 2] {
            let (doc, page) = doc_with(vec![shape(target)]);
            let pm = render_page(&doc, page, &opts).unwrap();
            for p in [outer, corner] {
                let a = alpha(&pm, p);
                assert!(half(a), "target {target} at {p:?}: {a}");
                assert_eq!(colour(&pm, p), (0, 0, 0));
            }
            let (f, i) = (alpha(&pm, fill), alpha(&pm, inner));
            if target == 1 {
                // The fill as it is, the outline over it half see-through.
                assert_eq!((f, colour(&pm, fill)), (255, (0, 0, 255)));
                let (r, g, b) = colour(&pm, inner);
                assert!(i == 255 && r == 0 && g == 0 && half(b), "{r} {g} {b}");
            } else {
                // The whole object half see-through.
                assert!(half(f) && half(i), "{f} {i}");
                assert_eq!(colour(&pm, inner), (0, 0, 0));
            }
        }
        // The fill alone: the outline stays opaque all round.
        let (doc, page) = doc_with(vec![shape(0)]);
        let pm = render_page(&doc, page, &opts).unwrap();
        for p in [outer, corner, inner] {
            assert_eq!(alpha(&pm, p), 255, "at {p:?}");
            assert_eq!(colour(&pm, p), (0, 0, 0));
        }
        assert!(half(alpha(&pm, fill)));
        assert_eq!(colour(&pm, fill), (0, 0, 255));
    }

    /// A live test of 0.41: a variable outline on a closed object (an
    /// ellipse here) spread over the whole object, stepped and left an
    /// open node where the path closes. A node on the closing point (the
    /// right of the ellipse) is wide there on both sides of it, with no
    /// gap, and the outline tapers to its base width half way round.
    #[test]
    fn a_variable_outline_on_an_ellipse_tapers_across_its_closing_point() {
        use tracedraw_core::style::WidthNode;
        let mut s = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Ellipse {
                rect: Rect::new(20.0, 20.0, 80.0, 80.0),
                arc: None,
            },
        );
        s.fill = Fill::None;
        let mut st = tracedraw_core::Stroke::new(Color::BLACK, 2.0);
        st.widths = vec![WidthNode::symmetric(0.0, 8.0)];
        s.stroke = Some(st);
        let (doc, page) = doc_with(vec![s]);
        let opts = RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            ..RenderOptions::default()
        };
        let pm = render_page(&doc, page, &opts).unwrap();
        let ink = |x: f64, y: f64| pm.pixel(x as u32, (100.0 - y) as u32).unwrap().alpha() > 200;
        // At the closing point, (80, 50): 4 mm each way, just above and
        // just below it alike.
        for y in [47.5, 49.5, 50.5, 52.5] {
            assert!(ink(83.2, y) && ink(76.8, y), "at {y}");
            assert!(!ink(84.8, y), "at {y}");
        }
        // Half way round, (20, 50): the base width, 1 mm each way.
        assert!(ink(20.2, 50.5) && !ink(17.5, 50.5) && !ink(22.5, 50.5));
    }

    #[test]
    fn calligraphic_outline_is_wide_across_the_nib_and_thin_along_it() {
        // 25.4 dpi: one pixel per millimetre. Flat 6 mm nib at 0 degrees.
        let mut nib = tracedraw_core::Stroke::hairline(Color::BLACK);
        nib.width = 6.0;
        nib.stretch = 0.1;
        nib.nib_angle = 0.0;
        let mut vertical = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Path {
                path: {
                    let mut p = tracedraw_core::geometry::BezPath::new();
                    p.move_to(tracedraw_core::geometry::Point::new(30.0, 10.0));
                    p.line_to(tracedraw_core::geometry::Point::new(30.0, 90.0));
                    p
                },
                closed: false,
            },
        );
        vertical.fill = Fill::None;
        vertical.stroke = Some(nib.clone());
        let mut horizontal = vertical.clone();
        horizontal.id = tracedraw_core::ShapeId(2);
        horizontal.kind = ShapeKind::Path {
            path: {
                let mut p = tracedraw_core::geometry::BezPath::new();
                p.move_to(tracedraw_core::geometry::Point::new(10.0, 70.0));
                p.line_to(tracedraw_core::geometry::Point::new(90.0, 70.0));
                p
            },
            closed: false,
        };
        horizontal.stroke = Some(nib);
        let (doc, page) = doc_with(vec![vertical, horizontal]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        // The vertical stroke covers x 27..33 at y = 50 (image y = 50).
        assert_eq!(px(&pm, 28, 50), (0, 0, 0));
        assert_eq!(px(&pm, 32, 50), (0, 0, 0));
        assert_ne!(px(&pm, 25, 50), (0, 0, 0));
        // The horizontal stroke is only 0.6 mm tall: two pixels away it is gone.
        assert_ne!(px(&pm, 60, 28), (0, 0, 0));
        assert_ne!(px(&pm, 60, 32), (0, 0, 0));
    }

    #[test]
    fn invert_lens_inverts_what_is_beneath() {
        let base = rect(1, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb8(255, 0, 0));
        let mut lens = rect(2, Rect::new(25.0, 25.0, 75.0, 75.0), Color::WHITE);
        lens.effects.push(tracedraw_core::live::Effect::Lens(
            tracedraw_core::live::Lens::Invert,
        ));
        let (doc, page) = doc_with(vec![base, lens]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&pm, 50, 50), (0, 255, 255));
        assert_eq!(px(&pm, 5, 5), (255, 0, 0));
    }

    #[test]
    fn bitmap_effects_lens_runs_the_registered_effects_beneath_it() {
        // A stand-in for the application's effects: "swap" swaps red and
        // blue.
        fn run(effects: &[tracedraw_core::BitmapEffect], _w: u32, _h: u32, rgba: &mut Vec<u8>) {
            if effects.iter().any(|e| e.id == "swap") {
                for px in rgba.chunks_mut(4) {
                    px.swap(0, 2);
                }
            }
        }
        set_pixel_effects(run);
        let base = rect(1, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb8(255, 0, 0));
        let mut lens = rect(2, Rect::new(25.0, 25.0, 75.0, 75.0), Color::WHITE);
        lens.effects.push(tracedraw_core::live::Effect::Lens(
            tracedraw_core::live::Lens::Effects {
                effects: vec![tracedraw_core::BitmapEffect {
                    id: "swap".into(),
                    params: Default::default(),
                    visible: true,
                }],
            },
        ));
        let (doc, page) = doc_with(vec![base, lens]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&pm, 50, 50), (0, 0, 255));
        assert_eq!(px(&pm, 5, 5), (255, 0, 0));
    }

    #[test]
    fn simulated_overprint_multiplies_the_fill_over_what_is_beneath() {
        let yellow = rect(
            1,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Color::rgb8(255, 255, 0),
        );
        let mut grey = rect(
            2,
            Rect::new(25.0, 25.0, 75.0, 75.0),
            Color::rgb8(128, 128, 128),
        );
        grey.overprint_fill = true;
        let (doc, page) = doc_with(vec![yellow, grey]);
        // Off (the default): the grey knocks the yellow out.
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&pm, 50, 50), (128, 128, 128));
        // On: multiply gives a darker yellow, and the surroundings stay yellow.
        let opts = RenderOptions {
            simulate_overprints: true,
            ..RenderOptions::default()
        };
        let pm = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
        let (r, g, b) = px(&pm, 50, 50);
        assert!((120..=136).contains(&r) && r == g && b == 0, "{r} {g} {b}");
        assert_eq!(px(&pm, 5, 5), (255, 255, 0));
    }

    #[test]
    fn simulated_overprint_applies_to_outlines_and_leaves_plain_objects_alone() {
        let yellow = rect(
            1,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Color::rgb8(255, 255, 0),
        );
        // A thick grey outline across the page, no fill.
        let mut line = Shape::new(
            tracedraw_core::ShapeId(2),
            ShapeKind::Path {
                path: {
                    let mut p = tracedraw_core::geometry::BezPath::new();
                    p.move_to((0.0, 50.0));
                    p.line_to((100.0, 50.0));
                    p
                },
                closed: false,
            },
        );
        line.fill = Fill::None;
        line.stroke = Some(Stroke::new(Color::rgb8(128, 128, 128), 10.0));
        line.overprint_outline = true;
        // A plain grey square that does not overprint.
        let plain = rect(
            3,
            Rect::new(5.0, 5.0, 20.0, 20.0),
            Color::rgb8(128, 128, 128),
        );
        let (doc, page) = doc_with(vec![yellow, line, plain]);
        let opts = RenderOptions {
            simulate_overprints: true,
            ..RenderOptions::default()
        };
        let pm = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
        let (r, g, b) = px(&pm, 50, 50);
        assert!(
            (120..=136).contains(&r) && r == g && b == 0,
            "outline {r} {g} {b}"
        );
        assert_eq!(px(&pm, 12, 87), (128, 128, 128), "plain fill knocks out");
    }

    #[test]
    fn outlines_thinner_than_a_pixel_stay_one_pixel_wide() {
        // A 0.1 mm black line at 25.4 dpi (a tenth of a pixel): drawn as a
        // full-strength one-pixel line, not faded to a tenth.
        let mut line = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Path {
                path: {
                    let mut p = tracedraw_core::geometry::BezPath::new();
                    p.move_to((0.0, 50.5));
                    p.line_to((100.0, 50.5));
                    p
                },
                closed: false,
            },
        );
        line.fill = Fill::None;
        line.stroke = Some(Stroke::new(Color::BLACK, 0.1));
        let (doc, page) = doc_with(vec![line]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        let darkest = (0..100).map(|y| px(&pm, 50, y).0).min().unwrap_or(255);
        assert!(darkest < 64, "darkest {darkest}");
    }

    #[test]
    fn outlines_behind_the_fill_show_only_outside_it() {
        // A red square with a 10 mm white outline behind its fill: inside
        // the square stays red, the outline shows only outside.
        let mut s = rect(1, Rect::new(20.0, 20.0, 80.0, 80.0), Color::rgb8(255, 0, 0));
        let mut st = Stroke::new(Color::rgb8(0, 0, 255), 10.0);
        st.behind_fill = true;
        s.stroke = Some(st);
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&pm, 22, 50), (255, 0, 0), "fill over the inner half");
        assert_eq!(px(&pm, 17, 50), (0, 0, 255), "outline outside");
    }

    #[test]
    fn wireframe_draws_hairlines_whatever_the_outline() {
        // A red square with a 10 mm outline at half opacity.
        let mut s = rect(1, Rect::new(20.0, 20.0, 80.0, 80.0), Color::rgb8(255, 0, 0));
        s.stroke = Some(Stroke::new(Color::BLACK, 10.0));
        s.opacity = 0.5;
        let (doc, page) = doc_with(vec![s]);
        let opts = RenderOptions {
            wireframe: true,
            ..RenderOptions::default()
        };
        let pm = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
        assert_eq!(px(&pm, 50, 50), (255, 255, 255), "no fill");
        assert_eq!(px(&pm, 24, 50), (255, 255, 255), "no outline width");
        assert_eq!(px(&pm, 16, 50), (255, 255, 255), "no outline width outside");
        let edge = (18..23).map(|x| px(&pm, x, 50).0).min().unwrap();
        assert!(edge < 200, "a dark hairline on the edge, darkest {edge}");
        // The hairline is black whatever the outline's colour (#2041): a
        // blue outline gives no blue line, and a white one still shows.
        for colour in [Color::rgb8(0, 0, 255), Color::WHITE] {
            let mut s = rect(1, Rect::new(20.0, 20.0, 80.0, 80.0), Color::WHITE);
            s.stroke = Some(Stroke::new(colour, 10.0));
            let (doc, page) = doc_with(vec![s]);
            let pm = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
            let line: Vec<_> = (18..23).map(|x| px(&pm, x, 50)).collect();
            assert!(
                line.iter().all(|p| p.0 == p.1 && p.1 == p.2),
                "a grey to black line, no colour: {line:?}"
            );
            let darkest = line.iter().map(|p| p.0).min().unwrap();
            assert!(darkest < 200, "a dark hairline, darkest {darkest}");
        }
    }

    /// The Wireframe view cuts a clip frame's contents to the frame, as
    /// the other views do (#2146): a content reaching past the frame
    /// shows its line inside the frame only. It was drawn whole.
    #[test]
    fn wireframe_cuts_clip_frame_contents_to_the_frame() {
        let frame = rect(1, Rect::new(20.0, 20.0, 60.0, 60.0), Color::WHITE);
        // From inside the frame to far outside it at the right.
        let content = rect(2, Rect::new(40.0, 30.0, 90.0, 50.0), Color::rgb8(255, 0, 0));
        let clip = Shape::new(
            ShapeId(3),
            ShapeKind::ClipFrame {
                frame: Box::new(frame),
                contents: vec![content],
            },
        );
        let (doc, page) = doc_with(vec![clip]);
        let opts = RenderOptions {
            wireframe: true,
            ..RenderOptions::default()
        };
        let pm = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
        let dark = |x0: u32, x1: u32, y: u32| (x0..x1).any(|x| px(&pm, x, y).0 < 200);
        // Page y 50 is row 50 of the 100 px picture; the content's top and
        // bottom sides run along rows 50 and 70 (page y 50 and 30).
        assert!(
            dark(38, 43, 60),
            "the content's left side, inside the frame"
        );
        assert!(dark(58, 63, 60), "the frame's right side");
        assert!(!dark(64, 95, 60), "nothing of the content's far side");
        let row = |y: u32| (66..88).any(|x| px(&pm, x, y).0 < 200);
        assert!(
            !(45..75).any(row),
            "no content lines to the right of the frame"
        );
        assert_eq!(px(&pm, 50, 60), (255, 255, 255), "no fill");
    }

    /// A picture in the Wireframe view is a grey box crossed corner to
    /// corner, the mark of a picture left out, not a plain box (#2146).
    #[test]
    fn wireframe_marks_pictures_with_a_cross() {
        let mut photo = Pixmap::new(2, 2).unwrap();
        photo.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let png = photo.encode_png().unwrap();
        let bmp = Shape::new(
            ShapeId(1),
            ShapeKind::Bitmap {
                rect: Rect::new(20.0, 20.0, 80.0, 80.0),
                width_px: 2,
                height_px: 2,
                png: png.into(),
                fx: None,
            },
        );
        let (doc, page) = doc_with(vec![bmp]);
        let opts = RenderOptions {
            wireframe: true,
            ..RenderOptions::default()
        };
        let pm = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
        // Off the diagonals: the grey of the box.
        assert_eq!(px(&pm, 50, 30), (208, 208, 208));
        // On them, at a quarter and at the middle: a dark line.
        for (x, y) in [(35, 35), (65, 35), (50, 50)] {
            let darkest = (x - 1..=x + 1)
                .flat_map(|i| (y - 1..=y + 1).map(move |j| (i, j)))
                .map(|(i, j)| px(&pm, i, j).0)
                .min()
                .unwrap();
            assert!(darkest < 120, "a diagonal at {x}, {y}: {darkest}");
        }
    }

    #[test]
    fn complex_effects_off_skips_the_lens() {
        let base = rect(1, Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb8(255, 0, 0));
        let mut lens = rect(2, Rect::new(25.0, 25.0, 75.0, 75.0), Color::WHITE);
        lens.effects.push(tracedraw_core::live::Effect::Lens(
            tracedraw_core::live::Lens::Invert,
        ));
        let (doc, page) = doc_with(vec![base, lens]);
        let full = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&full, 50, 50), (0, 255, 255), "lens inverts");
        let opts = RenderOptions {
            complex_effects: false,
            ..RenderOptions::default()
        };
        let fast = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
        assert_eq!(
            px(&fast, 50, 50),
            (255, 255, 255),
            "plain white fill instead"
        );
        assert_eq!(px(&fast, 5, 5), (255, 0, 0));
        assert_ne!(full.data(), fast.data());
    }

    #[test]
    fn complex_effects_off_skips_the_transparency_mask() {
        let mut s = rect(1, Rect::new(0.0, 0.0, 100.0, 10.0), Color::rgb8(0, 0, 0));
        s.effects.push(tracedraw_core::live::Effect::Transparency {
            mask: Fill::linear(Color::WHITE, Color::BLACK, 0.0),
            merge: tracedraw_core::live::MergeMode::Normal,
            target: 2,
        });
        let (doc, page) = doc_with(vec![s]);
        let opts = RenderOptions {
            complex_effects: false,
            ..RenderOptions::default()
        };
        let pm = render_page_image_with(&doc, page, 25.4, &opts).unwrap();
        // Without the mask the bar is solid black from end to end.
        assert_eq!(px(&pm, 3, 95), (0, 0, 0));
        assert_eq!(px(&pm, 96, 95), (0, 0, 0));
    }

    #[test]
    fn custom_arrowhead_paints_the_line_end() {
        // A right-pointing triangle, 20 wide and 20 tall, as a custom head.
        let mut tri = tracedraw_core::geometry::BezPath::new();
        tri.move_to((0.0, 0.0));
        tri.line_to((20.0, 10.0));
        tri.line_to((0.0, 20.0));
        tri.close_path();
        let head = tracedraw_core::Arrowhead::from_shape_path(&tri, "Tri");
        let mut line = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Path {
                path: {
                    let mut p = tracedraw_core::geometry::BezPath::new();
                    p.move_to((10.0, 50.0));
                    p.line_to((60.0, 50.0));
                    p
                },
                closed: false,
            },
        );
        line.fill = Fill::None;
        line.stroke = Some(Stroke {
            end_arrow: head,
            ..Stroke::new(Color::rgb8(0, 0, 255), 2.0)
        });
        let (doc, page) = doc_with(vec![line]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        // Size = 4 widths = 8 mm: the head spans x 52..60, y 46..54. Its
        // base is 8 mm tall at x = 52, well outside the 2 mm line; its tip
        // is at x = 60.
        let blue = |x: u32, y: u32| {
            let (r, g, b) = px(&pm, x, y);
            assert!(
                b == 255 && r < 40 && g < 40,
                "({x}, {y}) should be blue: {r} {g} {b}"
            );
        };
        blue(53, 48);
        blue(53, 52);
        blue(58, 50);
        // Beside the line, before the head, there is nothing.
        assert_eq!(px(&pm, 40, 47), (255, 255, 255));
        // Beyond the tip, nothing either.
        assert_eq!(px(&pm, 63, 50), (255, 255, 255));
        // Without a head the same spots beside the line end stay white.
        let mut plain = doc.clone();
        if let Some(s) = plain.pages[0].layers[0].shapes.first_mut() {
            if let Some(st) = s.stroke.as_mut() {
                st.end_arrow = tracedraw_core::Arrowhead::None;
            }
        }
        let pm2 = render_page_image(&plain, page, 25.4).unwrap();
        assert_eq!(px(&pm2, 53, 48), (255, 255, 255));
        assert_eq!(px(&pm2, 53, 52), (255, 255, 255));
    }

    /// A symmetry's mirrored copy is as see-through as the object (#2121):
    /// the copies were drawn opaque, with fill and outline only.
    #[test]
    fn a_mirrored_copy_keeps_the_transparency() {
        let mut s = rect(1, Rect::new(10.0, 0.0, 30.0, 10.0), Color::rgb8(0, 0, 0));
        s.effects.push(tracedraw_core::live::Effect::Symmetry {
            center: tracedraw_core::geometry::Point::new(50.0, 0.0),
            angle: 90.0,
            lines: 1,
        });
        s.effects.push(tracedraw_core::live::Effect::Transparency {
            mask: Fill::Solid(Color::rgb8(128, 128, 128)),
            merge: tracedraw_core::live::MergeMode::Normal,
            target: 2,
        });
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        let original = px(&pm, 20, 95).0;
        let copy = px(&pm, 80, 95).0;
        assert!(
            (90..170).contains(&original),
            "the object is half see-through: {original}"
        );
        assert!(
            original.abs_diff(copy) <= 2,
            "the copy as the object: {original} against {copy}"
        );
    }

    /// An inner shadow darkens the inside of its object along the edges
    /// facing away from its offset, and nothing outside it (#2473).
    #[test]
    fn an_inner_shadow_darkens_the_inside_away_from_its_offset() {
        let mut s = rect(1, Rect::new(20.0, 20.0, 80.0, 80.0), Color::WHITE);
        s.effects.push(tracedraw_core::live::Effect::InnerShadow {
            offset: tracedraw_core::geometry::Vec2::new(6.0, -6.0),
            color: Color::BLACK,
            opacity: 1.0,
            blur: 0.0,
            merge: tracedraw_core::live::MergeMode::Normal,
        });
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        // Page y grows upwards; the picture's rows grow downwards.
        assert!(px(&pm, 23, 50).0 < 30, "left inside is dark");
        assert!(px(&pm, 50, 23).0 < 30, "top inside is dark");
        assert_eq!(px(&pm, 76, 50), (255, 255, 255), "right inside is lit");
        assert_eq!(px(&pm, 50, 76), (255, 255, 255), "bottom inside is lit");
        assert_eq!(px(&pm, 50, 50), (255, 255, 255), "middle is lit");
        assert_eq!(px(&pm, 15, 50), (255, 255, 255), "outside is untouched");
        // Feathered, the edge fades into the middle.
        let mut f = rect(1, Rect::new(20.0, 20.0, 80.0, 80.0), Color::WHITE);
        f.effects.push(tracedraw_core::live::Effect::InnerShadow {
            offset: tracedraw_core::geometry::Vec2::new(6.0, -6.0),
            color: Color::BLACK,
            opacity: 1.0,
            blur: 4.0,
            merge: tracedraw_core::live::MergeMode::Normal,
        });
        let (doc, page) = doc_with(vec![f]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        let (edge, mid) = (px(&pm, 22, 50).0, px(&pm, 30, 50).0);
        assert!(edge < mid && mid < 255, "fades from {edge} to {mid}");
        assert_eq!(px(&pm, 15, 50), (255, 255, 255), "outside is untouched");
    }

    #[test]
    fn fountain_transparency_fades_the_object() {
        let mut s = rect(1, Rect::new(0.0, 0.0, 100.0, 10.0), Color::rgb8(0, 0, 0));
        s.effects.push(tracedraw_core::live::Effect::Transparency {
            mask: Fill::linear(Color::WHITE, Color::BLACK, 0.0),
            merge: tracedraw_core::live::MergeMode::Normal,
            target: 2,
        });
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        let left = px(&pm, 3, 95).0;
        let right = px(&pm, 96, 95).0;
        assert!(left < 40, "left should be opaque black: {left}");
        assert!(
            right > 215,
            "right should be transparent (white page): {right}"
        );
    }

    /// Each merge mode mixes a source drawn through an opaque mask with
    /// the backdrop as its formula says (source and backdrop in 0..1).
    #[test]
    fn merge_modes_mix_with_the_backdrop_as_their_formulas_say() {
        use tracedraw_core::live::MergeMode as M;
        let cb = [0.6f32, 0.4, 0.2];
        let cs = [0.5f32, 0.8, 0.25];
        let to8 = |v: f32| (v * 255.0).round() as u8;
        let backdrop = rect(
            1,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Color::rgb8(to8(cb[0]), to8(cb[1]), to8(cb[2])),
        );
        let screen = |b: f32, s: f32| b + s - b * s;
        let hard_light = |b: f32, s: f32| {
            if s <= 0.5 {
                b * 2.0 * s
            } else {
                screen(b, 2.0 * s - 1.0)
            }
        };
        let soft_light = |b: f32, s: f32| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        };
        let dodge = |b: f32, s: f32| {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        };
        let burn = |b: f32, s: f32| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        };
        let lum = 0.2126 * cs[0] + 0.7152 * cs[1] + 0.0722 * cs[2];
        let per_channel =
            |f: &dyn Fn(f32, f32) -> f32| [f(cb[0], cs[0]), f(cb[1], cs[1]), f(cb[2], cs[2])];
        let cases: Vec<(M, [f32; 3])> = vec![
            (M::Screen, per_channel(&screen)),
            (M::Overlay, per_channel(&|b, s| hard_light(s, b))),
            (M::SoftLight, per_channel(&soft_light)),
            (M::HardLight, per_channel(&hard_light)),
            (M::ColorDodge, per_channel(&dodge)),
            (M::ColorBurn, per_channel(&burn)),
            (M::Exclusion, per_channel(&|b, s| b + s - 2.0 * b * s)),
            (M::Texturize, [cb[0] * lum, cb[1] * lum, cb[2] * lum]),
            // Backdrop and source added, less white (#2512).
            (M::Subtract, per_channel(&|b, s| (b + s - 1.0).max(0.0))),
            // Hue 30 degrees and saturation 0.5 of the backdrop at the
            // source's lightness (0.8 + 0.25) / 2.
            (M::Lightness, [0.7625, 0.525, 0.2875]),
            // Nothing shows over an opaque backdrop.
            (M::Behind, cb),
        ];
        for (merge, want) in cases {
            let mut top = rect(
                2,
                Rect::new(20.0, 20.0, 80.0, 80.0),
                Color::rgb8(to8(cs[0]), to8(cs[1]), to8(cs[2])),
            );
            top.effects
                .push(tracedraw_core::live::Effect::Transparency {
                    mask: Fill::Solid(Color::WHITE),
                    merge,
                    target: 2,
                });
            let (doc, page) = doc_with(vec![backdrop.clone(), top]);
            let pm = render_page_image(&doc, page, 25.4).unwrap();
            let got = px(&pm, 50, 50);
            let want = (to8(want[0]), to8(want[1]), to8(want[2]));
            for (g, w) in [got.0, got.1, got.2]
                .into_iter()
                .zip([want.0, want.1, want.2])
            {
                assert!(
                    (g as i32 - w as i32).abs() <= 3,
                    "{merge:?}: got {got:?}, want {want:?}"
                );
            }
            // Outside the source the backdrop is untouched.
            assert_eq!(
                px(&pm, 5, 5),
                (to8(cb[0]), to8(cb[1]), to8(cb[2])),
                "{merge:?}"
            );
        }
    }

    /// Subtract over coloured artwork (#2512): a white source leaves the
    /// artwork as it is and a grey one darkens it, as Multiply does; it
    /// no longer turns the artwork into its negative.
    #[test]
    fn subtract_darkens_the_backdrop_by_the_source() {
        let mut target = Pixmap::new(2, 1).unwrap();
        let mut layer = Pixmap::new(2, 1).unwrap();
        for p in target.pixels_mut() {
            *p = tiny_skia::ColorU8::from_rgba(40, 120, 220, 255).premultiply();
        }
        layer.pixels_mut()[0] = tiny_skia::ColorU8::from_rgba(255, 255, 255, 255).premultiply();
        layer.pixels_mut()[1] = tiny_skia::ColorU8::from_rgba(200, 200, 200, 255).premultiply();
        composite_layer(
            &mut target,
            &layer,
            (0, 0),
            tracedraw_core::live::MergeMode::Subtract,
        );
        assert_eq!(px(&target, 0, 0), (40, 120, 220));
        assert_eq!(px(&target, 1, 0), (0, 65, 165));
    }

    /// Behind shows the source only where the backdrop is clear.
    #[test]
    fn behind_draws_only_where_the_backdrop_is_clear() {
        let mut target = Pixmap::new(4, 1).unwrap();
        let mut layer = Pixmap::new(4, 1).unwrap();
        let red = tiny_skia::ColorU8::from_rgba(255, 0, 0, 255).premultiply();
        let blue = tiny_skia::ColorU8::from_rgba(0, 0, 255, 255).premultiply();
        let half_blue = tiny_skia::ColorU8::from_rgba(0, 0, 255, 128).premultiply();
        target.pixels_mut()[0] = red;
        target.pixels_mut()[2] = half_blue;
        for p in layer.pixels_mut() {
            *p = blue;
        }
        composite_layer(
            &mut target,
            &layer,
            (0, 0),
            tracedraw_core::live::MergeMode::Behind,
        );
        // Over the opaque red: red. Over nothing: the source. Over the half
        // blue: blue filled in to opaque.
        assert_eq!(px(&target, 0, 0), (255, 0, 0));
        assert_eq!(px(&target, 1, 0), (0, 0, 255));
        assert_eq!(target.pixel(2, 0).unwrap().alpha(), 255);
        assert_eq!(px(&target, 2, 0), (0, 0, 255));
    }

    /// Subtract, Texturize and Lightness, drawn pixel by pixel, keep the
    /// alpha compositing of the blend modes: a half-clear source over a
    /// clear backdrop is the source at half alpha, over an opaque one the
    /// mix.
    #[test]
    fn pixel_merges_composite_alpha_like_blend_modes() {
        use tracedraw_core::live::MergeMode as M;
        for merge in [M::Subtract, M::Texturize, M::Lightness] {
            let mut target = Pixmap::new(2, 1).unwrap();
            let mut layer = Pixmap::new(2, 1).unwrap();
            target.pixels_mut()[1] = tiny_skia::ColorU8::from_rgba(200, 100, 50, 255).premultiply();
            for p in layer.pixels_mut() {
                *p = tiny_skia::ColorU8::from_rgba(255, 255, 255, 128).premultiply();
            }
            composite_layer(&mut target, &layer, (0, 0), merge);
            let clear = target.pixel(0, 0).unwrap();
            assert_eq!(clear.alpha(), 128, "{merge:?}");
            assert_eq!(px(&target, 0, 0), (255, 255, 255), "{merge:?}");
            let over = target.pixel(1, 0).unwrap();
            assert_eq!(over.alpha(), 255, "{merge:?}");
            // White at half strength: Subtract and Texturize leave the
            // backdrop (white adds 1 and takes 1 away; its lightness is
            // 1), Lightness lifts it half way to white.
            let got = px(&target, 1, 0);
            let want = match merge {
                M::Subtract | M::Texturize => (200, 100, 50),
                _ => (228, 178, 153),
            };
            for (g, w) in [got.0, got.1, got.2]
                .into_iter()
                .zip([want.0, want.1, want.2])
            {
                assert!(
                    (g as i32 - w as i32).abs() <= 2,
                    "{merge:?}: {got:?} vs {want:?}"
                );
            }
        }
    }

    #[test]
    fn shaped_transparency_follows_the_object() {
        // White at the edge (opaque), black inside (clear): a black disc
        // fades towards its middle, the same all round.
        let id = tracedraw_core::ShapeId(1);
        let mut s = Shape::new(
            id,
            ShapeKind::Ellipse {
                rect: Rect::new(0.0, 0.0, 60.0, 60.0),
                arc: None,
            },
        );
        s.fill = Fill::Solid(Color::BLACK);
        s.stroke = None;
        s.effects.push(tracedraw_core::live::Effect::Transparency {
            mask: Fill::Fountain(Fountain::two(
                FountainKind::Shaped,
                Color::WHITE,
                Color::BLACK,
                0.0,
            )),
            merge: tracedraw_core::live::MergeMode::Normal,
            target: 2,
        });
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        let (centre, east, north) = (px(&pm, 30, 70).0, px(&pm, 52, 70).0, px(&pm, 30, 48).0);
        assert!(centre > 215, "the middle is clear: {centre}");
        assert!(
            east < 120 && (east as i32 - north as i32).abs() < 12,
            "{east} {north}"
        );
    }

    #[test]
    fn contour_effect_draws_outside_steps() {
        let mut s = rect(1, Rect::new(40.0, 40.0, 60.0, 60.0), Color::rgb8(255, 0, 0));
        s.effects.push(tracedraw_core::live::Effect::Contour {
            steps: 2,
            offset: 5.0,
            outside: true,
            to_center: false,
            fill_to: Color::rgb8(0, 0, 255),
            outline_to: None,
            color_blend: 0,
            corners: None,
            accel_objects: 0.0,
            accel_colors: 0.0,
        });
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        // y is flipped in the image: page y 50 is image row 50 either way (100 tall).
        assert_eq!(px(&pm, 50, 50), (255, 0, 0));
        let (r, _, b) = px(&pm, 32, 50);
        assert!(b > r, "outer contour step should be bluish");
        assert_eq!(px(&pm, 5, 5), (255, 255, 255));
    }

    /// Smooth colour carries a bright middle node's colour further into
    /// its cells than the bilinear mix does.
    #[test]
    fn smooth_mesh_colour_flows_through_the_nodes() {
        let render = |smooth: bool| {
            let mut s = rect(1, Rect::new(0.0, 0.0, 100.0, 100.0), Color::WHITE);
            let mut m =
                tracedraw_core::Mesh::new(Rect::new(0.0, 0.0, 100.0, 100.0), 2, 2, Color::BLACK);
            m.nodes[4].color = Color::WHITE; // the middle node
            m.smooth = smooth;
            s.fill = Fill::Mesh(m);
            let (doc, page) = doc_with(vec![s]);
            render_page_image(&doc, page, 25.4).unwrap()
        };
        let (flat, smooth) = (render(false), render(true));
        let (f, _, _) = px(&flat, 30, 50);
        let (g, _, _) = px(&smooth, 30, 50);
        assert!(g > f + 8, "smooth {g} should be brighter than bilinear {f}");
        // Next to the middle node it is nearly white either way.
        assert!(px(&smooth, 50, 50).0 > 200 && px(&flat, 50, 50).0 > 200);
    }

    /// A mesh whose grid line bends (the nodes carry handles) fills the
    /// bulge, and a pan reuses the picture worked out before (#896).
    #[test]
    fn curved_mesh_cells_follow_their_handles() {
        use tracedraw_core::MeshHandle;
        let area = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut m = tracedraw_core::Mesh::new(Rect::new(0.0, 0.0, 50.0, 100.0), 1, 1, Color::BLACK);
        // The right edge bows out to x = 80 in the middle.
        m.nodes[1].set_handle(MeshHandle::Down, Point::new(90.0, 33.0));
        m.nodes[3].set_handle(MeshHandle::Up, Point::new(90.0, 67.0));
        let mut s = rect(1, area, Color::WHITE);
        s.kind = ShapeKind::Path {
            path: {
                let mut p = BezPath::new();
                p.move_to((0.0, 0.0));
                p.line_to((50.0, 0.0));
                p.curve_to((90.0, 33.0), (90.0, 67.0), (50.0, 100.0));
                p.line_to((0.0, 100.0));
                p.close_path();
                p
            },
            closed: true,
        };
        s.fill = Fill::Mesh(m.clone());
        let (doc, page) = doc_with(vec![s.clone()]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        // Inside the bulge the mesh is black; the straight grid would
        // leave it to the path's white.
        let (r, g, b) = px(&pm, 70, 50);
        assert!(
            r < 40 && g < 40 && b < 40,
            "bulge should be black: {r} {g} {b}"
        );
        let (r, _, _) = px(&pm, 95, 50);
        assert!(r > 200, "outside the bulge is white: {r}");
        let p = m.patch_point(0, 0, 1.0, 0.5).unwrap();
        assert!(
            (p.x - 80.0).abs() < 1e-6 && (p.y - 50.0).abs() < 1e-6,
            "{p:?}"
        );
        // The same mesh moved: the picture is found in the cache.
        let mut moved = s.clone();
        moved.transform = tracedraw_core::geometry::Affine::translate((10.0, 0.0));
        let (doc2, page2) = doc_with(vec![moved]);
        // (Other tests share the cache: only this mesh's pictures count,
        // told by their size.)
        let mine = || {
            procedural_cache_keys()
                .into_iter()
                .filter(|k| k.starts_with("mesh|") && k.ends_with("|95,105,1"))
                .count()
        };
        let n = mine();
        assert!(n >= 1, "{:?}", procedural_cache_keys());
        let pm2 = render_page_image(&doc2, page2, 25.4).unwrap();
        assert_eq!(mine(), n);
        let (r, _, _) = px(&pm2, 80, 50);
        assert!(r < 40, "moved bulge should be black: {r}");
    }

    /// A mesh over part of its object leaves the rest of the path clear
    /// (nothing is drawn outside the grid), also on screen where a large
    /// picture is worked out on a coarser grid (steps 2 and 3).
    #[test]
    fn nothing_shows_outside_a_mesh_grid() {
        let mut s = rect(1, Rect::new(0.0, 0.0, 100.0, 100.0), Color::WHITE);
        s.fill = Fill::Mesh(tracedraw_core::Mesh::new(
            Rect::new(0.0, 0.0, 50.0, 100.0),
            2,
            2,
            Color::BLACK,
        ));
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert!(px(&pm, 25, 50).0 < 40);
        for x in [55, 75, 95] {
            let (r, g, b) = px(&pm, x, 50);
            assert!(r > 250 && g > 250 && b > 250, "{x}: {r} {g} {b}");
        }
        // A page of 200 x 150 mm with the grid over 20..100 x 20..130 mm
        // of a path over the whole page: at 8 px/mm the picture is 640 x
        // 880 px (step 2), at 14 px/mm 1120 x 1540 px (step 3).
        for zoom in [8.0, 14.0] {
            let mut s = rect(1, Rect::new(0.0, 0.0, 200.0, 150.0), Color::WHITE);
            s.fill = Fill::Mesh(tracedraw_core::Mesh::new(
                Rect::new(20.0, 20.0, 100.0, 130.0),
                3,
                3,
                Color::BLACK,
            ));
            let (doc, page) = doc_with(vec![s]);
            let (w, h) = ((200.0 * zoom) as u32, (150.0 * zoom) as u32);
            let opts = RenderOptions {
                width: w,
                height: h,
                view: ViewTransform {
                    zoom,
                    origin_x: 0.0,
                    origin_y: h as f64,
                },
                draft: true,
                ..Default::default()
            };
            let pm = render_page(&doc, page, &opts).unwrap();
            // (No page is painted here: outside the grid the pixels stay
            // clear.)
            let at = |x_mm: f64, y_mm: f64| {
                pm.pixel((x_mm * zoom) as u32, (h as f64 - y_mm * zoom) as u32)
                    .unwrap()
                    .demultiply()
            };
            let inside = at(60.0, 75.0);
            assert!(
                inside.alpha() == 255 && inside.red() < 40,
                "zoom {zoom}: inside the grid {inside:?}"
            );
            for (x, y) in [
                (10.0, 75.0),
                (150.0, 75.0),
                (60.0, 10.0),
                (60.0, 140.0),
                (3.0, 3.0),
            ] {
                let c = at(x, y);
                assert!(
                    c.alpha() == 0,
                    "zoom {zoom}: ({x}, {y}) mm outside the grid is {c:?}"
                );
            }
        }
    }

    /// A large render is drawn in bands on several threads: the bands
    /// share one mesh picture (worked out once, one cache entry), rather
    /// than each working it out.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn band_threads_share_one_mesh_picture() {
        let mut s = rect(1, Rect::new(0.0, 0.0, 180.0, 180.0), Color::WHITE);
        s.fill = Fill::Mesh(tracedraw_core::Mesh::new(
            Rect::new(0.0, 0.0, 180.0, 180.0),
            4,
            4,
            Color::rgb8(10, 20, 30),
        ));
        let (doc, page) = doc_with(vec![s]);
        let (w, h) = (1300u32, 1300u32);
        let opts = RenderOptions {
            width: w,
            height: h,
            view: ViewTransform {
                zoom: 7.0,
                origin_x: 20.0,
                origin_y: 1280.0,
            },
            ..Default::default()
        };
        let before: Vec<String> = procedural_cache_keys();
        let pm = render_page(&doc, page, &opts).unwrap();
        assert_eq!(px(&pm, 650, 650), (10, 20, 30));
        // (Other tests share the cache: this mesh's pictures are told by
        // their size.)
        let new: Vec<String> = procedural_cache_keys()
            .into_iter()
            .filter(|k| {
                k.starts_with("mesh|") && k.ends_with("|1265,1265,1") && !before.contains(k)
            })
            .collect();
        assert_eq!(new.len(), 1, "{new:?}");
        let mut sorted = procedural_cache_keys();
        let n = sorted.len();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "a key kept twice");
    }

    /// A 30 x 30 mesh zoomed until each cell is 670 pixels across costs
    /// only what shows on a 1920 x 1080 picture: cells and pieces off the
    /// picture are left out and the triangles come in bounded batches,
    /// where the whole mesh would be some 50 million triangles (and
    /// gigabytes) at once.
    #[test]
    fn a_zoomed_mesh_costs_only_what_shows() {
        let m = tracedraw_core::Mesh::new(Rect::new(0.0, 0.0, 200.0, 200.0), 30, 30, Color::BLACK);
        let (w, h) = (1920usize, 1080usize);
        let (mut total, mut biggest, mut batches) = (0usize, 0usize, 0usize);
        mesh_triangles(&m, Affine::scale(100.0), w, h, |t| {
            total += t.len();
            biggest = biggest.max(t.len());
            batches += 1;
        });
        // About two triangles per 16 pixels of the picture.
        assert!(total > 100_000 && total < 600_000, "{total}");
        assert!(biggest <= MESH_TRIANGLE_BATCH + 2, "{biggest}");
        assert!(batches >= 2);
        // Off the picture entirely: nothing.
        let mut none = 0;
        mesh_triangles(
            &m,
            Affine::translate((-30_000.0, 0.0)) * Affine::scale(100.0),
            w,
            h,
            |t| none += t.len(),
        );
        assert_eq!(none, 0);
        // The same mesh drawn on a page: the picture is bounded by the
        // canvas and the cells on it are painted.
        let mut s = rect(1, Rect::new(0.0, 0.0, 200.0, 200.0), Color::WHITE);
        s.fill = Fill::Mesh(m);
        let (doc, page) = doc_with(vec![s]);
        let opts = RenderOptions {
            width: w as u32,
            height: h as u32,
            view: ViewTransform {
                zoom: 100.0,
                origin_x: 0.0,
                origin_y: 1080.0,
            },
            draft: true,
            ..Default::default()
        };
        let pm = render_page(&doc, page, &opts).unwrap();
        assert!(px(&pm, 960, 540).0 < 40);
    }

    /// Peak memory and time of the zoomed 30 x 30 and 10 x 10 meshes
    /// (`cargo test --release -p tracedraw-render -- --ignored mesh_memory --nocapture`).
    #[test]
    #[ignore]
    fn mesh_memory_at_high_zoom() {
        let peak = || {
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("VmHWM"))
                        .map(|l| l.trim().to_string())
                })
                .unwrap_or_default()
        };
        for n in [10u32, 30] {
            let m =
                tracedraw_core::Mesh::new(Rect::new(0.0, 0.0, 200.0, 200.0), n, n, Color::BLACK);
            let mut s = rect(1, Rect::new(0.0, 0.0, 200.0, 200.0), Color::WHITE);
            s.fill = Fill::Mesh(m);
            let (doc, page) = doc_with(vec![s]);
            let opts = RenderOptions {
                width: 1920,
                height: 1080,
                view: ViewTransform {
                    zoom: 100.0,
                    origin_x: 0.0,
                    origin_y: 1080.0,
                },
                draft: true,
                ..Default::default()
            };
            let t = std::time::Instant::now();
            let _ = render_page(&doc, page, &opts).unwrap();
            eprintln!("{n}x{n}: {:?}, peak {}", t.elapsed(), peak());
        }
    }

    #[test]
    fn mesh_fill_blends_corner_colours() {
        let mut s = rect(1, Rect::new(0.0, 0.0, 100.0, 100.0), Color::WHITE);
        let mut m =
            tracedraw_core::Mesh::new(Rect::new(0.0, 0.0, 100.0, 100.0), 1, 1, Color::WHITE);
        m.nodes[0].color = Color::rgb8(255, 0, 0); // bottom-left
        m.nodes[1].color = Color::rgb8(0, 0, 255); // bottom-right
        m.nodes[2].color = Color::rgb8(255, 0, 0);
        m.nodes[3].color = Color::rgb8(0, 0, 255);
        s.fill = Fill::Mesh(m);
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        let (r, _, b) = px(&pm, 5, 50);
        assert!(r > 200 && b < 60, "left should be red: {r} {b}");
        let (r, _, b) = px(&pm, 95, 50);
        assert!(b > 200 && r < 60, "right should be blue: {r} {b}");
        let (r, _, b) = px(&pm, 50, 50);
        assert!(
            (r as i32 - b as i32).abs() < 40,
            "middle should be purple: {r} {b}"
        );
    }

    /// A fill whose nodes were placed freely runs from its start to its
    /// end node, held at the end colours beyond them; an elliptical one
    /// is centred on its start (#330).
    #[test]
    fn node_opacity_shows_through_the_gradient() {
        let b = Rect::new(0.0, 0.0, 100.0, 10.0);
        let mut f = Fountain::two(FountainKind::Linear, Color::BLACK, Color::BLACK, 0.0);
        f.stops[1].opacity = 0.0;
        for kind in [FountainKind::Linear, FountainKind::Square] {
            f.kind = kind;
            let pm = render_fill_image(&Fill::Fountain(f.clone()), b, 25.4).unwrap();
            let a = |x: u32| pm.pixel(x, 5).map(|p| p.alpha()).unwrap_or(0);
            if kind == FountainKind::Linear {
                assert!(a(2) > 240, "opaque start: {}", a(2));
                assert!(a(97) < 15, "clear end: {}", a(97));
            } else {
                // Square: opaque at the centre, clear at the corners.
                assert!(a(50) > a(1), "{} {}", a(50), a(1));
            }
        }
    }

    #[test]
    fn free_fill_nodes_place_the_gradient() {
        use tracedraw_core::style::FountainHandles;
        let b = Rect::new(0.0, 0.0, 100.0, 10.0);
        let rel = |x: f64, y: f64| Fountain::page_to_rel(b, Point::new(x, y));
        let mut f = Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        f.set_handles(
            b,
            FountainHandles {
                start: rel(20.0, 5.0),
                end: rel(60.0, 5.0),
                skew: rel(20.0, 10.0),
            },
        );
        let pm = render_fill_image(&Fill::Fountain(f.clone()), b, 25.4).unwrap();
        let (r, _, _) = px(&pm, 10, 5);
        assert!(r < 10, "before the start: {r}");
        let (r, _, _) = px(&pm, 40, 5);
        assert!((110..150).contains(&r), "half way: {r}");
        let (r, _, _) = px(&pm, 80, 5);
        assert!(r > 245, "beyond the end: {r}");
        // Elliptical, centred at x = 70.
        f.kind = FountainKind::Radial;
        f.set_handles(
            b,
            FountainHandles {
                start: rel(70.0, 5.0),
                end: rel(90.0, 5.0),
                skew: rel(70.0, 25.0),
            },
        );
        let pm = render_fill_image(&Fill::Fountain(f.clone()), b, 25.4).unwrap();
        let (r, _, _) = px(&pm, 70, 5);
        assert!(r < 20, "the centre: {r}");
        let (r, _, _) = px(&pm, 20, 5);
        assert!(r > 245, "far out: {r}");
        // Square, procedural: centre dark, beyond the half axis light.
        f.kind = FountainKind::Square;
        let pm = render_fill_image(&Fill::Fountain(f), b, 25.4).unwrap();
        let (r, _, _) = px(&pm, 70, 5);
        assert!(r < 30, "square centre: {r}");
        let (r, _, _) = px(&pm, 5, 5);
        assert!(r > 240, "square outside: {r}");
    }

    #[test]
    fn linear_fountain_three_stops_passes_through_middle_colour() {
        let fill = Fill::Fountain(Fountain {
            kind: FountainKind::Linear,
            stops: vec![
                Stop::new(0.0, Color::rgb8(0, 0, 0)),
                Stop::new(0.5, Color::rgb8(255, 0, 0)),
                Stop::new(1.0, Color::rgb8(255, 255, 255)),
            ],
            angle: 0.0,
            offset: Point::ZERO,
            edge_pad: 0.0,
            ..Default::default()
        });
        let pm = render_fill_image(&fill, Rect::new(0.0, 0.0, 100.0, 10.0), 25.4).unwrap();
        assert_eq!(pm.width(), 100);
        let (r, g, b) = px(&pm, 50, 5);
        assert!(r > 240 && g < 20 && b < 20, "middle {r} {g} {b}");
        let (r, _, _) = px(&pm, 2, 5);
        assert!(r < 20);
        let (_, g, _) = px(&pm, 97, 5);
        assert!(g > 230);
    }

    #[test]
    fn conical_fountain_differs_around_centre() {
        let fill = Fill::Fountain(Fountain::two(
            FountainKind::Conical,
            Color::rgb8(0, 0, 255),
            Color::rgb8(255, 255, 0),
            0.0,
        ));
        let pm = render_fill_image(&fill, Rect::new(0.0, 0.0, 100.0, 100.0), 25.4).unwrap();
        let a = px(&pm, 90, 50);
        let b = px(&pm, 10, 50);
        assert_ne!(a, b);
    }

    #[test]
    fn checker_pattern_alternates() {
        let fill = Fill::Pattern(Pattern::TwoColor {
            tile: PatternTile::Checker,
            front: Color::rgb8(0, 0, 0),
            back: Color::rgb8(255, 255, 255),
            size_mm: 20.0,
            placement: Default::default(),
        });
        let pm = render_fill_image(&fill, Rect::new(0.0, 0.0, 40.0, 40.0), 25.4).unwrap();
        let a = px(&pm, 5, 5);
        let b = px(&pm, 15, 5);
        assert_ne!(a.0, b.0, "checker cells should differ");
        assert!(a.0 == 0 || a.0 == 255);
    }

    #[test]
    fn vector_pattern_tiles_the_shape() {
        // A 10 mm tile with a red square in its lower-left quarter.
        let tile_square = rect(7, Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb8(255, 0, 0));
        let mut s = rect(1, Rect::new(0.0, 0.0, 20.0, 20.0), Color::WHITE);
        s.fill = Fill::Pattern(Pattern::Vector {
            shapes: vec![tile_square],
            tile: tracedraw_core::geometry::Size::new(10.0, 10.0),
            placement: Default::default(),
        });
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(20.0, 20.0));
        let page = doc.pages[0].id;
        let layer = doc.pages[0].layers[0].id;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let pm = render_page_image(&doc, page, 25.4).unwrap(); // 1 px per mm
        assert_eq!((pm.width(), pm.height()), (20, 20));
        // Tiles start at the top-left corner; the red square sits in the
        // lower-left quarter of each tile (image rows 5..10 and 15..20).
        for (x, y) in [(2, 7), (12, 7), (2, 17), (12, 17)] {
            assert_eq!(px(&pm, x, y), (255, 0, 0), "({x}, {y}) must be red");
        }
        for (x, y) in [(2, 2), (7, 7), (7, 2), (12, 12), (17, 17), (17, 7)] {
            assert_eq!(
                px(&pm, x, y),
                (255, 255, 255),
                "({x}, {y}) must be page white"
            );
        }
    }

    fn checker(size_mm: f64, placement: tracedraw_core::style::PatternPlacement) -> Fill {
        Fill::Pattern(Pattern::TwoColor {
            tile: PatternTile::Checker,
            front: Color::rgb8(0, 0, 0),
            back: Color::rgb8(255, 255, 255),
            size_mm,
            placement,
        })
    }

    /// Mirror tiles horizontally: the second column of tiles shows the
    /// tile flipped, so a checker's black bottom-left cell sits at the
    /// bottom right of the second tile.
    #[test]
    fn mirrored_checker_flips_the_second_column() {
        use tracedraw_core::style::PatternPlacement;
        let bounds = Rect::new(0.0, 0.0, 20.0, 10.0);
        // 1 px per mm: the first tile covers columns 0..10, the second
        // 10..20; the tile's bottom-left cell is image rows 5..10.
        let plain =
            render_fill_image(&checker(10.0, PatternPlacement::default()), bounds, 25.4).unwrap();
        assert_eq!(px(&plain, 2, 7), (0, 0, 0));
        assert_eq!(px(&plain, 12, 7), (0, 0, 0), "the same tile again");
        assert_eq!(px(&plain, 17, 7), (255, 255, 255));
        let mirrored = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    mirror_horizontal: true,
                    ..PatternPlacement::default()
                },
            ),
            bounds,
            25.4,
        )
        .unwrap();
        assert_eq!(px(&mirrored, 2, 7), (0, 0, 0), "the first tile as before");
        assert_eq!(px(&mirrored, 12, 7), (255, 255, 255), "flipped");
        assert_eq!(px(&mirrored, 17, 7), (0, 0, 0), "flipped");
        // Vertically: the second row of tiles is turned upside down.
        let upside = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    mirror_vertical: true,
                    ..PatternPlacement::default()
                },
            ),
            Rect::new(0.0, 0.0, 10.0, 20.0),
            25.4,
        )
        .unwrap();
        assert_eq!(px(&upside, 2, 17), (0, 0, 0), "the first row as before");
        assert_eq!(px(&upside, 2, 7), (255, 255, 255), "the second row flipped");
        assert_eq!(px(&upside, 2, 2), (0, 0, 0));
    }

    /// A row offset of 50 percent moves every other row half a tile along;
    /// a column offset does the same to every other column.
    #[test]
    fn row_offset_shifts_every_other_row_by_half_a_tile() {
        use tracedraw_core::style::{PatternPlacement, TileShift};
        let bounds = Rect::new(0.0, 0.0, 20.0, 20.0);
        let shifted = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    shift_percent: 50.0,
                    ..PatternPlacement::default()
                },
            ),
            bounds,
            25.4,
        )
        .unwrap();
        // The bottom row (image rows 10..20) is not shifted: black at the
        // bottom left of each tile.
        assert_eq!(px(&shifted, 2, 17), (0, 0, 0));
        assert_eq!(px(&shifted, 7, 17), (255, 255, 255));
        // The row above starts 5 mm along: its tiles' black cells sit at
        // x 5..10 and 15..20 (image rows 5..10 are the tiles' lower halves).
        assert_eq!(px(&shifted, 7, 7), (0, 0, 0));
        assert_eq!(px(&shifted, 2, 7), (255, 255, 255));
        assert_eq!(px(&shifted, 17, 7), (0, 0, 0));
        let columns = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    shift_percent: 50.0,
                    shift: TileShift::Column,
                    ..PatternPlacement::default()
                },
            ),
            bounds,
            25.4,
        )
        .unwrap();
        // The first column (x 0..10) stays; the second is 5 mm up, so its
        // first tile's black cell covers y 5..10 (image rows 10..15).
        assert_eq!(px(&columns, 2, 17), (0, 0, 0));
        assert_eq!(px(&columns, 12, 12), (0, 0, 0));
        assert_eq!(px(&columns, 12, 17), (255, 255, 255));
    }

    /// The offset moves the tiles, the rotation turns them and the tile
    /// size scales them, the tiles still one pattern draw each.
    #[test]
    fn placement_offsets_turns_and_sizes_the_tiles() {
        use tracedraw_core::style::PatternPlacement;
        let bounds = Rect::new(0.0, 0.0, 20.0, 20.0);
        let moved = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    offset_x: 5.0,
                    ..PatternPlacement::default()
                },
            ),
            bounds,
            25.4,
        )
        .unwrap();
        // The tile edge is now at x 5: black at x 5..10 in the bottom cell.
        assert_eq!(px(&moved, 7, 17), (0, 0, 0));
        assert_eq!(px(&moved, 2, 17), (255, 255, 255));
        let sized = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    size: Some(tracedraw_core::geometry::Size::new(20.0, 20.0)),
                    ..PatternPlacement::default()
                },
            ),
            bounds,
            25.4,
        )
        .unwrap();
        // One 20 mm tile: its bottom-left cell is the whole bottom-left
        // quarter.
        assert_eq!(px(&sized, 7, 17), (0, 0, 0));
        assert_eq!(px(&sized, 7, 7), (255, 255, 255));
        let turned = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    rotation: 45.0,
                    ..PatternPlacement::default()
                },
            ),
            bounds,
            25.4,
        )
        .unwrap();
        // Turned by 45 degrees counter-clockwise around the origin, the
        // tile's bottom edge runs along y = x: the page point (0.5, 3.5)
        // lies 2.8 mm along the tile and 2.1 mm up it, in the black cell,
        // and (3.5, 0.5) lies 2.1 mm below the edge, in the white cell
        // of the tile row beneath.
        assert_eq!(px(&turned, 0, 16), (0, 0, 0));
        assert_eq!(px(&turned, 3, 19), (255, 255, 255));
        // The skew leans the columns: at the top of the tile the edge has
        // moved by tan(45) * 10 mm.
        let skewed = render_fill_image(
            &checker(
                10.0,
                PatternPlacement {
                    skew: 45.0,
                    ..PatternPlacement::default()
                },
            ),
            bounds,
            25.4,
        )
        .unwrap();
        // Page (3.5, 1.5) is 2 mm into the first tile, black; (13.5,
        // 11.5) is 2 mm into the tile above, which starts 10 mm right;
        // (8.5, 11.5) is 7 mm into the tile left of that, white.
        assert_eq!(px(&skewed, 3, 18), (0, 0, 0), "the bottom is as before");
        assert_eq!(px(&skewed, 13, 8), (0, 0, 0), "a tile up, 10 mm right");
        assert_eq!(px(&skewed, 8, 8), (255, 255, 255));
    }

    /// Transform with object: moving the object through the engine moves
    /// its tiles along; without it they stay anchored to the page.
    #[test]
    fn transform_with_object_moves_the_tiles_with_the_object() {
        use tracedraw_core::style::PatternPlacement;
        use tracedraw_core::{Command, Engine};
        let make = |with_object: bool| {
            let tile_square = rect(7, Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb8(255, 0, 0));
            let mut s = rect(1, Rect::new(0.0, 0.0, 20.0, 20.0), Color::WHITE);
            s.fill = Fill::Pattern(Pattern::Vector {
                shapes: vec![tile_square],
                tile: tracedraw_core::geometry::Size::new(10.0, 10.0),
                placement: PatternPlacement {
                    transform_with_object: with_object,
                    ..PatternPlacement::default()
                },
            });
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(30.0, 20.0));
            let page = doc.pages[0].id;
            let layer = doc.pages[0].layers[0].id;
            doc.layer_mut(layer).unwrap().shapes.push(s);
            let mut e = Engine::new(doc);
            e.run(&Command::TransformShapes {
                shapes: vec![tracedraw_core::ShapeId(1)],
                transform: Affine::translate((5.0, 0.0)),
            })
            .unwrap();
            render_page_image(e.document(), page, 25.4).unwrap()
        };
        // Anchored: the tiles stay on the page grid, the object now covers
        // x 5..25, so red squares show at x 10..15 and 20..25 (rows 15..20).
        let anchored = make(false);
        assert_eq!(px(&anchored, 12, 17), (255, 0, 0));
        assert_eq!(px(&anchored, 7, 17), (255, 255, 255));
        // With the object: the tiles moved 5 mm right too, red at x 5..10
        // and 15..20.
        let along = make(true);
        assert_eq!(px(&along, 7, 17), (255, 0, 0));
        assert_eq!(px(&along, 12, 17), (255, 255, 255));
        assert_eq!(px(&along, 17, 17), (255, 0, 0));
    }

    /// A pattern as a transparency mask (a group's vector pattern
    /// transparency read from a `.cdr`): set to transform with its
    /// object, the mask moves with the object; it stayed on the page.
    #[test]
    fn a_pattern_mask_set_to_transform_with_its_object_follows_it() {
        use tracedraw_core::live::{Effect, MergeMode};
        use tracedraw_core::style::PatternPlacement;
        use tracedraw_core::{Command, Engine};
        let make = |with_object: bool| {
            // The mask: white (opaque) over the left half of the tile,
            // black (clear) over the right half; one tile over the object.
            let white = rect(7, Rect::new(0.0, 0.0, 10.0, 20.0), Color::WHITE);
            let black = rect(8, Rect::new(10.0, 0.0, 20.0, 20.0), Color::BLACK);
            let mut s = rect(1, Rect::new(0.0, 0.0, 20.0, 20.0), Color::rgb8(255, 0, 0));
            s.effects.push(Effect::Transparency {
                mask: Fill::Pattern(Pattern::Vector {
                    shapes: vec![white, black],
                    tile: tracedraw_core::geometry::Size::new(20.0, 20.0),
                    placement: PatternPlacement {
                        transform_with_object: with_object,
                        ..PatternPlacement::default()
                    },
                }),
                merge: MergeMode::Normal,
                target: 2,
            });
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(30.0, 20.0));
            let page = doc.pages[0].id;
            let layer = doc.pages[0].layers[0].id;
            doc.layer_mut(layer).unwrap().shapes.push(s);
            let mut e = Engine::new(doc);
            e.run(&Command::TransformShapes {
                shapes: vec![tracedraw_core::ShapeId(1)],
                transform: Affine::translate((5.0, 0.0)),
            })
            .unwrap();
            render_page_image(e.document(), page, 25.4).unwrap()
        };
        // The object covers x 5..25. With the object, its left half
        // (x 5..15) is opaque and its right half clear.
        let along = make(true);
        assert_eq!(px(&along, 7, 10), (255, 0, 0));
        assert_eq!(px(&along, 12, 10), (255, 0, 0));
        assert_eq!(px(&along, 17, 10), (255, 255, 255));
        assert_eq!(px(&along, 22, 10), (255, 255, 255));
        // Anchored to the page, the tiles stay where they were: opaque
        // over x 0..10 and 20..30.
        let anchored = make(false);
        assert_eq!(px(&anchored, 7, 10), (255, 0, 0));
        assert_eq!(px(&anchored, 12, 10), (255, 255, 255));
        assert_eq!(px(&anchored, 22, 10), (255, 0, 0));
    }

    /// Mirrored and shifted bitmap tiles: the picture's block is built
    /// once from the decoded picture; a two-by-two block holds the picture
    /// four times, flipped in turn.
    #[test]
    fn bitmap_pattern_mirrors_its_picture() {
        use tracedraw_core::style::PatternPlacement;
        // A 2 x 1 picture: red left, blue right.
        let mut pm = Pixmap::new(2, 1).unwrap();
        pm.pixels_mut()[0] = tiny_skia::ColorU8::from_rgba(255, 0, 0, 255).premultiply();
        pm.pixels_mut()[1] = tiny_skia::ColorU8::from_rgba(0, 0, 255, 255).premultiply();
        let png = pm.encode_png().unwrap();
        let fill = |placement| {
            Fill::Pattern(Pattern::Bitmap {
                png: png.clone().into(),
                width_px: 2,
                height_px: 1,
                size_mm: 10.0,
                placement,
            })
        };
        let bounds = Rect::new(0.0, 0.0, 20.0, 5.0);
        let plain = render_fill_image(&fill(PatternPlacement::default()), bounds, 25.4).unwrap();
        assert_eq!(px(&plain, 2, 2), (255, 0, 0));
        assert_eq!(px(&plain, 12, 2), (255, 0, 0));
        let mirrored = render_fill_image(
            &fill(PatternPlacement {
                mirror_horizontal: true,
                ..PatternPlacement::default()
            }),
            bounds,
            25.4,
        )
        .unwrap();
        assert_eq!(px(&mirrored, 2, 2), (255, 0, 0));
        assert_eq!(px(&mirrored, 12, 2), (0, 0, 255), "the second tile flipped");
        assert_eq!(px(&mirrored, 17, 2), (255, 0, 0));
    }

    /// The block exporters tile: one tile with its placement transform.
    #[test]
    fn pattern_cell_for_exporters_carries_the_placement() {
        use tracedraw_core::style::PatternPlacement;
        let p = Pattern::TwoColor {
            tile: PatternTile::Checker,
            front: Color::BLACK,
            back: Color::WHITE,
            size_mm: 10.0,
            placement: PatternPlacement {
                mirror_horizontal: true,
                offset_x: 3.0,
                ..PatternPlacement::default()
            },
        };
        let (pm, to_page) = render_pattern_cell(&p, 25.4).unwrap();
        assert_eq!((pm.width(), pm.height()), (20, 10));
        let c = to_page.as_coeffs();
        assert_eq!(c, [20.0, 0.0, 0.0, 10.0, 3.0, 0.0]);
        // The second half is the mirrored tile.
        assert_eq!(px(&pm, 2, 7), (0, 0, 0));
        assert_eq!(px(&pm, 12, 7), (255, 255, 255));
    }

    #[test]
    fn vector_pattern_tile_draws_nested_groups_and_outlines() {
        let mut child = rect(8, Rect::new(2.0, 2.0, 8.0, 8.0), Color::rgb8(0, 0, 255));
        child.stroke = Some(Stroke::new(Color::rgb8(0, 255, 0), 2.0));
        let group = Shape::new(
            tracedraw_core::ShapeId(9),
            ShapeKind::Group {
                children: vec![child],
            },
        );
        let fill = Fill::Pattern(Pattern::Vector {
            shapes: vec![group],
            tile: tracedraw_core::geometry::Size::new(10.0, 10.0),
            placement: Default::default(),
        });
        let pm = render_fill_image(&fill, Rect::new(0.0, 0.0, 10.0, 10.0), 254.0).unwrap();
        // 10 px per mm: the centre is blue, the 2 mm outline band is green.
        assert_eq!(px(&pm, 50, 50), (0, 0, 255));
        assert_eq!(px(&pm, 20, 50), (0, 255, 0));
        assert_eq!(px(&pm, 5, 5), (0, 0, 0));
        assert_eq!(
            pm.pixel(5, 5).unwrap().alpha(),
            0,
            "outside the tile content is clear"
        );
    }

    #[test]
    fn degenerate_vector_pattern_does_not_panic() {
        for tile in [
            tracedraw_core::geometry::Size::new(0.0, 10.0),
            tracedraw_core::geometry::Size::new(f64::NAN, 10.0),
            tracedraw_core::geometry::Size::new(0.0001, 0.0001),
            tracedraw_core::geometry::Size::new(1.0e9, 1.0e9),
        ] {
            let fill = Fill::Pattern(Pattern::Vector {
                shapes: vec![rect(1, Rect::new(0.0, 0.0, 1.0, 1.0), Color::BLACK)],
                tile,
                placement: Default::default(),
            });
            assert!(render_fill_image(&fill, Rect::new(0.0, 0.0, 10.0, 10.0), 96.0).is_some());
        }
    }

    #[test]
    fn texture_is_deterministic_for_seed() {
        let t = Fill::Texture(Texture {
            kind: TextureKind::Clouds,
            color_a: Color::rgb8(0, 0, 0),
            color_b: Color::rgb8(255, 255, 255),
            scale: 10.0,
            seed: 7,
        });
        let a = render_fill_image(&t, Rect::new(0.0, 0.0, 30.0, 30.0), 25.4).unwrap();
        let b = render_fill_image(&t, Rect::new(0.0, 0.0, 30.0, 30.0), 25.4).unwrap();
        assert_eq!(a.data(), b.data());
        let v: std::collections::HashSet<u8> = (0..30).map(|x| px(&a, x, 15).0).collect();
        assert!(v.len() > 3, "texture should vary");
    }

    /// A bricks fill drawn at 4 px/mm: the mortar between the rows and
    /// between the bricks shows the background colour, the bricks the
    /// foreground.
    #[test]
    fn procedural_bricks_show_mortar_in_the_background_colour() {
        use tracedraw_core::{Procedural, ProceduralPattern};
        let mut p = Procedural::new(ProceduralPattern::Bricks, Color::rgb8(0, 0, 0));
        p.params.line_width_mm = 1.0;
        p.background = Some(Color::rgb8(255, 255, 255));
        let img = render_fill_image(&Fill::Procedural(p), Rect::new(0.0, 0.0, 20.0, 20.0), 101.6)
            .unwrap();
        assert_eq!((img.width(), img.height()), (80, 80));
        // Page y = 5 mm (row 60) is the mortar between the two rows.
        assert_eq!(px(&img, 10, 60), (255, 255, 255));
        assert_eq!(px(&img, 50, 60), (255, 255, 255));
        // Page y = 7.5 mm (row 50): a brick at x = 2.5 mm, the joint at
        // x = 5 mm, a brick again at 7.5 mm.
        assert_eq!(px(&img, 10, 50), (0, 0, 0));
        assert_eq!(px(&img, 20, 50), (255, 255, 255));
        assert_eq!(px(&img, 30, 50), (0, 0, 0));
        // The lower row's joint is half a brick along, at x = 0 and 10 mm.
        assert_eq!(px(&img, 40, 70), (255, 255, 255));
        assert_eq!(px(&img, 20, 70), (0, 0, 0));
        // The second tile (y = 15 mm, row 20) repeats the first.
        assert_eq!(px(&img, 10, 20), (255, 255, 255));
        assert_eq!(px(&img, 10, 10), (0, 0, 0));
    }

    /// Dots covering half the tile paint about half of its pixels.
    #[test]
    fn procedural_dots_cover_their_share() {
        use tracedraw_core::{Procedural, ProceduralPattern};
        let mut p = Procedural::new(ProceduralPattern::Dots, Color::rgb8(0, 0, 0));
        p.params.share = 0.5;
        p.background = Some(Color::rgb8(255, 255, 255));
        let img = render_fill_image(&Fill::Procedural(p), Rect::new(0.0, 0.0, 10.0, 10.0), 254.0)
            .unwrap();
        assert_eq!((img.width(), img.height()), (100, 100));
        let dark = img
            .pixels()
            .iter()
            .filter(|c| c.demultiply().red() < 128)
            .count();
        assert!((4800..=5200).contains(&dark), "{dark} dark pixels of 10000");
        // The centre is in the dot, a corner is not.
        assert_eq!(px(&img, 50, 50), (0, 0, 0));
        assert_eq!(px(&img, 2, 2), (255, 255, 255));
    }

    /// The speckle is the same picture every time for a seed, another
    /// picture for another seed, and without a background it leaves the
    /// rest clear.
    #[test]
    fn procedural_speckle_is_deterministic_and_can_be_clear() {
        use tracedraw_core::{Procedural, ProceduralPattern};
        let mut p = Procedural::new(ProceduralPattern::Speckle, Color::rgb8(255, 0, 0));
        p.seed = 5;
        p.background = None;
        let f = Fill::Procedural(p.clone());
        let a = render_fill_image(&f, Rect::new(0.0, 0.0, 20.0, 20.0), 101.6).unwrap();
        let b = render_fill_image(&f, Rect::new(0.0, 0.0, 20.0, 20.0), 101.6).unwrap();
        assert_eq!(a.data(), b.data());
        let painted = a.pixels().iter().filter(|c| c.alpha() > 0).count();
        let clear = a.pixels().iter().filter(|c| c.alpha() == 0).count();
        assert!(
            painted > 50 && clear > painted,
            "{painted} painted, {clear} clear"
        );
        p.seed = 6;
        let c = render_fill_image(&Fill::Procedural(p), Rect::new(0.0, 0.0, 20.0, 20.0), 101.6)
            .unwrap();
        assert_ne!(a.data(), c.data());
    }

    #[test]
    fn large_textures_on_screen_are_quick_and_exports_keep_every_pixel() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(300.0, 300.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 300.0, 300.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Texture(Texture {
            kind: TextureKind::Clouds,
            color_a: Color::Cmyk {
                c: 1.0,
                m: 0.2,
                y: 0.0,
                k: 0.1,
            },
            color_b: Color::rgb8(255, 255, 255),
            scale: 40.0,
            seed: 11,
        });
        s.stroke = None;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let page = doc.pages[0].id;
        let opts = |draft: bool| RenderOptions {
            width: 1600,
            height: 1000,
            view: ViewTransform {
                zoom: 6.0,
                origin_x: -100.0,
                origin_y: 1500.0,
            },
            draft,
            ..Default::default()
        };
        // Drawn whole, on one thread, so the times compare.
        let exact = render_page_whole(&doc, page, &opts(false), None).unwrap();
        let t = std::time::Instant::now();
        let draft = render_page_whole(&doc, page, &opts(true), None).unwrap();
        let first = t.elapsed();
        // The best of three, with room for other tests loading the cores.
        let mut cached = std::time::Duration::MAX;
        let mut again = None;
        for _ in 0..3 {
            let t = std::time::Instant::now();
            again = render_page_whole(&doc, page, &opts(true), None);
            cached = cached.min(t.elapsed());
        }
        assert_eq!(
            draft.data(),
            again.unwrap().data(),
            "the kept result is the same"
        );
        assert!(cached <= first * 2, "{cached:?} {first:?}");
        // The coarser grid stays close to the exact picture.
        let mut worst = 0i32;
        for (a, b) in exact.pixels().iter().zip(draft.pixels()).step_by(97) {
            worst = worst.max((a.red() as i32 - b.red() as i32).abs());
        }
        assert!(worst < 40, "{worst}");
    }

    #[test]
    fn outlines_take_a_gradient_fill() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 20.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut path = tracedraw_core::BezPath::new();
        path.move_to((5.0, 10.0));
        path.line_to((95.0, 10.0));
        let mut s = Shape::new(
            id,
            ShapeKind::Path {
                path,
                closed: false,
            },
        );
        s.fill = Fill::None;
        let mut stroke = Stroke::new(Color::rgb8(0, 255, 0), 6.0);
        stroke.fill = Some(Box::new(Fill::Fountain(tracedraw_core::Fountain::two(
            tracedraw_core::FountainKind::Linear,
            Color::rgb8(255, 0, 0),
            Color::rgb8(0, 0, 255),
            0.0,
        ))));
        s.stroke = Some(stroke);
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let img = render_page_image(&doc, doc.pages[0].id, 25.4).unwrap();
        let left = img.pixel(8, 10).unwrap();
        let right = img.pixel(92, 10).unwrap();
        assert!(left.red() > 200 && left.blue() < 60, "{left:?}");
        assert!(right.blue() > 200 && right.red() < 60, "{right:?}");
        assert!(left.green() < 30, "not the outline colour");
        // Outside the stroke nothing is painted.
        assert_eq!(img.pixel(50, 2).unwrap().green(), 255);
    }

    /// A 100 x 10 mm rectangle filled with `fill`, drawn at 1 px/mm.
    fn strip(fill: Fill) -> Pixmap {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 10.0));
        let page = doc.pages[0].id;
        let mut s = rect(1, Rect::new(0.0, 0.0, 100.0, 10.0), Color::WHITE);
        s.fill = fill;
        s.stroke = None;
        doc.layer_mut(doc.pages[0].layers[0].id)
            .unwrap()
            .shapes
            .push(s);
        render_page_image(&doc, page, 25.4).unwrap()
    }

    #[test]
    fn linear_fountains_repeat_mirror_and_invert() {
        let bw = || Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        let red_at = |img: &Pixmap, x: u32| img.pixel(x, 5).unwrap().red();
        // Sawtooth twice: light just before the middle, dark just after.
        let mut f = bw();
        f.repeat = tracedraw_core::FountainRepeat::Repeat;
        f.repeats = 2;
        let img = strip(Fill::Fountain(f));
        assert!(red_at(&img, 48) > 220, "{}", red_at(&img, 48));
        assert!(red_at(&img, 51) < 35, "{}", red_at(&img, 51));
        assert!((red_at(&img, 25) as i32 - 128).abs() < 20);
        // Triangular twice: white in the middle, black at both ends.
        let mut m = bw();
        m.repeat = tracedraw_core::FountainRepeat::Mirror;
        m.repeats = 2;
        let img = strip(Fill::Fountain(m));
        assert!(red_at(&img, 50) > 240 && red_at(&img, 1) < 15 && red_at(&img, 98) < 15);
        // Bilinear gives the same run.
        let mut b = bw();
        b.bilinear = true;
        let bimg = strip(Fill::Fountain(b));
        for x in [1, 25, 50, 75, 98] {
            assert!((red_at(&img, x) as i32 - red_at(&bimg, x) as i32).abs() <= 3);
        }
        // Inverted: white on the left.
        let mut i = bw();
        i.invert = true;
        let img = strip(Fill::Fountain(i));
        assert!(red_at(&img, 1) > 240 && red_at(&img, 98) < 15);
        // Conical and square fountains take the options too.
        let mut sq = Fountain::two(FountainKind::Square, Color::BLACK, Color::WHITE, 0.0);
        sq.invert = true;
        let img = strip(Fill::Fountain(sq));
        assert!(red_at(&img, 50) > 200, "{}", red_at(&img, 50));
        assert!(red_at(&img, 1) < 30, "{}", red_at(&img, 1));
    }

    /// A stepped fill is drawn as exactly `steps` colours, by the gradient
    /// shader (linear) and pixel by pixel (square); a hue path and the
    /// smooth transition show in the pixels too (#1208).
    #[test]
    fn stepped_and_hue_fountains_draw_their_colours() {
        let bw = || Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        let row = |img: &Pixmap| -> Vec<u8> {
            let mut v: Vec<u8> = (1..99).map(|x| img.pixel(x, 5).unwrap().red()).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        for steps in [2u32, 4, 7] {
            let mut f = bw();
            f.steps = steps;
            let greys = row(&strip(Fill::Fountain(f)));
            assert_eq!(greys.len(), steps as usize, "{steps}: {greys:?}");
            assert_eq!((greys[0], greys[greys.len() - 1]), (0, 255));
            let mut sq = Fountain::two(FountainKind::Square, Color::BLACK, Color::WHITE, 0.0);
            sq.steps = steps;
            let greys = row(&strip(Fill::Fountain(sq)));
            assert_eq!(greys.len(), steps as usize, "square {steps}: {greys:?}");
        }
        // Four bands: the left quarter black, the next 85.
        let mut f = bw();
        f.steps = 4;
        let img = strip(Fill::Fountain(f));
        assert_eq!(img.pixel(10, 5).unwrap().red(), 0);
        assert_eq!(img.pixel(40, 5).unwrap().red(), 85);
        assert_eq!(img.pixel(90, 5).unwrap().red(), 255);
        // Red to blue clockwise: green in the middle; straight: purple.
        let (red, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255));
        let mut h = Fountain::two(FountainKind::Linear, red, blue, 0.0);
        let mid = strip(Fill::Fountain(h.clone())).pixel(50, 5).unwrap();
        assert!(
            mid.green() < 10 && mid.red() > 100 && mid.blue() > 100,
            "{mid:?}"
        );
        h.blend_direction = tracedraw_core::ColorBlend::Clockwise;
        let mid = strip(Fill::Fountain(h.clone())).pixel(50, 5).unwrap();
        assert!(
            mid.green() > 240 && mid.red() < 20 && mid.blue() < 20,
            "{mid:?}"
        );
        // Conical fills take the hue path too (drawn pixel by pixel):
        // the colour half way round is green as well.
        h.kind = FountainKind::Conical;
        let img = strip(Fill::Fountain(h));
        let mid = img.pixel(50, 9).unwrap();
        assert!(mid.green() > 200, "{mid:?}");
        // Smooth transition: darker early on, lighter late, grey in the middle.
        let mut s = bw();
        s.smooth = true;
        let img = strip(Fill::Fountain(s));
        let plain = strip(Fill::Fountain(bw()));
        assert!(img.pixel(15, 5).unwrap().red() < plain.pixel(15, 5).unwrap().red() - 10);
        assert!(img.pixel(85, 5).unwrap().red() > plain.pixel(85, 5).unwrap().red() + 10);
        assert!((img.pixel(50, 5).unwrap().red() as i32 - 128).abs() <= 2);
        // Acceleration +100: light almost from the start.
        let mut a = bw();
        a.acceleration = 100;
        let img = strip(Fill::Fountain(a));
        assert!(
            img.pixel(10, 5).unwrap().red() > 150,
            "{}",
            img.pixel(10, 5).unwrap().red()
        );
    }

    #[test]
    fn shaped_fountains_follow_the_outline() {
        // Black on the edge, white deepest inside.
        let f = Fountain::two(FountainKind::Shaped, Color::BLACK, Color::WHITE, 0.0);
        let img = strip(Fill::Fountain(f.clone()));
        let at = |img: &Pixmap, x: u32, y: u32| img.pixel(x, y).unwrap().red() as i32;
        assert!(at(&img, 50, 0) < 40, "edge {}", at(&img, 50, 0));
        assert!(at(&img, 50, 5) > 200, "middle {}", at(&img, 50, 5));
        // Along a long strip the colour depends on the distance to the
        // nearest edge, not on the distance to the centre.
        assert!((at(&img, 20, 5) - at(&img, 80, 5)).abs() <= 6);
        assert!((at(&img, 20, 5) - at(&img, 50, 5)).abs() <= 6);
        assert!(at(&img, 2, 5) < at(&img, 20, 5));
        // A circle: rings of equal colour.
        let (mut doc, page) = doc_with(vec![]);
        let id = doc.ids_mut().shape();
        let mut c = Shape::new(
            id,
            ShapeKind::Ellipse {
                rect: Rect::new(0.0, 0.0, 60.0, 60.0),
                arc: None,
            },
        );
        c.fill = Fill::Fountain(f);
        c.stroke = None;
        doc.layer_mut(doc.pages[0].layers[0].id)
            .unwrap()
            .shapes
            .push(c);
        let img = render_page_image(&doc, page, 25.4).unwrap();
        // Page y is up: the circle's centre is at (30, page height - 30).
        let cy = img.height() - 30;
        let (e, w, n) = (at(&img, 45, cy), at(&img, 15, cy), at(&img, 30, cy - 15));
        assert!((e - w).abs() <= 8 && (e - n).abs() <= 8, "{e} {w} {n}");
        assert!(at(&img, 30, cy) > 230, "{}", at(&img, 30, cy));
        assert!(at(&img, 2, cy) < at(&img, 15, cy));
    }

    #[test]
    fn edge_distance_measures_to_the_nearest_outside_pixel() {
        // A 7 x 5 grid with a 5 x 3 block inside.
        let (w, h) = (7usize, 5usize);
        let inside: Vec<bool> = (0..w * h)
            .map(|i| (1..6).contains(&(i % w)) && (1..4).contains(&(i / w)))
            .collect();
        let d = edge_distance(&inside, w, h);
        assert_eq!(d[0], 0.0);
        assert!((d[w + 1] - 0.5).abs() < 1e-9);
        assert!((d[2 * w + 3] - 1.5).abs() < 1e-9);
        assert!(edge_distance(&[], 0, 0).is_empty());
    }

    #[test]
    fn outlines_take_pattern_texture_mesh_and_shaped_paints() {
        let line = |paint: Fill| {
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 20.0));
            let layer = doc.pages[0].layers[0].id;
            let id = doc.ids_mut().shape();
            let mut path = tracedraw_core::BezPath::new();
            path.move_to((5.0, 10.0));
            path.line_to((95.0, 10.0));
            let mut s = Shape::new(
                id,
                ShapeKind::Path {
                    path,
                    closed: false,
                },
            );
            s.fill = Fill::None;
            let mut stroke = Stroke::new(Color::rgb8(0, 255, 0), 6.0);
            stroke.fill = Some(Box::new(paint));
            s.stroke = Some(stroke);
            doc.layer_mut(layer).unwrap().shapes.push(s);
            render_page_image(&doc, doc.pages[0].id, 25.4).unwrap()
        };
        let red = Color::rgb8(255, 0, 0);
        let blue = Color::rgb8(0, 0, 255);
        // A two-colour pattern: both colours along the outline, never the
        // outline colour, nothing beside it.
        let img = line(Fill::Pattern(Pattern::TwoColor {
            tile: tracedraw_core::PatternTile::Stripes,
            front: red,
            back: blue,
            size_mm: 4.0,
            placement: Default::default(),
        }));
        let row: Vec<_> = (6..94).map(|x| img.pixel(x, 10).unwrap()).collect();
        assert!(row.iter().all(|p| p.green() < 30));
        assert_eq!(img.pixel(50, 2).unwrap().green(), 255);
        let col: Vec<_> = (7..13).map(|y| img.pixel(50, y).unwrap()).collect();
        assert!(col.iter().any(|p| p.red() > 200) && col.iter().any(|p| p.blue() > 200));
        // A texture.
        let img = line(Fill::Texture(Texture {
            kind: tracedraw_core::TextureKind::Noise,
            color_a: red,
            color_b: blue,
            scale: 2.0,
            seed: 3,
        }));
        let p = img.pixel(50, 10).unwrap();
        assert!(p.green() < 30 && p.red() + p.blue() > 150, "{p:?}");
        // A mesh: its nodes are local to the object, red on the left and
        // blue on the right.
        let mut m = tracedraw_core::Mesh::new(Rect::new(5.0, 7.0, 95.0, 13.0), 1, 1, red);
        for (i, n) in m.nodes.iter_mut().enumerate() {
            if i % 2 == 1 {
                n.color = blue;
            }
        }
        let img = line(Fill::Mesh(m));
        let (l, r) = (img.pixel(8, 10).unwrap(), img.pixel(92, 10).unwrap());
        assert!(l.red() > 200 && l.blue() < 60, "{l:?}");
        assert!(r.blue() > 200 && r.red() < 60, "{r:?}");
        // A shaped gradient follows the outline's own area: its edge
        // dark, its middle light.
        let img = line(Fill::Fountain(Fountain::two(
            FountainKind::Shaped,
            Color::BLACK,
            Color::WHITE,
            0.0,
        )));
        let (edge, mid) = (img.pixel(50, 7).unwrap(), img.pixel(50, 10).unwrap());
        assert!(mid.red() > edge.red() + 60, "{edge:?} {mid:?}");
    }

    #[test]
    fn procedural_fills_work_out_only_the_visible_part() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 2.0, 2.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Texture(Texture {
            kind: TextureKind::Clouds,
            color_a: Color::rgb8(0, 0, 0),
            color_b: Color::rgb8(255, 255, 255),
            scale: 0.5,
            seed: 3,
        });
        s.stroke = None;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let page = doc.pages[0].id;
        let view = |w: u32, ox: f64, zoom: f64| RenderOptions {
            width: w,
            height: 64,
            view: ViewTransform {
                zoom,
                origin_x: ox,
                origin_y: 64.0,
            },
            ..Default::default()
        };
        // The whole square, then its right half alone: the same pixels.
        let whole = render_page(&doc, page, &view(64, 0.0, 32.0)).unwrap();
        let half = render_page(&doc, page, &view(32, -32.0, 32.0)).unwrap();
        for y in [5u32, 30, 60] {
            for x in [2u32, 15, 29] {
                assert_eq!(
                    whole.pixel(x + 32, y).unwrap(),
                    half.pixel(x, y).unwrap(),
                    "{x},{y}"
                );
            }
        }
        // Zoomed so far that the square is 200 000 pixels wide: only the
        // window is worked out, at once.
        let t = std::time::Instant::now();
        let deep = render_page(&doc, page, &view(64, 0.0, 100_000.0)).unwrap();
        assert!(t.elapsed().as_secs_f64() < 2.0);
        assert!(deep.pixel(10, 10).unwrap().alpha() == 255);
    }

    #[test]
    fn star_fill_covers_center_and_not_corner() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Polygon {
                rect: Rect::new(10.0, 10.0, 90.0, 90.0),
                points: 5,
                sharpness: 0.6,
                complex: None,
                twist: 0.0,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = None;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let page = doc.pages[0].id;
        let img = render_page_image(&doc, page, 25.4).unwrap(); // 1 px per mm
        let px = |x: u32, y: u32| img.pixel(x, y).unwrap();
        // Centre of the star is red; the page corner is white.
        assert_eq!(px(50, 50).red(), 255);
        assert_eq!(px(50, 50).green(), 0);
        assert_eq!(px(2, 2).green(), 255);
    }

    #[test]
    fn without_anti_aliasing_edges_are_hard() {
        let mut ellipse = Shape::new(
            tracedraw_core::ShapeId(3),
            ShapeKind::Ellipse {
                rect: Rect::new(10.0, 10.0, 90.0, 70.0),
                arc: None,
            },
        );
        ellipse.fill = Fill::Solid(Color::rgb8(0, 0, 0));
        ellipse.stroke = None;
        let (doc, page) = doc_with(vec![ellipse]);
        let render = |anti_alias: bool| {
            let opts = RenderOptions {
                width: 100,
                height: 100,
                view: ViewTransform {
                    zoom: 1.0,
                    origin_x: 0.0,
                    origin_y: 100.0,
                },
                anti_alias,
                ..RenderOptions::default()
            };
            render_page(&doc, page, &opts).unwrap()
        };
        let partial = |pm: &Pixmap| {
            pm.pixels()
                .iter()
                .filter(|p| p.alpha() != 0 && p.alpha() != 255)
                .count()
        };
        assert!(partial(&render(true)) > 20);
        assert_eq!(partial(&render(false)), 0);
    }

    #[test]
    fn half_transparent_red_over_white_is_pink() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(20.0, 20.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = None;
        s.opacity = 0.5;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let page = doc.pages[0].id;
        let img = render_page_image(&doc, page, 25.4).unwrap();
        let p = img.pixel(10, 10).unwrap();
        assert_eq!(p.red(), 255);
        assert!((120..=136).contains(&p.green()), "green was {}", p.green());
    }

    /// Two overlapping squares in a group on a 20 mm page: the left one
    /// red, the right one blue, overlapping in the middle.
    fn group_of_two_squares(opacity: f64) -> (Document, tracedraw_core::PageId) {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(20.0, 20.0));
        let layer = doc.pages[0].layers[0].id;
        let square = |doc: &mut Document, x0: f64, x1: f64, color: Color| {
            let id = doc.ids_mut().shape();
            let mut s = Shape::new(
                id,
                ShapeKind::Rect {
                    rect: Rect::new(x0, 0.0, x1, 20.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = Fill::Solid(color);
            s.stroke = None;
            s
        };
        let red = square(&mut doc, 0.0, 14.0, Color::rgb8(255, 0, 0));
        let blue = square(&mut doc, 6.0, 20.0, Color::rgb8(0, 0, 255));
        let gid = doc.ids_mut().shape();
        let mut group = Shape::new(
            gid,
            ShapeKind::Group {
                children: vec![red, blue],
            },
        );
        group.stroke = None;
        group.opacity = opacity;
        doc.layer_mut(layer).unwrap().shapes.push(group);
        let page = doc.pages[0].id;
        (doc, page)
    }

    /// A group's uniform transparency is drawn on each member, exactly as
    /// the same transparency set on each member of an opaque group: the
    /// screenshot of #1386, "first grouped, then transparency" looking as
    /// "first transparency, then grouped".
    #[test]
    fn a_group_with_uniform_transparency_is_drawn_see_through() {
        let (doc, page) = group_of_two_squares(0.5);
        let img = render_page_image(&doc, page, 25.4).unwrap();
        // Red alone: half red over white is pink.
        let red = img.pixel(3, 10).unwrap();
        assert_eq!(red.red(), 255);
        assert!(
            (120..=136).contains(&red.green()),
            "green was {}",
            red.green()
        );
        // Where they overlap the red one shows through the blue one: the
        // overlap is darker than the blue one alone (less green).
        let overlap = img.pixel(10, 10).unwrap();
        let blue = img.pixel(17, 10).unwrap();
        assert!(overlap.green() + 30 < blue.green(), "{overlap:?} {blue:?}");
        // The same as each member set half transparent.
        let (mut each, _) = group_of_two_squares(1.0);
        let layer = each.pages[0].layers[0].id;
        if let Some(ShapeKind::Group { children }) = each
            .layer_mut(layer)
            .ok()
            .and_then(|l| l.shapes.first_mut())
            .map(|g| &mut g.kind)
        {
            for c in children {
                c.opacity = 0.5;
            }
        }
        let want = render_page_image(&each, page, 25.4).unwrap();
        assert_eq!(img.data(), want.data());
    }

    /// Nested groups multiply their transparencies, and a member's own.
    #[test]
    fn nested_group_transparencies_multiply() {
        let (mut doc, page) = group_of_two_squares(0.5);
        let layer = doc.pages[0].layers[0].id;
        let gid = doc.ids_mut().shape();
        let l = doc.layer_mut(layer).unwrap();
        let outer = l.shapes.remove(0);
        let mut wrap = Shape::new(
            gid,
            ShapeKind::Group {
                children: vec![outer],
            },
        );
        wrap.stroke = None;
        wrap.opacity = 0.5;
        l.shapes.push(wrap);
        let img = render_page_image(&doc, page, 25.4).unwrap();
        // Red alone at a quarter: 255 - 255 / 4 green and blue.
        let red = img.pixel(3, 10).unwrap();
        assert_eq!(red.red(), 255);
        assert!(
            (184..=200).contains(&red.green()),
            "green was {}",
            red.green()
        );
    }

    #[test]
    fn a_fully_see_through_group_draws_nothing() {
        let (doc, page) = group_of_two_squares(0.0);
        let img = render_page_image(&doc, page, 25.4).unwrap();
        for x in [3, 10, 17] {
            let p = img.pixel(x, 10).unwrap();
            assert_eq!((p.red(), p.green(), p.blue()), (255, 255, 255), "x {x}");
        }
    }

    #[test]
    fn an_opaque_group_is_drawn_as_before() {
        let (doc, page) = group_of_two_squares(1.0);
        let img = render_page_image(&doc, page, 25.4).unwrap();
        let red = img.pixel(3, 10).unwrap();
        assert_eq!((red.red(), red.green(), red.blue()), (255, 0, 0));
        let overlap = img.pixel(10, 10).unwrap();
        assert_eq!(
            (overlap.red(), overlap.green(), overlap.blue()),
            (0, 0, 255)
        );
    }

    #[test]
    fn shadow_darkens_pixels_beside_the_object() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(40.0, 40.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 25.0, 25.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(0, 0, 255));
        s.stroke = None;
        s.shadow = Some(tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(5.0, -5.0),
            opacity: 0.8,
            blur: 0.0,
            color: Color::BLACK,
            ..Default::default()
        });
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let page = doc.pages[0].id;
        let img = render_page_image(&doc, page, 25.4).unwrap();
        // Just right of the rect and below: shadow only (y down in the image).
        let p = img.pixel(28, 32).unwrap();
        assert!(p.red() < 100, "red was {}", p.red());
        let far = img.pixel(2, 2).unwrap();
        assert_eq!(far.red(), 255);
    }

    /// A drop shadow is painted on a layer of only the pixels it can
    /// reach, which a drag redraws every frame (#935): the picture is
    /// the same, pixel for pixel, as with a layer the size of the whole
    /// window, for flat and perspective shadows, every feather
    /// direction, merge modes over a filled object, shadows running off
    /// the window's edges, and a group's shadow.
    #[test]
    fn shadows_on_their_own_pixels_match_a_layer_of_the_whole_window() {
        use tracedraw_core::live::MergeMode;
        use tracedraw_core::shadow::{FeatherDirection, Perspective, ShadowEdge};
        use tracedraw_core::Shadow;
        let opts = RenderOptions {
            width: 240,
            height: 180,
            view: ViewTransform {
                zoom: 3.0,
                origin_x: -15.0,
                origin_y: 200.0,
            },
            ..Default::default()
        };
        let screen = opts.view.affine();
        let rect = |doc: &mut Document, r: Rect, fill: Fill| {
            let mut s = Shape::new(
                doc.ids_mut().shape(),
                ShapeKind::Rect {
                    rect: r,
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = fill;
            s.stroke = None;
            s
        };
        let flat = |dx: f64, dy: f64, blur: f64, dir: FeatherDirection, merge: MergeMode| Shadow {
            offset: tracedraw_core::geometry::Vec2::new(dx, dy),
            opacity: 0.7,
            blur,
            color: Color::rgb8(20, 40, 160),
            merge,
            feather_direction: dir,
            ..Default::default()
        };
        let mut floor = flat(0.0, 0.0, 2.0, FeatherDirection::Middle, MergeMode::Multiply);
        floor.perspective = Some(Perspective {
            fade: 0.6,
            ..Perspective::new(ShadowEdge::Bottom)
        });
        let body = Rect::new(30.0, 30.0, 50.0, 60.0);
        let cases = [
            (
                body,
                flat(
                    6.0,
                    -4.0,
                    1.5,
                    FeatherDirection::Middle,
                    MergeMode::Multiply,
                ),
            ),
            (
                body,
                flat(3.0, 3.0, 2.5, FeatherDirection::Outside, MergeMode::Normal),
            ),
            (
                body,
                flat(
                    -4.0,
                    -2.0,
                    3.0,
                    FeatherDirection::Average,
                    MergeMode::Difference,
                ),
            ),
            (
                Rect::new(40.0, 40.0, 60.0, 50.0),
                flat(2.0, 2.0, 1.0, FeatherDirection::Inside, MergeMode::Add),
            ),
            (body, floor),
            // Off the window's left and top edges.
            (
                Rect::new(0.0, 55.0, 15.0, 70.0),
                flat(
                    -3.0,
                    3.0,
                    4.0,
                    FeatherDirection::Middle,
                    MergeMode::Multiply,
                ),
            ),
            // Off the right and bottom edges, unfeathered.
            (
                Rect::new(75.0, 5.0, 90.0, 20.0),
                flat(4.0, -4.0, 0.0, FeatherDirection::Middle, MergeMode::Normal),
            ),
        ];
        for (k, (r, sh)) in cases.into_iter().enumerate() {
            for group in [false, true] {
                let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
                let layer = doc.pages[0].layers[0].id;
                // A filled object beneath, so the merge mode tells.
                let under = rect(
                    &mut doc,
                    Rect::new(20.0, 20.0, 90.0, 140.0),
                    Fill::Solid(Color::rgb8(230, 180, 40)),
                );
                // The shadowed object draws nothing itself, so only its
                // shadow lies over what is beneath.
                let mut top = rect(&mut doc, r, Fill::None);
                if group {
                    let second = rect(
                        &mut doc,
                        Rect::new(r.x0 + 2.0, r.y1 + 3.0, r.x1, r.y1 + 9.0),
                        Fill::None,
                    );
                    top = Shape::new(
                        doc.ids_mut().shape(),
                        ShapeKind::Group {
                            children: vec![top, second],
                        },
                    );
                    top.fill = Fill::None;
                    top.stroke = None;
                }
                let mut without = doc.clone();
                without.layer_mut(layer).unwrap().shapes = vec![under.clone(), top.clone()];
                top.shadow = Some(sh);
                let shadowed = top.clone();
                doc.layer_mut(layer).unwrap().shapes = vec![under, top];
                let page = doc.pages[0].id;
                let drawn = render_page(&doc, page, &opts).expect("render");
                let plain = render_page(&without, page, &opts).expect("render");
                // The reference: a layer the size of the window.
                let mut expected = plain.clone();
                let mut full = Pixmap::new(opts.width, opts.height).expect("layer");
                let zoom = opts.view.zoom;
                if group {
                    let b = group_shadow_box(&shadowed, Affine::IDENTITY).expect("box");
                    group_shadow_layer(
                        &mut full,
                        &sh,
                        &shadowed,
                        Affine::IDENTITY,
                        b,
                        screen,
                        zoom,
                        true,
                    );
                } else {
                    let path = shadowed.page_path();
                    assert!(shadow_layer(
                        &mut full,
                        &sh,
                        Shown {
                            shape: &shadowed,
                            path: &path,
                            object: shadowed.transform,
                            rule: shape_fill_rule(&shadowed, false),
                        },
                        screen,
                        zoom,
                        true
                    ));
                }
                let pp = tiny_skia::PixmapPaint {
                    blend_mode: merge_blend(sh.merge),
                    ..Default::default()
                };
                expected.draw_pixmap(0, 0, full.as_ref(), &pp, Transform::identity(), None);
                assert!(drawn.data() == expected.data(), "case {k}, group {group}");
                // And the shadow shows at all.
                assert!(drawn.data() != plain.data(), "case {k}, group {group}");
            }
        }
        // A shadow wholly outside the window paints nothing and takes no layer.
        let sh = flat(500.0, 0.0, 1.0, FeatherDirection::Middle, MergeMode::Normal);
        assert_eq!(shadow_pixels(&sh, body, screen, 3.0, 240, 180), None);
        // A small one takes a small layer.
        let sh = flat(2.0, -2.0, 1.0, FeatherDirection::Middle, MergeMode::Normal);
        let small = Rect::new(30.0, 30.0, 40.0, 40.0);
        let (_, _, w, h) = shadow_pixels(&sh, small, screen, 3.0, 240, 180).expect("pixels");
        assert!(w < 60 && h < 60, "{w} x {h}");
    }

    /// A clip frame and a group with a shadow of their own draw it: the
    /// silhouette of what they show, a clip frame's contents cut to its
    /// frame (an empty part of the frame or contents outside it cast
    /// none), each object of a group, moved with the object (#873).
    #[test]
    fn clip_frames_and_groups_cast_their_shadows() {
        let shadow = tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(15.0, 0.0),
            opacity: 1.0,
            blur: 0.0,
            color: Color::BLACK,
            merge: tracedraw_core::live::MergeMode::Normal,
            ..Default::default()
        };
        // Page 80 x 40 mm at 1 px a mm; image y = 40 - page y.
        let dark = |img: &Pixmap, x: u32, page_y: u32| {
            let p = img.pixel(x, 40 - page_y).expect("pixel");
            p.red() < 60 && p.green() < 60 && p.blue() < 60
        };
        let white = |img: &Pixmap, x: u32, page_y: u32| {
            let p = img.pixel(x, 40 - page_y).expect("pixel");
            p.red() > 240 && p.green() > 240 && p.blue() > 240
        };
        for moved in [0.0, 30.0] {
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(80.0, 40.0));
            let layer = doc.pages[0].layers[0].id;
            let mut frame = rect(1, Rect::new(10.0, 10.0, 30.0, 30.0), Color::BLACK);
            frame.fill = Fill::None;
            let inside = rect(2, Rect::new(5.0, 5.0, 20.0, 20.0), Color::rgb8(0, 0, 255));
            let mut clip = Shape::new(
                tracedraw_core::ShapeId(3),
                ShapeKind::ClipFrame {
                    frame: Box::new(frame),
                    contents: vec![inside],
                },
            );
            clip.shadow = Some(shadow);
            clip.transform = Affine::translate((moved, 0.0));
            doc.layer_mut(layer)
                .expect("layer")
                .shapes
                .push(clip.clone());
            let img = render_page_image(&doc, doc.pages[0].id, 25.4).expect("image");
            let x = |v: f64| (v + moved) as u32;
            // The shown part of the contents (10..20 square) casts a
            // shadow 15 mm to its right.
            assert!(dark(&img, x(30.0), 15), "{moved}");
            assert!(dark(&img, x(26.0), 12), "{moved}");
            // The empty part of the frame casts none, nor do the contents
            // outside it.
            assert!(white(&img, x(40.0), 25), "{moved}");
            assert!(white(&img, x(22.0), 7), "{moved}");
            // The contents still show.
            let c = img.pixel(x(15.0), 40 - 15).expect("pixel");
            assert!(c.blue() > 200 && c.red() < 60, "{c:?}");
            // Break Drop Shadow Apart: the same silhouette.
            let (r, pm) = render_shadow_image(&clip, 25.4).expect("shadow image");
            let at = |page_x: f64, page_y: f64| {
                pm.pixel((page_x - r.x0) as u32, (r.y1 - page_y) as u32)
                    .map_or(0, |p| p.alpha())
            };
            assert!(at(moved + 30.0, 15.0) > 200, "{r:?}");
            assert!(at(moved + 40.0, 25.0) < 20, "{r:?}");
        }
        // A group: each object's shadow, nothing between them.
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(80.0, 40.0));
        let layer = doc.pages[0].layers[0].id;
        let a = rect(1, Rect::new(5.0, 10.0, 10.0, 20.0), Color::rgb8(0, 0, 255));
        let b = rect(2, Rect::new(30.0, 10.0, 35.0, 20.0), Color::rgb8(0, 0, 255));
        let mut group = Shape::new(
            tracedraw_core::ShapeId(3),
            ShapeKind::Group {
                children: vec![a, b],
            },
        );
        group.shadow = Some(shadow);
        doc.layer_mut(layer).expect("layer").shapes.push(group);
        let img = render_page_image(&doc, doc.pages[0].id, 25.4).expect("image");
        assert!(dark(&img, 22, 15));
        assert!(dark(&img, 47, 15));
        assert!(white(&img, 40, 15));
        assert!(white(&img, 15, 15));
    }

    /// A clip frame with a drop shadow, its frame off whole pixels, draws
    /// as the plain object it shows with the same shadow: no thin line of
    /// the frame's fill or of the shadow along its edge, and nothing past
    /// the shadow. A shadow the frame had of its own goes once, beneath
    /// the contents, not a second time over them (live test of 0.41).
    #[test]
    fn a_clip_frame_with_a_shadow_draws_no_line_at_its_edge() {
        use tracedraw_core::live::MergeMode;
        let soft = tracedraw_core::Shadow::default();
        let sharp = tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(3.0, -3.0),
            opacity: 1.0,
            blur: 0.0,
            merge: MergeMode::Normal,
            ..soft
        };
        let blue = Color::rgb8(0, 0, 255);
        let box_ = Rect::new(20.06, 20.06, 40.06, 40.06);
        let render = |s: Shape| {
            let (doc, page) = doc_with(vec![s]);
            render_page_image(&doc, page, 25.4 * 8.0).expect("image")
        };
        let worst = |a: &Pixmap, b: &Pixmap| {
            let mut worst = (0i32, 0u32, 0u32);
            for y in 0..a.height() {
                for x in 0..a.width() {
                    let (p, q) = (px(a, x, y), px(b, x, y));
                    let d = [(p.0, q.0), (p.1, q.1), (p.2, q.2)]
                        .iter()
                        .map(|(u, v)| (*u as i32 - *v as i32).abs())
                        .max()
                        .unwrap_or(0);
                    if d > worst.0 {
                        worst = (d, x, y);
                    }
                }
            }
            worst
        };
        for fill in [
            Fill::Solid(Color::rgb8(255, 0, 0)),
            Fill::Solid(Color::WHITE),
            Fill::None,
        ] {
            for (k, sh) in [soft, sharp].into_iter().enumerate() {
                for on_frame in [false, true] {
                    let mut frame = rect(1, box_, Color::WHITE);
                    frame.fill = fill.clone();
                    let inside = rect(2, Rect::new(10.0, 10.0, 50.0, 50.0), blue);
                    if on_frame {
                        frame.shadow = Some(sh);
                    }
                    let mut clip = Shape::new(
                        ShapeId(3),
                        ShapeKind::ClipFrame {
                            frame: Box::new(frame),
                            contents: vec![inside],
                        },
                    );
                    if !on_frame {
                        clip.shadow = Some(sh);
                    }
                    let mut plain = rect(4, box_, blue);
                    plain.shadow = Some(sh);
                    let (d, x, y) = worst(&render(clip), &render(plain));
                    assert!(
                        d <= 3,
                        "fill {fill:?}, shadow {k}, on frame {on_frame}: off by {d} at {x}, {y}"
                    );
                }
            }
        }
    }

    /// A native ellipse or star casts the same shadow, flat or in
    /// perspective, as the same outline converted to curves: its own
    /// outline, not its box (#1323).
    #[test]
    fn native_shapes_cast_the_shadow_of_their_outline() {
        use tracedraw_core::shadow::{Perspective, ShadowEdge};
        let flat = tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(6.0, -6.0),
            opacity: 1.0,
            blur: 0.0,
            color: Color::BLACK,
            ..Default::default()
        };
        let persp = tracedraw_core::Shadow {
            perspective: Some(Perspective::new(ShadowEdge::Bottom)),
            ..flat
        };
        let kinds = [
            ShapeKind::Ellipse {
                rect: Rect::new(8.0, 12.0, 28.0, 32.0),
                arc: None,
            },
            ShapeKind::Polygon {
                rect: Rect::new(8.0, 12.0, 28.0, 32.0),
                points: 5,
                sharpness: 0.5,
                complex: None,
                twist: 0.0,
            },
        ];
        for kind in kinds {
            for sh in [flat, persp] {
                let scene = |curves: bool| {
                    let mut doc =
                        Document::new("t", tracedraw_core::geometry::Size::new(40.0, 40.0));
                    let layer = doc.pages[0].layers[0].id;
                    let mut s = Shape::new(doc.ids_mut().shape(), kind.clone());
                    s.transform = Affine::rotate_about(0.3, Point::new(18.0, 22.0));
                    if curves {
                        s.kind = ShapeKind::Path {
                            path: s.local_path(),
                            closed: true,
                        };
                    }
                    s.fill = Fill::Solid(Color::rgb8(0, 0, 255));
                    s.stroke = None;
                    s.shadow = Some(sh);
                    doc.layer_mut(layer).unwrap().shapes.push(s);
                    let page = doc.pages[0].id;
                    render_page_image(&doc, page, 25.4).unwrap()
                };
                let (native, curves) = (scene(false), scene(true));
                let differ = native
                    .pixels()
                    .iter()
                    .zip(curves.pixels())
                    .filter(|(a, b)| {
                        (i32::from(a.red()) - i32::from(b.red())).abs() > 8
                            || (i32::from(a.blue()) - i32::from(b.blue())).abs() > 8
                    })
                    .count();
                assert!(
                    differ < 4,
                    "{kind:?} {:?}: {differ} pixels differ",
                    sh.perspective
                );
            }
        }
    }

    /// The shadow is of what the object shows: an ellipse with no fill and
    /// a wide variable outline casts the ring its outline draws, its hole
    /// clear and its outer edge whole past the ellipse's own box, the same
    /// as after Convert to Curves. It cast the whole filled ellipse; only
    /// the shadow of its outline converted to an object was right.
    #[test]
    fn an_unfilled_object_casts_the_shadow_of_its_outline() {
        use tracedraw_core::style::WidthNode;
        let sh = tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(100.0, 0.0),
            opacity: 1.0,
            blur: 0.0,
            color: Color::BLACK,
            merge: tracedraw_core::live::MergeMode::Normal,
            ..Default::default()
        };
        let scene = |curves: bool, fill: Fill| {
            let mut s = Shape::new(
                tracedraw_core::ShapeId(1),
                ShapeKind::Ellipse {
                    rect: Rect::new(30.0, 30.0, 70.0, 70.0),
                    arc: None,
                },
            );
            if curves {
                s.kind = ShapeKind::Path {
                    path: s.local_path(),
                    closed: true,
                };
            }
            s.fill = fill;
            let mut st = tracedraw_core::Stroke::new(Color::BLACK, 1.0);
            // 8 mm each way at the bottom, tapering to 0.5 mm at the top.
            st.widths = vec![WidthNode::symmetric(0.75, 16.0)];
            s.stroke = Some(st);
            s.shadow = Some(sh);
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(200.0, 100.0));
            let layer = doc.pages[0].layers[0].id;
            doc.layer_mut(layer).unwrap().shapes.push(s);
            let page = doc.pages[0].id;
            render_page_image(&doc, page, 25.4).unwrap()
        };
        // One pixel a millimetre, y down from the page's top.
        let dark = |pm: &Pixmap, x: f64, y: f64| {
            let px = pm.pixel(x as u32, (100.0 - y) as u32).unwrap();
            px.red() < 128
        };
        for curves in [false, true] {
            let pm = scene(curves, Fill::None);
            // The shadow's middle (the hole) is clear, its ring dark.
            assert!(!dark(&pm, 150.5, 50.5), "curves {curves}: hole");
            assert!(dark(&pm, 150.5, 25.5), "curves {curves}: wide bottom");
            // Past the ellipse's box (30 to 70, moved 100): the outline's
            // reach at the bottom, 4 mm out, is not cut off.
            assert!(dark(&pm, 150.5, 27.5), "curves {curves}: outer edge");
            // A filled one stays whole.
            let pm = scene(curves, Fill::Solid(Color::WHITE));
            assert!(dark(&pm, 150.5, 50.5), "curves {curves}: filled");
        }
        let (a, b) = (scene(false, Fill::None), scene(true, Fill::None));
        let differ = a
            .pixels()
            .iter()
            .zip(b.pixels())
            .filter(|(p, q)| (i32::from(p.red()) - i32::from(q.red())).abs() > 8)
            .count();
        assert!(differ < 8, "{differ} pixels differ");
    }

    /// #2505: a square with no fill and a thick black outline, given a
    /// soft drop shadow (opacity 63 %, feathering 1.5 mm, offset 2 mm
    /// right and 2 mm down, Multiply): the shadow follows the outline and
    /// the inside stays clear. A filled square beside it shadows its
    /// whole inside.
    #[test]
    fn an_unfilled_square_shadows_only_its_outline() {
        let sh = tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(2.0, -2.0),
            opacity: 0.63,
            blur: 1.5,
            color: Color::BLACK,
            merge: tracedraw_core::live::MergeMode::Multiply,
            ..Default::default()
        };
        let square = |id: u64, x: f64, fill: Fill| {
            let mut s = Shape::new(
                tracedraw_core::ShapeId(id),
                ShapeKind::Rect {
                    rect: Rect::new(x, 20.0, x + 40.0, 60.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = fill;
            s.stroke = Some(tracedraw_core::Stroke::new(Color::BLACK, 2.0));
            s.shadow = Some(sh);
            s
        };
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(140.0, 80.0));
        let layer = doc.pages[0].layers[0].id;
        let shapes = &mut doc.layer_mut(layer).unwrap().shapes;
        shapes.push(square(1, 10.0, Fill::None));
        shapes.push(square(2, 80.0, Fill::Solid(Color::rgb8(190, 225, 245))));
        let page = doc.pages[0].id;
        // Two pixels a millimetre, y down from the page's top.
        let pm = render_page_image(&doc, page, 50.8).unwrap();
        let red = |x: f64, y: f64| {
            pm.pixel((x * 2.0) as u32, ((80.0 - y) * 2.0) as u32)
                .unwrap()
                .red()
        };
        // Inside the unfilled square, where its shadow's inside falls:
        // the white page.
        assert!(
            red(32.0, 38.0) > 245,
            "shadow over the inside: {}",
            red(32.0, 38.0)
        );
        // Its shadow beside the outline, right of the right side and
        // below the bottom.
        assert!(
            red(51.8, 30.0) < 200,
            "no shadow by the outline: {}",
            red(51.8, 30.0)
        );
        assert!(
            red(35.0, 18.2) < 200,
            "no shadow below: {}",
            red(35.0, 18.2)
        );
        // The filled square's shadow is under all of it, seen past its
        // right side.
        assert!(red(121.8, 30.0) < 200, "the filled square's shadow");
    }

    /// A blue square at 20 to 40 mm with a body running 10 mm up and to
    /// the right and a flat, sharp black shadow.
    fn extruded_square(offset: (f64, f64), turn: [f64; 3]) -> Shape {
        let mut s = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Rect {
                rect: Rect::new(20.0, 20.0, 40.0, 40.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(0, 0, 255));
        s.stroke = None;
        s.effects = vec![tracedraw_core::live::Effect::Extrude {
            depth: tracedraw_core::geometry::Vec2::new(10.0, 10.0),
            vanishing: None,
            amount: 0.0,
            shade_from: None,
            shade_to: None,
            light_angle: 135.0,
            light_intensity: 50.0,
            bevel: 0.0,
            kind: None,
            on_object: false,
            solid: None,
            space: tracedraw_core::extrude3d::Solid {
                rotation: turn,
                ..Default::default()
            },
        }];
        s.shadow = Some(tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(offset.0, offset.1),
            opacity: 1.0,
            blur: 0.0,
            color: Color::BLACK,
            merge: tracedraw_core::live::MergeMode::Normal,
            ..Default::default()
        });
        s
    }

    /// d#2596: the picture of a drop shadow for PDF output follows a
    /// picture's pixels, as on the page (#2027). A picture with a
    /// transparent background cast the shadow of its whole box, a grey
    /// slab under a cut-out drawing.
    #[test]
    fn the_shadow_picture_of_a_picture_follows_its_pixels() {
        // 10 by 10 pixels, opaque only in the left half.
        let mut src = Pixmap::new(10, 10).unwrap();
        let mut paint = Paint::default();
        paint.set_color_rgba8(255, 0, 0, 255);
        src.fill_rect(
            tiny_skia::Rect::from_xywh(0.0, 0.0, 5.0, 10.0).unwrap(),
            &paint,
            Transform::identity(),
            None,
        );
        let mut s = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Bitmap {
                rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                width_px: 10,
                height_px: 10,
                png: src.encode_png().unwrap().into(),
                fx: None,
            },
        );
        s.fill = Fill::None;
        s.stroke = None;
        s.shadow = Some(tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(0.0, 0.0),
            opacity: 1.0,
            blur: 0.0,
            color: Color::BLACK,
            merge: tracedraw_core::live::MergeMode::Normal,
            ..Default::default()
        });
        let (area, pm) = render_shadow_image(&s, 25.4).expect("picture");
        let at = |x: f64, y: f64| {
            pm.pixel((x - area.x0) as u32, (area.y1 - y) as u32)
                .map_or(0, |p| p.alpha())
        };
        assert_eq!(at(5.0, 10.0), 255, "under the opaque half");
        assert_eq!(at(15.0, 10.0), 0, "none under the transparent half");
    }

    /// #2215: an extruded object casts the shadow of the object and its
    /// body together, beneath both. The shadow was the object's alone and
    /// was drawn over the body behind it, a dark patch on the body's
    /// faces; a body turned in space cast none.
    #[test]
    fn an_extruded_object_casts_the_shadow_of_its_body_beneath_it() {
        let black = (0, 0, 0);
        // The shadow 4 mm to the right: the square's own would cover the
        // body's right face up to x = 44.
        let s = extruded_square((4.0, 0.0), [0.0; 3]);
        let (doc, page) = doc_with(vec![s.clone()]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        // Image rows run down: page y = 100 - row.
        let (r, g, b) = px(&pm, 42, 68);
        assert!(b > 60 && r < 40 && g < 40, "the body's face: {r} {g} {b}");
        assert_eq!(px(&pm, 30, 70), (0, 0, 255), "the object");
        // Beyond the body (it ends at x = 50) lies the body's shadow.
        assert_eq!(px(&pm, 52, 60), black, "the body's shadow");
        assert_eq!(px(&pm, 56, 60), (255, 255, 255));
        // The picture of the shadow has the body in it.
        let (area, alone) = render_shadow_image(&s, 25.4).expect("picture");
        assert!(
            (area.x1 - 54.0).abs() < 1.5 && (area.y1 - 50.0).abs() < 1.5,
            "{area:?}"
        );
        assert!(
            (area.x0 - 24.0).abs() < 1.5 && (area.y0 - 20.0).abs() < 1.5,
            "{area:?}"
        );
        let at = |x: f64, y: f64| {
            alone
                .pixel((x - area.x0) as u32, (area.y1 - y) as u32)
                .map_or(0, |p| p.alpha())
        };
        assert_eq!(at(52.5, 45.5), 255, "the body's far corner");
        assert_eq!(at(26.5, 45.5), 0, "above the object, left of the body");
        // In a group that has the shadow the body casts it too.
        let mut child = s.clone();
        let shadow = child.shadow.take();
        let mut group = Shape::new(
            tracedraw_core::ShapeId(2),
            ShapeKind::Group {
                children: vec![child],
            },
        );
        group.shadow = shadow;
        let (doc, page) = doc_with(vec![group]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&pm, 52, 60), black, "a group's shadow");
        // Turned in space the body casts a shadow at all.
        let turned = extruded_square((40.0, 0.0), [0.0, 30.0, 0.0]);
        let (doc, page) = doc_with(vec![turned]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&pm, 70, 70), black, "a turned body's shadow");
    }

    /// The picture of the shadow of an enveloped object (Break Drop
    /// Shadow Apart, PDF and `.cdr` output) is of the outline the envelope
    /// draws, where the page shows the shadow; it was of the outline
    /// without the envelope (#2217).
    #[test]
    fn the_shadow_picture_of_an_enveloped_object_is_of_what_is_drawn() {
        let mut s = extruded_square((40.0, 0.0), [0.0; 3]);
        // The envelope stretches the square to twice its height.
        s.effects = vec![tracedraw_core::live::Effect::envelope_around(
            Rect::new(20.0, 20.0, 40.0, 60.0),
            false,
        )];
        let (area, pm) = render_shadow_image(&s, 25.4).expect("picture");
        assert!(
            (area.x0 - 60.0).abs() < 1.5 && (area.x1 - 80.0).abs() < 1.5,
            "{area:?}"
        );
        assert!(
            (area.y0 - 20.0).abs() < 1.5 && (area.y1 - 60.0).abs() < 1.5,
            "{area:?}"
        );
        assert_eq!(pm.pixel(10, 3).map(|p| p.alpha()), Some(255));
        // The page shows it there: beside the drawn top, page y = 55.
        let (doc, page) = doc_with(vec![s]);
        let pm = render_page_image(&doc, page, 25.4).unwrap();
        assert_eq!(px(&pm, 70, 45), (0, 0, 0));
        assert_eq!(px(&pm, 30, 45), (0, 0, 255));
    }

    /// A ring as a user makes one, with a flat black shadow 30 mm to the
    /// right: two circles combined, or one curve whose inner circle runs
    /// the same way round as the outer one (`reversed` false) or the
    /// other way, filled by `rule`.
    fn shadowed_ring(
        combined: bool,
        reversed: bool,
        rule: tracedraw_core::shaping::FillRule,
    ) -> Shape {
        let circle = |r: f64| tracedraw_core::geometry::Circle::new((30.0, 50.0), r).to_path(0.01);
        let mut ring = if combined {
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
            let layer = doc.pages[0].layers[0].id;
            let mut ids = Vec::new();
            for r in [20.0, 10.0] {
                let mut s = Shape::new(
                    doc.ids_mut().shape(),
                    ShapeKind::Ellipse {
                        rect: Rect::new(30.0 - r, 50.0 - r, 30.0 + r, 50.0 + r),
                        arc: None,
                    },
                );
                s.fill = Fill::Solid(Color::rgb8(0, 0, 255));
                ids.push(s.id);
                tracedraw_core::Command::AddShape { layer, shape: s }
                    .apply(&mut doc)
                    .unwrap();
            }
            tracedraw_core::Command::Combine { shapes: ids }
                .apply(&mut doc)
                .unwrap();
            doc.layer(layer).unwrap().shapes[0].clone()
        } else {
            let mut path = circle(20.0);
            let inner = circle(10.0);
            path.extend(if reversed {
                inner.reverse_subpaths()
            } else {
                inner
            });
            Shape::new(ShapeId(1), ShapeKind::Path { path, closed: true })
        };
        ring.fill = Fill::Solid(Color::rgb8(0, 0, 255));
        ring.stroke = None;
        ring.fill_rule = rule;
        ring.shadow = Some(tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(30.0, 0.0),
            opacity: 1.0,
            blur: 0.0,
            color: Color::BLACK,
            ..Default::default()
        });
        ring
    }

    /// The drop shadow of an object with a hole has the hole too (#2216:
    /// it was filled by the nonzero rule whatever the object's own, so
    /// a ring of two circles combined cast a solid disc). It is open
    /// wherever the object is: on the page, alone as a picture (Break
    /// Drop Shadow Apart, PDF and `.cdr` output) and as part of a
    /// group's shadow. A curve set to the nonzero rule whose inner
    /// circle runs the same way round has no hole, and neither has its
    /// shadow.
    #[test]
    fn a_drop_shadow_keeps_the_holes_of_its_object() {
        use tracedraw_core::shaping::FillRule as Rule;
        let white = (255, 255, 255);
        let cases = [
            ("combined", true, false, Rule::EvenOdd),
            ("same way, even-odd", false, false, Rule::EvenOdd),
            ("reversed, even-odd", false, true, Rule::EvenOdd),
            ("reversed, nonzero", false, true, Rule::NonZero),
            ("same way, nonzero", false, false, Rule::NonZero),
        ];
        let mut failed = Vec::new();
        for (name, combined, reversed, rule) in cases {
            let ring = shadowed_ring(combined, reversed, rule);
            let (doc, page) = doc_with(vec![ring.clone()]);
            let pm = render_page_image(&doc, page, 25.4).expect("page");
            // The object itself says whether it has a hole.
            let hole = px(&pm, 30, 50) == white;
            assert_eq!(hole, !(rule == Rule::NonZero && !reversed), "{name}");
            assert_eq!(px(&pm, 15, 50), (0, 0, 255), "{name}: the ring");
            assert_eq!(px(&pm, 75, 50), (0, 0, 0), "{name}: the shadow's ring");
            // On the page: the shadow's middle, 30 mm right of the hole.
            if (px(&pm, 60, 50) == white) != hole {
                failed.push(format!("{name}: on the page"));
            }
            // Alone as a picture.
            let (area, alone) = render_shadow_image(&ring, 25.4).expect("picture");
            let at = |x: f64| (x - area.x0).floor() as u32;
            let y = (area.y1 - 50.0).floor() as u32;
            assert_eq!(alone.pixel(at(75.0), y).unwrap().alpha(), 255, "{name}");
            if (alone.pixel(at(60.0), y).unwrap().alpha() == 0) != hole {
                failed.push(format!("{name}: as a picture"));
            }
            // In a group that has the shadow.
            let mut child = ring.clone();
            let shadow = child.shadow.take();
            let mut group = Shape::new(
                ShapeId(9),
                ShapeKind::Group {
                    children: vec![child],
                },
            );
            group.shadow = shadow;
            let (doc, page) = doc_with(vec![group]);
            let pm = render_page_image(&doc, page, 25.4).expect("page");
            assert_eq!(px(&pm, 75, 50), (0, 0, 0), "{name}: the group's shadow");
            if (px(&pm, 60, 50) == white) != hole {
                failed.push(format!("{name}: in a group"));
            }
        }
        assert!(failed.is_empty(), "shadow and object differ: {failed:#?}");
    }

    /// A 40 x 40 mm page with a red square under a blue square that has
    /// `shadow`; rendered at 1 px per mm.
    fn shadow_scene(shadow: tracedraw_core::Shadow) -> Pixmap {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(40.0, 40.0));
        let layer = doc.pages[0].layers[0].id;
        let mut under = rect(
            doc.ids_mut().shape().0,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            Color::rgb8(255, 0, 0),
        );
        under.stroke = None;
        let mut s = rect(
            doc.ids_mut().shape().0,
            Rect::new(10.0, 20.0, 20.0, 30.0),
            Color::rgb8(0, 0, 255),
        );
        s.stroke = None;
        s.shadow = Some(shadow);
        let l = doc.layer_mut(layer).unwrap();
        l.shapes.push(under);
        l.shapes.push(s);
        let page = doc.pages[0].id;
        render_page_image(&doc, page, 25.4).unwrap()
    }

    #[test]
    fn shadow_merge_mode_multiplies_with_what_is_beneath() {
        let base = tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(5.0, -5.0),
            opacity: 1.0,
            blur: 0.0,
            color: Color::rgb8(0, 255, 0),
            ..Default::default()
        };
        // Normal: green over red is green.
        let normal = shadow_scene(tracedraw_core::Shadow {
            merge: tracedraw_core::live::MergeMode::Normal,
            ..base
        });
        // Multiply (the default): green times red is black.
        let multiply = shadow_scene(base);
        // (22, 22) in the image is inside the shadow, outside the object.
        let n = normal.pixel(22, 22).unwrap();
        let m = multiply.pixel(22, 22).unwrap();
        assert!(n.green() > 200 && n.red() < 50, "{n:?}");
        assert!(m.green() < 50 && m.red() < 50, "{m:?}");
    }

    #[test]
    fn perspective_shadow_falls_on_the_floor_and_fades() {
        use tracedraw_core::shadow::{Perspective, ShadowEdge};
        let mut sh = tracedraw_core::Shadow {
            opacity: 1.0,
            blur: 0.0,
            color: Color::BLACK,
            merge: tracedraw_core::live::MergeMode::Normal,
            perspective: Some(Perspective::new(ShadowEdge::Bottom)),
            ..Default::default()
        };
        // The square's base at y 20 stays; its top lands at (38, 22).
        let b = Rect::new(10.0, 20.0, 20.0, 30.0);
        sh.set_end_point(b, tracedraw_core::geometry::Point::new(38.0, 22.0));
        if let Some(p) = sh.perspective.as_mut() {
            p.fade = 0.8;
        }
        let img = shadow_scene(sh);
        // Image y = 40 - page y. Near the base, right of the square: dark.
        let near = img.pixel(22, 19).unwrap();
        // Far along the shadow: lighter (faded), still darker than the red.
        let far = img.pixel(33, 18).unwrap();
        assert!(near.red() < 120, "{near:?}");
        assert!(far.red() > near.red(), "{far:?} {near:?}");
        // Above the square's top where a flat shadow would not be: red.
        let above = img.pixel(15, 5).unwrap();
        assert_eq!((above.red(), above.green()), (255, 0));
    }

    #[test]
    fn shadow_image_holds_the_shadow_alone() {
        let mut s = rect(1, Rect::new(10.0, 10.0, 20.0, 20.0), Color::rgb8(0, 0, 255));
        assert!(render_shadow_image(&s, 100.0).is_none());
        s.shadow = Some(tracedraw_core::Shadow {
            offset: tracedraw_core::geometry::Vec2::new(5.0, -5.0),
            blur: 1.0,
            ..Default::default()
        });
        let (r, pm) = render_shadow_image(&s, 100.0).expect("image");
        assert!(
            r.x0 < 15.0 && r.x1 > 25.0 && r.y0 < 5.0 && r.y1 > 15.0,
            "{r:?}"
        );
        let zoom = 100.0 / 25.4;
        assert!((pm.width() as f64 - r.width() * zoom).abs() < 1.0);
        // The middle of the shadow is half opaque black.
        let cx = ((20.0 - r.x0) * zoom) as u32;
        let cy = ((r.y1 - 10.0) * zoom) as u32;
        let c = pm.pixel(cx, cy).unwrap().demultiply();
        assert!((c.alpha() as i32 - 128).abs() < 10, "{c:?}");
        assert!(render_shadow_image(&s, f64::NAN).is_none());
    }

    #[test]
    fn inside_outlines_stay_inside_the_fill() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(40.0, 40.0));
        let layer = doc.pages[0].layers[0].id;
        let mut s = rect(
            doc.ids_mut().shape().0,
            Rect::new(10.0, 10.0, 30.0, 30.0),
            Color::rgb8(0, 0, 255),
        );
        let mut st = Stroke::new(Color::BLACK, 4.0);
        st.position = tracedraw_core::OutlinePosition::Inside;
        s.stroke = Some(st.clone());
        doc.layer_mut(layer).unwrap().shapes.push(s.clone());
        let page = doc.pages[0].id;
        let img = render_page_image(&doc, page, 25.4).unwrap();
        // 1 mm outside the edge: white; 1 mm and 3 mm inside: black.
        assert_eq!(img.pixel(8, 20).unwrap().red(), 255);
        assert!(img.pixel(11, 20).unwrap().red() < 60);
        assert!(img.pixel(13, 20).unwrap().red() < 60);
        // 5 mm inside: the blue fill.
        assert!(img.pixel(15, 20).unwrap().blue() > 200);
        // Outside: the same width beyond the edge, the fill untouched.
        st.position = tracedraw_core::OutlinePosition::Outside;
        if let Some(sh) = doc.find_shape_mut(s.id) {
            sh.stroke = Some(st);
        }
        let img = render_page_image(&doc, page, 25.4).unwrap();
        assert!(img.pixel(7, 20).unwrap().red() < 60);
        assert!(img.pixel(11, 20).unwrap().blue() > 200);
        assert_eq!(img.pixel(5, 20).unwrap().red(), 255);
    }

    /// A dashed hairline is drawn dashed (#2463): along a hairline with
    /// the pattern 4 on, 4 off, some pixels are dark and some stay white.
    /// It was drawn solid.
    #[test]
    fn a_dashed_hairline_keeps_its_gaps() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(50.0, 50.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Path {
                path: {
                    let mut p = BezPath::new();
                    p.move_to((5.0, 25.0));
                    p.line_to((45.0, 25.0));
                    p
                },
                closed: false,
            },
        );
        let mut st = Stroke::new(Color::BLACK, Stroke::HAIRLINE);
        st.dash = vec![4.0, 4.0];
        s.stroke = Some(st);
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let page = doc.pages[0].id;
        // 4 pixels a millimetre: the hairline is one pixel wide and a unit
        // of the pattern one pixel.
        let img = render_page_image(&doc, page, 101.6).unwrap();
        let row: Vec<u8> = (30..170)
            .map(|x| {
                (99..102)
                    .map(|y| img.pixel(x, y).unwrap().red())
                    .min()
                    .unwrap()
            })
            .collect();
        let dark = row.iter().filter(|r| **r < 128).count();
        let white = row.iter().filter(|r| **r > 230).count();
        assert!(
            dark > 30 && white > 30,
            "{dark} dark, {white} white: {row:?}"
        );
    }

    #[test]
    fn dashed_hairline_does_not_panic() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(50.0, 50.0));
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(5.0, 5.0, 45.0, 45.0),
                radius: 0.0,
                corners: None,
            },
        );
        let mut st = Stroke::new(Color::BLACK, 1.0);
        st.dash = vec![3.0, 1.0, 1.0];
        s.stroke = Some(st);
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let page = doc.pages[0].id;
        assert!(render_page_image(&doc, page, 96.0).is_some());
    }
}

#[cfg(test)]
mod culling_tests {
    use super::*;
    use tracedraw_core::document::{Shape, ShapeKind};

    fn rect_shape(id: u64, r: Rect) -> Shape {
        let mut s = Shape::new(
            tracedraw_core::ShapeId(id),
            ShapeKind::Rect {
                rect: r,
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = tracedraw_core::Fill::Solid(tracedraw_core::Color::rgb8(10, 20, 30));
        s
    }

    /// Zoomed in on a large group, only its members that show are drawn
    /// (discussion #2389): every member was drawn at the zoom's detail,
    /// so a zoom in on a grouped design drew far slower than the page
    /// fitted in the window. The picture is the same as with every
    /// member drawn, and a semi-transparent group leaves them out too.
    #[test]
    fn a_zoom_into_a_group_draws_only_the_members_that_show() {
        let members: Vec<Shape> = (0..400u64)
            .map(|i| {
                let (x, y) = ((i % 20) as f64 * 5.0, (i / 20) as f64 * 5.0);
                let mut r = rect_shape(10 + i, Rect::new(x, y, x + 4.0, y + 4.0));
                r.stroke = Some(tracedraw_core::Stroke::new(
                    tracedraw_core::Color::BLACK,
                    0.2,
                ));
                r
            })
            .collect();
        let group = |opacity: f64| {
            let mut g = Shape::new(
                tracedraw_core::ShapeId(1),
                ShapeKind::Group {
                    children: members.clone(),
                },
            );
            g.opacity = opacity;
            g
        };
        // 40 px per mm on a 200 x 200 window: page x 0..5, y 95..100,
        // about four members of the 400.
        let opts = RenderOptions {
            width: 200,
            height: 200,
            view: ViewTransform {
                zoom: 40.0,
                origin_x: 0.0,
                origin_y: 100.0 * 40.0,
            },
            ..RenderOptions::default()
        };
        let doc_of = |shapes: Vec<Shape>| {
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
            let page = doc.pages[0].id;
            doc.pages[0].layers[0].shapes = shapes;
            (doc, page)
        };
        let drawn = |doc: &Document, page| {
            PATHS_DRAWN.with(|n| n.set(0));
            let pm = render_page_whole(doc, page, &opts, None).expect("render");
            (pm, PATHS_DRAWN.with(|n| n.get()))
        };
        let (doc, page) = doc_of(vec![group(1.0)]);
        let (grouped, n) = drawn(&doc, page);
        assert!(n > 0 && n <= 16, "{n} members drawn of 400");
        // The same members loose on the layer: the same picture.
        let (loose_doc, page) = doc_of(members.clone());
        let (loose, _) = drawn(&loose_doc, page);
        assert!(grouped.data() == loose.data(), "the picture changed");
        let (doc, page) = doc_of(vec![group(0.5)]);
        let (_, n) = drawn(&doc, page);
        assert!(
            n > 0 && n <= 16,
            "{n} members drawn of 400, half transparent"
        );
        // The whole group in sight: every member drawn.
        let whole = RenderOptions {
            view: ViewTransform {
                zoom: 2.0,
                origin_x: 0.0,
                origin_y: 200.0,
            },
            ..opts.clone()
        };
        PATHS_DRAWN.with(|n| n.set(0));
        let (doc, page) = doc_of(vec![group(1.0)]);
        render_page_whole(&doc, page, &whole, None).expect("render");
        assert_eq!(PATHS_DRAWN.with(|n| n.get()), 400);
    }

    /// A clip frame's contents are put together on a layer the size of
    /// the frame's box, not of the whole canvas (#2522): the contents
    /// outside the box are left out as off screen, and the picture is
    /// the one the contents inside the frame give.
    #[test]
    fn a_clip_frame_puts_its_contents_together_on_its_own_box() {
        let contents: Vec<Shape> = (0..400u64)
            .map(|i| {
                let (x, y) = ((i % 20) as f64 * 5.0, (i / 20) as f64 * 5.0);
                let mut r = rect_shape(10 + i, Rect::new(x, y, x + 4.0, y + 4.0));
                r.fill = tracedraw_core::Fill::Solid(tracedraw_core::Color::rgb8(
                    (i * 7 % 256) as u8,
                    90,
                    (i * 13 % 256) as u8,
                ));
                r.stroke = Some(tracedraw_core::Stroke::new(
                    tracedraw_core::Color::BLACK,
                    0.3,
                ));
                r
            })
            .collect();
        let clip = |contents: Vec<Shape>| {
            // A round frame over page x 2..13, y 2..13, filled pale.
            let mut frame = Shape::new(
                tracedraw_core::ShapeId(2),
                ShapeKind::Ellipse {
                    rect: Rect::new(2.0, 2.0, 13.0, 13.0),
                    arc: None,
                },
            );
            frame.fill = tracedraw_core::Fill::Solid(tracedraw_core::Color::rgb8(250, 240, 200));
            frame.stroke = None;
            Shape::new(
                tracedraw_core::ShapeId(1),
                ShapeKind::ClipFrame {
                    frame: Box::new(frame),
                    contents,
                },
            )
        };
        let mut under = rect_shape(3, Rect::new(0.0, 0.0, 40.0, 40.0));
        under.fill = tracedraw_core::Fill::Solid(tracedraw_core::Color::rgb8(30, 160, 60));
        // The whole page on a 200 x 200 window, 2 px per mm.
        let opts = RenderOptions {
            width: 200,
            height: 200,
            view: ViewTransform {
                zoom: 2.0,
                origin_x: 0.0,
                origin_y: 200.0,
            },
            ..RenderOptions::default()
        };
        let render = |contents: Vec<Shape>| {
            let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
            let page = doc.pages[0].id;
            doc.pages[0].layers[0].shapes = vec![under.clone(), clip(contents)];
            PATHS_DRAWN.with(|n| n.set(0));
            let pm = render_page_whole(&doc, page, &opts, None).expect("render");
            (pm, PATHS_DRAWN.with(|n| n.get()))
        };
        let (all, n) = render(contents.clone());
        // The frame's box (with a few pixels round it) takes in the
        // members at the page's corner: far fewer than the 400 (each a
        // fill and an outline), which were all drawn on a canvas-sized
        // layer.
        assert!(
            n <= 100,
            "{n} paths drawn for a frame over 9 of 400 members"
        );
        // Only the members over the frame: the same picture.
        let near: Vec<Shape> = contents
            .iter()
            .filter(|s| {
                let b = s.bounds();
                b.x0 < 15.0 && b.y0 < 15.0
            })
            .cloned()
            .collect();
        let (few, _) = render(near);
        assert!(all.data() == few.data(), "the picture changed");
        // Inside the frame the members show; outside, what lies beneath.
        let px = |pm: &Pixmap, x: f64, y: f64| {
            let p = pm
                .pixel((x * 2.0) as u32, (200.0 - y * 2.0) as u32)
                .expect("pixel");
            (p.red(), p.green(), p.blue())
        };
        assert_eq!(px(&all, 30.0, 30.0), (30, 160, 60));
        assert_eq!(
            px(&all, 2.5, 12.5),
            (30, 160, 60),
            "outside the round frame"
        );
        assert_ne!(
            px(&all, 7.0, 7.0),
            (30, 160, 60),
            "a member inside the frame"
        );
    }

    /// A render told to give up (the view zoomed on, #2522) gives no
    /// picture; one never told gives the picture of `render_page`.
    #[test]
    fn a_cancelled_render_gives_no_picture() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let page = doc.pages[0].id;
        doc.pages[0].layers[0].shapes = (0..50u64)
            .map(|i| rect_shape(i + 1, Rect::new(i as f64, 10.0, i as f64 + 1.0, 90.0)))
            .collect();
        let opts = RenderOptions {
            width: 600,
            height: 600,
            view: ViewTransform {
                zoom: 6.0,
                origin_x: 0.0,
                origin_y: 600.0,
            },
            ..RenderOptions::default()
        };
        let go = std::sync::atomic::AtomicBool::new(false);
        let drawn = render_page_cancellable(&doc, page, &opts, &go).expect("drawn");
        let plain = render_page(&doc, page, &opts).expect("drawn");
        assert!(drawn.data() == plain.data());
        let stop = std::sync::atomic::AtomicBool::new(true);
        assert!(render_page_cancellable(&doc, page, &opts, &stop).is_none());
    }

    /// Objects wholly outside the part of the page on screen are not
    /// drawn (a page of thousands of objects pans at the cost of the
    /// visible ones); an outline, a shadow or an effect that may reach
    /// in keeps an object drawn.
    #[test]
    fn objects_out_of_sight_are_skipped() {
        let opts = RenderOptions {
            width: 400,
            height: 300,
            // 4 px per mm, the view's bottom-left at page (100, 100):
            // page x 100..200, y 100..175 on screen.
            view: ViewTransform {
                zoom: 4.0,
                origin_x: -400.0,
                origin_y: 300.0 + 400.0,
            },
            ..RenderOptions::default()
        };
        let visible = visible_page_rect(&opts);
        assert!((visible.x0 - 100.0).abs() < 1e-9 && (visible.x1 - 200.0).abs() < 1e-9);
        assert!((visible.y0 - 100.0).abs() < 1e-9 && (visible.y1 - 175.0).abs() < 1e-9);
        let mut pixmap = Pixmap::new(400, 300).unwrap();
        let r = Renderer {
            pixmap: &mut pixmap,
            screen: opts.view.affine(),
            zoom: 4.0,
            flags: Flags::from_options(&opts),
            symbols: &[],
            preview: None,
            origin: (0, 0),
            cancel: None,
        };
        let inside = rect_shape(1, Rect::new(120.0, 120.0, 130.0, 130.0));
        let far = rect_shape(2, Rect::new(0.0, 0.0, 10.0, 10.0));
        let touching = rect_shape(3, Rect::new(90.0, 90.0, 100.5, 100.5));
        assert!(!r.out_of_sight(&inside, &visible));
        assert!(r.out_of_sight(&far, &visible));
        assert!(!r.out_of_sight(&touching, &visible));
        // Two millimetres short of the view, but a 1 mm outline reaches it.
        let mut near = rect_shape(4, Rect::new(80.0, 120.0, 97.0, 130.0));
        assert!(r.out_of_sight(&near, &visible));
        near.stroke = Some(tracedraw_core::Stroke::new(
            tracedraw_core::Color::BLACK,
            1.0,
        ));
        assert!(!r.out_of_sight(&near, &visible));
        // Scaled up five times: the outline reaches five times as far.
        let mut scaled = rect_shape(5, Rect::new(10.0, 25.0, 15.0, 27.0));
        scaled.transform = Affine::scale(5.0);
        scaled.stroke = Some(tracedraw_core::Stroke::new(
            tracedraw_core::Color::BLACK,
            0.5,
        ));
        assert!(!r.out_of_sight(&scaled, &visible), "{:?}", scaled.bounds());
        // A shadow may fall anywhere: drawn.
        let mut shadowed = rect_shape(6, Rect::new(0.0, 0.0, 10.0, 10.0));
        shadowed.shadow = Some(tracedraw_core::document::Shadow::default());
        assert!(!r.out_of_sight(&shadowed, &visible));
        // A thin outline with a Variable Length node reaching 8 mm: the
        // band reaches the view from 5 mm away, so it is drawn (#626).
        let mut path = BezPath::new();
        path.move_to((120.0, 95.0));
        path.line_to((150.0, 95.0));
        let mut thin = Shape::new(
            tracedraw_core::ShapeId(7),
            ShapeKind::Path {
                path,
                closed: false,
            },
        );
        let mut st = tracedraw_core::Stroke::new(tracedraw_core::Color::BLACK, 0.2);
        thin.stroke = Some(st.clone());
        assert!(r.out_of_sight(&thin, &visible));
        st.widths = vec![tracedraw_core::WidthNode::symmetric(0.5, 16.0)];
        thin.stroke = Some(st);
        assert!(!r.out_of_sight(&thin, &visible));
        // Hidden objects never draw.
        let mut hidden = inside.clone();
        hidden.visible = false;
        assert!(r.out_of_sight(&hidden, &visible));
        // Through the page render: the far object leaves no pixel.
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(300.0, 300.0));
        let page = doc.pages[0].id;
        let layer = doc.pages[0].layers[0].id;
        doc.layer_mut(layer).unwrap().shapes.push(far);
        let px = render_page(&doc, page, &opts).unwrap();
        assert!(px.pixels().iter().all(|p| p.alpha() == 0));
        doc.layer_mut(layer).unwrap().shapes.push(inside);
        let px = render_page(&doc, page, &opts).unwrap();
        assert!(px.pixels().iter().any(|p| p.alpha() > 0));
    }

    /// Focus at 0 % visibility draws the objects in focus and nothing
    /// else: no page colour, no other object (#981). Above 0 the rest
    /// still shows, faded.
    #[test]
    fn focus_at_no_visibility_draws_only_the_focus() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let page = doc.pages[0].id;
        let layer = doc.pages[0].layers[0].id;
        doc.pages[0].background = Some(tracedraw_core::Fill::Solid(tracedraw_core::Color::rgb8(
            200, 0, 0,
        )));
        let shapes = &mut doc.layer_mut(layer).unwrap().shapes;
        shapes.push(rect_shape(1, Rect::new(0.0, 0.0, 40.0, 100.0)));
        shapes.push(rect_shape(2, Rect::new(60.0, 0.0, 100.0, 100.0)));
        let opts = |v: f32| RenderOptions {
            width: 100,
            height: 100,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 100.0,
            },
            focus: Some((vec![tracedraw_core::ShapeId(2)], v)),
            ..RenderOptions::default()
        };
        assert!(rest_hidden(&opts(0.0)));
        assert!(rest_hidden(&opts(f32::NAN)));
        assert!(!rest_hidden(&opts(0.2)));
        assert!(!rest_hidden(&RenderOptions::default()));
        let alpha = |pm: &Pixmap, x: u32| pm.pixel(x, 50).unwrap().alpha();
        let hidden = render_page(&doc, page, &opts(0.0)).unwrap();
        assert_eq!(
            alpha(&hidden, 20),
            0,
            "the object out of focus is not drawn"
        );
        assert_eq!(alpha(&hidden, 50), 0, "the page colour is not drawn");
        assert_eq!(alpha(&hidden, 80), 255, "the object in focus is drawn");
        let faded = render_page(&doc, page, &opts(0.5)).unwrap();
        assert!((120..=135).contains(&alpha(&faded, 20)));
        assert!((120..=135).contains(&alpha(&faded, 50)));
        assert_eq!(alpha(&faded, 80), 255);
    }
}

#[cfg(test)]
mod band_tests {
    use super::*;
    use tracedraw_core::document::{Shape, ShapeKind};
    use tracedraw_core::geometry::BezPath;

    /// A thin outline whose Variable Length node reaches far past its
    /// width is drawn whole when the page is drawn in strips: the part
    /// in the strips above and below the path is not left out (#626).
    #[test]
    fn a_variable_outline_is_drawn_whole_across_the_strips() {
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(200.0, 120.0));
        let page = doc.pages[0].id;
        let layer = doc.pages[0].layers[0].id;
        let mut path = BezPath::new();
        path.move_to((20.0, 45.0));
        path.line_to((180.0, 45.0));
        let mut s = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Path {
                path,
                closed: false,
            },
        );
        let mut st = tracedraw_core::Stroke::new(tracedraw_core::Color::BLACK, 0.2);
        // 40 mm across in the middle: 20 mm above and below the path.
        st.widths = vec![tracedraw_core::WidthNode::symmetric(0.5, 40.0)];
        s.stroke = Some(st);
        if let Ok(l) = doc.layer_mut(layer) {
            l.shapes.push(s);
        }
        // 5 px per mm: 1000 x 600 px, enough to be drawn in strips.
        let opts = RenderOptions {
            width: 1000,
            height: 600,
            view: ViewTransform {
                zoom: 5.0,
                origin_x: 0.0,
                origin_y: 600.0,
            },
            ..RenderOptions::default()
        };
        let px = render_page(&doc, page, &opts).expect("render");
        let dark = |x: f64, y: f64| {
            let (col, row) = ((x * 5.0) as u32, (600.0 - y * 5.0) as u32);
            px.pixel(col, row).is_some_and(|p| p.alpha() > 200)
        };
        // At the middle, every row from 19 mm below the path to 19 mm
        // above it is inside the band, whichever strip it fell in.
        for k in 0..=38 {
            let y = 26.0 + k as f64;
            assert!(dark(100.0, y), "{y} mm");
        }
        assert!(!dark(100.0, 66.0) && !dark(100.0, 24.0));
    }

    /// Clip frames and groups moved by their own transform (the pages a
    /// printed sheet repeats, each a clip frame placed on the sheet) are
    /// drawn in every strip they reach: only the strip holding their
    /// untransformed frame drew them, so Print Preview showed one row of
    /// a repeated page and white above it (#848).
    #[test]
    fn moved_clip_frames_and_groups_are_drawn_in_every_strip() {
        let rect = |id: u64, r: Rect| {
            let mut s = Shape::new(
                tracedraw_core::ShapeId(id),
                ShapeKind::Rect {
                    rect: r,
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = tracedraw_core::Fill::Solid(tracedraw_core::Color::BLACK);
            s.stroke = None;
            s
        };
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 200.0));
        let page = doc.pages[0].id;
        let layer = doc.pages[0].layers[0].id;
        let mut shapes = Vec::new();
        // A 40 x 20 mm card repeated four times up the sheet, left.
        for row in 0..4u64 {
            let mut frame = rect(100 + row, Rect::new(0.0, 0.0, 40.0, 20.0));
            frame.fill = tracedraw_core::Fill::None;
            let mut clip = Shape::new(
                tracedraw_core::ShapeId(10 + row),
                ShapeKind::ClipFrame {
                    frame: Box::new(frame),
                    contents: vec![rect(200 + row, Rect::new(0.0, 0.0, 40.0, 20.0))],
                },
            );
            clip.transform = Affine::translate((5.0, 10.0 + 45.0 * row as f64));
            shapes.push(clip);
        }
        // A group moved up the sheet, right.
        let mut group = Shape::new(
            tracedraw_core::ShapeId(50),
            ShapeKind::Group {
                children: vec![rect(51, Rect::new(0.0, 0.0, 40.0, 20.0))],
            },
        );
        group.transform = Affine::translate((55.0, 160.0));
        shapes.push(group);
        if let Ok(l) = doc.layer_mut(layer) {
            l.shapes.extend(shapes);
        }
        // 5 px per mm: 500 x 1000 px, enough to be drawn in strips.
        let opts = RenderOptions {
            width: 500,
            height: 1000,
            view: ViewTransform {
                zoom: 5.0,
                origin_x: 0.0,
                origin_y: 1000.0,
            },
            ..RenderOptions::default()
        };
        let visible = Rect::new(0.0, 100.0, 100.0, 200.0);
        let r_px = &mut Pixmap::new(1, 1).expect("pixmap");
        let r = Renderer {
            pixmap: r_px,
            screen: opts.view.affine(),
            zoom: opts.view.zoom,
            flags: Flags::from_options(&opts),
            symbols: &doc.symbols,
            preview: None,
            origin: (0, 0),
            cancel: None,
        };
        let drawn = &doc.pages[0].layers[0].shapes;
        assert!(r.out_of_sight(&drawn[0], &visible), "the bottom card");
        assert!(!r.out_of_sight(&drawn[3], &visible), "the top card");
        assert!(!r.out_of_sight(&drawn[4], &visible), "the group");
        let px = render_page(&doc, page, &opts).expect("render");
        let dark = |x: f64, y: f64| {
            let (col, row) = ((x * 5.0) as u32, (1000.0 - y * 5.0) as u32);
            px.pixel(col, row).is_some_and(|p| p.alpha() > 200)
        };
        for row in 0..4 {
            let y = 20.0 + 45.0 * row as f64;
            assert!(dark(25.0, y), "card {row} at {y} mm");
            assert!(!dark(25.0, y + 20.0), "gap above card {row}");
        }
        assert!(dark(75.0, 170.0), "the group");
    }

    /// The same inside a group, nested twice, and an ordinary 16 mm
    /// outline inside a group: the strips no longer cut the band flat at
    /// the group's bounds, which count no outline (#626).
    #[test]
    fn outlines_inside_groups_are_drawn_whole_across_the_strips() {
        let line = |id: u64, y: f64, stroke: tracedraw_core::Stroke| {
            let mut path = BezPath::new();
            path.move_to((20.0, y));
            path.line_to((180.0, y));
            let mut s = Shape::new(
                tracedraw_core::ShapeId(id),
                ShapeKind::Path {
                    path,
                    closed: false,
                },
            );
            s.stroke = Some(stroke);
            s
        };
        let mut st = tracedraw_core::Stroke::new(tracedraw_core::Color::BLACK, 0.2);
        st.widths = vec![tracedraw_core::WidthNode::symmetric(0.5, 40.0)];
        // Each crosses into the strip below its group's bounds.
        let variable = line(1, 40.0, st);
        let wide = line(
            2,
            95.0,
            tracedraw_core::Stroke::new(tracedraw_core::Color::BLACK, 16.0),
        );
        let group = |id: u64, children: Vec<Shape>| {
            let mut g = Shape::new(tracedraw_core::ShapeId(id), ShapeKind::Group { children });
            g.stroke = None;
            g
        };
        let nested = group(11, vec![group(10, vec![variable])]);
        let plain = group(12, vec![wide]);
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(200.0, 120.0));
        let page = doc.pages[0].id;
        let layer = doc.pages[0].layers[0].id;
        if let Ok(l) = doc.layer_mut(layer) {
            l.shapes.push(nested);
            l.shapes.push(plain);
        }
        let opts = RenderOptions {
            width: 1000,
            height: 600,
            view: ViewTransform {
                zoom: 5.0,
                origin_x: 0.0,
                origin_y: 600.0,
            },
            ..RenderOptions::default()
        };
        let px = render_page(&doc, page, &opts).expect("render");
        let dark = |x: f64, y: f64| {
            let (col, row) = ((x * 5.0) as u32, (600.0 - y * 5.0) as u32);
            px.pixel(col, row).is_some_and(|p| p.alpha() > 200)
        };
        for k in 0..=38 {
            let y = 21.0 + k as f64;
            assert!(dark(100.0, y), "variable band at {y} mm");
        }
        assert!(!dark(100.0, 61.0) && !dark(100.0, 19.0));
        // The 16 mm outline: 8 mm either side of its path.
        for k in 0..=14 {
            let y = 88.0 + k as f64;
            assert!(dark(100.0, y), "wide outline at {y} mm");
        }
        assert!(!dark(100.0, 104.0) && !dark(100.0, 86.0));
    }

    /// A heavy page: `n` filled and stroked curves spread over A4.
    fn heavy(n: usize) -> (Document, PageId) {
        let mut doc = Document::default();
        let page = doc.pages[0].id;
        let layer = doc.layers_for_page(page).unwrap()[0].id;
        let mut s = 7u64;
        let mut r = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / (1u64 << 31) as f64
        };
        for i in 0..n {
            let (x, y) = (r() * 210.0, r() * 297.0);
            let mut p = BezPath::new();
            p.move_to((x, y));
            for _ in 0..6 {
                p.curve_to(
                    (x + r() * 20.0 - 10.0, y + r() * 20.0 - 10.0),
                    (x + r() * 20.0 - 10.0, y + r() * 20.0 - 10.0),
                    (x + r() * 20.0 - 10.0, y + r() * 20.0 - 10.0),
                );
            }
            p.close_path();
            let mut sh = Shape::new(
                tracedraw_core::ShapeId(1000 + i as u64),
                ShapeKind::Path {
                    path: p,
                    closed: true,
                },
            );
            sh.fill =
                tracedraw_core::Fill::Solid(tracedraw_core::Color::rgb8((i % 255) as u8, 80, 160));
            let _ = tracedraw_core::Command::AddShape { layer, shape: sh }.apply(&mut doc);
        }
        (doc, page)
    }

    /// Inside a clip frame an object with the Multiply merge mode
    /// multiplies what lies beneath the frame, as outside it (a design on
    /// a photo of a shirt keeps the cloth's shading).
    #[test]
    fn merge_modes_inside_a_clip_frame_reach_the_backdrop() {
        use tracedraw_core::live::{Effect, MergeMode};
        let mut doc = Document::default();
        let page = doc.pages[0].id;
        let layer = doc.layers_for_page(page).unwrap()[0].id;
        let rect = |id: u64, r: tracedraw_core::geometry::Rect, c: Color| {
            let mut s = Shape::new(
                tracedraw_core::ShapeId(id),
                ShapeKind::Rect {
                    rect: r,
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = tracedraw_core::Fill::Solid(c);
            s.stroke = None;
            s
        };
        let area = tracedraw_core::geometry::Rect::new(0.0, 0.0, 40.0, 40.0);
        let back = rect(1, area, Color::rgb8(128, 128, 128));
        let mut red = rect(2, area, Color::rgb8(255, 0, 0));
        red.effects.push(Effect::Transparency {
            mask: tracedraw_core::Fill::Solid(Color::Gray { v: 1.0 }),
            merge: MergeMode::Multiply,
            target: 2,
        });
        let mut frame = rect(3, area, Color::WHITE);
        frame.fill = tracedraw_core::Fill::None;
        let clip = Shape::new(
            tracedraw_core::ShapeId(4),
            ShapeKind::ClipFrame {
                frame: Box::new(frame),
                contents: vec![red],
            },
        );
        for s in [back, clip] {
            let _ = tracedraw_core::Command::AddShape { layer, shape: s }.apply(&mut doc);
        }
        let opts = RenderOptions {
            width: 40,
            height: 40,
            view: ViewTransform {
                zoom: 1.0,
                origin_x: 0.0,
                origin_y: 40.0,
            },
            ..RenderOptions::default()
        };
        let px = render_page(&doc, page, &opts).unwrap();
        let c = px.pixel(20, 20).unwrap().demultiply();
        assert!(
            (c.red() as i32 - 128).abs() <= 3 && c.green() <= 3,
            "red times grey: {c:?}"
        );
    }

    /// A picture shape of `w` x `h` pixels from `pm` over `rect`.
    fn picture(id: u64, pm: &Pixmap, rect: Rect) -> Shape {
        let mut s = Shape::new(
            tracedraw_core::ShapeId(id),
            ShapeKind::Bitmap {
                rect,
                width_px: pm.width(),
                height_px: pm.height(),
                png: pm.encode_png().unwrap().into(),
                fx: None,
            },
        );
        s.fill = Fill::None;
        s.stroke = None;
        s
    }

    /// #853: pictures laid side by side meet without a pale seam, at any
    /// zoom (their shared edge falls between pixels here).
    #[test]
    fn pictures_side_by_side_meet_without_a_seam() {
        let mut pm = Pixmap::new(16, 16).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(200, 0, 0, 255));
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let page = doc.pages[0].id;
        for (i, x) in [10.0, 30.3].into_iter().enumerate() {
            doc.pages[0].layers[0].shapes.push(picture(
                i as u64 + 1,
                &pm,
                Rect::new(x, 10.0, x + 20.3, 30.3),
            ));
        }
        for zoom in [1.0, 1.37, 2.9, 0.41] {
            let opts = RenderOptions {
                width: 300,
                height: 300,
                view: ViewTransform {
                    zoom,
                    origin_x: 0.3,
                    origin_y: 299.6,
                },
                anti_alias: true,
                ..RenderOptions::default()
            };
            let out = render_page_whole(&doc, page, &opts, None).unwrap();
            let y = (299.6 - 20.0 * zoom) as u32;
            let x0 = (12.0 * zoom) as u32;
            let x1 = (48.0 * zoom) as u32;
            for x in x0..=x1 {
                let p = out.pixel(x, y).unwrap();
                assert_eq!(
                    (p.red(), p.green(), p.alpha()),
                    (200, 0, 255),
                    "zoom {zoom}, x {x}"
                );
            }
        }
    }

    /// #853: a picture drawn far smaller than its pixels shows their
    /// average, not a few scattered pixels: a one-pixel checkerboard at
    /// about a tenth of its size is an even grey.
    #[test]
    fn a_picture_drawn_small_shows_the_average_of_its_pixels() {
        let mut pm = Pixmap::new(400, 400).unwrap();
        for (i, px) in pm.data_mut().chunks_mut(4).enumerate() {
            let (x, y) = (i % 400, i / 400);
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            px.copy_from_slice(&[v, v, v, 255]);
        }
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let page = doc.pages[0].id;
        doc.pages[0].layers[0]
            .shapes
            .push(picture(1, &pm, Rect::new(0.0, 0.0, 100.0, 100.0)));
        // 400 pixels over 37: about a tenth of their size.
        let opts = RenderOptions {
            width: 37,
            height: 37,
            view: ViewTransform {
                zoom: 0.37,
                origin_x: 0.0,
                origin_y: 37.0,
            },
            anti_alias: true,
            ..RenderOptions::default()
        };
        let out = render_page_whole(&doc, page, &opts, None).unwrap();
        for y in 2..35 {
            for x in 2..35 {
                let r = out.pixel(x, y).unwrap().red();
                assert!((110..=145).contains(&r), "({x}, {y}): {r}");
            }
        }
        assert_eq!(bitmap_level(Affine::scale(0.1)), 3);
        assert_eq!(bitmap_level(Affine::scale(0.5)), 0);
        assert_eq!(bitmap_level(Affine::scale(0.49)), 1);
        assert_eq!(bitmap_level(Affine::scale(2.0)), 0);
    }

    /// One picture cache serves every band: a picture across all bands of
    /// a render is decoded once, and the next render decodes nothing.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn bands_share_one_decode_of_a_picture() {
        let mut pm = Pixmap::new(37, 23).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(12, 200, 99, 255));
        let png = pm.encode_png().unwrap();
        let mut s = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Bitmap {
                rect: Rect::new(0.0, 0.0, 100.0, 100.0),
                width_px: 37,
                height_px: 23,
                png: png.clone().into(),
                fx: None,
            },
        );
        s.fill = Fill::None;
        s.stroke = None;
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(100.0, 100.0));
        let page = doc.pages[0].id;
        doc.pages[0].layers[0].shapes.push(s);
        let opts = RenderOptions {
            width: 800,
            height: 800,
            view: ViewTransform {
                zoom: 8.0,
                origin_x: 0.0,
                origin_y: 800.0,
            },
            ..RenderOptions::default()
        };
        let Some(first) = render_page_in_bands(&doc, page, &opts, None) else {
            // One core: no bands.
            return;
        };
        let green = |y| first.pixel(400, y).map(|p| p.green());
        assert_eq!(green(30), Some(200));
        assert_eq!(green(770), Some(200));
        render_page_in_bands(&doc, page, &opts, None).unwrap();
        assert_eq!(image_cache::decodes_of(&png), 1);
    }

    /// A picture handed over a band at a time (#2389): the bands cover
    /// every row once, the middle row's comes among the first, the
    /// picture handed back is the bands put together, and it is the
    /// picture drawn at once (but for the few edge pixels of curves cut
    /// by a band's edge, as with [`render_page`]'s own bands). A render
    /// told to give up hands over no picture.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_picture_comes_a_band_at_a_time_from_the_middle() {
        let (doc, page) = heavy(60);
        let opts = RenderOptions {
            width: 900,
            height: 700,
            view: ViewTransform {
                zoom: 3.0,
                origin_x: 20.0,
                origin_y: 650.0,
            },
            anti_alias: true,
            ..RenderOptions::default()
        };
        let whole = render_page_whole(&doc, page, &opts, None).expect("drawn");
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let got: std::sync::Mutex<Vec<(u32, Pixmap)>> = std::sync::Mutex::new(Vec::new());
        let band = |y0: u32, rows: &Pixmap| got.lock().unwrap().push((y0, rows.clone()));
        let picture =
            render_page_progressive(&doc, page, &opts, &cancel, 350, &band).expect("drawn");
        let (mut worst, mut off) = (0u8, 0usize);
        for (x, y) in picture.data().iter().zip(whole.data()) {
            let d = x.abs_diff(*y);
            worst = worst.max(d);
            if d > 2 {
                off += 1;
            }
        }
        assert!(worst <= 128, "bands differ by {worst}");
        assert!(off * 2000 < whole.data().len(), "{off} values differ");
        let got = got.into_inner().unwrap();
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        if cores < 2 {
            assert!(got.is_empty(), "one core: drawn whole");
            return;
        }
        assert!(
            got.len() > cores.min(16),
            "more bands than cores: {}",
            got.len()
        );
        let row = 900 * 4;
        let mut covered = vec![0u8; 700];
        for (y0, rows) in &got {
            assert_eq!(rows.width(), 900);
            let (a, b) = (*y0 as usize, *y0 as usize + rows.height() as usize);
            for c in &mut covered[a..b] {
                *c += 1;
            }
            assert!(
                rows.data() == &picture.data()[a * row..b * row],
                "rows {a}..{b}"
            );
        }
        assert!(covered.iter().all(|c| *c == 1), "every row once");
        // The band done first is the one holding the middle row, or one
        // drawn beside it at the same time: the first few handed over
        // include it.
        let first_few = &got[..cores.min(got.len())];
        assert!(
            first_few
                .iter()
                .any(|(y0, r)| (*y0..*y0 + r.height()).contains(&350)),
            "the middle comes among the first"
        );
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(render_page_progressive(&doc, page, &opts, &cancel, 0, &|_, _| ()).is_none());
    }

    /// A page drawn on one thread is the page drawn whole (large enough
    /// here for [`render_page`] to draw it in bands), and an image too
    /// large is refused as [`render_page`] refuses it.
    #[test]
    fn one_thread_draws_the_whole_page() {
        let (doc, page) = heavy(60);
        let opts = RenderOptions {
            width: 900,
            height: 700,
            view: ViewTransform {
                zoom: 3.0,
                origin_x: 20.0,
                origin_y: 650.0,
            },
            anti_alias: true,
            ..RenderOptions::default()
        };
        let one = render_page_on_one_thread(&doc, page, &opts).expect("drawn");
        let whole = render_page_whole(&doc, page, &opts, None).expect("drawn");
        assert_eq!((one.width(), one.height()), (900, 700));
        assert!(one.data() == whole.data());
        assert!(one.data().iter().any(|v| *v != 0), "something is drawn");
        let huge = RenderOptions {
            width: 1 << 20,
            height: 1 << 20,
            ..opts
        };
        assert!(render_page_on_one_thread(&doc, page, &huge).is_none());
    }

    /// A page drawn in bands on several threads matches the page drawn
    /// whole, drop shadows included (their blur reaches across bands).
    #[test]
    fn bands_match_a_whole_render() {
        let (mut doc, page) = heavy(300);
        let layer = doc.layers_for_page(page).unwrap()[0].id;
        for (i, s) in doc.layer_mut(layer).unwrap().shapes.iter_mut().enumerate() {
            if i % 7 == 0 {
                s.shadow = Some(tracedraw_core::document::Shadow {
                    blur: 3.0,
                    ..Default::default()
                });
            }
        }
        let opts = RenderOptions {
            width: 900,
            height: 700,
            view: ViewTransform {
                zoom: 3.0,
                origin_x: 20.0,
                origin_y: 650.0,
            },
            anti_alias: true,
            ..RenderOptions::default()
        };
        let (a, b) = match render_page_in_bands(&doc, page, &opts, None) {
            Some(a) => (a, render_page_whole(&doc, page, &opts, None).unwrap()),
            // One core: nothing to compare.
            None => return,
        };
        // A curve cut by a band's edge is flattened a little differently,
        // which moves the coverage of a few edge pixels slightly; nothing
        // else may differ.
        let (mut worst, mut off) = (0u8, 0usize);
        for (x, y) in a.data().iter().zip(b.data()) {
            let d = x.abs_diff(*y);
            worst = worst.max(d);
            if d > 2 {
                off += 1;
            }
        }
        assert!(worst <= 48, "bands differ by {worst}");
        assert!(off * 2000 < a.data().len(), "{off} values differ");
    }

    /// Render times of a page of 200 photo tiles (6000 x 4000 pixels in
    /// all) at screen zooms from 2 % to 100 %, the second render of each
    /// with the pictures decoded (#853).
    #[test]
    #[ignore]
    fn picture_render_times() {
        let (tw, th) = (300u32, 400u32);
        let mut doc = Document::new("t", tracedraw_core::geometry::Size::new(210.0, 297.0));
        let page = doc.pages[0].id;
        for j in 0..10u32 {
            for i in 0..20u32 {
                let mut pm = Pixmap::new(tw, th).unwrap();
                for (k, px) in pm.data_mut().chunks_mut(4).enumerate() {
                    let v = ((k as u32 * 7 + i * 13 + j * 29) % 251) as u8;
                    px.copy_from_slice(&[v, v / 2, 255 - v, 255]);
                }
                let (w_mm, h_mm) = (33.33, 44.44);
                let x = -150.0 + i as f64 * w_mm;
                let y = 100.0 - (j + 1) as f64 * h_mm;
                doc.pages[0].layers[0].shapes.push(picture(
                    (j * 20 + i + 1) as u64,
                    &pm,
                    Rect::new(x, y, x + w_mm, y + h_mm),
                ));
            }
        }
        for pct in [2.0, 10.0, 21.0, 100.0] {
            // 96 pixels per inch on screen.
            let zoom = pct / 100.0 * 96.0 / 25.4;
            let opts = RenderOptions {
                width: 1360,
                height: 768,
                view: ViewTransform {
                    zoom,
                    origin_x: 680.0 + 100.0 * zoom,
                    origin_y: 384.0 + 100.0 * zoom,
                },
                anti_alias: true,
                draft: true,
                ..RenderOptions::default()
            };
            let t = std::time::Instant::now();
            let _ = render_page(&doc, page, &opts);
            let first = t.elapsed();
            let t = std::time::Instant::now();
            let _ = render_page(&doc, page, &opts);
            eprintln!("{pct} %: first {first:?}, again {:?}", t.elapsed());
        }
    }

    /// Render times of a page of 2000 half transparent objects, as a
    /// transparency slider dragged over them redraws it (#1335):
    /// `cargo test --release -p tracedraw-render -- --ignored render_times_of_transparent_objects --nocapture`.
    #[test]
    #[ignore]
    fn render_times_of_transparent_objects() {
        let (mut doc, page) = heavy(2000);
        for l in doc.pages.iter_mut().flat_map(|p| p.layers.iter_mut()) {
            for s in &mut l.shapes {
                s.opacity = 0.5;
            }
        }
        for (w, h) in [(1600u32, 1000u32), (3200, 2000)] {
            let zoom = h as f64 / 297.0;
            let opts = RenderOptions {
                width: w,
                height: h,
                view: ViewTransform {
                    zoom,
                    origin_x: 0.0,
                    origin_y: h as f64,
                },
                anti_alias: true,
                ..RenderOptions::default()
            };
            let t = std::time::Instant::now();
            let _ = render_page(&doc, page, &opts);
            eprintln!("{w} x {h}: {:?}", t.elapsed());
        }
    }

    #[test]
    #[ignore]
    fn render_times() {
        let one = std::env::var_os("BENCH_ONE").is_some();
        let ns: &[usize] = if one { &[1000] } else { &[1000, 5000, 20000] };
        for &n in ns {
            let (doc, page) = heavy(n);
            let t = std::time::Instant::now();
            let c = doc.clone();
            eprintln!("n={n} clone: {:?}", t.elapsed());
            drop(c);
            let views: &[(u32, u32, f64)] = if one {
                &[(1920, 1080, 3.0)]
            } else {
                &[(1920, 1080, 3.0), (3840, 2160, 6.0), (1920, 1080, 20.0)]
            };
            for &(w, h, zoom) in views {
                let opts = RenderOptions {
                    width: w,
                    height: h,
                    view: ViewTransform {
                        zoom,
                        origin_x: 100.0,
                        origin_y: h as f64 - 50.0,
                    },
                    anti_alias: true,
                    draft: true,
                    ..RenderOptions::default()
                };
                let t = std::time::Instant::now();
                let _ = render_page(&doc, page, &opts);
                eprintln!("n={n} {w}x{h} zoom={zoom}: {:?}", t.elapsed());
            }
        }
    }
}
