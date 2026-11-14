//! Colour and Attributes eyedroppers.
//!
//! Both tools have two phases. In the sample phase a click reads from the
//! object under the pointer; in the apply phase a click writes what was
//! sampled to the object under the pointer. Shift inverts the phase for one
//! click, and Esc clears the sample. The Colour eyedropper reads a solid
//! fill (or the pixel of a bitmap, averaged over the sample size) and
//! applies it to the fill, or to the outline with Shift. The Attributes
//! eyedropper copies the ticked groups: object properties (outline, fill,
//! text), transformations (size, rotation, position) and effects
//! (opacity, drop shadow, live effects).

use crate::app::App;
use tracedraw_core::{
    document::{Shadow, ShapeKind, TextSpan},
    geometry::{Affine, Point},
    id::ShapeId,
    live::Effect,
    Color, Command, Fill, Stroke,
};

/// A decoded picture, shared with the readout's cache.
type Picture = std::sync::Arc<image::RgbaImage>;

/// The colour of the painted pixels around (`fx`, `fy`) of `img`, over a
/// box of `n` x `n` pixels, each as much as it covers; `None` where none
/// is painted.
fn painted_average(img: &image::RgbaImage, fx: f64, fy: f64, n: u32) -> Option<Color> {
    let n = n.max(1) as i64;
    let half = (n - 1) / 2;
    let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
    for dy in -half..(n - half) {
        for dx in -half..(n - half) {
            let x = fx.floor() as i64 + dx;
            let y = fy.floor() as i64 + dy;
            if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64 {
                continue;
            }
            let px = img.get_pixel(x as u32, y as u32);
            let w = px[3] as u64;
            r += px[0] as u64 * w;
            g += px[1] as u64 * w;
            b += px[2] as u64 * w;
            a += w;
        }
    }
    let ch = |v: u64| (v.checked_div(a)).map(|v| v.min(255) as u8);
    Some(Color::rgb8(ch(r)?, ch(g)?, ch(b)?))
}

/// The CMYK inks of the pixels around (`fx`, `fy`) of a `w` x `h` picture
/// in `planes` (its cyan, magenta, yellow and black channels one under
/// the other, 255 full ink), averaged over a box of `n` x `n` pixels.
/// `None` when the planes do not fit the picture.
fn ink_average(
    planes: &image::RgbaImage,
    w: u32,
    h: u32,
    fx: f64,
    fy: f64,
    n: u32,
) -> Option<Color> {
    if planes.width() != w || planes.height() != h.checked_mul(4)? {
        return None;
    }
    let n = i64::from(n.max(1));
    let half = (n - 1) / 2;
    let mut sum = [0u64; 4];
    let mut cnt = 0u64;
    for dy in -half..(n - half) {
        for dx in -half..(n - half) {
            let x = fx.floor() as i64 + dx;
            let y = fy.floor() as i64 + dy;
            if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
                continue;
            }
            for (k, v) in sum.iter_mut().enumerate() {
                *v += u64::from(planes.get_pixel(x as u32, y as u32 + k as u32 * h)[0]);
            }
            cnt += 1;
        }
    }
    let ink = |k: usize| Some(sum[k].checked_div(cnt)? as f32 / 255.0);
    Some(Color::Cmyk {
        c: ink(0)?,
        m: ink(1)?,
        y: ink(2)?,
        k: ink(3)?,
    })
}

/// What the colour eyedroppers show beside the pointer while it hovers
/// (#990): the colour under it, the palette or ink name it has, and its
/// values in the drawing's colour model. Worked out again only when the
/// pointer moves, the drawing changes or Shift is pressed or released;
/// the last picture read stays decoded, so hovering a bitmap does not
/// decode it each frame.
#[derive(Default)]
pub struct HoverReadout {
    /// The pointer (page position in 1/1000 mm), the drawing's revision,
    /// Shift, and whether a picker asked: the input the reading is for.
    key: Option<ReadoutKey>,
    /// The picture decoded last, by its bytes.
    picture: Option<(tracedraw_core::SharedBytes, Picture)>,
    /// What the readout shows, none when nothing is under the pointer.
    pub reading: Option<Reading>,
    /// How many times the colour was worked out, and pictures decoded
    /// (for the tests).
    pub computed: u64,
    pub decoded: u64,
}

impl std::fmt::Debug for HoverReadout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HoverReadout")
            .field("reading", &self.reading)
            .finish()
    }
}

/// The pointer (page position in 1/1000 mm), the zoom (for the hit
/// tolerance), the page, the drawing's revision, Shift, and whether a
/// picker asked.
type ReadoutKey = (i64, i64, i64, tracedraw_core::PageId, u64, bool, bool);

/// The readout's content.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub color: Color,
    /// The palette colour's or spot ink's name; empty for none.
    pub name: String,
    /// The values in the drawing's model, one per line, then the hex.
    pub lines: Vec<String>,
}

/// The readout for `c`: its values in its own model, whatever the
/// drawing's, one component a line: a colour converted to RGB reads as
/// RGB (#1254), one set in HSB, HSL, Lab, YIQ or grayscale in that model
/// (#2612: they all read as CMYK), a spot ink as its tint; then its hex
/// value. `_mode`, the drawing's model, no longer changes the reading.
pub fn reading_of(
    c: Color,
    name: String,
    _mode: tracedraw_core::document::PrimaryColorMode,
) -> Reading {
    let mut lines: Vec<String> = match c {
        Color::Hsb { .. } | Color::Hsl { .. } | Color::Lab { .. } | Color::Yiq { .. } => {
            crate::app::color_description(c)
                .split(' ')
                .map(str::to_string)
                .collect()
        }
        _ => crate::ui::palette::color_lines(c)
            .lines()
            .map(str::to_string)
            .collect(),
    };
    lines.push(c.to_hex().to_uppercase());
    Reading {
        color: c,
        name,
        lines,
    }
}

impl App {
    /// The colour eyedropper's readout, while a colour eyedropper waits
    /// to sample: the Colour Eyedropper in Select mode, or a colour
    /// picker's eyedropper (`picker`). `p` is the pointer on the page.
    /// Cheap when nothing changed since the last frame.
    pub fn hover_reading(&mut self, p: Point, shift: bool, picker: bool) -> Option<&Reading> {
        let q = |v: f64| (v * 1000.0).round() as i64;
        let key = (
            q(p.x),
            q(p.y),
            q(self.view.zoom as f64),
            self.page,
            self.engine.revision(),
            shift && !picker,
            picker,
        );
        if self.eyedropper_hover.key != Some(key) {
            let mut cache = std::mem::take(&mut self.eyedropper_hover.picture);
            let mut decoded = 0;
            let mut decode = |png: &tracedraw_core::SharedBytes| -> Option<Picture> {
                if let Some((b, img)) = &cache {
                    if tracedraw_core::SharedBytes::ptr_eq(b, png) {
                        return Some(img.clone());
                    }
                }
                decoded += 1;
                let img = std::sync::Arc::new(image::load_from_memory(png).ok()?.to_rgba8());
                cache = Some((png.clone(), img.clone()));
                Some(img)
            };
            let c = if picker {
                // A picker's click takes the fill, else the outline, else
                // the paper's white.
                Some(
                    self.sample_color_with(p, false, &mut decode)
                        .or_else(|| self.sample_color_with(p, true, &mut decode))
                        .unwrap_or(Color::WHITE),
                )
            } else {
                self.sample_color_with(p, shift, &mut decode)
            };
            let reading = c.map(|c| {
                let name = self.color_name(c);
                reading_of(c, name, self.doc().metadata.primary_color_mode)
            });
            self.eyedropper_hover.picture = cache;
            self.eyedropper_hover.reading = reading;
            self.eyedropper_hover.key = Some(key);
            self.eyedropper_hover.computed += 1;
            self.eyedropper_hover.decoded += decoded;
        }
        self.eyedropper_hover.reading.as_ref()
    }

    /// The name `c` has: a spot ink's, or a palette colour's.
    pub fn color_name(&self, c: Color) -> String {
        if let Color::Spot { ink, .. } = c {
            if let Some(i) = self.doc().ink(ink) {
                return i.name.clone();
            }
        }
        crate::ui::palette::name_of(self, c)
    }

    /// Does the Colour Eyedropper wait to sample (Select mode)?
    pub fn eyedropper_sampling(&self, resample: bool) -> bool {
        self.tool == crate::tools::Tool::ColorEyedropper
            && (resample || !(self.eyedropper_apply && self.eyedropper_color.is_some()))
    }
}

/// Which groups the Attributes eyedropper copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttrGroups {
    pub outline: bool,
    pub fill: bool,
    pub text: bool,
    /// The transparency (uniform, or a fountain, pattern or texture
    /// mask, with its merge mode), also without the other effects
    /// (#1338).
    pub transparency: bool,
    pub size: bool,
    pub rotation: bool,
    pub position: bool,
    pub effects: bool,
}

impl Default for AttrGroups {
    fn default() -> Self {
        AttrGroups {
            outline: true,
            fill: true,
            text: true,
            transparency: true,
            size: false,
            rotation: false,
            position: false,
            effects: false,
        }
    }
}

/// What the Attributes eyedropper picked up.
#[derive(Clone, Debug, PartialEq)]
pub struct SampledAttrs {
    pub fill: Fill,
    pub stroke: Option<Stroke>,
    pub text: Option<TextSpan>,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub center: Point,
    pub opacity: f64,
    pub shadow: Option<Shadow>,
    pub effects: Vec<Effect>,
    /// The object the fill was read from, which a mesh fill is fitted
    /// from.
    pub fill_source: Option<Box<tracedraw_core::Shape>>,
}

/// Rotation of a shape's transform in radians (angle of its x axis).
pub fn rotation_of(t: Affine) -> f64 {
    let c = t.as_coeffs();
    c[1].atan2(c[0])
}

/// The colour a lens lays over what lies beneath, for the lenses that
/// have one; `None` for the others (Invert, Magnify and the like show
/// what is beneath in their own way).
fn lens_color(l: &tracedraw_core::live::Lens) -> Option<Color> {
    use tracedraw_core::live::Lens;
    match l {
        Lens::Transparency { color, .. }
        | Lens::ColorAdd { color, .. }
        | Lens::ColorLimit { color, .. }
        | Lens::TintedGrayscale { color } => Some(*color),
        Lens::Wireframe { fill, .. } => Some(*fill),
        _ => None,
    }
}

impl App {
    /// The usual hit test, else the topmost unfilled closed object whose
    /// interior contains `p`: what the paint bucket fills, and what a
    /// connector or blend drag starts from and ends on.
    pub fn hit_test_inside(&self, p: Point) -> Option<ShapeId> {
        if let Some(id) = self.hit_test(p) {
            return Some(id);
        }
        for layer in self.page_layers().into_iter().rev() {
            if !layer.visible || layer.locked {
                continue;
            }
            for s in layer.shapes.iter().rev() {
                if s.locked || !s.visible || !s.bounds().contains(p) {
                    continue;
                }
                if crate::tools2::point_in_path(&s.page_path(), p) {
                    return Some(s.id);
                }
            }
        }
        None
    }

    /// The colour under `p`, locked objects included: a bitmap pixel
    /// (averaged over the sample box), else the solid fill (first stop of
    /// a fountain), else the outline.
    /// With `outline` the outline colour is read first.
    /// The object read is the innermost one under `p`: a group's child, a
    /// clip frame's content (#403: a picture in a frame gave nothing).
    pub fn sample_color_at(&self, p: Point, outline: bool) -> Option<Color> {
        self.sample_color_with(p, outline, &mut |png| {
            image::load_from_memory(png)
                .ok()
                .map(|i| std::sync::Arc::new(i.to_rgba8()))
        })
    }

    /// [`App::sample_color_at`] with the pictures decoded by `decode`
    /// (the hover readout keeps the last one decoded).
    fn sample_color_with(
        &self,
        p: Point,
        outline: bool,
        decode: &mut dyn FnMut(&tracedraw_core::SharedBytes) -> Option<Picture>,
    ) -> Option<Color> {
        let s = self.leaf_to_read(p)?;
        let s = &s;
        if outline {
            if let Some(st) = &s.stroke {
                return Some(st.color);
            }
        }
        if let ShapeKind::Bitmap {
            rect,
            width_px,
            height_px,
            png,
            fx: _,
        } = &s.kind
        {
            let lp = s.transform.inverse() * p;
            if rect.contains(lp) && rect.width() > 0.0 && rect.height() > 0.0 {
                // A picture with the CMYK inks it was made or read with
                // gives them, not the screen pixels converted back: a red
                // of C0 M100 Y100 converted to a CMYK bitmap read C2 M99
                // Y96 (#2381).
                let fx = (lp.x - rect.x0) / rect.width() * *width_px as f64;
                let fy = (rect.y1 - lp.y) / rect.height() * *height_px as f64;
                if let Some(c) = s
                    .picture_inks
                    .as_deref()
                    .filter(|i| {
                        i.fits(png.as_slice())
                            && i.inks
                                == (0..4)
                                    .map(tracedraw_core::picture_inks::PictureInk::Process)
                                    .collect::<Vec<_>>()
                    })
                    .and_then(|i| decode(&i.png))
                    .and_then(|planes| {
                        ink_average(
                            &planes,
                            *width_px,
                            *height_px,
                            fx,
                            fy,
                            self.eyedropper_sample,
                        )
                    })
                {
                    return Some(c);
                }
                // Pictures keep the bytes they came with: PNG, JPEG and others.
                if let Some(img) = decode(png) {
                    let n = self.eyedropper_sample.max(1) as i64;
                    let half = (n - 1) / 2;
                    let (mut r, mut g, mut b, mut cnt) = (0u64, 0u64, 0u64, 0u64);
                    for dy in -half..(n - half) {
                        for dx in -half..(n - half) {
                            let x = fx.floor() as i64 + dx;
                            let y = fy.floor() as i64 + dy;
                            if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64
                            {
                                continue;
                            }
                            let px = img.get_pixel(x as u32, y as u32);
                            r += px[0] as u64;
                            g += px[1] as u64;
                            b += px[2] as u64;
                            cnt += 1;
                        }
                    }
                    if let (Some(r), Some(g), Some(b)) =
                        (r.checked_div(cnt), g.checked_div(cnt), b.checked_div(cnt))
                    {
                        // A bitmap in CMYK mode gives its colour in CMYK,
                        // so a picture converted with Convert Colors reads
                        // in the model it went to (#1254).
                        let c = Color::rgb8(r as u8, g as u8, b as u8);
                        return Some(if s.cmyk_bitmap {
                            c.convert_to("CMYK")
                        } else {
                            c
                        });
                    }
                }
            }
        }
        // An object with bitmap effects (Hue/Saturation/Lightness and the
        // others) shows its picture with them: the colour is read there,
        // as drawn, not from the fill they change (#2380).
        if let Some(fx) = s.raster_fx.as_deref().filter(|fx| {
            !fx.png.is_empty()
                && fx.width_px > 0
                && fx.height_px > 0
                && fx.rect.width() > 0.0
                && fx.rect.height() > 0.0
        }) {
            let lp = s.transform.inverse() * p;
            if fx.rect.contains(lp) {
                if let Some(c) = decode(&fx.png).and_then(|img| {
                    let px = (lp.x - fx.rect.x0) / fx.rect.width() * fx.width_px as f64;
                    let py = (fx.rect.y1 - lp.y) / fx.rect.height() * fx.height_px as f64;
                    painted_average(&img, px, py, self.eyedropper_sample)
                }) {
                    return Some(c);
                }
            }
        }
        // A lens draws its own colour over what lies beneath, not its fill
        // (an unfilled closed shape with a Transparency lens shows only
        // there): the colour read is the lens's (#2549).
        if let Some(c) = s.effects.iter().find_map(|e| match e {
            tracedraw_core::live::Effect::Lens(l) => lens_color(l),
            _ => None,
        }) {
            return Some(c);
        }
        match &s.fill {
            Fill::Solid(c) => Some(*c),
            Fill::Fountain(f) => f.stops.first().map(|st| st.color),
            _ => s.stroke.as_ref().map(|st| st.color),
        }
    }

    /// Apply a sampled colour to the object under `p`, to its outline when
    /// `outline` (a hairline is created when the object has none).
    pub fn apply_color_at(&mut self, p: Point, c: Color, outline: bool) -> bool {
        let Some(id) = self.hit_test_inside(p) else {
            return false;
        };
        if outline {
            let Ok((_, s)) = self.doc().shape(id) else {
                return false;
            };
            let mut st = s.stroke.clone().unwrap_or_default();
            st.color = c;
            self.run(Command::SetStroke {
                shapes: vec![id],
                stroke: Some(st),
            });
        } else {
            self.run(Command::SetFill {
                shapes: vec![id],
                fill: Fill::Solid(c),
            });
        }
        true
    }

    /// Read every attribute group from the object under `p` (locked
    /// objects included).
    pub fn sample_attrs_at(&self, p: Point) -> Option<SampledAttrs> {
        let id = self.hit_test_to_read(p)?;
        let (_, s) = self.doc().shape(id).ok()?;
        // The outline, fill and text of the object clicked inside a
        // group or clip frame, which shows them (#1550: a group has none
        // of its own, and applying them took the fill and outline away);
        // the size, place and effects of the object as a whole.
        let shown = self
            .leaf_of(id, p)
            .unwrap_or_else(|| crate::copy_props::properties_source(s));
        let b = s.bounds();
        let text = match &shown.kind {
            ShapeKind::Text { spans, .. } => spans
                .iter()
                .find(|sp| !sp.text.is_empty())
                .or(spans.first())
                .cloned(),
            _ => None,
        };
        Some(SampledAttrs {
            fill: crate::copy_props::shown_fill(&shown),
            stroke: crate::copy_props::shown_stroke(&shown),
            fill_source: Some(Box::new(shown.clone())),
            text,
            width: b.width(),
            height: b.height(),
            rotation: rotation_of(s.transform),
            center: b.center(),
            opacity: s.opacity,
            shadow: s.shadow,
            effects: s.effects.clone(),
        })
    }

    /// Write the ticked groups of the sample to the object under `p`.
    pub fn apply_attrs_at(&mut self, p: Point, a: &SampledAttrs, g: AttrGroups) -> bool {
        let Some(id) = self.hit_test_inside(p) else {
            return false;
        };
        let Ok((_, s)) = self.doc().shape(id) else {
            return false;
        };
        let s = s.clone();
        let mut cmds = Vec::new();
        let shapes = vec![id];
        let targets = [s.clone()];
        if g.fill {
            let src = a.fill_source.as_deref().unwrap_or(&s);
            cmds.extend(crate::copy_props::fill_commands(&a.fill, src, &targets));
        }
        if g.outline {
            cmds.extend(crate::copy_props::clear_character_outlines(&targets));
            cmds.push(Command::SetStroke {
                shapes: shapes.clone(),
                stroke: a.stroke.clone(),
            });
        }
        if g.text {
            if let Some(t) = &a.text {
                // The kind as the commands above left it (the
                // characters' own colours let go).
                let now = crate::copy_props::kind_after(&cmds, &s);
                let mut kind = now.clone();
                if let ShapeKind::Text { spans, .. } = &mut kind {
                    for sp in spans.iter_mut() {
                        *sp = crate::copy_props::with_character_format(sp, t);
                    }
                    if kind != now {
                        cmds.push(Command::SetShapeKind { shape: id, kind });
                    }
                }
            }
        }
        if g.transparency && !g.effects {
            // The sampled transparency in place of the object's own; its
            // other effects stay.
            let mut effects: Vec<Effect> = s
                .effects
                .iter()
                .filter(|e| !matches!(e, Effect::Transparency { .. }))
                .cloned()
                .collect();
            effects.extend(
                a.effects
                    .iter()
                    .filter(|e| matches!(e, Effect::Transparency { .. }))
                    .cloned(),
            );
            if effects != s.effects {
                cmds.push(Command::SetEffects { shape: id, effects });
            }
            cmds.push(Command::SetOpacity {
                shapes: shapes.clone(),
                opacity: a.opacity,
            });
        }
        if g.effects {
            cmds.push(Command::SetOpacity {
                shapes: shapes.clone(),
                opacity: a.opacity,
            });
            cmds.push(Command::SetShadow {
                shapes: shapes.clone(),
                shadow: a.shadow,
            });
            cmds.push(Command::SetEffects {
                shape: id,
                effects: a.effects.clone(),
            });
        }
        // Transformations, about the object's centre: rotate to the sampled
        // angle, scale to the sampled size, then move to the sampled position.
        let b = s.bounds();
        let c = b.center();
        let mut t = Affine::IDENTITY;
        if g.rotation {
            t = Affine::rotate_about(a.rotation - rotation_of(s.transform), c) * t;
        }
        if g.size && b.width() > 1e-9 && b.height() > 1e-9 {
            let sx = a.width / b.width();
            let sy = a.height / b.height();
            t = Affine::translate(c.to_vec2())
                * Affine::scale_non_uniform(sx, sy)
                * Affine::translate(-c.to_vec2())
                * t;
        }
        if g.position {
            t = Affine::translate(a.center - c) * t;
        }
        if t != Affine::IDENTITY {
            cmds.push(Command::TransformShapes {
                shapes: shapes.clone(),
                transform: t,
            });
        }
        if cmds.is_empty() {
            return false;
        }
        if let Err(e) = self.engine.run_batch("Copy Attributes", &cmds) {
            self.status = e.to_string();
            return false;
        }
        true
    }

    /// Clicks of both eyedropper tools. In Select mode a click samples and
    /// switches to Apply mode; in Apply mode a click applies. For the
    /// colour eyedropper Shift reads or writes the outline colour; for the
    /// attributes eyedropper Shift samples again for this click. With
    /// `resample` (Ctrl held, #1302) either tool samples again in Apply
    /// mode; the colour eyedropper's Ctrl+Shift+click reads the outline
    /// first.
    pub fn eyedropper_click(&mut self, p: Point, attributes: bool, shift: bool, resample: bool) {
        if attributes {
            let apply =
                self.eyedropper_apply && self.eyedropper_attrs.is_some() && !shift && !resample;
            if apply {
                if let Some(a) = self.eyedropper_attrs.clone() {
                    let g = self.eyedropper_groups;
                    self.apply_attrs_at(p, &a, g);
                }
                // Back to sampling by itself, where the bar asks for it:
                // no key has to be held (#2380).
                if self.settings.eyedropper_sample_after_apply {
                    self.eyedropper_apply = false;
                }
            } else if let Some(a) = self.sample_attrs_at(p) {
                self.eyedropper_attrs = Some(a);
                self.eyedropper_apply = true;
                self.status = crate::i18n::tr("status.sampled_attrs");
            }
        } else if self.mask.picking {
            self.pick_mask_color_at(p);
        } else {
            let apply = self.eyedropper_apply && self.eyedropper_color.is_some() && !resample;
            if apply {
                if let Some(c) = self.eyedropper_color {
                    self.apply_color_at(p, c, shift);
                }
                if self.settings.eyedropper_sample_after_apply {
                    self.eyedropper_apply = false;
                }
            } else if let Some(c) = self.sample_color_at(p, shift) {
                self.eyedropper_color = Some(c);
                self.eyedropper_apply = true;
                self.status = crate::i18n::trf(
                    "status.sampled_color",
                    &[("c", &crate::app::color_description(c))],
                );
                // With objects selected the colour goes straight onto them,
                // as a palette swatch would: the fill, or the outline with
                // Shift, in one undo step (#2551).
                if !self.selection.is_empty() {
                    if shift {
                        self.apply_outline_color(Some(c));
                    } else {
                        self.apply_fill(Fill::Solid(c));
                    }
                }
            }
        }
    }

    /// Esc or the Select Color button: back to sampling.
    pub fn eyedropper_reset(&mut self) {
        self.eyedropper_color = None;
        self.eyedropper_attrs = None;
        self.eyedropper_apply = false;
        self.mask.picking = false;
    }

    /// The eyedropper of Image > Color Mask: the next click on the
    /// drawing picks the colour for the slot chosen in the docker.
    pub fn start_mask_color_pick(&mut self) {
        self.mask.picking = true;
        self.set_tool(crate::tools::Tool::ColorEyedropper);
    }

    /// A click of the Color Mask's eyedropper (#2479): the colour under
    /// `p` (a picture's pixel) goes into the chosen slot, which is turned
    /// on, and the tool used before comes back, the picture still
    /// selected for Apply, as CorelDRAW's Bitmap Color Mask does. Each
    /// pick samples: the second one applied the first colour to the
    /// picture as a fill instead, the Colour Eyedropper being in its
    /// Apply mode, so only one colour of an imported picture could be
    /// picked. A click on nothing keeps the eyedropper waiting.
    fn pick_mask_color_at(&mut self, p: Point) {
        use crate::bitmap_fx::MASK_SLOTS;
        let Some(c) = self.sample_color_at(p, false) else {
            return;
        };
        let i = self.mask.picked.min(MASK_SLOTS - 1);
        self.mask.slots[i].color = c;
        self.mask.slots[i].on = true;
        self.mask.seen = Some(c);
        self.mask.picking = false;
        self.status = crate::i18n::trf(
            "status.sampled_color",
            &[("c", &crate::app::color_description(c))],
        );
        let back = match self.previous_tool {
            crate::tools::Tool::ColorEyedropper => crate::tools::Tool::Pick,
            t => t,
        };
        self.set_tool(back);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::geometry::Rect;

    fn rect(app: &mut App, r: Rect) -> ShapeId {
        app.new_shape(ShapeKind::Rect {
            rect: r,
            radius: 0.0,
            corners: None,
        })
        .expect("shape")
    }

    /// An object with a Hue/Saturation/Lightness effect gives the colour
    /// it shows (#2380): a red turned green by the hue reads green, and
    /// paints the next object green; it read the red of its fill.
    #[test]
    fn the_colour_of_an_object_with_a_bitmap_effect_is_read_as_shown() {
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 20.0, 20.0));
        let b = rect(&mut app, Rect::new(40.0, 0.0, 50.0, 10.0));
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(Color::rgb8(220, 0, 0)),
        });
        let mut hsl = crate::fx::new_effect("hue_saturation_lightness");
        hsl.params.insert("hue".into(), 120.0);
        app.select(vec![a]);
        app.add_bitmap_effect(hsl);
        assert!(
            app.doc().find_shape(a).expect("a").raster_fx.is_some(),
            "{}",
            app.status
        );
        app.select(Vec::new());
        app.eyedropper_click(Point::new(10.0, 10.0), false, false, false);
        let [r, g, b8] = app.eyedropper_color.expect("sampled").to_rgb8();
        assert!(g > 150 && r < 60 && b8 < 60, "{r} {g} {b8}");
        app.eyedropper_click(Point::new(45.0, 5.0), false, false, false);
        let sb = app.doc().find_shape(b).expect("b");
        assert_eq!(sb.fill, Fill::Solid(Color::rgb8(r, g, b8)));
    }

    #[test]
    fn colour_eyedropper_samples_fill_then_applies_to_fill_or_outline() {
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(&mut app, Rect::new(20.0, 0.0, 30.0, 10.0));
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(Color::rgb8(200, 10, 10)),
        });
        app.select(Vec::new());
        app.eyedropper_click(Point::new(5.0, 5.0), false, false, false);
        assert_eq!(app.eyedropper_color, Some(Color::rgb8(200, 10, 10)));
        // An unfilled object is hit inside too when applying.
        app.eyedropper_click(Point::new(25.0, 5.0), false, false, false);
        let sb = app.doc().find_shape(b).expect("b");
        assert_eq!(sb.fill, Fill::Solid(Color::rgb8(200, 10, 10)));
        // Shift+click in the apply phase colours the outline.
        app.eyedropper_click(Point::new(25.0, 5.0), false, true, false);
        let sb = app.doc().find_shape(b).expect("b");
        assert_eq!(
            sb.stroke.as_ref().map(|s| s.color),
            Some(Color::rgb8(200, 10, 10))
        );
    }

    /// With "Sample again after applying" on, each eyedropper goes back
    /// to sampling after the object it was applied to, so the next click
    /// picks up a colour without a key held; off, it stays in Apply mode
    /// as before (#2380).
    #[test]
    fn the_eyedropper_samples_again_after_applying_when_asked() {
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(&mut app, Rect::new(20.0, 0.0, 30.0, 10.0));
        let target = rect(&mut app, Rect::new(40.0, 0.0, 50.0, 10.0));
        let (red, green) = (Color::rgb8(200, 10, 10), Color::rgb8(10, 200, 10));
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(red),
        });
        app.run(Command::SetFill {
            shapes: vec![b],
            fill: Fill::Solid(green),
        });
        app.select(Vec::new());
        let fill = |app: &App, id| app.doc().find_shape(id).expect("shape").fill.clone();
        // Off: sample, apply, and the next click applies again.
        app.eyedropper_click(Point::new(5.0, 5.0), false, false, false);
        app.eyedropper_click(Point::new(45.0, 5.0), false, false, false);
        assert_eq!(fill(&app, target), Fill::Solid(red));
        assert!(app.eyedropper_apply, "stays in Apply mode");
        // On: after applying, the next click samples.
        app.eyedropper_reset();
        app.settings.eyedropper_sample_after_apply = true;
        app.eyedropper_click(Point::new(25.0, 5.0), false, false, false);
        assert!(app.eyedropper_apply);
        app.eyedropper_click(Point::new(45.0, 5.0), false, false, false);
        assert_eq!(fill(&app, target), Fill::Solid(green));
        assert!(!app.eyedropper_apply, "back to sampling");
        app.eyedropper_click(Point::new(5.0, 5.0), false, false, false);
        assert_eq!(app.eyedropper_color, Some(red));
        assert_eq!(fill(&app, a), Fill::Solid(red), "sampling changes nothing");
        // The attributes eyedropper too.
        app.eyedropper_reset();
        app.eyedropper_click(Point::new(5.0, 5.0), true, false, false);
        assert!(app.eyedropper_apply);
        app.eyedropper_click(Point::new(25.0, 5.0), true, false, false);
        assert_eq!(fill(&app, b), Fill::Solid(red));
        assert!(!app.eyedropper_apply);
    }

    /// Ctrl+click in Apply mode samples again, with either eyedropper,
    /// and changes nothing on the drawing; the colour eyedropper's
    /// Ctrl+Shift+click reads the outline first (#1302).
    #[test]
    fn ctrl_click_samples_again_in_apply_mode() {
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(&mut app, Rect::new(20.0, 0.0, 30.0, 10.0));
        let target = rect(&mut app, Rect::new(40.0, 0.0, 50.0, 10.0));
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(Color::rgb8(200, 10, 10)),
        });
        app.run(Command::SetFill {
            shapes: vec![b],
            fill: Fill::Solid(Color::rgb8(10, 200, 10)),
        });
        app.select(Vec::new());
        app.eyedropper_click(Point::new(5.0, 5.0), false, false, false);
        assert!(app.eyedropper_apply);
        let steps = app.engine.history_labels().0.len();
        app.eyedropper_click(Point::new(25.0, 5.0), false, false, true);
        assert_eq!(app.eyedropper_color, Some(Color::rgb8(10, 200, 10)));
        assert!(app.eyedropper_apply, "still in Apply mode");
        assert_eq!(
            app.engine.history_labels().0.len(),
            steps,
            "nothing changed"
        );
        app.eyedropper_click(Point::new(45.0, 5.0), false, false, false);
        let t = app.doc().find_shape(target).expect("target");
        assert_eq!(t.fill, Fill::Solid(Color::rgb8(10, 200, 10)));
        // Ctrl+Shift reads the outline first.
        app.run(Command::SetStroke {
            shapes: vec![a],
            stroke: Some(Stroke::new(Color::rgb8(10, 10, 200), 0.5)),
        });
        app.eyedropper_click(Point::new(5.0, 5.0), false, true, true);
        assert_eq!(app.eyedropper_color, Some(Color::rgb8(10, 10, 200)));
        // The colour eyedropper shows the readout while Ctrl is held.
        app.tool = crate::tools::Tool::ColorEyedropper;
        assert!(!app.eyedropper_sampling(false));
        assert!(app.eyedropper_sampling(true));
        // The attributes eyedropper: Ctrl samples again as Shift does.
        app.eyedropper_reset();
        app.eyedropper_click(Point::new(5.0, 5.0), true, false, false);
        app.eyedropper_click(Point::new(25.0, 5.0), true, false, true);
        assert_eq!(
            app.eyedropper_attrs.as_ref().map(|a| a.fill.clone()),
            Some(Fill::Solid(Color::rgb8(10, 200, 10)))
        );
        assert!(app.eyedropper_apply);
    }

    /// Image > Color Mask's eyedropper picks a colour of an imported
    /// picture for each slot, one click each, and never paints the
    /// picture: the second pick applied the first colour to it as a fill
    /// (#2479). After a pick the tool used before comes back with the
    /// picture still selected; another tool or Esc ends a pick.
    #[test]
    fn color_mask_eyedropper_picks_each_colour_of_a_picture() {
        let mut app = App::headless();
        let img = image::RgbaImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                image::Rgba([30, 160, 40, 255])
            } else {
                image::Rgba([200, 20, 30, 255])
            }
        });
        app.paste_as_bitmap(img);
        let pic = app.selection[0];
        let before = app.doc().find_shape(pic).expect("picture").clone();
        let b = before.bounds();
        let left = Point::new(b.x0 + b.width() * 0.25, b.center().y);
        let right = Point::new(b.x0 + b.width() * 0.75, b.center().y);
        for (slot, at, want) in [(0, left, [30, 160, 40]), (1, right, [200, 20, 30])] {
            app.mask.picked = slot;
            app.start_mask_color_pick();
            assert_eq!(app.tool, crate::tools::Tool::ColorEyedropper);
            app.eyedropper_click(at, false, false, false);
            assert!(app.mask.slots[slot].on, "slot {slot} on");
            assert_eq!(app.mask.slots[slot].color.to_rgb8(), want, "slot {slot}");
            assert_eq!(app.tool, crate::tools::Tool::Pick, "back to Pick");
            assert_eq!(app.selection, vec![pic], "the picture stays selected");
        }
        let after = app.doc().find_shape(pic).expect("picture");
        assert_eq!(after.fill, before.fill, "the picture was not painted");
        // The Colour Eyedropper chosen later samples and applies as usual.
        app.set_tool(crate::tools::Tool::ColorEyedropper);
        app.eyedropper_click(right, false, false, false);
        assert_eq!(
            app.eyedropper_color.map(|c| c.to_rgb8()),
            Some([200, 20, 30])
        );
        assert!(!app.mask.slots[2].on, "no slot taken");
        // Another tool, or Esc, ends a pick not made.
        app.start_mask_color_pick();
        app.set_tool(crate::tools::Tool::Rectangle);
        assert!(!app.mask.picking);
        app.start_mask_color_pick();
        app.eyedropper_reset();
        assert!(!app.mask.picking);
    }

    /// Both eyedroppers read from a locked object (and from an object on
    /// a locked layer); applying to a locked object stays blocked (#365).
    #[test]
    fn eyedroppers_read_from_locked_objects_but_never_write_to_them() {
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(&mut app, Rect::new(20.0, 0.0, 30.0, 10.0));
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(Color::rgb8(10, 120, 200)),
        });
        app.run(Command::SetLocked {
            shapes: vec![a, b],
            locked: true,
        });
        app.select(Vec::new());
        // The colour eyedropper.
        app.eyedropper_click(Point::new(5.0, 5.0), false, false, false);
        assert_eq!(app.eyedropper_color, Some(Color::rgb8(10, 120, 200)));
        // The document palette's eyedropper reads the same.
        assert_eq!(
            app.sample_color_at(Point::new(5.0, 5.0), false),
            Some(Color::rgb8(10, 120, 200))
        );
        // The attributes eyedropper.
        let attrs = app
            .sample_attrs_at(Point::new(5.0, 5.0))
            .expect("attributes");
        assert_eq!(attrs.fill, Fill::Solid(Color::rgb8(10, 120, 200)));
        // Applying to the locked object does nothing.
        let before = app.doc().find_shape(b).expect("b").fill.clone();
        app.eyedropper_click(Point::new(25.0, 5.0), false, false, false);
        assert_eq!(app.doc().find_shape(b).expect("b").fill, before);
        // An object on a locked layer is read as well.
        app.run(Command::SetLocked {
            shapes: vec![a, b],
            locked: false,
        });
        let layer = app.doc().pages[0].layers[0].id;
        app.run(Command::SetLayerLocked {
            layer,
            locked: true,
        });
        assert_eq!(
            app.sample_color_at(Point::new(5.0, 5.0), false),
            Some(Color::rgb8(10, 120, 200))
        );
        // Selection still ignores locked objects.
        assert_eq!(app.hit_test(Point::new(5.0, 5.0)), None);
    }

    #[test]
    fn attributes_eyedropper_copies_ticked_groups_only() {
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 20.0));
        let b = rect(&mut app, Rect::new(40.0, 0.0, 50.0, 10.0));
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(Color::rgb8(0, 0, 250)),
        });
        app.run(Command::SetOpacity {
            shapes: vec![a],
            opacity: 0.5,
        });
        app.select(Vec::new());
        app.eyedropper_click(Point::new(5.0, 5.0), true, false, false);
        assert!(app.eyedropper_attrs.is_some());
        // Defaults: fill, outline and transparency, no transformations,
        // no effects.
        app.eyedropper_click(Point::new(45.0, 5.0), true, false, false);
        let sb = app.doc().find_shape(b).expect("b");
        assert_eq!(sb.fill, Fill::Solid(Color::rgb8(0, 0, 250)));
        assert!(
            (sb.opacity - 0.5).abs() < 1e-9,
            "the transparency comes too"
        );
        assert!((sb.bounds().width() - 10.0).abs() < 1e-9);
        // Tick size and effects: b takes a's size about its own centre.
        app.eyedropper_groups.size = true;
        app.eyedropper_groups.effects = true;
        app.eyedropper_click(Point::new(45.0, 5.0), true, false, false);
        let sb = app.doc().find_shape(b).expect("b");
        let bb = sb.bounds();
        assert!((bb.height() - 20.0).abs() < 1e-6, "{bb:?}");
        assert!((bb.center().x - 45.0).abs() < 1e-6, "{bb:?}");
        assert!((sb.opacity - 0.5).abs() < 1e-9);
        // Shift+click samples again instead of applying.
        app.eyedropper_click(Point::new(45.0, 5.0), true, true, false);
        let s = app.eyedropper_attrs.as_ref().expect("sample");
        assert!((s.height - 20.0).abs() < 1e-6);
    }

    /// The attributes eyedropper copies transparency by default, a
    /// fountain mask with its merge mode too, in place of the target's
    /// own and keeping its other effects; unticked, the target keeps its
    /// own (#1338: only the colour came across).
    #[test]
    fn attributes_eyedropper_copies_transparency() {
        use tracedraw_core::live::MergeMode;
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(&mut app, Rect::new(40.0, 0.0, 50.0, 10.0));
        let c = rect(&mut app, Rect::new(80.0, 0.0, 90.0, 10.0));
        let mask = Effect::Transparency {
            mask: Fill::Fountain(Default::default()),
            merge: MergeMode::Multiply,
            target: 2,
        };
        app.run(Command::SetEffects {
            shape: a,
            effects: vec![mask.clone()],
        });
        let warp = Effect::Perspective {
            corners: [
                Point::new(40.0, 0.0),
                Point::new(50.0, 0.0),
                Point::new(50.0, 10.0),
                Point::new(40.0, 10.0),
            ],
        };
        app.run(Command::SetEffects {
            shape: b,
            effects: vec![warp.clone()],
        });
        app.select(Vec::new());
        app.eyedropper_click(Point::new(5.0, 5.0), true, false, false);
        app.eyedropper_click(Point::new(45.0, 5.0), true, false, false);
        let sb = app.doc().find_shape(b).expect("b");
        assert_eq!(sb.effects, vec![warp, mask.clone()]);
        // Unticked: c keeps having none.
        app.eyedropper_groups.transparency = false;
        app.eyedropper_click(Point::new(85.0, 5.0), true, false, false);
        let sc = app.doc().find_shape(c).expect("c");
        assert!(sc.effects.is_empty(), "{:?}", sc.effects);
        // A sample without transparency clears the target's.
        app.eyedropper_groups.transparency = true;
        app.eyedropper_click(Point::new(85.0, 5.0), true, true, false);
        app.eyedropper_click(Point::new(5.0, 5.0), true, false, false);
        assert!(app.doc().find_shape(a).expect("a").effects.is_empty());
    }

    /// Clear Transparency takes a fountain transparency and a uniform one
    /// away in one step, and is greyed with nothing to clear (#1338).
    #[test]
    fn clear_transparency_removes_masks_and_opacity() {
        use tracedraw_core::live::MergeMode;
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(&mut app, Rect::new(40.0, 0.0, 50.0, 10.0));
        app.run(Command::SetEffects {
            shape: a,
            effects: vec![Effect::Transparency {
                mask: Fill::Fountain(Default::default()),
                merge: MergeMode::Normal,
                target: 2,
            }],
        });
        app.run(Command::SetOpacity {
            shapes: vec![b],
            opacity: 0.4,
        });
        app.select(vec![a, b]);
        assert!(app.can_clear_transparency());
        let steps = app.engine.history_labels().0.len();
        app.clear_transparency();
        assert!(app.doc().find_shape(a).expect("a").effects.is_empty());
        assert!((app.doc().find_shape(b).expect("b").opacity - 1.0).abs() < 1e-9);
        assert_eq!(app.engine.history_labels().0.len(), steps + 1);
        assert!(!app.can_clear_transparency());
    }

    /// Pictures give their pixels: placed alone, in a group, inside a clip
    /// frame, and kept as JPEG bytes (#403: pictures gave no colour).
    #[test]
    fn pictures_give_their_pixels_wherever_they_are() {
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        // Left half red, right half blue.
        let img = image::RgbaImage::from_fn(20, 20, |x, _| {
            if x < 10 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let id = app.place_image_at(img, None, None).expect("picture");
        let b = app.doc().find_shape(id).unwrap().bounds();
        let left = Point::new(b.x0 + b.width() * 0.25, b.center().y);
        let right = Point::new(b.x0 + b.width() * 0.75, b.center().y);
        let red = Color::rgb8(255, 0, 0);
        let blue = Color::rgb8(0, 0, 255);
        assert_eq!(app.sample_color_at(left, false), Some(red));
        assert_eq!(app.sample_color_at(right, false), Some(blue));
        // In a group with a rectangle beside it.
        let r = rect(&mut app, Rect::new(b.x1 + 10.0, b.y0, b.x1 + 20.0, b.y1));
        app.select(vec![id, r]);
        app.group_selection();
        assert_eq!(app.sample_color_at(left, false), Some(red));
        assert_eq!(app.sample_color_at(right, false), Some(blue));
        app.undo();
        // Inside a clip frame covering it.
        let frame = rect(&mut app, b.inflate(2.0, 2.0));
        app.run(Command::PlaceInside {
            contents: vec![id],
            frame,
        });
        assert!(
            matches!(
                app.doc().find_shape(frame).map(|s| &s.kind),
                Some(ShapeKind::ClipFrame { .. })
            ),
            "placed inside"
        );
        assert_eq!(app.sample_color_at(left, false), Some(red));
        assert_eq!(app.sample_color_at(right, false), Some(blue));
        // The frame out again, and the rectangle that was to be it.
        app.undo();
        app.undo();
        // A picture kept as JPEG bytes (as imported files carry them).
        let mut jpeg = Vec::new();
        let green = image::RgbImage::from_pixel(16, 16, image::Rgb([0, 200, 0]));
        image::DynamicImage::ImageRgb8(green)
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        if let Some(ShapeKind::Bitmap {
            rect,
            width_px,
            height_px,
            fx,
            ..
        }) = app.doc().find_shape(id).map(|s| s.kind.clone())
        {
            app.run(Command::SetShapeKind {
                shape: id,
                kind: ShapeKind::Bitmap {
                    rect,
                    width_px,
                    height_px,
                    png: jpeg.into(),
                    fx,
                },
            });
        }
        let c = app
            .sample_color_at(left, false)
            .expect("a colour")
            .to_rgb8();
        assert!(c[0] < 20 && c[1] > 180 && c[2] < 20, "{c:?}");
        // Converted to CMYK: the picture gives CMYK colours, and the
        // readout shows C, M, Y, K; back to RGB, RGB again (#1254).
        app.select(vec![id]);
        let to = |target| crate::ui::convert_colors::ConvertColorsState {
            to: target,
            whole: false,
            spots: false,
        };
        assert!(app.convert_colors(&to(tracedraw_core::color_convert::ColorTarget::Cmyk)));
        let c = app.sample_color_at(left, false).expect("a colour");
        assert_eq!(c.model_name(), "CMYK", "{c:?}");
        let r = app
            .hover_reading(left, false, false)
            .cloned()
            .expect("read");
        assert!(r.lines[0].starts_with("C:"), "{:?}", r.lines);
        assert!(app.convert_colors(&to(tracedraw_core::color_convert::ColorTarget::Rgb)));
        let c = app.sample_color_at(left, false).expect("a colour");
        assert_eq!(c.model_name(), "RGB", "{c:?}");
        let r = app
            .hover_reading(left, false, false)
            .cloned()
            .expect("read");
        assert!(r.lines[0].starts_with("R:"), "{:?}", r.lines);
    }

    /// The readout names a palette colour and gives its values in the
    /// drawing's model and its hex; it is worked out again only when the
    /// pointer moves or the drawing changes, and a picture hovered is
    /// decoded once (#990).
    #[test]
    fn the_hover_readout_names_the_colour_and_keeps_its_work() {
        use tracedraw_core::document::PrimaryColorMode;
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        let (name, c) = app
            .palette
            .iter()
            .find(|(n, c)| !n.is_empty() && matches!(c, Color::Cmyk { .. }))
            .cloned()
            .expect("a named CMYK colour");
        let id = rect(&mut app, Rect::new(10.0, 10.0, 50.0, 40.0));
        app.select(vec![id]);
        app.apply_fill(Fill::Solid(c));
        let inside = Point::new(30.0, 25.0);
        let r = app
            .hover_reading(inside, false, false)
            .cloned()
            .expect("read");
        assert_eq!(r.color, c);
        assert_eq!(r.name, name);
        assert!(r.lines[0].starts_with("C:"), "{:?}", r.lines);
        assert_eq!(r.lines.len(), 5, "C, M, Y, K, hex: {:?}", r.lines);
        assert_eq!(r.lines[4], c.to_hex().to_uppercase());
        // Every colour reads in its own model whatever the drawing's:
        // CMYK and RGB (#1254), HSB, HSL, Lab, YIQ and grayscale too, one
        // component a line, then the hex (#2612: they read as CMYK).
        let cmyk = reading_of(c, String::new(), PrimaryColorMode::Rgb);
        assert!(cmyk.lines[0].starts_with("C:"), "{:?}", cmyk.lines);
        let red = Color::rgb8(200, 30, 30);
        for mode in [PrimaryColorMode::Rgb, PrimaryColorMode::Cmyk] {
            for (model, first) in [
                ("HSB", ["H:", "S:", "B:"]),
                ("HSL", ["H:", "S:", "L:"]),
                ("Lab", ["L:", "a:", "b:"]),
                ("YIQ", ["Y:", "I:", "Q:"]),
            ] {
                let v = red.convert_to(model);
                let r = reading_of(v, String::new(), mode);
                assert_eq!(r.lines.len(), 4, "{model}: {:?}", r.lines);
                for (line, start) in r.lines.iter().zip(first) {
                    assert!(line.starts_with(start), "{model}: {:?}", r.lines);
                }
                assert_eq!(r.lines[3], v.to_hex().to_uppercase());
            }
            let gray = reading_of(Color::Gray { v: 0.5 }, String::new(), mode);
            assert_eq!(gray.lines.len(), 2, "{:?}", gray.lines);
            assert!(!gray.lines[0].starts_with("C:"), "{:?}", gray.lines);
            assert!(gray.lines[0].contains("128"), "{:?}", gray.lines);
        }
        // A colour of no palette has no name.
        let odd = reading_of(Color::rgb8(1, 2, 3), String::new(), PrimaryColorMode::Cmyk);
        assert!(odd.lines[0].starts_with("R:"), "{:?}", odd.lines);
        assert_eq!(odd.lines.last().map(String::as_str), Some("#010203"));
        // Nothing worked out again while nothing changes.
        let n = app.eyedropper_hover.computed;
        for _ in 0..5 {
            app.hover_reading(inside, false, false);
        }
        assert_eq!(app.eyedropper_hover.computed, n);
        app.hover_reading(Point::new(31.0, 25.0), false, false);
        assert_eq!(app.eyedropper_hover.computed, n + 1, "the pointer moved");
        app.apply_fill(Fill::Solid(Color::rgb8(0, 255, 0)));
        let r = app.hover_reading(Point::new(31.0, 25.0), false, false);
        assert_eq!(r.map(|r| r.color), Some(Color::rgb8(0, 255, 0)), "edited");
        // Off every object: nothing to sample, but a picker gets white.
        assert!(app
            .hover_reading(Point::new(150.0, 200.0), false, false)
            .is_none());
        let r = app.hover_reading(Point::new(150.0, 200.0), false, true);
        assert_eq!(r.map(|r| r.color), Some(Color::WHITE));
        // A picture: decoded once however the pointer moves over it.
        let img = image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 255, 255]));
        let pic = app.place_image_at(img, None, None).expect("picture");
        let b = app.doc().find_shape(pic).unwrap().bounds();
        let before = app.eyedropper_hover.decoded;
        for i in 0..10 {
            let p = Point::new(b.x0 + b.width() * (0.1 + i as f64 * 0.08), b.center().y);
            let r = app.hover_reading(p, false, false);
            assert_eq!(r.map(|r| r.color), Some(Color::rgb8(0, 0, 255)));
        }
        assert_eq!(app.eyedropper_hover.decoded, before + 1);
    }

    #[test]
    fn rotation_of_reads_the_transform_angle() {
        let t = Affine::rotate(0.3);
        assert!((rotation_of(t) - 0.3).abs() < 1e-9);
    }

    /// A closed shape with a Transparency lens over other objects gives
    /// the lens's colour, filled or not, and is picked inside (#2549): an
    /// unfilled one gave the colour of the object beneath, a filled one
    /// its fill, which the lens does not draw.
    #[test]
    fn a_shape_with_a_transparency_lens_over_others_gives_its_colour() {
        use tracedraw_core::live::{Effect, Lens};
        let mut app = App::headless();
        let under = rect(&mut app, Rect::new(0.0, 0.0, 100.0, 100.0));
        app.run(Command::SetFill {
            shapes: vec![under],
            fill: Fill::Solid(Color::rgb8(200, 0, 0)),
        });
        let blue = Color::rgb8(0, 0, 200);
        let lens = Effect::Lens(Lens::Transparency {
            rate: 50.0,
            color: blue,
        });
        let unfilled = app
            .new_shape(ShapeKind::Ellipse {
                rect: Rect::new(10.0, 10.0, 40.0, 40.0),
                arc: None,
            })
            .expect("ellipse");
        app.run(Command::SetFill {
            shapes: vec![unfilled],
            fill: Fill::None,
        });
        app.run(Command::SetEffects {
            shape: unfilled,
            effects: vec![lens.clone()],
        });
        let filled = rect(&mut app, Rect::new(60.0, 10.0, 90.0, 40.0));
        app.run(Command::SetFill {
            shapes: vec![filled],
            fill: Fill::Solid(Color::rgb8(0, 200, 0)),
        });
        app.run(Command::SetEffects {
            shape: filled,
            effects: vec![lens],
        });
        app.select(Vec::new());
        for (at, id) in [
            (Point::new(25.0, 25.0), unfilled),
            (Point::new(75.0, 25.0), filled),
        ] {
            assert_eq!(app.hit_test_to_read(at), Some(id));
            assert_eq!(app.sample_color_at(at, false), Some(blue));
        }
        // Outside the lens the object beneath is read as before.
        assert_eq!(
            app.sample_color_at(Point::new(50.0, 70.0), false),
            Some(Color::rgb8(200, 0, 0))
        );
    }

    /// With objects selected, the colour sampled goes straight onto them
    /// (#2551): the fill, or the outline with Shift, in one undo step.
    #[test]
    fn a_sampled_colour_is_applied_to_the_selection() {
        let mut app = App::headless();
        let a = rect(&mut app, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(&mut app, Rect::new(20.0, 0.0, 30.0, 10.0));
        let c = rect(&mut app, Rect::new(40.0, 0.0, 50.0, 10.0));
        let red = Color::rgb8(200, 10, 10);
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(red),
        });
        app.run(Command::SetFill {
            shapes: vec![b, c],
            fill: Fill::Solid(Color::rgb8(0, 0, 200)),
        });
        app.select(vec![b, c]);
        app.eyedropper_click(Point::new(5.0, 5.0), false, false, false);
        assert_eq!(app.eyedropper_color, Some(red));
        for id in [b, c] {
            assert_eq!(
                app.doc().find_shape(id).expect("shape").fill,
                Fill::Solid(red)
            );
        }
        assert_eq!(app.doc().find_shape(a).expect("a").fill, Fill::Solid(red));
        // One undo puts both back.
        app.engine.undo().expect("undo");
        for id in [b, c] {
            assert_eq!(
                app.doc().find_shape(id).expect("shape").fill,
                Fill::Solid(Color::rgb8(0, 0, 200))
            );
        }
        // Shift samples the outline colour and puts it on the outlines.
        let green = Color::rgb8(0, 150, 0);
        app.run(Command::SetStroke {
            shapes: vec![a],
            stroke: Some(tracedraw_core::Stroke {
                color: green,
                ..Default::default()
            }),
        });
        app.select(vec![b]);
        app.eyedropper_reset();
        app.eyedropper_click(Point::new(5.0, 5.0), false, true, false);
        let sb = app.doc().find_shape(b).expect("b");
        assert_eq!(sb.stroke.as_ref().map(|s| s.color), Some(green));
        assert_eq!(sb.fill, Fill::Solid(Color::rgb8(0, 0, 200)));
    }
}
