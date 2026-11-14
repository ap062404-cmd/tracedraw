//! Fill and outline (stroke) properties of a shape.

use crate::color::Color;
use crate::geometry::{Affine, BezPath, Point, Rect, Size, Vec2};
use serde::{Deserialize, Serialize};

/// Fountain (gradient) fill geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FountainKind {
    #[default]
    Linear,
    Radial,
    Conical,
    Square,
    /// Follows the object's outline: 0 on its edge, 1 at the point
    /// deepest inside it.
    Shaped,
}

impl FountainKind {
    /// Every kind, in the order the fill editor lists them.
    pub const ALL: [FountainKind; 5] = [
        FountainKind::Linear,
        FountainKind::Radial,
        FountainKind::Conical,
        FountainKind::Square,
        FountainKind::Shaped,
    ];
}

/// How a fountain runs its colours over its span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FountainRepeat {
    /// Once; the end colours hold beyond the span.
    #[default]
    None,
    /// Sawtooth: each cycle starts again at the first colour.
    Repeat,
    /// Triangular: cycles run there and back.
    Mirror,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    /// 0.0 at the start, 1.0 at the end.
    pub pos: f64,
    pub color: Color,
    /// Where the blend to the next stop is half way, as a fraction of the
    /// way there (0.5: in the middle; the Interactive Fill tool's
    /// midpoint slider). Unused on the last stop.
    #[serde(default = "half", skip_serializing_if = "is_half")]
    pub mid: f64,
    /// 1 opaque, 0 clear (the node's transparency).
    #[serde(default = "one_f32", skip_serializing_if = "is_one_f32")]
    pub opacity: f32,
}

impl Stop {
    /// An opaque stop with its blend half way to the next.
    pub fn new(pos: f64, color: Color) -> Self {
        Stop {
            pos,
            color,
            mid: 0.5,
            opacity: 1.0,
        }
    }
}

fn half() -> f64 {
    0.5
}

fn is_half(v: &f64) -> bool {
    (*v - 0.5).abs() < 1e-12
}

fn one_f32() -> f32 {
    1.0
}

fn is_one_f32(v: &f32) -> bool {
    *v == 1.0
}

/// The blend from one stop to the next at `k` (0..1 of the way) with its
/// midpoint at `mid`: `k^g` with `mid^g = 1/2`.
pub fn midpoint_curve(k: f64, mid: f64) -> f64 {
    let m = mid.clamp(0.01, 0.99);
    if (m - 0.5).abs() < 1e-9 {
        return k;
    }
    k.clamp(0.0, 1.0).powf(0.5f64.ln() / m.ln())
}

/// The smooth transition's ease over a blend: `3w^2 - 2w^3`, flat at
/// both ends, half way at the middle.
pub fn smoothstep(w: f64) -> f64 {
    let w = w.clamp(0.0, 1.0);
    w * w * (3.0 - 2.0 * w)
}

/// How colours go from one to the next: straight through the colour
/// space, or around the colour wheel one way or the other. The Blend
/// effect's colour blend and a fountain fill's blend direction (#1208).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorBlend {
    /// Straight through the colour space.
    #[default]
    Direct,
    /// Around the colour wheel, hue increasing.
    Clockwise,
    /// Around the colour wheel, hue decreasing.
    CounterClockwise,
}

impl ColorBlend {
    /// Every direction, in the order an editor lists them.
    pub const ALL: [ColorBlend; 3] = [
        ColorBlend::Direct,
        ColorBlend::Clockwise,
        ColorBlend::CounterClockwise,
    ];
}

/// A colour between `a` and `b` at `t`, straight or around the wheel.
pub fn blend_color(a: Color, b: Color, t: f32, how: ColorBlend) -> Color {
    let turn = match how {
        ColorBlend::Direct => return lerp_color(a, b, t),
        ColorBlend::Clockwise => 1.0,
        ColorBlend::CounterClockwise => -1.0,
    };
    let (ha, sa, va) = a.to_hsb();
    let (hb, sb, vb) = b.to_hsb();
    // The hue's way round in the chosen direction, in degrees.
    let mut d = (hb - ha).rem_euclid(360.0);
    if turn < 0.0 {
        d -= 360.0;
        if d <= -360.0 {
            d += 360.0;
        }
    }
    let t = t as f64;
    let h = (ha + d * t).rem_euclid(360.0);
    Color::from_hsb(h, sa + (sb - sa) * t, va + (vb - va) * t)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fountain {
    pub kind: FountainKind,
    /// At least two stops, sorted by position.
    pub stops: Vec<Stop>,
    /// Degrees (0 = left to right, counter-clockwise positive).
    pub angle: f64,
    /// Centre offset in bounds-relative units (-1..1), for radial/conical/square.
    pub offset: Point,
    /// Edge padding in 0..0.49: fraction of the span held at the end colours.
    pub edge_pad: f64,
    /// Repeat mode over the span.
    #[serde(default, skip_serializing_if = "is_default")]
    pub repeat: FountainRepeat,
    /// Cycles across the span when `repeat` is not None (1 to
    /// [`Fountain::MAX_REPEATS`]).
    #[serde(default = "one_u32", skip_serializing_if = "is_one_u32")]
    pub repeats: u32,
    /// Mirrored about the centre: the colours run first to last and back.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bilinear: bool,
    /// Colours in reverse order.
    #[serde(default, skip_serializing_if = "is_default")]
    pub invert: bool,
    /// How the colours go from each stop to the next (#1208): straight
    /// through RGB, or around the colour wheel one way or the other.
    #[serde(default, skip_serializing_if = "is_default")]
    pub blend_direction: ColorBlend,
    /// Colour bands the fill is drawn with, [`Self::MIN_STEPS`] to
    /// [`Self::SMOOTH_STEPS`]; the top value means smooth (no bands).
    #[serde(default = "smooth_steps", skip_serializing_if = "is_smooth_steps")]
    pub steps: u32,
    /// Smooth transition: the colour change over each blend eases in and
    /// out ([`smoothstep`]) instead of running straight.
    #[serde(default, skip_serializing_if = "is_default")]
    pub smooth: bool,
    /// Acceleration, -100 to 100: skews the midpoint of every blend, on
    /// top of each stop's own `mid`. Positive pulls the midpoints toward
    /// the start (the colours change fast at first), negative toward the
    /// end; 0 leaves them.
    #[serde(default, skip_serializing_if = "is_default")]
    pub acceleration: i32,
    /// Where the Interactive Fill tool put the fill's nodes (#330): the
    /// start, the end and the skew handle, free of the object's box. When
    /// set they place the fill instead of `angle` and `offset` (which
    /// keep the nearest equivalent for formats that know only those).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<FountainHandles>,
}

/// The nodes of a fountain fill placed freely (see [`Fountain::handles`]),
/// relative to the object's bounding box `b`: a point `p` is stored as
/// `((p.x - c.x) / (w / 2), (p.y - c.y) / (h / 2))` with `c` the box's
/// centre, so they follow the object when it is moved or scaled.
///
/// The fill's own frame maps `(u, v)` to `start + u (end - start) +
/// v (skew - start)`. Linear: `t = u` (0 at the start, 1 at the end; the
/// bands run along `skew - start`). Radial (elliptical): `t = |(u, v)|`
/// (the end and the skew handle on the ellipse's two half axes). Conical:
/// the angle of `(u, v)` from the end, mirrored as the conical fill is.
/// Square: `t = max(|u|, |v|)` (the end and the skew handle on the
/// parallelogram's half axes).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FountainHandles {
    pub start: Point,
    pub end: Point,
    pub skew: Point,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

fn one_u32() -> u32 {
    1
}

fn is_one_u32(v: &u32) -> bool {
    *v == 1
}

fn smooth_steps() -> u32 {
    Fountain::SMOOTH_STEPS
}

fn is_smooth_steps(v: &u32) -> bool {
    *v >= Fountain::SMOOTH_STEPS
}

/// `t` only moves and scales along the page axes, without a mirror: a box
/// stays a box with its sides in the same order.
fn keeps_box_axes(t: Affine) -> bool {
    let [a, b, c, d, _, _] = t.as_coeffs();
    let scale = a.abs().max(d.abs()).max(1e-12);
    b.abs() <= 1e-12 * scale && c.abs() <= 1e-12 * scale && a > 0.0 && d > 0.0
}

impl Default for Fountain {
    /// Black to white, left to right, running once.
    fn default() -> Self {
        Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0)
    }
}

impl Fountain {
    /// Most cycles a repeating fountain runs across its span.
    pub const MAX_REPEATS: u32 = 50;

    /// Fewest colour bands a stepped fountain is drawn with.
    pub const MIN_STEPS: u32 = 2;

    /// The step count that means smooth: no bands at all.
    pub const SMOOTH_STEPS: u32 = 256;

    /// Furthest the acceleration goes either way.
    pub const MAX_ACCELERATION: i32 = 100;

    pub fn two(kind: FountainKind, from: Color, to: Color, angle: f64) -> Self {
        Fountain {
            kind,
            stops: vec![Stop::new(0.0, from), Stop::new(1.0, to)],
            angle,
            offset: Point::ZERO,
            edge_pad: 0.0,
            repeat: FountainRepeat::None,
            repeats: 1,
            bilinear: false,
            invert: false,
            blend_direction: ColorBlend::Direct,
            steps: Self::SMOOTH_STEPS,
            smooth: false,
            acceleration: 0,
            handles: None,
        }
    }

    /// The colour bands the fill is drawn with (the step count within its
    /// range; [`Self::SMOOTH_STEPS`] when smooth).
    pub fn band_count(&self) -> u32 {
        self.steps.clamp(Self::MIN_STEPS, Self::SMOOTH_STEPS)
    }

    /// Drawn as colour bands with hard edges (fewer steps than smooth).
    pub fn stepped(&self) -> bool {
        self.band_count() < Self::SMOOTH_STEPS
    }

    /// Any of the blend settings is off its default: a hue direction,
    /// steps, the smooth transition or an acceleration.
    pub fn has_blend_options(&self) -> bool {
        self.blend_direction != ColorBlend::Direct
            || self.stepped()
            || self.smooth
            || self.acceleration != 0
    }

    /// A stop's midpoint `mid` with the acceleration `a` (-1..1) worked
    /// in: `a > 0` pulls it toward the start, `mid (1 - a)`; `a < 0`
    /// pushes it toward the end, `mid + (1 - mid) (-a)`.
    pub fn skewed_mid(mid: f64, a: f64) -> f64 {
        let a = a.clamp(-1.0, 1.0);
        let m = mid.clamp(0.0, 1.0);
        if a >= 0.0 {
            m * (1.0 - a)
        } else {
            m + (1.0 - m) * (-a)
        }
    }

    /// The acceleration as a fraction, -1..1.
    fn acceleration_fraction(&self) -> f64 {
        self.acceleration
            .clamp(-Self::MAX_ACCELERATION, Self::MAX_ACCELERATION) as f64
            / Self::MAX_ACCELERATION as f64
    }

    /// The midpoint a blend starting at `stop` runs with: its own `mid`
    /// skewed by the acceleration.
    pub fn blend_mid(&self, stop: &Stop) -> f64 {
        Self::skewed_mid(stop.mid, self.acceleration_fraction())
    }

    /// How far the colour has gone at `k` (0..1) of the way through a
    /// blend with the midpoint `mid`: the midpoint curve, eased by the
    /// smooth transition when on.
    pub fn blend_weight(&self, k: f64, mid: f64) -> f64 {
        let w = midpoint_curve(k, mid);
        if self.smooth {
            smoothstep(w)
        } else {
            w
        }
    }

    /// The stop parameter `u` (0..1) as the one its colour band is
    /// sampled at: with `n` bands the band `i = floor(u n)` (the last one
    /// at the very end) takes the colour at `i / (n - 1)`, so the first
    /// and last bands are the end colours. Smooth: `u` itself.
    pub fn band_param(&self, u: f64) -> f64 {
        let n = self.band_count();
        if n >= Self::SMOOTH_STEPS {
            return u;
        }
        let nf = n as f64;
        let u = if u.is_finite() {
            u.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let band = (u * nf).floor().min(nf - 1.0);
        band / (nf - 1.0)
    }

    /// The colour and opacity at `t` (0..1 across the span) from this
    /// fountain's [`Self::blended_stops`], computed once by the caller:
    /// the stop parameter, its colour band when stepped, then the sample.
    /// The one place every drawing of the fill takes its colours from.
    pub fn sample_blended(&self, blended: &[Stop], t: f64) -> Stop {
        sample_stop(blended, self.band_param(self.stop_param(t)))
    }

    /// A box-relative point (see [`FountainHandles`]) on the page, for the
    /// object's bounding box `b`.
    pub fn rel_to_page(b: Rect, r: Point) -> Point {
        let c = b.center();
        Point::new(c.x + r.x * b.width() / 2.0, c.y + r.y * b.height() / 2.0)
    }

    /// A page point as a box-relative one for `b` (0 on a flat side).
    pub fn page_to_rel(b: Rect, p: Point) -> Point {
        let c = b.center();
        let rx = if b.width() > 1e-9 {
            (p.x - c.x) / (b.width() / 2.0)
        } else {
            0.0
        };
        let ry = if b.height() > 1e-9 {
            (p.y - c.y) / (b.height() / 2.0)
        } else {
            0.0
        };
        Point::new(rx, ry)
    }

    /// The fill's frame on the page for the object's box `b`, mapping
    /// `(u, v)` to `start + u (end - start) + v (skew - start)`: from the
    /// nodes when they are placed freely, else `None` (the angle and the
    /// offset place the fill). A frame that is flat counts as none.
    pub fn frame(&self, b: Rect) -> Option<Affine> {
        if self.kind == FountainKind::Shaped {
            return None;
        }
        let h = self.handles?;
        let s = Self::rel_to_page(b, h.start);
        let e = Self::rel_to_page(b, h.end) - s;
        let k = Self::rel_to_page(b, h.skew) - s;
        let det = e.x * k.y - e.y * k.x;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        Some(Affine::new([e.x, e.y, k.x, k.y, s.x, s.y]))
    }

    /// The fill's parameter `t` (before [`Self::stop_param`]) at the page
    /// point `p`, for a fill whose nodes are placed freely (`None`
    /// otherwise; see [`FountainHandles`]).
    pub fn param_at(&self, b: Rect, p: Point) -> Option<f64> {
        let q = self.frame(b)?.inverse() * p;
        let (u, v) = (q.x, q.y);
        Some(match self.kind {
            FountainKind::Linear => u,
            FountainKind::Radial => u.hypot(v),
            FountainKind::Conical => {
                let a = v.atan2(u).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
                if a < 0.5 {
                    a * 2.0
                } else {
                    2.0 - a * 2.0
                }
            }
            FountainKind::Square => u.abs().max(v.abs()),
            FountainKind::Shaped => return None,
        })
    }

    /// The nodes that place the fill as its angle and offset do now on
    /// the box `b` (where the Interactive Fill tool shows them before they
    /// are first dragged).
    pub fn default_handles(&self, b: Rect) -> FountainHandles {
        let a = self.angle.to_radians();
        let (ca, sa) = (a.cos(), a.sin());
        let c = b.center();
        let rel = |p: Point| Self::page_to_rel(b, p);
        match self.kind {
            FountainKind::Linear | FountainKind::Shaped => {
                let half = (b.width() * ca.abs() + b.height() * sa.abs()) / 2.0;
                let d = Vec2::new(ca, sa) * half;
                let s = c - d;
                let n = Vec2::new(-sa, ca) * (2.0 * half);
                FountainHandles {
                    start: rel(s),
                    end: rel(c + d),
                    skew: rel(s + n),
                }
            }
            FountainKind::Radial => {
                let cc = Point::new(
                    c.x + self.offset.x * b.width() / 2.0,
                    c.y + self.offset.y * b.height() / 2.0,
                );
                let r = b.width().max(b.height()) / 2.0 * std::f64::consts::SQRT_2;
                FountainHandles {
                    start: rel(cc),
                    end: rel(cc + Vec2::new(ca, sa) * r),
                    skew: rel(cc + Vec2::new(-sa, ca) * r),
                }
            }
            // Conical and square fills are measured in the box's own
            // units (half the width and half the height), around the
            // offset; the conical one turned by the angle.
            FountainKind::Conical => {
                let o = self.offset;
                FountainHandles {
                    start: o,
                    end: Point::new(o.x + ca, o.y + sa),
                    skew: Point::new(o.x - sa, o.y + ca),
                }
            }
            FountainKind::Square => {
                let o = self.offset;
                FountainHandles {
                    start: o,
                    end: Point::new(o.x + 1.0, o.y),
                    skew: Point::new(o.x, o.y + 1.0),
                }
            }
        }
    }

    /// Place the fill's nodes (box `b`) and keep the angle and the offset
    /// the nearest the formats that know only those can show: the angle
    /// of the axis, and for the kinds with a centre the start as the
    /// offset.
    pub fn set_handles(&mut self, b: Rect, h: FountainHandles) {
        let s = Self::rel_to_page(b, h.start);
        let e = Self::rel_to_page(b, h.end);
        let d = e - s;
        if d.hypot() > 1e-9 && d.x.is_finite() && d.y.is_finite() {
            self.angle = d.y.atan2(d.x).to_degrees();
        }
        if self.kind != FountainKind::Linear {
            self.offset = Point::new(h.start.x.clamp(-1.0, 1.0), h.start.y.clamp(-1.0, 1.0));
        }
        self.handles = Some(h);
    }

    /// Carry the fill along with `t`, a page-space transformation of its
    /// object whose page box goes from `before` to `after` (#775): a turn,
    /// a mirror or a skew turns, mirrors or skews the fill with the object,
    /// its nodes then placed freely unless its angle and centre place it
    /// the same (a quarter or half turn, a mirror). A move or a scale without a mirror
    /// changes nothing, the fill being placed relative to the box anyway;
    /// a shaped fill follows the outline by itself.
    pub fn carry(&mut self, t: Affine, before: Rect, after: Rect) {
        if self.kind == FountainKind::Shaped || keeps_box_axes(t) {
            return;
        }
        let flat = |b: Rect| !(b.width() > 1e-9 && b.height() > 1e-9);
        if flat(before) || flat(after) {
            return;
        }
        let h = self.handles.unwrap_or_else(|| self.default_handles(before));
        let moved = |p: Point| Self::page_to_rel(after, t * Self::rel_to_page(before, p));
        let h = FountainHandles {
            start: moved(h.start),
            end: moved(h.end),
            skew: moved(h.skew),
        };
        if ![h.start, h.end, h.skew]
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite())
        {
            return;
        }
        // A quarter or half turn, or a mirror, often gives a fill its angle
        // and centre can place exactly as well: it stays one placed that
        // way, so the angle field still turns it.
        let mut placed = self.clone();
        placed.set_handles(after, h);
        placed.handles = None;
        let d = placed.default_handles(after);
        let near = |a: Point, b: Point| (a - b).hypot() <= 1e-9;
        if self.handles.is_none()
            && near(d.start, h.start)
            && near(d.end, h.end)
            && near(d.skew, h.skew)
        {
            *self = placed;
        } else {
            self.set_handles(after, h);
        }
    }

    /// Keep the fill where it lies on the page while its object's outline
    /// changes (nodes moved, an area cropped off) and its page box goes
    /// from `before` to `after`: the fill is cut, not squeezed into the
    /// new box (#1491). Its nodes are then placed freely; a shaped fill
    /// follows the outline by itself.
    pub fn keep_in_place(&mut self, before: Rect, after: Rect) {
        if self.kind == FountainKind::Shaped {
            return;
        }
        let flat = |b: Rect| {
            !(b.width() > 1e-9
                && b.height() > 1e-9
                && b.x0.is_finite()
                && b.y0.is_finite()
                && b.x1.is_finite()
                && b.y1.is_finite())
        };
        if flat(before) || flat(after) {
            return;
        }
        let near = |a: f64, b: f64| (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()));
        if near(before.x0, after.x0)
            && near(before.y0, after.y0)
            && near(before.x1, after.x1)
            && near(before.y1, after.y1)
        {
            return;
        }
        let h = self.handles.unwrap_or_else(|| self.default_handles(before));
        let moved = |p: Point| Self::page_to_rel(after, Self::rel_to_page(before, p));
        let h = FountainHandles {
            start: moved(h.start),
            end: moved(h.end),
            skew: moved(h.skew),
        };
        if [h.start, h.end, h.skew]
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite())
        {
            self.set_handles(after, h);
        }
    }

    pub fn first_color(&self) -> Color {
        self.stops.first().map(|s| s.color).unwrap_or(Color::BLACK)
    }

    pub fn last_color(&self) -> Color {
        self.stops.last().map(|s| s.color).unwrap_or(Color::WHITE)
    }

    /// Cycles across the span (1 unless the fountain repeats).
    pub fn cycles(&self) -> u32 {
        match self.repeat {
            FountainRepeat::None => 1,
            _ => self.repeats.clamp(1, Self::MAX_REPEATS),
        }
    }

    /// No repeat, bilinear or invert: the stops run once over the span.
    pub fn runs_once(&self) -> bool {
        self.cycles() == 1 && !self.bilinear && !self.invert
    }

    /// A stop's midpoint is off the middle.
    pub fn has_midpoints(&self) -> bool {
        let n = self.stops.len();
        self.stops
            .iter()
            .take(n.saturating_sub(1))
            .any(|s| !is_half(&s.mid))
    }

    /// The sorted stops with each blend that does not run straight (its
    /// midpoint off the middle, the acceleration or the smooth transition
    /// on, or a hue direction) drawn as [`Self::MID_STEPS`] straight
    /// pieces along its curve, so a plain run of stops mixed in RGB gives
    /// the same colours (the midpoints set back to the middle). The
    /// colour bands of a stepped fountain are not in here: see
    /// [`Self::band_param`] and [`Self::sample_blended`].
    pub fn blended_stops(&self) -> Vec<Stop> {
        let base = self.sorted_stops();
        let mut out: Vec<Stop> = Vec::with_capacity(base.len());
        for (i, s) in base.iter().enumerate() {
            out.push(Stop { mid: 0.5, ..*s });
            let Some(next) = base.get(i + 1) else {
                continue;
            };
            let mid = self.blend_mid(s);
            let straight =
                is_half(&mid) && !self.smooth && self.blend_direction == ColorBlend::Direct;
            if straight || next.pos - s.pos < 1e-9 {
                continue;
            }
            for j in 1..Self::MID_STEPS {
                let k = j as f64 / Self::MID_STEPS as f64;
                let w = self.blend_weight(k, mid) as f32;
                out.push(Stop {
                    pos: s.pos + (next.pos - s.pos) * k,
                    color: blend_color(s.color, next.color, w, self.blend_direction),
                    mid: 0.5,
                    opacity: s.opacity + (next.opacity - s.opacity) * w,
                });
            }
        }
        out
    }

    /// Pieces per blend that does not run straight (see
    /// [`Self::blended_stops`]). A hue sweep is straight in RGB within
    /// each sixth of the wheel, so sixteen pieces follow it closely.
    pub const MID_STEPS: usize = 16;

    /// Any stop is not fully opaque.
    pub fn has_opacity(&self) -> bool {
        self.stops.iter().any(|s| s.opacity < 1.0)
    }

    /// The stops sorted by position, at least two.
    pub fn sorted_stops(&self) -> Vec<Stop> {
        let mut v = self.stops.clone();
        v.sort_by(|a, b| {
            a.pos
                .partial_cmp(&b.pos)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if v.len() < 2 {
            let c = v.first().map(|s| s.color).unwrap_or(Color::BLACK);
            v = vec![Stop::new(0.0, c), Stop::new(1.0, c)];
        }
        v
    }

    /// Where in the stops the colour at `t` (0..1 across the span) is
    /// taken: the bilinear mirror, then the repeat cycles, the edge pad
    /// and the inversion.
    pub fn stop_param(&self, t: f64) -> f64 {
        let mut u = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.bilinear {
            u = 1.0 - (2.0 * u - 1.0).abs();
        }
        let n = self.cycles() as f64;
        if n > 1.0 {
            let x = u * n;
            let k = x.floor();
            let fr = x - k;
            u = match self.repeat {
                FountainRepeat::Mirror if (k as i64) % 2 == 1 => 1.0 - fr,
                FountainRepeat::Mirror => fr,
                _ if x >= n => 1.0,
                _ => fr,
            };
        }
        let e = self.edge_pad.clamp(0.0, 0.49);
        if e > 0.0 {
            u = ((u - e) / (1.0 - 2.0 * e)).clamp(0.0, 1.0);
        }
        if self.invert {
            u = 1.0 - u;
        }
        u
    }

    /// Colour at `t` in 0..1, with every setting in (one sample: drawing
    /// many takes [`Self::blended_stops`] once and [`Self::sample_blended`]).
    pub fn color_at(&self, t: f64) -> Color {
        self.sample_blended(&self.blended_stops(), t).color
    }

    /// The opacity at `t` (0 clear, 1 opaque).
    pub fn opacity_at(&self, t: f64) -> f32 {
        self.sample_blended(&self.blended_stops(), t).opacity
    }

    /// The fountain as plain stops over 0..1 that give the same colours
    /// in one run, with the edge pad, repeat cycles, bilinear mirror,
    /// inversion, blend direction, smooth transition, acceleration and
    /// colour bands worked in (for gradient shaders and exports that only
    /// know a single run of stops). Equal positions are hard edges: a
    /// sawtooth's jumps, and the edge between two bands of a stepped
    /// fountain (two stops per band).
    pub fn resolved_stops(&self) -> Vec<Stop> {
        let base = self.blended_stops();
        let band = |u: f64| sample_stop(&base, self.band_param(u));
        let cycles = self.cycles();
        let n = cycles as f64;
        let e = self.edge_pad.clamp(0.0, 0.49);
        // Where the mapping from t to the stop parameter bends.
        let mut us = Vec::new();
        for k in 0..=cycles {
            let k = k as f64;
            us.push(k / n);
            if e > 0.0 && k < n {
                us.push((k + e) / n);
                us.push((k + 1.0 - e) / n);
            }
        }
        let mut ts: Vec<f64> = Vec::new();
        for u in us {
            if self.bilinear {
                ts.push(u / 2.0);
                ts.push(1.0 - u / 2.0);
            } else {
                ts.push(u);
            }
        }
        ts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let mut out: Vec<Stop> = Vec::new();
        let mut push = |pos: f64, s: Stop| {
            if let Some(l) = out.last() {
                if (l.pos - pos).abs() < 1e-12 && l.color == s.color && l.opacity == s.opacity {
                    return;
                }
            }
            out.push(Stop { pos, ..s });
        };
        for w in ts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b - a < 1e-12 {
                continue;
            }
            // The mapping is straight between bends: its ends from two
            // samples inside, so a sawtooth's jump lands on the right side.
            let (q1, q3) = (a + (b - a) * 0.25, a + (b - a) * 0.75);
            let (p1, p3) = (self.stop_param(q1), self.stop_param(q3));
            let slope = (p3 - p1) / (q3 - q1);
            let pa = (p1 - slope * (q1 - a)).clamp(0.0, 1.0);
            let pb = (p3 + slope * (b - q3)).clamp(0.0, 1.0);
            push(a, band(pa));
            if (pb - pa).abs() > 1e-12 {
                let (lo, hi) = (pa.min(pb), pa.max(pb));
                let to_t = |u: f64| a + (u - pa) / (pb - pa) * (b - a);
                if self.stepped() {
                    // The edges between bands inside this piece: a hard
                    // edge each, the band on either side of it.
                    let n = self.band_count();
                    let mut edges: Vec<f64> = (1..n)
                        .map(|k| k as f64 / n as f64)
                        .filter(|u| *u > lo + 1e-9 && *u < hi - 1e-9)
                        .collect();
                    if pb < pa {
                        edges.reverse();
                    }
                    let side = if pb > pa { 1e-7 } else { -1e-7 };
                    for u in edges {
                        let t = to_t(u);
                        push(t, band(u - side));
                        push(t, band(u + side));
                    }
                } else {
                    let mut inner: Vec<Stop> = base
                        .iter()
                        .filter(|s| s.pos > lo + 1e-9 && s.pos < hi - 1e-9)
                        .map(|s| Stop {
                            pos: to_t(s.pos),
                            ..*s
                        })
                        .collect();
                    if pb < pa {
                        inner.reverse();
                    }
                    for s in inner {
                        push(s.pos, s);
                    }
                }
            }
            push(b, band(pb));
        }
        if out.len() < 2 {
            return base;
        }
        out
    }

    /// The closest fountain that runs once with no edge pad and one of
    /// the four geometric kinds, for formats that know nothing more: the
    /// options (the blend settings too) are worked into the stops (at
    /// most `max_stops`), and a shaped fountain becomes a square one
    /// running from the centre out. A fountain with none of the options
    /// comes back unchanged.
    pub fn to_plain(&self, max_stops: usize) -> Fountain {
        if self.runs_once()
            && self.kind != FountainKind::Shaped
            && !self.has_midpoints()
            && !self.has_blend_options()
        {
            return self.clone();
        }
        let mut stops = self.resolved_stops();
        if self.kind == FountainKind::Shaped {
            // Shaped: 0 at the edge; square: 0 at the centre.
            stops.reverse();
            for s in &mut stops {
                s.pos = 1.0 - s.pos;
            }
        }
        let max_stops = max_stops.max(2);
        if stops.len() > max_stops {
            let k = (max_stops - 1) as f64;
            stops = (0..max_stops)
                .map(|i| {
                    let pos = i as f64 / k;
                    Stop {
                        pos,
                        ..sample_stop(&stops, pos)
                    }
                })
                .collect();
        }
        Fountain {
            kind: match self.kind {
                FountainKind::Shaped => FountainKind::Square,
                k => k,
            },
            stops,
            angle: self.angle,
            offset: self.offset,
            edge_pad: 0.0,
            repeat: FountainRepeat::None,
            repeats: 1,
            bilinear: false,
            invert: false,
            blend_direction: ColorBlend::Direct,
            steps: Self::SMOOTH_STEPS,
            smooth: false,
            acceleration: 0,
            handles: None,
        }
    }
}

/// Colour at `t` along `stops` (sorted by position): the two around it
/// mixed in sRGB, the end colours beyond them.
pub fn sample_stops(stops: &[Stop], t: f64) -> Color {
    sample_stop(stops, t).color
}

/// Opacity at `t` along `stops` (sorted by position), as
/// [`sample_stops`] mixes the colours.
pub fn sample_opacity(stops: &[Stop], t: f64) -> f32 {
    sample_stop(stops, t).opacity
}

/// The stop at `t` along `stops` (sorted by position): colour and opacity
/// mixed from the two around it, its blend half way. The stop after `t`
/// is found by bisection (this runs once per pixel for the fills drawn
/// pixel by pixel).
pub fn sample_stop(stops: &[Stop], t: f64) -> Stop {
    let Some(first) = stops.first() else {
        return Stop::new(t, Color::BLACK);
    };
    if t <= first.pos {
        return Stop {
            pos: t,
            mid: 0.5,
            ..*first
        };
    }
    // The first stop at or after `t`; the one before it starts the blend.
    let i = stops.partition_point(|s| s.pos < t);
    match (stops.get(i.wrapping_sub(1)), stops.get(i)) {
        (Some(a), Some(b)) => {
            let span = (b.pos - a.pos).max(1e-9);
            let k = ((t - a.pos) / span).clamp(0.0, 1.0) as f32;
            Stop {
                pos: t,
                color: lerp_color(a.color, b.color, k),
                mid: 0.5,
                opacity: a.opacity + (b.opacity - a.opacity) * k,
            }
        }
        _ => {
            let last = stops.last().copied().unwrap_or(*first);
            Stop {
                pos: t,
                mid: 0.5,
                ..last
            }
        }
    }
}

pub fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let [r1, g1, b1] = a.to_rgb8();
    let [r2, g2, b2] = b.to_rgb8();
    let l = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgb8(l(r1, r2), l(g1, g2), l(b1, b2))
}

/// Built-in two-colour pattern tiles (the two-colour pattern fill).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PatternTile {
    #[default]
    Checker,
    Dots,
    Stripes,
    DiagonalStripes,
    Grid,
    Bricks,
    Crosshatch,
    Waves,
}

impl PatternTile {
    pub const ALL: [PatternTile; 8] = [
        PatternTile::Checker,
        PatternTile::Dots,
        PatternTile::Stripes,
        PatternTile::DiagonalStripes,
        PatternTile::Grid,
        PatternTile::Bricks,
        PatternTile::Crosshatch,
        PatternTile::Waves,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PatternTile::Checker => "Checker",
            PatternTile::Dots => "Dots",
            PatternTile::Stripes => "Stripes",
            PatternTile::DiagonalStripes => "Diagonal stripes",
            PatternTile::Grid => "Grid",
            PatternTile::Bricks => "Bricks",
            PatternTile::Crosshatch => "Crosshatch",
            PatternTile::Waves => "Waves",
        }
    }

    /// Is the tile pixel at (u, v) in 0..1 "front" (true) or "back" colour?
    pub fn front(self, u: f64, v: f64) -> bool {
        match self {
            PatternTile::Checker => (u < 0.5) == (v < 0.5),
            PatternTile::Dots => ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() < 0.3,
            PatternTile::Stripes => v < 0.5,
            PatternTile::DiagonalStripes => (u + v).rem_euclid(1.0) < 0.5,
            PatternTile::Grid => u < 0.12 || v < 0.12,
            PatternTile::Bricks => {
                let row = v < 0.5;
                let uu = if row { u } else { (u + 0.5).rem_euclid(1.0) };
                v.rem_euclid(0.5) < 0.06 || uu < 0.06
            }
            PatternTile::Crosshatch => {
                (u + v).rem_euclid(0.5) < 0.07 || (u - v).rem_euclid(0.5) < 0.07
            }
            PatternTile::Waves => ((v - 0.5) - 0.2 * (u * std::f64::consts::TAU).sin()).abs() < 0.1,
        }
    }
}

/// Which run of tiles a pattern's row or column offset shifts: every row
/// against the one below it, or every column against the one left of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TileShift {
    #[default]
    Row,
    Column,
}

/// How a pattern's tiles lie on the page, the same for every pattern
/// kind. The defaults are today's tiling: tiles the kind's own size, the
/// first tile's bottom-left corner on the page origin, upright, every
/// tile the same way up, anchored to the page.
///
/// A placement is a transform from the tile to the page: tiles are laid
/// out as a lattice (`cell`), mirrored tiles and the row or column shift
/// worked into the block of tiles that repeats, then that block is scaled
/// to `size` and moved, turned and skewed by `place`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct PatternPlacement {
    /// Every second column of tiles is flipped left to right.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mirror_horizontal: bool,
    /// Every second row of tiles is flipped top to bottom.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mirror_vertical: bool,
    /// Where the first tile's bottom-left corner sits, from the page
    /// origin (or the object's origin, see `transform_with_object`), mm.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_x: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_y: f64,
    /// The tiles turned around the offset point, degrees counter-clockwise.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
    /// The tiles' vertical sides tilted by this angle, degrees (positive
    /// leans the tops to the right).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub skew: f64,
    /// Each row (or column, see `shift`) is moved along by this much of a
    /// tile against the one before it, percent 0..100.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shift_percent: f64,
    /// Whether `shift_percent` moves the rows or the columns.
    #[serde(default, skip_serializing_if = "is_default_shift")]
    pub shift: TileShift,
    /// The size of one tile on the page, mm; `None` keeps the kind's own
    /// size (`Pattern::natural_tile`). The tile's content is stretched to
    /// fit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    /// The tiles follow the object's transformation (they move, scale and
    /// turn with it) instead of staying anchored to the page.
    #[serde(default, skip_serializing_if = "is_false")]
    pub transform_with_object: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_default_shift(s: &TileShift) -> bool {
    *s == TileShift::Row
}

impl PatternPlacement {
    /// Today's tiling: nothing set.
    pub fn is_default(&self) -> bool {
        *self == PatternPlacement::default()
    }

    /// A placement with every value usable: nothing infinite, the shift
    /// within 0..100 percent, the tile size positive.
    pub fn sanitized(mut self) -> PatternPlacement {
        for v in [
            &mut self.offset_x,
            &mut self.offset_y,
            &mut self.rotation,
            &mut self.skew,
            &mut self.shift_percent,
        ] {
            if !v.is_finite() {
                *v = 0.0;
            }
        }
        self.shift_percent = self.shift_percent.clamp(0.0, 100.0);
        // A skew of 90 degrees would flatten the tiles to a line.
        self.skew = self.skew.clamp(-89.0, 89.0);
        if let Some(s) = self.size {
            if !(s.width.is_finite() && s.height.is_finite()) || s.width <= 0.0 || s.height <= 0.0 {
                self.size = None;
            }
        }
        self
    }

    /// Whether the block that repeats holds more than one tile: mirrored
    /// tiles or a row or column shift. Without, one tile is the block.
    pub fn needs_cell(&self) -> bool {
        self.mirror_horizontal || self.mirror_vertical || self.shift_percent != 0.0
    }

    /// Tiles across and down in the block that repeats (2 where mirrored).
    pub fn cell_tiles(&self) -> (u32, u32) {
        (
            1 + u32::from(self.mirror_horizontal),
            1 + u32::from(self.mirror_vertical),
        )
    }

    /// The lattice of the block that repeats: the unit square (0..1, Y up)
    /// to the block in tile millimetres (before `place`), for a tile of
    /// `tile` mm. With a row shift the block is a parallelogram leaning
    /// along X, with a column shift one leaning along Y, so the tiles of
    /// each row (column) sit `shift_percent` of a tile further along.
    pub fn cell(&self, tile: Size) -> Affine {
        let (cols, rows) = self.cell_tiles();
        let (w, h) = (tile.width, tile.height);
        let (cw, ch) = (w * f64::from(cols), h * f64::from(rows));
        let s = self.shift_percent / 100.0;
        match self.shift {
            TileShift::Row => Affine::new([cw, 0.0, s * w * f64::from(rows), ch, 0.0, 0.0]),
            TileShift::Column => Affine::new([cw, s * h * f64::from(cols), 0.0, ch, 0.0, 0.0]),
        }
    }

    /// Every copy of the tile that shows in the block that repeats: the
    /// transform from one tile (mm, Y up, origin at its bottom-left) to
    /// the block's millimetres (the space `cell` maps to), mirrored tiles
    /// flipped in place. With a shift, the copies at the sides fill the
    /// leaning edges of the block.
    pub fn tile_copies(&self, tile: Size) -> Vec<Affine> {
        let (cols, rows) = self.cell_tiles();
        let (w, h) = (tile.width, tile.height);
        let s = self.shift_percent / 100.0;
        let shifted = s != 0.0;
        let mut out = Vec::new();
        let (i0, i1, j0, j1) = match (self.shift, shifted) {
            (_, false) => (0, cols as i32 - 1, 0, rows as i32 - 1),
            (TileShift::Row, true) => (-1, cols as i32 + 1, 0, rows as i32 - 1),
            (TileShift::Column, true) => (0, cols as i32 - 1, -1, rows as i32 + 1),
        };
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (dx, dy) = match self.shift {
                    TileShift::Row => (f64::from(i) * w + f64::from(j) * s * w, f64::from(j) * h),
                    TileShift::Column => {
                        (f64::from(i) * w, f64::from(j) * h + f64::from(i) * s * h)
                    }
                };
                let mut m = Affine::translate((dx, dy));
                if self.mirror_horizontal && i.rem_euclid(2) == 1 {
                    m *= Affine::new([-1.0, 0.0, 0.0, 1.0, w, 0.0]);
                }
                if self.mirror_vertical && j.rem_euclid(2) == 1 {
                    m *= Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, h]);
                }
                out.push(m);
            }
        }
        out
    }

    /// The tile coordinates (u, v in 0..1, Y up) under a point of the
    /// block's millimetres (the space `cell` maps to): which tile the point
    /// is in, with the row or column shift and the mirroring of every
    /// second tile applied. For patterns sampled per pixel.
    pub fn tile_uv(&self, p: Point, tile: Size) -> (f64, f64) {
        let (w, h) = (tile.width.max(1e-12), tile.height.max(1e-12));
        let s = self.shift_percent / 100.0;
        let (i, j, u, v) = match self.shift {
            TileShift::Row => {
                let j = (p.y / h).floor();
                let x = p.x - j * s * w;
                let i = (x / w).floor();
                (i, j, x / w - i, p.y / h - j)
            }
            TileShift::Column => {
                let i = (p.x / w).floor();
                let y = p.y - i * s * h;
                let j = (y / h).floor();
                (i, j, p.x / w - i, y / h - j)
            }
        };
        let odd = |k: f64| (k as i64).rem_euclid(2) == 1;
        let u = if self.mirror_horizontal && odd(i) {
            1.0 - u
        } else {
            u
        };
        let v = if self.mirror_vertical && odd(j) {
            1.0 - v
        } else {
            v
        };
        (u, v)
    }

    /// The block's millimetres to the page (or the object, with
    /// `transform_with_object`): skew, then rotation, then the offset.
    pub fn place(&self) -> Affine {
        Affine::translate((self.offset_x, self.offset_y))
            * Affine::rotate(self.rotation.to_radians())
            * Affine::skew(self.skew.to_radians().tan(), 0.0)
    }

    /// The whole placement: the unit square of the block that repeats to
    /// the page, for a tile of `tile` mm (`place` after `cell`).
    pub fn transform(&self, tile: Size) -> Affine {
        self.place() * self.cell(tile)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "pattern", rename_all = "lowercase")]
pub enum Pattern {
    TwoColor {
        tile: PatternTile,
        front: Color,
        back: Color,
        size_mm: f64,
        #[serde(default, skip_serializing_if = "PatternPlacement::is_default")]
        placement: PatternPlacement,
    },
    /// Tiled PNG, `size_mm` is the tile width; height follows the aspect ratio.
    Bitmap {
        png: crate::SharedBytes,
        width_px: u32,
        height_px: u32,
        size_mm: f64,
        #[serde(default, skip_serializing_if = "PatternPlacement::is_default")]
        placement: PatternPlacement,
    },
    /// Full-colour vector tile (the vector pattern fill):
    /// `shapes` are in tile-local coordinates, origin at the bottom-left
    /// corner of the tile, Y up, and the tile repeats every `tile.width` by
    /// `tile.height` millimetres. The shapes carry their own transforms,
    /// fills and outlines; the placement is the only pattern transform.
    Vector {
        #[serde(default)]
        shapes: Vec<crate::document::Shape>,
        tile: Size,
        #[serde(default, skip_serializing_if = "PatternPlacement::is_default")]
        placement: PatternPlacement,
    },
}

impl Pattern {
    /// Smallest tile edge, in mm, so a degenerate selection still tiles.
    pub const MIN_TILE_MM: f64 = 0.01;

    /// How the tiles lie on the page.
    pub fn placement(&self) -> &PatternPlacement {
        match self {
            Pattern::TwoColor { placement, .. }
            | Pattern::Bitmap { placement, .. }
            | Pattern::Vector { placement, .. } => placement,
        }
    }

    pub fn placement_mut(&mut self) -> &mut PatternPlacement {
        match self {
            Pattern::TwoColor { placement, .. }
            | Pattern::Bitmap { placement, .. }
            | Pattern::Vector { placement, .. } => placement,
        }
    }

    /// The pattern with `placement`.
    pub fn with_placement(mut self, placement: PatternPlacement) -> Pattern {
        *self.placement_mut() = placement;
        self
    }

    /// The tile size the kind carries: a two-colour tile is square, a
    /// bitmap tile keeps its picture's proportions, a vector tile has its
    /// own size. Degenerate sizes come back as `MIN_TILE_MM`.
    pub fn natural_tile(&self) -> Size {
        let ok = |v: f64| {
            if v.is_finite() && v > 0.0 {
                v.max(Self::MIN_TILE_MM)
            } else {
                Self::MIN_TILE_MM
            }
        };
        match self {
            Pattern::TwoColor { size_mm, .. } => Size::new(ok(*size_mm), ok(*size_mm)),
            Pattern::Bitmap {
                width_px,
                height_px,
                size_mm,
                ..
            } => {
                let h = size_mm * f64::from(*height_px) / f64::from((*width_px).max(1));
                Size::new(ok(*size_mm), ok(h))
            }
            Pattern::Vector { tile, .. } => Size::new(ok(tile.width), ok(tile.height)),
        }
    }

    /// The size of one tile on the page, mm: the placement's size when
    /// set, else the kind's own.
    pub fn tile_size(&self) -> Size {
        match self.placement().size {
            Some(s)
                if s.width.is_finite()
                    && s.height.is_finite()
                    && s.width > 0.0
                    && s.height > 0.0 =>
            {
                s
            }
            _ => self.natural_tile(),
        }
    }

    /// Give the tiles the size `size` (mm) on the page. The kind's own size
    /// takes it where it can (a two-colour tile's edge, a bitmap tile's
    /// width, a vector tile's size with its content scaled along); a
    /// height the kind cannot hold goes to the placement's size.
    pub fn set_tile_size(&mut self, size: Size) {
        let w = if size.width.is_finite() {
            size.width.max(Self::MIN_TILE_MM)
        } else {
            Self::MIN_TILE_MM
        };
        let h = if size.height.is_finite() {
            size.height.max(Self::MIN_TILE_MM)
        } else {
            Self::MIN_TILE_MM
        };
        match self {
            Pattern::TwoColor {
                size_mm, placement, ..
            } => {
                *size_mm = w;
                placement.size = ((h - w).abs() > 1e-9).then_some(Size::new(w, h));
            }
            Pattern::Bitmap {
                width_px,
                height_px,
                size_mm,
                placement,
                ..
            } => {
                *size_mm = w;
                let natural_h = w * f64::from(*height_px) / f64::from((*width_px).max(1));
                placement.size = ((h - natural_h).abs() > 1e-9).then_some(Size::new(w, h));
            }
            Pattern::Vector {
                shapes,
                tile,
                placement,
            } => {
                // The content scales with the tile so the drawing keeps
                // its shape.
                let sx = w / tile.width.max(1e-6);
                let sy = h / tile.height.max(1e-6);
                if sx.is_finite() && sy.is_finite() && (sx != 1.0 || sy != 1.0) {
                    for s in shapes.iter_mut() {
                        s.transform = Affine::scale_non_uniform(sx, sy) * s.transform;
                    }
                }
                *tile = Size::new(w, h);
                placement.size = None;
            }
        }
    }

    /// Build a vector pattern from page-space shapes: the shapes are moved
    /// so their joint bounds start at the origin and the tile takes the
    /// size of those bounds.
    pub fn vector_from_shapes(shapes: &[crate::document::Shape]) -> Pattern {
        let mut bounds: Option<Rect> = None;
        for s in shapes {
            let b = s.bounds();
            if !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
                continue;
            }
            bounds = Some(match bounds {
                Some(acc) => acc.union(b),
                None => b,
            });
        }
        let bounds = bounds.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        let shift = Affine::translate((-bounds.x0, -bounds.y0));
        let shapes = shapes
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.transform_by(shift);
                s
            })
            .collect();
        Pattern::Vector {
            shapes,
            tile: Size::new(
                bounds.width().max(Self::MIN_TILE_MM),
                bounds.height().max(Self::MIN_TILE_MM),
            ),
            placement: PatternPlacement::default(),
        }
    }
}

/// Procedural textures (the texture fill).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TextureKind {
    #[default]
    Clouds,
    Marble,
    Noise,
    Wood,
}

impl TextureKind {
    pub const ALL: [TextureKind; 4] = [
        TextureKind::Clouds,
        TextureKind::Marble,
        TextureKind::Noise,
        TextureKind::Wood,
    ];
    pub fn name(self) -> &'static str {
        match self {
            TextureKind::Clouds => "Clouds",
            TextureKind::Marble => "Marble",
            TextureKind::Noise => "Noise",
            TextureKind::Wood => "Wood",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Texture {
    pub kind: TextureKind,
    pub color_a: Color,
    pub color_b: Color,
    /// Feature size in mm.
    pub scale: f64,
    pub seed: u32,
}

/// The patterns of the procedural fill (the PostScript-style fill): each
/// is a tile of filled paths drawn by our own code from a few numbers
/// (see [`crate::procedural::procedural_tile`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProceduralPattern {
    /// Regular octagons on a square grid, the gaps between them squares
    /// (the polygon tiling of octagons and squares).
    #[default]
    Octagons,
    /// A grid of equilateral triangles: lines at 0, 60 and 120 degrees.
    Triangles,
    /// Bricks in rows, each row half a brick along from the one below;
    /// the mortar shows the background.
    Bricks,
    /// Regular hexagons, point up, in staggered rows.
    Honeycomb,
    /// Two families of diagonal lines.
    Crosshatch,
    /// One dot per tile, its size from the share of the tile it covers.
    Dots,
    /// A sine wave across the tile.
    Waves,
    /// An evenly wound spiral in each tile.
    Spirals,
    /// Specks scattered by a pseudo-random sequence from the seed.
    Speckle,
}

/// One number of the procedural fill's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProceduralParam {
    /// The tile size (the frequency), in mm.
    Size,
    /// The width of the lines, in mm.
    LineWidth,
    /// The share of the tile a dot covers, 0 to 1.
    Share,
    /// A wave's amplitude as a share of the tile size.
    Amplitude,
    /// The turns of a spiral.
    Turns,
    /// Specks per tile.
    Density,
    /// The seed of the pseudo-random sequence.
    Seed,
}

impl ProceduralPattern {
    pub const ALL: [ProceduralPattern; 9] = [
        ProceduralPattern::Octagons,
        ProceduralPattern::Triangles,
        ProceduralPattern::Bricks,
        ProceduralPattern::Honeycomb,
        ProceduralPattern::Crosshatch,
        ProceduralPattern::Dots,
        ProceduralPattern::Waves,
        ProceduralPattern::Spirals,
        ProceduralPattern::Speckle,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ProceduralPattern::Octagons => "Octagons and squares",
            ProceduralPattern::Triangles => "Triangles",
            ProceduralPattern::Bricks => "Bricks",
            ProceduralPattern::Honeycomb => "Honeycomb",
            ProceduralPattern::Crosshatch => "Crosshatch",
            ProceduralPattern::Dots => "Dots",
            ProceduralPattern::Waves => "Waves",
            ProceduralPattern::Spirals => "Spirals",
            ProceduralPattern::Speckle => "Speckle",
        }
    }

    /// The parameters this pattern uses, in the order an editor shows
    /// them (the tile size first).
    pub fn params(self) -> &'static [ProceduralParam] {
        use ProceduralParam as P;
        match self {
            ProceduralPattern::Octagons
            | ProceduralPattern::Triangles
            | ProceduralPattern::Honeycomb
            | ProceduralPattern::Crosshatch => &[P::Size, P::LineWidth],
            ProceduralPattern::Bricks => &[P::Size, P::LineWidth],
            ProceduralPattern::Dots => &[P::Size, P::Share],
            ProceduralPattern::Waves => &[P::Size, P::LineWidth, P::Amplitude],
            ProceduralPattern::Spirals => &[P::Size, P::LineWidth, P::Turns],
            ProceduralPattern::Speckle => &[P::Size, P::LineWidth, P::Density, P::Seed],
        }
    }
}

/// The numbers of a procedural fill. Every pattern reads the ones it uses
/// ([`ProceduralPattern::params`]) and leaves the others as they are, so
/// switching patterns keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProceduralParams {
    /// The tile size, in mm (the pattern's frequency).
    #[serde(default = "ProceduralParams::default_size")]
    pub size_mm: f64,
    /// The width of the pattern's lines, in mm.
    #[serde(default = "ProceduralParams::default_line_width")]
    pub line_width_mm: f64,
    /// Dots: the share of the tile's area the dot covers (0 to 0.78).
    #[serde(default = "ProceduralParams::default_share")]
    pub share: f64,
    /// Waves: the amplitude as a share of the tile size (0 to 0.45).
    #[serde(default = "ProceduralParams::default_amplitude")]
    pub amplitude: f64,
    /// Spirals: the turns from the centre to the edge.
    #[serde(default = "ProceduralParams::default_turns")]
    pub turns: f64,
    /// Speckle: specks per tile.
    #[serde(default = "ProceduralParams::default_density")]
    pub density: f64,
}

impl ProceduralParams {
    pub const MIN_SIZE_MM: f64 = 0.5;
    pub const MAX_SIZE_MM: f64 = 200.0;
    pub const MIN_LINE_WIDTH_MM: f64 = 0.02;
    /// A dot can cover at most a disc inscribed in the tile.
    pub const MAX_SHARE: f64 = 0.78;
    pub const MAX_AMPLITUDE: f64 = 0.45;
    pub const MAX_TURNS: f64 = 20.0;
    pub const MAX_DENSITY: f64 = 1000.0;

    fn default_size() -> f64 {
        10.0
    }
    fn default_line_width() -> f64 {
        0.5
    }
    fn default_share() -> f64 {
        0.3
    }
    fn default_amplitude() -> f64 {
        0.2
    }
    fn default_turns() -> f64 {
        3.0
    }
    fn default_density() -> f64 {
        40.0
    }

    /// The same numbers within their ranges (a file may hold anything).
    pub fn clamped(self) -> ProceduralParams {
        let finite = |v: f64, d: f64| if v.is_finite() { v } else { d };
        let size_mm =
            finite(self.size_mm, Self::default_size()).clamp(Self::MIN_SIZE_MM, Self::MAX_SIZE_MM);
        ProceduralParams {
            size_mm,
            line_width_mm: finite(self.line_width_mm, Self::default_line_width())
                .clamp(Self::MIN_LINE_WIDTH_MM, size_mm / 2.0),
            share: finite(self.share, Self::default_share()).clamp(0.0, Self::MAX_SHARE),
            amplitude: finite(self.amplitude, Self::default_amplitude())
                .clamp(0.0, Self::MAX_AMPLITUDE),
            turns: finite(self.turns, Self::default_turns()).clamp(0.5, Self::MAX_TURNS),
            density: finite(self.density, Self::default_density()).clamp(1.0, Self::MAX_DENSITY),
        }
    }
}

impl Default for ProceduralParams {
    fn default() -> Self {
        ProceduralParams {
            size_mm: Self::default_size(),
            line_width_mm: Self::default_line_width(),
            share: Self::default_share(),
            amplitude: Self::default_amplitude(),
            turns: Self::default_turns(),
            density: Self::default_density(),
        }
    }
}

/// The procedural fill: a pattern from the library, its numbers, a
/// foreground colour the paths are filled with, a background colour
/// behind them (or none: what is beneath shows through), and the seed of
/// the patterns that scatter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Procedural {
    #[serde(default)]
    pub pattern: ProceduralPattern,
    #[serde(default)]
    pub params: ProceduralParams,
    #[serde(default = "black")]
    pub foreground: Color,
    #[serde(default)]
    pub background: Option<Color>,
    #[serde(default)]
    pub seed: u32,
}

impl Procedural {
    /// `pattern` in `foreground` on a white background, with the default
    /// numbers.
    pub fn new(pattern: ProceduralPattern, foreground: Color) -> Self {
        Procedural {
            pattern,
            params: ProceduralParams::default(),
            foreground,
            background: Some(Color::WHITE),
            seed: 1,
        }
    }
}

impl Default for Procedural {
    fn default() -> Self {
        Procedural::new(ProceduralPattern::Octagons, Color::BLACK)
    }
}

fn black() -> Color {
    Color::BLACK
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "fill", rename_all = "lowercase")]
pub enum Fill {
    #[default]
    None,
    Solid(Color),
    Fountain(Fountain),
    Pattern(Pattern),
    Texture(Texture),
    Mesh(Mesh),
    Procedural(Procedural),
}

/// Mesh fill: a grid of (rows+1) x (cols+1) nodes with a colour each;
/// the cells between them are patches whose sides are the grid lines
/// (cubic curves when the nodes carry handles, straight otherwise),
/// their colour mixed from the corners (Coons patches), clipped by the
/// object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub rows: u32,
    pub cols: u32,
    /// Row-major, (rows+1)*(cols+1) nodes in local space.
    pub nodes: Vec<MeshNode>,
    #[serde(default)]
    pub smooth: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeshNode {
    pub pos: Point,
    pub color: Color,
    /// 1.0 = opaque.
    #[serde(default = "one_f32")]
    pub alpha: f32,
    /// The handles of the grid lines through the node, in local space:
    /// towards the next column, the previous column, the next row and
    /// the previous row (see [`MeshHandle`]). `None` draws straight
    /// lines to the neighbours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<[Point; 4]>,
}

/// Which of a node's four handles: the index into [`MeshNode::handles`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshHandle {
    /// Towards the next column (the node to the right in the grid).
    Right = 0,
    /// Towards the previous column.
    Left = 1,
    /// Towards the next row.
    Down = 2,
    /// Towards the previous row.
    Up = 3,
}

impl MeshNode {
    /// A node without handles.
    pub fn new(pos: Point, color: Color) -> Self {
        MeshNode {
            pos,
            color,
            alpha: 1.0,
            handles: None,
        }
    }

    /// The handle `h`, or the point a third of the way to `towards` when
    /// the node has none there (a handle on the node itself counts as
    /// none: a straight grid line).
    pub fn handle(&self, h: MeshHandle, towards: Point) -> Point {
        match self.handles {
            Some(hs) if hs[h as usize] != self.pos => hs[h as usize],
            _ => self.pos.lerp(towards, 1.0 / 3.0),
        }
    }

    /// Set the handle `h`, keeping the others (straight when unset).
    pub fn set_handle(&mut self, h: MeshHandle, p: Point) {
        let hs = self.handles.get_or_insert([self.pos; 4]);
        hs[h as usize] = p;
    }

    /// The node moved by `d`, its handles with it.
    pub fn translate(&mut self, d: Vec2) {
        self.pos += d;
        if let Some(hs) = &mut self.handles {
            for h in hs.iter_mut() {
                *h += d;
            }
        }
    }

    /// The node and its handles through `t`.
    pub fn transform(&mut self, t: Affine) {
        self.pos = t * self.pos;
        if let Some(hs) = &mut self.handles {
            for h in hs.iter_mut() {
                *h = t * *h;
            }
        }
    }
}

impl Mesh {
    pub fn new(bounds: crate::geometry::Rect, rows: u32, cols: u32, base: Color) -> Self {
        let mut nodes = Vec::new();
        for r in 0..=rows {
            for c in 0..=cols {
                nodes.push(MeshNode::new(
                    Point::new(
                        bounds.x0 + bounds.width() * c as f64 / cols as f64,
                        bounds.y0 + bounds.height() * r as f64 / rows as f64,
                    ),
                    base,
                ));
            }
        }
        Mesh {
            rows,
            cols,
            nodes,
            smooth: false,
        }
    }

    pub fn node(&self, r: u32, c: u32) -> Option<&MeshNode> {
        self.nodes.get((r * (self.cols + 1) + c) as usize)
    }

    /// The mesh with every node and handle through `t`.
    pub fn transformed(&self, t: Affine) -> Mesh {
        let mut m = self.clone();
        for n in m.nodes.iter_mut() {
            n.transform(t);
        }
        m
    }

    /// Whether any node carries handles (curved grid lines).
    pub fn is_curved(&self) -> bool {
        self.nodes.iter().any(|n| n.handles.is_some())
    }

    /// The grid line between the nodes at `(r, c)` and `(r, c + 1)`
    /// (`across` true) or `(r + 1, c)` (false), as a cubic curve.
    pub fn edge(&self, r: u32, c: u32, across: bool) -> Option<crate::geometry::CubicBez> {
        let (b, h0, h1) = if across {
            (self.node(r, c + 1)?, MeshHandle::Right, MeshHandle::Left)
        } else {
            (self.node(r + 1, c)?, MeshHandle::Down, MeshHandle::Up)
        };
        let a = self.node(r, c)?;
        Some(crate::geometry::CubicBez::new(
            a.pos,
            a.handle(h0, b.pos),
            b.handle(h1, a.pos),
            b.pos,
        ))
    }

    /// The point of the cell `(r, c)` at `(u, v)` (0 to 1 across and
    /// down): the Coons patch bounded by the cell's four grid lines.
    pub fn patch_point(&self, r: u32, c: u32, u: f64, v: f64) -> Option<Point> {
        use crate::geometry::ParamCurve;
        let top = self.edge(r, c, true)?;
        let bottom = self.edge(r + 1, c, true)?;
        let left = self.edge(r, c, false)?;
        let right = self.edge(r, c + 1, false)?;
        let (a, b, d, e) = (top.p0, top.p3, bottom.p0, bottom.p3);
        let p = top.eval(u).to_vec2() * (1.0 - v)
            + bottom.eval(u).to_vec2() * v
            + left.eval(v).to_vec2() * (1.0 - u)
            + right.eval(v).to_vec2() * u
            - a.to_vec2() * ((1.0 - u) * (1.0 - v))
            - b.to_vec2() * (u * (1.0 - v))
            - d.to_vec2() * ((1.0 - u) * v)
            - e.to_vec2() * (u * v);
        Some(p.to_point())
    }

    /// Index of the node nearest to `p` (local space).
    pub fn nearest(&self, p: Point) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1.pos - p)
                    .hypot()
                    .partial_cmp(&(b.1.pos - p).hypot())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    }

    /// The node grid's (row, column) of node `i`.
    pub fn row_col(&self, i: usize) -> (usize, usize) {
        let cols = self.cols as usize + 1;
        (i / cols, i % cols)
    }

    /// The node next to node `i` along the grid line its handle `h`
    /// leaves on; None at the mesh's edge where no line leaves that way.
    pub fn neighbour(&self, i: usize, h: MeshHandle) -> Option<usize> {
        if i >= self.nodes.len() {
            return None;
        }
        let (r, c) = self.row_col(i);
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let (r, c) = match h {
            MeshHandle::Right if c < cols => (r, c + 1),
            MeshHandle::Left if c > 0 => (r, c - 1),
            MeshHandle::Down if r < rows => (r + 1, c),
            MeshHandle::Up if r > 0 => (r - 1, c),
            _ => return None,
        };
        let j = r * (cols + 1) + c;
        (j < self.nodes.len()).then_some(j)
    }

    /// Where handle `h` of node `i` is (local space): its own place, or a
    /// third of the way to the neighbour for a straight line; None where
    /// no grid line leaves the node that way.
    pub fn handle_point(&self, i: usize, h: MeshHandle) -> Option<Point> {
        let j = self.neighbour(i, h)?;
        Some(self.nodes.get(i)?.handle(h, self.nodes.get(j)?.pos))
    }

    /// Move handle `h` of node `i` by `d`, bending the grid line it
    /// shapes; false where there is no such handle.
    pub fn move_handle(&mut self, i: usize, h: MeshHandle, d: Vec2) -> bool {
        let Some(at) = self.handle_point(i, h) else {
            return false;
        };
        let to = at + d;
        if !(to.x.is_finite() && to.y.is_finite()) {
            return false;
        }
        match self.nodes.get_mut(i) {
            Some(n) => {
                n.set_handle(h, to);
                true
            }
            None => false,
        }
    }

    /// The colour the mesh shows on the whole: the mean of its nodes'
    /// colours weighted by their opacity (the one colour when they all
    /// have it), what a format without mesh fills keeps of it. None for
    /// a mesh without nodes.
    pub fn mean_color(&self) -> Option<Color> {
        let first = self.nodes.first()?.color;
        if self.nodes.iter().all(|n| n.color == first) {
            return Some(first);
        }
        let (mut sum, mut weight) = ([0.0f64; 3], 0.0f64);
        for n in &self.nodes {
            let a = if n.alpha.is_finite() {
                f64::from(n.alpha.clamp(0.0, 1.0))
            } else {
                1.0
            };
            let [r, g, b] = n.color.to_rgb8();
            sum[0] += f64::from(r) * a;
            sum[1] += f64::from(g) * a;
            sum[2] += f64::from(b) * a;
            weight += a;
        }
        if weight <= 0.0 {
            return Some(first);
        }
        let c = |v: f64| (v / weight).round().clamp(0.0, 255.0) as u8;
        Some(Color::rgb8(c(sum[0]), c(sum[1]), c(sum[2])))
    }

    /// The outer edge of the grid, round its first row, last column, last
    /// row and first column: the area the mesh paints (nothing outside
    /// it). Empty for a mesh without cells.
    pub fn outline(&self) -> BezPath {
        let mut out = BezPath::new();
        let (rows, cols) = (self.rows, self.cols);
        if rows == 0 || cols == 0 || self.nodes.len() < ((rows + 1) * (cols + 1)) as usize {
            return out;
        }
        let mut edges: Vec<crate::geometry::CubicBez> = Vec::new();
        edges.extend((0..cols).filter_map(|c| self.edge(0, c, true)));
        edges.extend((0..rows).filter_map(|r| self.edge(r, cols, false)));
        edges.extend((0..cols).rev().filter_map(|c| {
            self.edge(rows, c, true)
                .map(|e| crate::geometry::CubicBez::new(e.p3, e.p2, e.p1, e.p0))
        }));
        edges.extend((0..rows).rev().filter_map(|r| {
            self.edge(r, 0, false)
                .map(|e| crate::geometry::CubicBez::new(e.p3, e.p2, e.p1, e.p0))
        }));
        if let Some(first) = edges.first() {
            out.move_to(first.p0);
        }
        for e in &edges {
            out.curve_to(e.p1, e.p2, e.p3);
        }
        out.close_path();
        out
    }

    /// Whether the mesh paints all of `area` (in the mesh's space): what
    /// lies outside its grid would be left empty. Within a thousandth of
    /// the area, so a result whose edge runs along the grid's counts.
    pub fn covers(&self, area: &BezPath) -> bool {
        use crate::geometry::Shape as _;
        let outline = self.outline();
        if outline.elements().is_empty() {
            return false;
        }
        let whole = crate::shaping::overlay(area, &BezPath::new(), crate::shaping::Op::Weld)
            .area()
            .abs();
        let outside = crate::shaping::overlay(area, &outline, crate::shaping::Op::Trim)
            .area()
            .abs();
        whole.is_finite() && outside.is_finite() && outside <= whole * 1e-3 + 1e-9
    }

    /// The mesh as the grid sees it at row `v` and column `u` (fractions
    /// of the rows and columns, 0 to 1): position, colour and opacity
    /// mixed from the four nodes around it.
    pub fn sample(&self, u: f64, v: f64) -> Option<MeshNode> {
        let (rows, cols) = (self.rows.max(1) as f64, self.cols.max(1) as f64);
        let (x, y) = (u.clamp(0.0, 1.0) * cols, v.clamp(0.0, 1.0) * rows);
        let c = (x.floor() as u32).min(self.cols.saturating_sub(1));
        let r = (y.floor() as u32).min(self.rows.saturating_sub(1));
        let (fu, fv) = (x - c as f64, y - r as f64);
        let (a, b, d, e) = (
            *self.node(r, c)?,
            *self.node(r, c + 1)?,
            *self.node(r + 1, c)?,
            *self.node(r + 1, c + 1)?,
        );
        let mix = |p: MeshNode, q: MeshNode, t: f64| MeshNode {
            pos: p.pos.lerp(q.pos, t),
            color: lerp_color(p.color, q.color, t as f32),
            alpha: p.alpha + (q.alpha - p.alpha) * t as f32,
            handles: None,
        };
        Some(mix(mix(a, b, fu), mix(d, e, fu), fv))
    }

    /// The same mesh on a grid of `rows` by `cols` cells spread evenly over
    /// its nodes' bounds, each new node taking the colour and opacity the
    /// old grid has there.
    pub fn regrid(&self, rows: u32, cols: u32) -> Mesh {
        let (rows, cols) = (rows.clamp(1, 50), cols.clamp(1, 50));
        let bounds = self
            .nodes
            .iter()
            .map(|n| crate::geometry::Rect::from_points(n.pos, n.pos))
            .reduce(|a, b| a.union(b))
            .unwrap_or_default();
        let base = self.nodes.first().map(|n| n.color).unwrap_or(Color::WHITE);
        let mut out = Mesh::new(bounds, rows, cols, base);
        out.smooth = self.smooth;
        for r in 0..=rows {
            for c in 0..=cols {
                let i = (r * (cols + 1) + c) as usize;
                if let (Some(old), Some(n)) = (
                    self.sample(c as f64 / cols as f64, r as f64 / rows as f64),
                    out.nodes.get_mut(i),
                ) {
                    n.color = old.color;
                    n.alpha = old.alpha;
                }
            }
        }
        out
    }

    /// Remove the grid lines through node `i` that are not on the edge:
    /// its column, its row, or both. False when both are on the edge.
    /// Where the removed line had cut a curved grid line, the neighbours'
    /// handles are stretched back so the curve through them is restored.
    pub fn remove_lines_through(&mut self, i: usize) -> bool {
        if i >= self.nodes.len() {
            return false;
        }
        let (r, c) = self.row_col(i);
        let cols = self.cols as usize + 1;
        let drop_col = c > 0 && c < self.cols as usize;
        let drop_row = r > 0 && r < self.rows as usize;
        if !drop_col && !drop_row {
            return false;
        }
        if drop_col {
            for rr in 0..=self.rows as usize {
                let k = rr * cols + c;
                if let (Some(&removed), Some(&a), Some(&b)) = (
                    self.nodes.get(k),
                    self.nodes.get(k - 1),
                    self.nodes.get(k + 1),
                ) {
                    let (mut a, mut b) = (a, b);
                    merge_nodes(&mut a, &mut b, &removed, true);
                    self.nodes[k - 1] = a;
                    self.nodes[k + 1] = b;
                }
            }
        }
        if drop_row {
            for cc in 0..cols {
                let k = r * cols + cc;
                if let (Some(&removed), Some(&a), Some(&b)) = (
                    self.nodes.get(k),
                    self.nodes.get(k - cols),
                    self.nodes.get(k + cols),
                ) {
                    let (mut a, mut b) = (a, b);
                    merge_nodes(&mut a, &mut b, &removed, false);
                    self.nodes[k - cols] = a;
                    self.nodes[k + cols] = b;
                }
            }
        }
        let nodes: Vec<MeshNode> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(k, _)| !(drop_col && k % cols == c) && !(drop_row && k / cols == r))
            .map(|(_, n)| *n)
            .collect();
        self.nodes = nodes;
        if drop_col {
            self.cols -= 1;
        }
        if drop_row {
            self.rows -= 1;
        }
        true
    }

    /// The cell of the grid that holds `p` (local space) and where in
    /// it: (row, column, fraction across, fraction up), the fractions
    /// those of the cell's patch (the Coons patch of its grid lines; the
    /// bilinear patch when they are straight).
    pub fn locate(&self, p: Point) -> Option<(usize, usize, f64, f64)> {
        for r in 0..self.rows {
            for c in 0..self.cols {
                // Newton steps on the patch from its middle, the slopes
                // taken by small steps; kept near the cell, as a point in
                // another cell makes the steps wander.
                let (mut u, mut v) = (0.5, 0.5);
                const STEP: f64 = 1e-6;
                let at = |u: f64, v: f64| self.patch_point(r, c, u, v);
                for _ in 0..16 {
                    let (Some(q), Some(qu), Some(qv)) =
                        (at(u, v), at(u + STEP, v), at(u, v + STEP))
                    else {
                        break;
                    };
                    let f = q - p;
                    if f.hypot() < 1e-9 {
                        break;
                    }
                    let du = (qu - q) / STEP;
                    let dv = (qv - q) / STEP;
                    let det = du.x * dv.y - du.y * dv.x;
                    if det.abs() < 1e-12 {
                        break;
                    }
                    u -= (f.x * dv.y - f.y * dv.x) / det;
                    v -= (du.x * f.y - du.y * f.x) / det;
                    if !(u.is_finite() && v.is_finite()) {
                        break;
                    }
                    u = u.clamp(-0.5, 1.5);
                    v = v.clamp(-0.5, 1.5);
                }
                const EPS: f64 = 1e-6;
                if (-EPS..=1.0 + EPS).contains(&u)
                    && (-EPS..=1.0 + EPS).contains(&v)
                    && at(u, v).is_some_and(|q| (q - p).hypot() < 1e-6)
                {
                    return Some((r as usize, c as usize, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)));
                }
            }
        }
        None
    }

    /// A column of nodes between columns `c` and `c + 1`, `t` of the way
    /// across, positions, colours and opacity mixed from its neighbours;
    /// a curved grid line is cut there, the neighbours' handles shortened
    /// to the cut.
    pub fn insert_column(&mut self, c: usize, t: f64) {
        let cols = self.cols as usize + 1;
        if c + 1 >= cols {
            return;
        }
        let mut nodes = Vec::with_capacity(self.nodes.len() + self.rows as usize + 1);
        for (k, n) in self.nodes.iter().enumerate() {
            let mut n = *n;
            if k % cols == c {
                if let Some(next) = self.nodes.get(k + 1) {
                    let mut next = *next;
                    let between = split_nodes(&mut n, &mut next, t, true);
                    nodes.push(n);
                    nodes.push(between);
                    continue;
                }
            } else if k % cols == c + 1 {
                if let Some(prev) = self.nodes.get(k - 1) {
                    let mut prev = *prev;
                    split_nodes(&mut prev, &mut n, t, true);
                }
            }
            nodes.push(n);
        }
        self.nodes = nodes;
        self.cols += 1;
    }

    /// A row of nodes between rows `r` and `r + 1`, `t` of the way up.
    pub fn insert_row(&mut self, r: usize, t: f64) {
        let cols = self.cols as usize + 1;
        if r >= self.rows as usize {
            return;
        }
        let at = (r + 1) * cols;
        let mut above: Vec<MeshNode> = Vec::with_capacity(cols);
        let mut below: Vec<MeshNode> = Vec::with_capacity(cols);
        let row: Vec<MeshNode> = (0..cols)
            .filter_map(|c| {
                let mut n = *self.nodes.get(r * cols + c)?;
                let mut next = *self.nodes.get((r + 1) * cols + c)?;
                let between = split_nodes(&mut n, &mut next, t, false);
                above.push(n);
                below.push(next);
                Some(between)
            })
            .collect();
        if row.len() != cols || at > self.nodes.len() {
            return;
        }
        for (c, (a, b)) in above.into_iter().zip(below).enumerate() {
            if let Some(n) = self.nodes.get_mut(r * cols + c) {
                *n = a;
            }
            if let Some(n) = self.nodes.get_mut((r + 1) * cols + c) {
                *n = b;
            }
        }
        self.nodes.splice(at..at, row);
        self.rows += 1;
    }

    /// Double-click at `p` (local space) inside the mesh: a node there,
    /// on a new grid line across and a new one up and down through `p`;
    /// within `tol` of a grid line only the line across it is added, and
    /// on a node nothing. The lines go through `p` itself, not halfway
    /// between the nearest nodes (#804). False when nothing changed.
    pub fn add_lines_at(&mut self, p: Point, tol: f64) -> bool {
        let Some((r, c, u, v)) = self.locate(p) else {
            return false;
        };
        let cols = self.cols as usize + 1;
        let pos = |r: usize, c: usize| self.nodes.get(r * cols + c).map(|n| n.pos);
        // The cell's size across and up, at `p`.
        let (Some(a), Some(b), Some(d), Some(e)) =
            (pos(r, c), pos(r, c + 1), pos(r + 1, c), pos(r + 1, c + 1))
        else {
            return false;
        };
        let width = (a.lerp(d, v) - b.lerp(e, v)).hypot().max(1e-9);
        let height = (a.lerp(b, u) - d.lerp(e, u)).hypot().max(1e-9);
        let near_column = u * width <= tol || (1.0 - u) * width <= tol;
        let near_row = v * height <= tol || (1.0 - v) * height <= tol;
        if !near_column {
            self.insert_column(c, u);
        }
        if !near_row {
            self.insert_row(r, v);
        }
        !(near_column && near_row)
    }
}

/// The node `removed` between `a` and `b` on a grid line (`across`
/// true: along a row, else down a column) goes: the handles of `a` and
/// `b` towards it are stretched back to span the whole line, undoing the
/// cut `split_nodes` made (the cut's place taken from the chords).
fn merge_nodes(a: &mut MeshNode, b: &mut MeshNode, removed: &MeshNode, across: bool) {
    if a.handles.is_none() && b.handles.is_none() && removed.handles.is_none() {
        return;
    }
    let (fwd, back) = if across {
        (MeshHandle::Right, MeshHandle::Left)
    } else {
        (MeshHandle::Down, MeshHandle::Up)
    };
    let la = (removed.pos - a.pos).hypot();
    let lb = (b.pos - removed.pos).hypot();
    if la + lb <= 0.0 {
        return;
    }
    let t = (la / (la + lb)).clamp(0.05, 0.95);
    let ha = a.pos + (a.handle(fwd, removed.pos) - a.pos) / t;
    let hb = b.pos + (b.handle(back, removed.pos) - b.pos) / (1.0 - t);
    a.set_handle(fwd, ha);
    b.set_handle(back, hb);
}

/// The node `t` of the way from `a` to `b` along the grid line between
/// them (`across` true: `b` is the next column, else the next row).
/// With handles, the line is cut there: the new node takes the cut's
/// handles and `a` and `b` have theirs shortened to it, so the line
/// keeps its shape; its handles across the line are mixed from the two.
fn split_nodes(a: &mut MeshNode, b: &mut MeshNode, t: f64, across: bool) -> MeshNode {
    let color = lerp_color(a.color, b.color, t as f32);
    let alpha = a.alpha + (b.alpha - a.alpha) * t as f32;
    if a.handles.is_none() && b.handles.is_none() {
        return MeshNode {
            pos: a.pos.lerp(b.pos, t),
            color,
            alpha,
            handles: None,
        };
    }
    use crate::geometry::{CubicBez, ParamCurve};
    let (fwd, back, side0, side1) = if across {
        (
            MeshHandle::Right,
            MeshHandle::Left,
            MeshHandle::Down,
            MeshHandle::Up,
        )
    } else {
        (
            MeshHandle::Down,
            MeshHandle::Up,
            MeshHandle::Right,
            MeshHandle::Left,
        )
    };
    let curve = CubicBez::new(a.pos, a.handle(fwd, b.pos), b.handle(back, a.pos), b.pos);
    let (first, second) = (curve.subsegment(0.0..t), curve.subsegment(t..1.0));
    let pos = first.p3;
    // The sideways handles: the neighbours' offsets mixed.
    let side = |h: MeshHandle| {
        let oa = a.handle(h, a.pos) - a.pos;
        let ob = b.handle(h, b.pos) - b.pos;
        pos + oa.lerp(ob, t)
    };
    let mut n = MeshNode {
        pos,
        color,
        alpha,
        handles: None,
    };
    n.set_handle(side0, side(side0));
    n.set_handle(side1, side(side1));
    n.set_handle(back, first.p2);
    n.set_handle(fwd, second.p1);
    a.set_handle(fwd, first.p1);
    b.set_handle(back, second.p2);
    n
}

impl Fill {
    /// Carry a fountain fill along with a turn, mirror or skew of its
    /// object (see [`Fountain::carry`]); other fills need nothing.
    pub fn carry(&mut self, t: Affine, before: Rect, after: Rect) {
        if let Fill::Fountain(f) = self {
            f.carry(t, before, after);
        }
    }

    /// Keep a fountain fill in its place on the page while its object's
    /// outline changes (see [`Fountain::keep_in_place`]).
    pub fn keep_in_place(&mut self, before: Rect, after: Rect) {
        if let Fill::Fountain(f) = self {
            f.keep_in_place(before, after);
        }
    }

    /// The fill an object made in page space from others (Weld, Combine
    /// and the other shaping operations), covering `area` (page space),
    /// takes from this fill of an object whose transform was `t`. A mesh
    /// fill's nodes go to page space with it while the mesh still covers
    /// the whole result; a result reaching past the mesh's grid (which
    /// paints nothing outside it) takes the mesh's mean colour instead, so
    /// it is never left empty. Other fills stay as they are.
    pub fn for_result(&self, t: Affine, area: &BezPath) -> Fill {
        match self {
            Fill::Mesh(m) => {
                let placed = m.transformed(t);
                if placed.covers(area) {
                    Fill::Mesh(placed)
                } else {
                    placed
                        .mean_color()
                        .map_or(Fill::Solid(Color::Gray { v: 0.5 }), Fill::Solid)
                }
            }
            f => f.clone(),
        }
    }

    pub fn linear(from: Color, to: Color, angle: f64) -> Self {
        Fill::Fountain(Fountain::two(FountainKind::Linear, from, to, angle))
    }

    pub fn radial(from: Color, to: Color) -> Self {
        Fill::Fountain(Fountain::two(FountainKind::Radial, from, to, 0.0))
    }

    /// A representative colour (first stop, front colour) for swatches.
    pub fn preview_color(&self) -> Option<Color> {
        match self {
            Fill::None => None,
            Fill::Solid(c) => Some(*c),
            Fill::Fountain(f) => Some(f.first_color()),
            Fill::Pattern(Pattern::TwoColor { front, .. }) => Some(*front),
            Fill::Pattern(Pattern::Bitmap { .. }) => Some(Color::Gray { v: 0.5 }),
            Fill::Pattern(Pattern::Vector { shapes, .. }) => Some(
                shapes
                    .iter()
                    .find_map(|s| s.fill.preview_color())
                    .unwrap_or(Color::Gray { v: 0.5 }),
            ),
            Fill::Texture(t) => Some(t.color_a),
            Fill::Mesh(m) => m.nodes.first().map(|n| n.color),
            Fill::Procedural(p) => Some(p.foreground),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// The preset arrowhead size in outline widths (the head-space unit; see
/// [`Arrowhead`]). Hairlines count as 0.25 mm and the size is at least 1 mm.
pub const HEAD_SIZE_PER_WIDTH: f64 = 4.0;

/// Arrowheads at the ends of open curves.
///
/// Presets are drawn by [`arrowhead_paths`]; a `Custom` head carries its own
/// outline in a normalised head space: the bounding box is centred on
/// y = 0, the tip (largest x) sits at x = 0 and the shape extends towards
/// negative x, i.e. back along the curve, exactly one unit tall
/// ([`Arrowhead::from_shape_path`]). Heads an importer reads with their own
/// placement and size ([`Arrowhead::from_placed_path`]) keep both instead:
/// the origin is the curve's end point and the head may reach past it. At
/// draw time the unit is the preset size ([`HEAD_SIZE_PER_WIDTH`] outline
/// widths, at least 1 mm), x follows the curve direction at the end point
/// and y its left normal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Arrowhead {
    #[default]
    None,
    Arrow,
    OpenArrow,
    Circle,
    Square,
    Bar,
    Diamond,
    /// User-drawn head (Object > Create > Arrowhead).
    Custom {
        path: crate::geometry::BezPath,
        name: String,
    },
}

impl Arrowhead {
    /// The built-in presets, in menu order. Custom heads are not listed.
    pub const ALL: [Arrowhead; 7] = [
        Arrowhead::None,
        Arrowhead::Arrow,
        Arrowhead::OpenArrow,
        Arrowhead::Circle,
        Arrowhead::Square,
        Arrowhead::Bar,
        Arrowhead::Diamond,
    ];

    pub fn name(&self) -> &str {
        match self {
            Arrowhead::None => "None",
            Arrowhead::Arrow => "Arrow",
            Arrowhead::OpenArrow => "Open arrow",
            Arrowhead::Circle => "Circle",
            Arrowhead::Square => "Square",
            Arrowhead::Bar => "Bar",
            Arrowhead::Diamond => "Diamond",
            Arrowhead::Custom { name, .. } => name,
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, Arrowhead::Custom { .. })
    }

    /// Build a custom head from any closed outline (page or local space,
    /// orientation: tip on the right, pointing towards +x). The outline is
    /// moved and scaled into head space (see the enum docs). An empty or
    /// degenerate outline (no height and no width) gives `Arrowhead::None`.
    pub fn from_shape_path(path: &crate::geometry::BezPath, name: impl Into<String>) -> Arrowhead {
        use crate::geometry::Shape as _;
        if path.elements().is_empty() {
            return Arrowhead::None;
        }
        let b = path.bounding_box();
        if !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
            return Arrowhead::None;
        }
        // A flat outline (a bar seen edge on) still gets a height from its
        // width so the head stays visible.
        let height = if b.height() > 1e-9 {
            b.height()
        } else if b.width() > 1e-9 {
            b.width()
        } else {
            return Arrowhead::None;
        };
        let scale = 1.0 / height;
        let center_y = (b.y0 + b.y1) / 2.0;
        let normalised =
            Affine::scale(scale) * Affine::translate((-b.x1, -center_y)) * path.clone();
        Arrowhead::Custom {
            path: normalised,
            name: name.into(),
        }
    }

    /// Build a custom head from an imported outline that is already placed
    /// the way the file draws it: the curve's end point at the origin, the
    /// curve coming in from -x, and `units_per_width` outline units making
    /// one outline width. The placement and size are kept as they are (no
    /// normalising), so the head is drawn as the file sizes it, whatever
    /// part of it lies ahead of or beside the end point. A head larger than
    /// 100 outline widths is scaled down to that. An empty or degenerate
    /// outline, or a unit that is not a finite positive number, gives
    /// `Arrowhead::None`.
    pub fn from_placed_path(
        path: &crate::geometry::BezPath,
        name: impl Into<String>,
        units_per_width: f64,
    ) -> Arrowhead {
        use crate::geometry::Shape as _;
        if path.elements().is_empty() || !(units_per_width.is_finite() && units_per_width > 0.0) {
            return Arrowhead::None;
        }
        let b = path.bounding_box();
        if !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
            return Arrowhead::None;
        }
        let extent = b.width().max(b.height()) / units_per_width;
        if extent <= 1e-9 {
            return Arrowhead::None;
        }
        // Head space: one unit is HEAD_SIZE_PER_WIDTH outline widths.
        let scale = 1.0 / (units_per_width * HEAD_SIZE_PER_WIDTH) * (100.0 / extent).min(1.0);
        Arrowhead::Custom {
            path: Affine::scale(scale) * path.clone(),
            name: name.into(),
        }
    }
}

/// How an arrowhead sits on its end of the line: its size, where it is
/// along the line, which way it faces and how far it is turned. The
/// Arrowhead editor sets one per end; untouched placements are not
/// written to files.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ArrowheadPlacement {
    /// Size as a percentage of the preset head size ([`HEAD_SIZE_PER_WIDTH`]
    /// outline widths, at least 1 mm): 100 is the preset size, 200 twice
    /// as long and tall. Kept in [`ArrowheadPlacement::SIZE_RANGE`].
    #[serde(default = "hundred", skip_serializing_if = "is_hundred")]
    pub size: f64,
    /// Offset along the line in millimetres: positive moves the head
    /// outwards, past the end point; negative back along the line.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f64,
    /// Mirror along the line: the head faces back along the line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror_h: bool,
    /// Mirror across the line: the head's two sides change places.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror_v: bool,
    /// Rotation about the tip in degrees, counter-clockwise positive on
    /// the page.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
}

fn hundred() -> f64 {
    100.0
}

fn is_hundred(v: &f64) -> bool {
    *v == 100.0
}

impl Default for ArrowheadPlacement {
    fn default() -> Self {
        ArrowheadPlacement {
            size: 100.0,
            offset: 0.0,
            mirror_h: false,
            mirror_v: false,
            rotation: 0.0,
        }
    }
}

impl ArrowheadPlacement {
    /// The range the size percentage is kept in.
    pub const SIZE_RANGE: std::ops::RangeInclusive<f64> = 10.0..=1000.0;

    /// Nothing set away from the defaults (the file need not hold it).
    pub fn is_default(&self) -> bool {
        *self == ArrowheadPlacement::default()
    }

    /// The size as a factor of the preset size: the percentage kept in
    /// [`Self::SIZE_RANGE`] over 100; a value that is not a number counts
    /// as 100 %.
    pub fn scale(&self) -> f64 {
        let pct = if self.size.is_finite() {
            self.size
                .clamp(*Self::SIZE_RANGE.start(), *Self::SIZE_RANGE.end())
        } else {
            100.0
        };
        pct / 100.0
    }

    /// The offset along the line, 0 when it is not a number.
    pub fn offset_mm(&self) -> f64 {
        if self.offset.is_finite() {
            self.offset
        } else {
            0.0
        }
    }

    /// The rotation in radians, 0 when it is not a number.
    pub fn rotation_rad(&self) -> f64 {
        if self.rotation.is_finite() {
            self.rotation.to_radians()
        } else {
            0.0
        }
    }

    /// The transform from the head's own frame (tip at the origin, the
    /// line's direction along +x, its left normal along +y, the preset
    /// head one unit) to the frame of the line's end at `tip` with
    /// direction `dir` (a unit vector pointing out of the line) and left
    /// normal `n`: the head is scaled by `size`, mirrored, turned about
    /// its tip and moved along the line by the offset.
    pub fn place(&self, tip: Point, dir: Vec2, n: Vec2) -> Affine {
        let tip = tip + dir * self.offset_mm();
        let mirror = Affine::scale_non_uniform(
            if self.mirror_h { -1.0 } else { 1.0 },
            if self.mirror_v { -1.0 } else { 1.0 },
        );
        Affine::new([dir.x, dir.y, n.x, n.y, tip.x, tip.y])
            * Affine::rotate(self.rotation_rad())
            * mirror
            * Affine::scale(self.scale())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Color,
    /// Width in millimetres. A "hairline" is 0.0762 mm (0.216 pt).
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    /// Dash pattern as alternating on/off lengths, in multiples of the width.
    /// Empty = solid.
    pub dash: Vec<f64>,
    /// When true, the outline scales with the object ("Scale with
    /// object"). When false, width stays fixed under transforms.
    pub scale_with_object: bool,
    /// Draw the outline behind the fill.
    pub behind_fill: bool,
    #[serde(default)]
    pub start_arrow: Arrowhead,
    #[serde(default)]
    pub end_arrow: Arrowhead,
    /// How the start head sits on the line (size, offset, mirror,
    /// rotation).
    #[serde(default, skip_serializing_if = "ArrowheadPlacement::is_default")]
    pub start_head: ArrowheadPlacement,
    /// How the end head sits on the line.
    #[serde(default, skip_serializing_if = "ArrowheadPlacement::is_default")]
    pub end_head: ArrowheadPlacement,
    /// Share attributes: the end takes the start's head and placement
    /// and follows it while this is on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub share_arrowheads: bool,
    /// Calligraphic nib stretch 0..1 and angle (1.0 = round nib).
    #[serde(default = "one")]
    pub stretch: f64,
    #[serde(default)]
    pub nib_angle: f64,
    /// A fill for the outline itself (a gradient, pattern or texture);
    /// None paints it with `color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Box<Fill>>,
    /// Width nodes of a variable-width outline, in order along the path.
    /// Empty: the outline has `width` everywhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widths: Vec<WidthNode>,
    /// Drawn as painterly brush marks along the path instead of a line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub painterly: Option<crate::painterly::Painterly>,
    /// Where a closed path's outline lies: outside the path, centred on
    /// it or inside it. Open paths are always centred.
    #[serde(default, skip_serializing_if = "OutlinePosition::is_center")]
    pub position: OutlinePosition,
    /// Mitre limit as an angle in degrees: mitred corners sharper than
    /// this are bevelled. Outlines saved before the field existed keep the
    /// limit they were drawn with (a length ratio of 4, about 29 degrees).
    #[serde(default = "legacy_miter_limit")]
    pub miter_limit: f64,
    /// Fit the dash pattern to each run between corners so that corners
    /// and line ends fall in the middle of a dash.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub align_dashes: bool,
}

/// Where an outline lies relative to the path of a closed object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutlinePosition {
    /// The whole width outside the path; the object looks larger.
    Outside,
    /// Half the width on each side of the path.
    #[default]
    Center,
    /// The whole width inside the path; the object keeps its size.
    Inside,
}

impl OutlinePosition {
    pub const ALL: [OutlinePosition; 3] = [
        OutlinePosition::Outside,
        OutlinePosition::Center,
        OutlinePosition::Inside,
    ];

    pub fn is_center(&self) -> bool {
        *self == OutlinePosition::Center
    }
}

fn legacy_miter_limit() -> f64 {
    Stroke::miter_angle_from_ratio(4.0)
}

/// A point of a variable-width outline: where it is along the path and
/// how far the outline reaches on each side of it there.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WidthNode {
    /// Place along the path, 0 (start) to 1 (end), by length over all
    /// subpaths.
    pub at: f64,
    /// Reach to the left of the path's direction, in millimetres.
    pub left: f64,
    /// Reach to the right, in millimetres.
    pub right: f64,
}

impl WidthNode {
    pub fn symmetric(at: f64, width: f64) -> Self {
        WidthNode {
            at,
            left: width / 2.0,
            right: width / 2.0,
        }
    }
}

/// The left and right reach of a variable outline at `t` (0 to 1) on an
/// open path: the path's ends keep half the base width unless a node
/// sits there, and the reach eases between neighbouring nodes.
pub fn width_at(base: f64, nodes: &[WidthNode], t: f64) -> (f64, f64) {
    width_around(base, nodes, t, false)
}

/// Where the reach of a lone width node on a `closed` loop falls back to
/// half the base width: the point half way round the loop from it. `None`
/// unless the path is a loop with exactly one node. The Variable Outline
/// tool sets this node down for real when a second node is added, so the
/// far side of the loop keeps its taper.
pub fn loop_anchor(base: f64, nodes: &[WidthNode], closed: bool) -> Option<WidthNode> {
    match nodes {
        [only] if closed && only.at.is_finite() => Some(WidthNode::symmetric(
            (only.at + 0.5).rem_euclid(1.0),
            base.max(0.0),
        )),
        _ => None,
    }
}

/// [`width_at`] on a path that is `closed` (one loop): the loop has no
/// ends, so the reach eases across its seam too, from the last node on
/// to the first as between any two neighbours, and a node can sit
/// anywhere round the loop (#1322). A lone node on a loop is its own
/// neighbour both ways round, so it eases to [`loop_anchor`] half way
/// round the loop and back, a taper as on an open path, instead of giving
/// the whole loop its reach. With `closed` false it is [`width_at`].
pub fn width_around(base: f64, nodes: &[WidthNode], t: f64, closed: bool) -> (f64, f64) {
    if let (true, [only], Some(anchor)) = (closed, nodes, loop_anchor(base, nodes, closed)) {
        // In path order, as the nodes are kept.
        let pair = if anchor.at < only.at {
            [anchor, *only]
        } else {
            [*only, anchor]
        };
        return width_around(base, &pair, t, true);
    }
    let half = (base / 2.0).max(0.0);
    let t = t.clamp(0.0, 1.0);
    // The knots the reach runs through, in path order: the nodes, and on
    // an open path its ends at half the base width unless a node sits
    // there; round a loop the nodes either side of the seam stand in
    // for the ones across it.
    let mut knots: Vec<WidthNode> = nodes
        .iter()
        .map(|n| WidthNode {
            at: n.at.clamp(0.0, 1.0),
            left: n.left.max(0.0),
            right: n.right.max(0.0),
        })
        .collect();
    knots.sort_by(|a, b| a.at.total_cmp(&b.at));
    let end = |at: f64| WidthNode {
        at,
        left: half,
        right: half,
    };
    if closed {
        let count = knots.len();
        if count == 0 {
            return (half, half);
        }
        let behind: Vec<WidthNode> = knots
            .iter()
            .rev()
            .take(2)
            .rev()
            .map(|k| WidthNode {
                at: k.at - 1.0,
                ..*k
            })
            .collect();
        let ahead: Vec<WidthNode> = knots
            .iter()
            .take(2)
            .map(|k| WidthNode {
                at: k.at + 1.0,
                ..*k
            })
            .collect();
        knots.splice(0..0, behind);
        knots.extend(ahead);
    } else {
        if knots.first().is_none_or(|k| k.at > 0.0) {
            knots.insert(0, end(0.0));
        }
        if knots.last().is_none_or(|k| k.at < 1.0) {
            knots.push(end(1.0));
        }
    }
    // The span `t` falls in: from the last knot at or before it.
    let i = knots
        .iter()
        .rposition(|k| k.at <= t)
        .unwrap_or(0)
        .min(knots.len().saturating_sub(2));
    let (Some(&a), Some(&b)) = (knots.get(i), knots.get(i + 1)) else {
        return knots.first().map_or((half, half), |k| (k.left, k.right));
    };
    let span = b.at - a.at;
    if span <= 1e-12 || t <= a.at {
        return (a.left, a.right);
    }
    // A monotone cubic through the knots (Fritsch and Carlson): the
    // reach climbs or falls through a run of nodes in one smooth sweep
    // and never overshoots them. Each span eased on its own (flat at
    // both its nodes) made a run of nodes, each a little wider than the
    // last, into a staircase: the more nodes were added to even an
    // edge, the more bumps it had (#2547). A node wider or narrower
    // than both its neighbours, and the ends of an open path, stay flat
    // there, so with one or two nodes the reach eases as before.
    let slope = |k: usize, side: fn(&WidthNode) -> f64| -> f64 {
        let (Some(p), Some(c), Some(n)) = (
            k.checked_sub(1).and_then(|j| knots.get(j)),
            knots.get(k),
            knots.get(k + 1),
        ) else {
            return 0.0;
        };
        let (h0, h1) = (c.at - p.at, n.at - c.at);
        if h0 <= 1e-12 || h1 <= 1e-12 {
            return 0.0;
        }
        let (d0, d1) = ((side(c) - side(p)) / h0, (side(n) - side(c)) / h1);
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let (w0, w1) = (2.0 * h1 + h0, h1 + 2.0 * h0);
        (w0 + w1) / (w0 / d0 + w1 / d1)
    };
    let u = ((t - a.at) / span).clamp(0.0, 1.0);
    let (h00, h10, h01, h11) = (
        (1.0 + 2.0 * u) * (1.0 - u) * (1.0 - u),
        u * (1.0 - u) * (1.0 - u),
        u * u * (3.0 - 2.0 * u),
        u * u * (u - 1.0),
    );
    let side = |f: fn(&WidthNode) -> f64| {
        let v = f(&a) * h00 + slope(i, f) * span * h10 + f(&b) * h01 + slope(i + 1, f) * span * h11;
        v.max(0.0)
    };
    (side(|k| k.left), side(|k| k.right))
}

fn one() -> f64 {
    1.0
}

impl Stroke {
    pub const HAIRLINE: f64 = 0.0762;
    /// Mitre limit of new outlines, degrees: a corner sharper than a
    /// right angle's half is bevelled, so a thick outline on lettering
    /// does not grow long spikes at its narrow turns (#894).
    pub const DEFAULT_MITER_LIMIT: f64 = 45.0;
    /// The range the mitre limit is kept in, degrees.
    pub const MITER_LIMIT_RANGE: std::ops::RangeInclusive<f64> = 1.0..=179.0;

    /// The mitre limit as the ratio of the mitre length to the outline
    /// width (what renderers, SVG and PDF take): `1 / sin(angle / 2)`.
    /// A value that is not a finite angle uses the default.
    pub fn miter_ratio(&self) -> f64 {
        let a = if self.miter_limit.is_finite() {
            self.miter_limit
        } else {
            Self::DEFAULT_MITER_LIMIT
        };
        let a = a.clamp(
            *Self::MITER_LIMIT_RANGE.start(),
            *Self::MITER_LIMIT_RANGE.end(),
        );
        1.0 / (a.to_radians() / 2.0).sin()
    }

    /// The mitre limit angle (degrees) for a ratio of mitre length to
    /// width (SVG `stroke-miterlimit`, PDF `M`): `2 asin(1 / ratio)`.
    /// Ratios below 1 or not finite give the default.
    pub fn miter_angle_from_ratio(ratio: f64) -> f64 {
        if !ratio.is_finite() || ratio < 1.0 {
            return Self::DEFAULT_MITER_LIMIT;
        }
        (2.0 * (1.0 / ratio).asin().to_degrees()).clamp(
            *Self::MITER_LIMIT_RANGE.start(),
            *Self::MITER_LIMIT_RANGE.end(),
        )
    }

    pub fn hairline(color: Color) -> Self {
        Stroke {
            color,
            width: Self::HAIRLINE,
            ..Default::default()
        }
    }

    /// Whether the outline is drawn as a brush stroke: a band of its own
    /// shape along the path (widths along it, or a calligraphic nib)
    /// rather than a line of one width.
    pub fn is_brush(&self) -> bool {
        !self.widths.is_empty() || (self.stretch < 0.999 && self.width > Self::HAIRLINE + 1e-9)
    }

    pub fn new(color: Color, width: f64) -> Self {
        Stroke {
            color,
            width,
            ..Default::default()
        }
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Stroke {
            color: Color::BLACK,
            width: Self::HAIRLINE,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            dash: Vec::new(),
            scale_with_object: false,
            behind_fill: false,
            start_arrow: Arrowhead::None,
            end_arrow: Arrowhead::None,
            start_head: ArrowheadPlacement::default(),
            end_head: ArrowheadPlacement::default(),
            share_arrowheads: false,
            stretch: 1.0,
            nib_angle: 0.0,
            fill: None,
            widths: Vec::new(),
            painterly: None,
            position: OutlinePosition::Center,
            miter_limit: Self::DEFAULT_MITER_LIMIT,
            align_dashes: false,
        }
    }
}

impl Stroke {
    /// The head drawn at the end of the line and its placement: the end's
    /// own, or the start's with Share attributes on.
    pub fn end_head_drawn(&self) -> (&Arrowhead, &ArrowheadPlacement) {
        if self.share_arrowheads {
            (&self.start_arrow, &self.start_head)
        } else {
            (&self.end_arrow, &self.end_head)
        }
    }

    /// Give the end the start's head and placement (Share attributes).
    pub fn share_start_head(&mut self) {
        self.end_arrow = self.start_arrow.clone();
        self.end_head = self.start_head;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run of width nodes, each wider than the last, widens in one
    /// smooth sweep: the reach keeps climbing through every node instead
    /// of levelling off at each, which made a staircase of bumps along
    /// the edge the more nodes were added (#2547). A lone node still
    /// eases in and out as before, and the reach never overshoots.
    #[test]
    fn a_run_of_width_nodes_widens_in_one_sweep() {
        let nodes: Vec<WidthNode> = [(0.2, 2.0), (0.4, 3.0), (0.6, 4.0), (0.8, 5.0)]
            .into_iter()
            .map(|(at, r)| WidthNode {
                at,
                left: r,
                right: r,
            })
            .collect();
        let left = |t: f64| width_at(2.0, &nodes, t).0;
        // At every node in the run the reach still climbs (5 mm over
        // the whole path, 0.6 of it): it levelled off there before.
        for at in [0.4, 0.6] {
            let slope = (left(at + 0.005) - left(at - 0.005)) / 0.01;
            assert!(slope > 3.0, "flat at {at}: {slope}");
        }
        // Through the nodes, rising all the way, and never past them.
        let mut last = left(0.2);
        for k in 1..=60 {
            let t = 0.2 + 0.6 * k as f64 / 60.0;
            let w = left(t);
            assert!(
                w > last - 1e-12 && w <= 5.0 + 1e-12,
                "{t}: {w} after {last}"
            );
            last = w;
        }
        for n in &nodes {
            assert!((left(n.at) - n.left).abs() < 1e-12);
        }
        // The widest node: flat there, its reach held within a
        // thousandth of a millimetre a thousandth of the path either side.
        for t in [0.799, 0.801] {
            assert!((left(t) - 5.0).abs() < 1e-3, "{t}: {}", left(t));
        }
        // A lone node: the same ease as before, half way to it at the
        // quarter.
        let one = [WidthNode::symmetric(0.5, 6.0)];
        assert!((width_at(2.0, &one, 0.25).0 - 2.0).abs() < 1e-12);
        assert!((width_at(2.0, &one, 0.125).0 - (1.0 + 2.0 * 0.15625)).abs() < 1e-12);
    }

    /// #1322: on a closed loop the reach eases across the seam from the
    /// last node to the first, the same at `t` 0 and 1, and one node
    /// alone gives the whole loop its reach; an open path still keeps
    /// half the base width at its ends.
    #[test]
    fn width_around_a_loop_eases_across_its_seam() {
        let two = [
            WidthNode::symmetric(0.2, 10.0),
            WidthNode::symmetric(0.8, 2.0),
        ];
        // Open: the ends keep half the base width (1 mm).
        assert_eq!(width_at(2.0, &two, 0.0), (1.0, 1.0));
        assert_eq!(width_at(2.0, &two, 1.0), (1.0, 1.0));
        // Closed: halfway from the 1 mm node to the 5 mm one, 3 mm on
        // both sides of the seam, and the same reach at 0 and at 1.
        let (l0, r0) = width_around(2.0, &two, 0.0, true);
        let (l1, r1) = width_around(2.0, &two, 1.0, true);
        assert!(
            (l0 - 3.0).abs() < 1e-9 && (r0 - 3.0).abs() < 1e-9,
            "{l0} {r0}"
        );
        assert!(
            (l1 - 3.0).abs() < 1e-9 && (r1 - 3.0).abs() < 1e-9,
            "{l1} {r1}"
        );
        // Just past the seam the reach is on its way to the 5 mm node,
        // without a step.
        let (l, _) = width_around(2.0, &two, 0.01, true);
        assert!(l > 3.0 && l < 3.2, "{l}");
        // Between the nodes it eases as on an open path.
        assert_eq!(width_around(2.0, &two, 0.5, true), width_at(2.0, &two, 0.5));
        // One node: its reach at the node, tapering both ways round to
        // half the base width half way round the loop (a live test of
        // 0.41: a loop with one node did not taper); none: the base
        // width.
        let one = [WidthNode::symmetric(0.9, 10.0)];
        assert_eq!(width_around(2.0, &one, 0.9, true), (5.0, 5.0));
        let (far, _) = width_around(2.0, &one, 0.4, true);
        assert!((far - 1.0).abs() < 1e-9, "{far}");
        // Across the seam as on either side of the node, the same both
        // ways round.
        let (a, _) = width_around(2.0, &one, 0.0, true);
        let (b, _) = width_around(2.0, &one, 1.0, true);
        let (c, _) = width_around(2.0, &one, 0.8, true);
        assert!((a - b).abs() < 1e-9 && (a - c).abs() < 1e-9, "{a} {b} {c}");
        assert!(a > 1.0 && a < 5.0, "{a}");
        let anchor = loop_anchor(2.0, &one, true).unwrap_or(WidthNode::symmetric(0.0, 0.0));
        assert!((anchor.at - 0.4).abs() < 1e-9, "{anchor:?}");
        assert_eq!((anchor.left, anchor.right), (1.0, 1.0));
        assert_eq!(loop_anchor(2.0, &one, false), None);
        assert_eq!(loop_anchor(2.0, &two, true), None);
        assert_eq!(width_around(2.0, &[], 0.5, true), (1.0, 1.0));
        assert_eq!(
            width_around(2.0, &two, 0.5, false),
            width_at(2.0, &two, 0.5)
        );
    }

    /// A mesh node's handles lie along the grid lines that leave it (a
    /// third of the way to the neighbour while straight); an edge node has
    /// none towards the outside; moving one bends its line (discussion
    /// #2073). The mean colour weighs the nodes by their opacity.
    #[test]
    fn mesh_handles_lie_on_the_grid_lines_and_bend_them() {
        let b = crate::geometry::Rect::new(0.0, 0.0, 30.0, 30.0);
        let mut m = Mesh::new(b, 1, 1, Color::WHITE);
        // Node 0 at (0, 0): right to node 1 (30, 0), down to node 2 (0, 30).
        assert_eq!(m.neighbour(0, MeshHandle::Right), Some(1));
        assert_eq!(m.neighbour(0, MeshHandle::Down), Some(2));
        assert_eq!(m.neighbour(0, MeshHandle::Left), None);
        assert_eq!(m.neighbour(0, MeshHandle::Up), None);
        assert_eq!(m.neighbour(9, MeshHandle::Up), None);
        assert_eq!(
            m.handle_point(0, MeshHandle::Right),
            Some(Point::new(10.0, 0.0))
        );
        assert_eq!(m.handle_point(0, MeshHandle::Up), None);
        assert!(!m.move_handle(0, MeshHandle::Up, Vec2::new(1.0, 1.0)));
        assert!(m.move_handle(0, MeshHandle::Right, Vec2::new(0.0, -6.0)));
        assert_eq!(
            m.handle_point(0, MeshHandle::Right),
            Some(Point::new(10.0, -6.0))
        );
        // The top line bends; the left one stays straight.
        assert!(m.is_curved());
        let top = m.edge(0, 0, true).expect("top line");
        assert_eq!(top.p1, Point::new(10.0, -6.0));
        let left = m.edge(0, 0, false).expect("left line");
        assert_eq!(left.p1, Point::new(0.0, 10.0));
        // Moving the node carries its handles along.
        m.nodes[0].translate(Vec2::new(2.0, 0.0));
        assert_eq!(
            m.handle_point(0, MeshHandle::Right),
            Some(Point::new(12.0, -6.0))
        );
        // One colour: that colour; else the mean, a transparent node
        // counting for nothing.
        assert_eq!(m.mean_color(), Some(Color::WHITE));
        m.nodes[0].color = Color::rgb8(0, 0, 0);
        m.nodes[1].color = Color::rgb8(0, 0, 0);
        assert_eq!(m.mean_color(), Some(Color::rgb8(128, 128, 128)));
        m.nodes[1].alpha = 0.0;
        assert_eq!(m.mean_color(), Some(Color::rgb8(170, 170, 170)));
    }

    /// A procedural fill written without its numbers (an older or a
    /// hand-made file) takes the defaults; one written whole reads back
    /// as it was.
    #[test]
    fn procedural_fill_has_serde_defaults() {
        let bare: Procedural =
            serde_json::from_str(r#"{"pattern":"waves","foreground":{"model":"gray","v":0.2}}"#)
                .unwrap();
        assert_eq!(bare.pattern, ProceduralPattern::Waves);
        assert_eq!(bare.params, ProceduralParams::default());
        assert_eq!(bare.background, None);
        assert_eq!(bare.seed, 0);
        let fill: Fill = serde_json::from_str(r#"{"fill":"procedural"}"#).unwrap();
        assert!(matches!(fill, Fill::Procedural(p) if p.foreground == Color::BLACK));
        let mut p = Procedural::new(ProceduralPattern::Spirals, Color::rgb8(10, 20, 30));
        p.params.turns = 7.5;
        p.seed = 9;
        let text = serde_json::to_string(&Fill::Procedural(p.clone())).unwrap();
        assert!(text.contains("\"fill\":\"procedural\""), "{text}");
        assert!(text.contains("\"pattern\":\"spirals\""), "{text}");
        let back: Fill = serde_json::from_str(&text).unwrap();
        assert_eq!(back, Fill::Procedural(p));
        // Every pattern has a name and uses the tile size first.
        for pattern in ProceduralPattern::ALL {
            assert!(!pattern.name().is_empty());
            assert_eq!(pattern.params().first(), Some(&ProceduralParam::Size));
        }
    }

    /// #804: a double-click inside a mesh puts a node exactly where it
    /// was, also in a cell pulled out of shape, its colour mixed from
    /// the cell's corners; next to a grid line only the line across it
    /// is added, and on a node nothing.
    #[test]
    fn a_double_click_puts_the_new_mesh_node_where_it_was() {
        let b = crate::geometry::Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut m = Mesh::new(b, 1, 2, Color::WHITE);
        // Pull the top-right node of the first cell out of shape and
        // colour the first cell's corners.
        m.nodes[4].pos = Point::new(26.0, 27.0);
        m.nodes[0].color = Color::rgb8(0, 0, 0);
        let p = Point::new(9.0, 13.0);
        let (r, c, u, v) = m.locate(p).expect("inside");
        assert_eq!((r, c), (0, 0));
        assert!(m.add_lines_at(p, 0.5));
        assert_eq!((m.rows, m.cols), (2, 3));
        let i = m.nearest(p).expect("node");
        assert!((m.nodes[i].pos - p).hypot() < 1e-6, "{:?}", m.nodes[i].pos);
        assert_eq!(m.row_col(i), (1, 1));
        // Its colour: the patch's colour there.
        let k = ((1.0 - u) * (1.0 - v) * 255.0).round();
        let red = m.nodes[i].color.to_rgb8()[0] as f64;
        assert!((red - (255.0 - k)).abs() <= 2.0, "{red} {k}");
        // Next to the new column, only a row; on a node, nothing.
        let below = m.node(0, 1).expect("node").pos;
        let on_column = m.nodes[i].pos.midpoint(below);
        let q = Point::new(on_column.x + 0.2, on_column.y);
        assert!(m.add_lines_at(q, 0.5));
        assert_eq!((m.rows, m.cols), (3, 3));
        assert!(!m.add_lines_at(p, 0.5));
        assert_eq!((m.rows, m.cols), (3, 3));
        assert!(m.locate(Point::new(-5.0, 5.0)).is_none());
        assert!(!m.add_lines_at(Point::new(-5.0, 5.0), 0.5));
    }

    /// A node's handles follow it when it moves or the mesh is
    /// transformed; a column added through a curved cell cuts the curve
    /// where the new node is, keeping the grid line's shape (#896).
    #[test]
    fn mesh_handles_move_with_their_nodes_and_cut_at_new_lines() {
        use crate::geometry::{Affine, ParamCurve, Vec2};
        let b = crate::geometry::Rect::new(0.0, 0.0, 30.0, 10.0);
        let mut m = Mesh::new(b, 1, 1, Color::WHITE);
        assert!(!m.is_curved());
        // The top line bows down to y = 7.5 in the middle.
        m.nodes[0].set_handle(MeshHandle::Right, Point::new(10.0, 10.0));
        m.nodes[1].set_handle(MeshHandle::Left, Point::new(20.0, 10.0));
        assert!(m.is_curved());
        let before = m.edge(0, 0, true).unwrap();
        assert!((before.eval(0.5).y - 7.5).abs() < 1e-9);
        // The other handles are straight thirds.
        let third = m.nodes[0].handle(MeshHandle::Down, m.nodes[2].pos);
        assert!(
            (third - Point::new(0.0, 10.0 / 3.0)).hypot() < 1e-9,
            "{third:?}"
        );
        // Moved and transformed, the handles come along.
        let mut moved = m.clone();
        moved.nodes[0].translate(Vec2::new(1.0, 2.0));
        assert_eq!(moved.nodes[0].handles.unwrap()[0], Point::new(11.0, 12.0));
        let scaled = m.transformed(Affine::scale(2.0));
        assert_eq!(scaled.nodes[1].handles.unwrap()[1], Point::new(40.0, 20.0));
        assert_eq!(scaled.nodes[1].pos, Point::new(60.0, 0.0));
        // A column through the middle: the curve is cut there and keeps
        // its shape; the new node sits on it.
        m.insert_column(0, 0.5);
        assert_eq!((m.rows, m.cols), (1, 2));
        let n = m.node(0, 1).unwrap();
        assert!((n.pos - before.eval(0.5)).hypot() < 1e-9, "{:?}", n.pos);
        let left = m.edge(0, 0, true).unwrap();
        let right = m.edge(0, 1, true).unwrap();
        assert!((left.eval(0.5) - before.eval(0.25)).hypot() < 1e-9);
        assert!((right.eval(0.5) - before.eval(0.75)).hypot() < 1e-9);
        // The straight bottom line stays straight.
        let bottom = m.edge(1, 0, true).unwrap();
        assert!((bottom.eval(0.5) - Point::new(7.5, 10.0)).hypot() < 1e-9);
        // The patch's corner and middle.
        let mid = m.patch_point(0, 0, 0.5, 0.5).unwrap();
        assert!(
            (mid.x - 7.5).abs() < 1e-9 && mid.y > 5.0 && mid.y < 10.0,
            "{mid:?}"
        );
        assert_eq!(m.patch_point(0, 0, 0.0, 0.0).unwrap(), Point::new(0.0, 0.0));
        // Locating goes by the curved patch: a point under the bowed top
        // line is in the cell, a point just above it (inside the straight
        // box) is not; the place found maps back to the point.
        let (r, c, u, v) = m.locate(Point::new(7.5, 9.0)).expect("inside");
        assert_eq!((r, c), (0, 0));
        let back = m.patch_point(r as u32, c as u32, u, v).unwrap();
        assert!((back - Point::new(7.5, 9.0)).hypot() < 1e-6, "{back:?}");
        assert!(m.locate(Point::new(15.0, 6.5)).is_none());
        // Removing the added column stretches the handles back: the top
        // line is the curve it was.
        let i = m.nearest(n.pos).unwrap();
        assert!(m.remove_lines_through(i));
        assert_eq!((m.rows, m.cols), (1, 1));
        let restored = m.edge(0, 0, true).unwrap();
        for k in 0..=10 {
            let t = k as f64 / 10.0;
            assert!((restored.eval(t) - before.eval(t)).hypot() < 1e-6, "{t}");
        }
    }

    #[test]
    fn meshes_regrid_and_lose_lines_keeping_their_colours() {
        let b = crate::geometry::Rect::new(0.0, 0.0, 20.0, 20.0);
        let mut m = Mesh::new(b, 1, 1, Color::WHITE);
        // Left column red, right column blue.
        for (i, n) in m.nodes.iter_mut().enumerate() {
            n.color = if i % 2 == 0 {
                Color::rgb8(255, 0, 0)
            } else {
                Color::rgb8(0, 0, 255)
            };
        }
        let g = m.regrid(2, 4);
        assert_eq!((g.rows, g.cols, g.nodes.len()), (2, 4, 15));
        // The middle column is the mix of red and blue; the ends keep theirs.
        let mid = g.node(1, 2).expect("node").color.to_rgb8();
        assert!(
            (mid[0] as i32 - 128).abs() <= 2 && (mid[2] as i32 - 128).abs() <= 2,
            "{mid:?}"
        );
        assert_eq!(g.node(0, 0).expect("node").color.to_rgb8(), [255, 0, 0]);
        assert_eq!(g.node(2, 4).expect("node").color.to_rgb8(), [0, 0, 255]);
        assert_eq!(g.node(2, 4).expect("node").pos, Point::new(20.0, 20.0));
        // The middle node's lines go; an edge node's cannot.
        let mut h = g.clone();
        let centre = 7; // row 1, column 2
        assert_eq!(h.row_col(centre), (1, 2));
        assert!(h.remove_lines_through(centre));
        assert_eq!((h.rows, h.cols, h.nodes.len()), (1, 3, 8));
        assert!(!h.remove_lines_through(0));
        assert!(!h.remove_lines_through(99));
        // Sampling between nodes mixes them.
        let s = m.sample(0.5, 0.5).expect("sample");
        assert_eq!(s.pos, Point::new(10.0, 10.0));
    }

    #[test]
    fn fountain_interpolates_stops() {
        let f = Fountain {
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
        };
        assert_eq!(f.color_at(0.25).to_rgb8(), [128, 0, 0]);
        assert_eq!(f.color_at(0.5).to_rgb8(), [255, 0, 0]);
        assert_eq!(f.color_at(1.5).to_rgb8(), [255, 255, 255]);
    }

    /// Free nodes: a fill placed with its default nodes looks as the
    /// angle and offset place it, for every kind; a skew handle tilts
    /// the bands; the nodes follow the box (#330).
    #[test]
    fn midpoints_and_node_opacity_shape_the_blend() {
        let mut f = Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        let grey = |c: Color| c.to_rgb8()[0];
        // In the middle by default.
        assert!((126..=129).contains(&grey(f.color_at(0.5))));
        // The midpoint at a quarter: half way there at 0.25, lighter at 0.5.
        f.stops[0].mid = 0.25;
        assert!(f.has_midpoints());
        assert!(
            (122..=134).contains(&grey(f.color_at(0.25))),
            "{:?}",
            f.color_at(0.25)
        );
        assert!(grey(f.color_at(0.5)) > 160);
        // The plain run of stops gives the same colours.
        let plain = f.resolved_stops();
        assert!(plain.len() > 2);
        let at = sample_stops(&plain, 0.25);
        assert!((i32::from(grey(at)) - i32::from(grey(f.color_at(0.25)))).abs() <= 3);
        // Opacity runs from node to node like the colour.
        f.stops[0].mid = 0.5;
        f.stops[1].opacity = 0.0;
        assert!(f.has_opacity());
        assert!((f.opacity_at(0.5) - 0.5).abs() < 0.02);
        assert_eq!(f.resolved_stops().last().map(|s| s.opacity), Some(0.0));
        // Saved without the defaults; read back from files that lack them.
        let plain_json = serde_json::to_string(&Stop::new(0.0, Color::BLACK)).unwrap();
        assert!(!plain_json.contains("mid") && !plain_json.contains("opacity"));
        let back: Stop = serde_json::from_str(&plain_json).unwrap();
        assert_eq!(back, Stop::new(0.0, Color::BLACK));
    }

    #[test]
    fn free_nodes_place_the_fill_as_the_frame_says() {
        let b = Rect::new(10.0, 20.0, 110.0, 70.0);
        for kind in [
            FountainKind::Linear,
            FountainKind::Radial,
            FountainKind::Conical,
            FountainKind::Square,
        ] {
            let mut f = Fountain::two(kind, Color::BLACK, Color::WHITE, 30.0);
            f.offset = Point::new(0.2, -0.1);
            let h = f.default_handles(b);
            let mut g = f.clone();
            g.handles = Some(h);
            for p in [
                Point::new(30.0, 30.0),
                Point::new(60.0, 45.0),
                Point::new(100.0, 65.0),
            ] {
                let t = g.param_at(b, p).expect("a frame");
                let want = legacy_param(&f, b, p);
                assert!((t - want).abs() < 1e-9, "{kind:?} at {p:?}: {t} vs {want}");
            }
        }
        // Linear from (20, 40) to (60, 40): the start is 0, the end 1.
        let mut f = Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        let rel = |p| Fountain::page_to_rel(b, p);
        f.set_handles(
            b,
            FountainHandles {
                start: rel(Point::new(20.0, 40.0)),
                end: rel(Point::new(60.0, 40.0)),
                skew: rel(Point::new(20.0, 60.0)),
            },
        );
        assert!((f.param_at(b, Point::new(20.0, 50.0)).unwrap()).abs() < 1e-9);
        assert!((f.param_at(b, Point::new(40.0, 40.0)).unwrap() - 0.5).abs() < 1e-9);
        assert!((f.angle).abs() < 1e-9);
        // A skew handle leaning right: the bands lean with it.
        f.handles = Some(FountainHandles {
            skew: rel(Point::new(40.0, 60.0)),
            ..f.handles.unwrap()
        });
        assert!((f.param_at(b, Point::new(40.0, 60.0)).unwrap()).abs() < 1e-9);
        // The nodes follow the box: the same fill on a box twice as wide
        // has its start twice as far from the centre.
        let wide = Rect::new(10.0, 20.0, 210.0, 70.0);
        let s = Fountain::rel_to_page(wide, f.handles.unwrap().start);
        assert!((s.x - (110.0 - 2.0 * 40.0)).abs() < 1e-9, "{s:?}");
        // A flat frame places nothing.
        f.handles = Some(FountainHandles {
            start: Point::ZERO,
            end: Point::ZERO,
            skew: Point::ZERO,
        });
        assert!(f.frame(b).is_none());
    }

    /// The parameter as the renderer works it out without free nodes.
    fn legacy_param(f: &Fountain, b: Rect, p: Point) -> f64 {
        let a = f.angle.to_radians();
        let c = b.center();
        match f.kind {
            FountainKind::Linear => {
                let half = (b.width() * a.cos().abs() + b.height() * a.sin().abs()) / 2.0;
                let s = Point::new(c.x - half * a.cos(), c.y - half * a.sin());
                let d = Vec2::new(a.cos(), a.sin()) * (2.0 * half);
                (p - s).dot(d) / d.hypot2()
            }
            FountainKind::Radial => {
                let cc = Point::new(
                    c.x + f.offset.x * b.width() / 2.0,
                    c.y + f.offset.y * b.height() / 2.0,
                );
                (p - cc).hypot() / (b.width().max(b.height()) / 2.0 * std::f64::consts::SQRT_2)
            }
            _ => {
                let u = (p.x - b.x0) / b.width();
                let v = (p.y - b.y0) / b.height();
                let dx = u - 0.5 - f.offset.x / 2.0;
                let dy = v - 0.5 - f.offset.y / 2.0;
                if f.kind == FountainKind::Conical {
                    let a = (dy.atan2(dx) - a).rem_euclid(std::f64::consts::TAU)
                        / std::f64::consts::TAU;
                    if a < 0.5 {
                        a * 2.0
                    } else {
                        2.0 - a * 2.0
                    }
                } else {
                    dx.abs().max(dy.abs()) * 2.0
                }
            }
        }
    }

    fn black_white() -> Fountain {
        Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0)
    }

    fn grey(c: Color) -> u8 {
        c.to_rgb8()[0]
    }

    #[test]
    fn fountain_repeats_mirrors_and_inverts() {
        // Sawtooth, three cycles: dark again at the start of each.
        let mut f = black_white();
        f.repeat = FountainRepeat::Repeat;
        f.repeats = 3;
        assert_eq!(grey(f.color_at(0.0)), 0);
        assert!(grey(f.color_at(0.33)) > 240);
        assert!(grey(f.color_at(0.34)) < 15);
        assert_eq!(grey(f.color_at(1.0)), 255);
        // Triangular: there and back.
        f.repeat = FountainRepeat::Mirror;
        f.repeats = 2;
        assert_eq!(grey(f.color_at(0.5)), 255);
        assert_eq!(grey(f.color_at(1.0)), 0);
        assert_eq!(grey(f.color_at(0.25)), grey(f.color_at(0.75)));
        // No repeat: the count is ignored.
        f.repeat = FountainRepeat::None;
        assert_eq!(f.cycles(), 1);
        assert_eq!(grey(f.color_at(0.5)), 128);
        // Bilinear: first colour at both ends, last in the middle.
        let mut b = black_white();
        b.bilinear = true;
        assert_eq!(grey(b.color_at(0.0)), 0);
        assert_eq!(grey(b.color_at(0.5)), 255);
        assert_eq!(grey(b.color_at(1.0)), 0);
        // Inverted: the order reversed.
        let mut i = black_white();
        i.invert = true;
        assert_eq!(grey(i.color_at(0.0)), 255);
        assert_eq!(grey(i.color_at(1.0)), 0);
        // Out of range and not a number stay inside the stops.
        assert_eq!(grey(i.color_at(f64::NAN)), 255);
        let mut huge = black_white();
        huge.repeat = FountainRepeat::Repeat;
        huge.repeats = u32::MAX;
        assert_eq!(huge.cycles(), Fountain::MAX_REPEATS);
    }

    #[test]
    fn resolved_stops_draw_the_same_colours_in_one_run() {
        let three = Fountain {
            stops: vec![
                Stop::new(0.0, Color::rgb8(255, 0, 0)),
                Stop::new(0.4, Color::rgb8(0, 255, 0)),
                Stop::new(1.0, Color::rgb8(0, 0, 255)),
            ],
            ..Default::default()
        };
        for repeat in [
            FountainRepeat::None,
            FountainRepeat::Repeat,
            FountainRepeat::Mirror,
        ] {
            for repeats in [1, 2, 3] {
                for bilinear in [false, true] {
                    for invert in [false, true] {
                        for edge_pad in [0.0, 0.2] {
                            let f = Fountain {
                                repeat,
                                repeats,
                                bilinear,
                                invert,
                                edge_pad,
                                ..three.clone()
                            };
                            let r = f.resolved_stops();
                            assert_eq!(r[0].pos, 0.0);
                            assert!((r[r.len() - 1].pos - 1.0).abs() < 1e-12);
                            assert!(r.windows(2).all(|w| w[0].pos <= w[1].pos));
                            for k in 0..=200 {
                                // Away from the sawtooth's jumps.
                                let t = (k as f64 + 0.37) / 201.0;
                                let a = f.color_at(t).to_rgb8();
                                let b = sample_stops(&r, t).to_rgb8();
                                for c in 0..3 {
                                    assert!(
                                        (a[c] as i32 - b[c] as i32).abs() <= 2,
                                        "{f:?} at {t}: {a:?} {b:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        // A sawtooth's jump is a hard edge: two stops at one place.
        let mut saw = black_white();
        saw.repeat = FountainRepeat::Repeat;
        saw.repeats = 2;
        let r = saw.resolved_stops();
        assert_eq!(r.len(), 4);
        assert!((r[1].pos - 0.5).abs() < 1e-12 && (r[2].pos - 0.5).abs() < 1e-12);
        assert_eq!((grey(r[1].color), grey(r[2].color)), (255, 0));
        // A plain fountain keeps its stops.
        assert_eq!(three.resolved_stops(), three.stops);
    }

    #[test]
    fn plain_versions_carry_the_options_in_their_stops() {
        let f = black_white();
        assert_eq!(f.to_plain(64), f);
        let mut i = black_white();
        i.invert = true;
        i.edge_pad = 0.1;
        let p = i.to_plain(64);
        assert!(p.runs_once() && p.edge_pad == 0.0);
        assert_eq!(grey(p.first_color()), 255);
        // Shaped becomes square, the inside (1) at the centre (0).
        let s = Fountain::two(FountainKind::Shaped, Color::BLACK, Color::WHITE, 0.0).to_plain(64);
        assert_eq!(s.kind, FountainKind::Square);
        assert_eq!(grey(s.first_color()), 255);
        assert_eq!(grey(s.last_color()), 0);
        // Too many stops: sampled down evenly.
        let mut many = black_white();
        many.repeat = FountainRepeat::Repeat;
        many.repeats = 50;
        assert_eq!(many.to_plain(16).stops.len(), 16);
    }

    #[test]
    fn fountains_without_the_options_read_and_write_as_before() {
        // As files written before the options had it.
        let old = r#"{"kind":"radial","stops":[{"pos":0.0,"color":{"model":"rgb","r":0.0,"g":0.0,"b":0.0}},{"pos":1.0,"color":{"model":"gray","v":1.0}}],"angle":0.0,"offset":{"x":0.0,"y":0.0},"edge_pad":0.1}"#;
        let f: Fountain = serde_json::from_str(old).expect("old fountain reads");
        assert_eq!(f.kind, FountainKind::Radial);
        assert!(f.runs_once());
        assert_eq!((f.repeat, f.repeats), (FountainRepeat::None, 1));
        assert!(!f.has_blend_options() && !f.stepped());
        assert_eq!(f.steps, Fountain::SMOOTH_STEPS);
        // Defaults are not written, so files stay as they were.
        let json = serde_json::to_string(&black_white()).expect("json");
        for field in [
            "repeat",
            "bilinear",
            "invert",
            "blend_direction",
            "steps",
            "smooth",
            "acceleration",
        ] {
            assert!(!json.contains(field), "{json}");
        }
        let mut g = black_white();
        g.kind = FountainKind::Shaped;
        g.repeat = FountainRepeat::Mirror;
        g.repeats = 4;
        g.bilinear = true;
        g.invert = true;
        g.blend_direction = ColorBlend::CounterClockwise;
        g.steps = 12;
        g.smooth = true;
        g.acceleration = -40;
        let json = serde_json::to_string(&g).expect("json");
        assert!(
            json.contains("\"shaped\"")
                && json.contains("\"mirror\"")
                && json.contains("\"CounterClockwise\"")
                && json.contains("\"steps\":12")
                && json.contains("\"acceleration\":-40"),
            "{json}"
        );
        let back: Fountain = serde_json::from_str(&json).expect("reads");
        assert_eq!(back, g);
    }

    /// A 4-step black to white fill is four greys with hard edges between
    /// them, the end colours first and last; the resolved stops carry the
    /// bands as a pair of stops each (#1208).
    #[test]
    fn stepped_fountains_draw_as_bands() {
        let mut f = black_white();
        f.steps = 4;
        assert!(f.stepped() && f.has_blend_options());
        let mut greys: Vec<u8> = (0..100)
            .map(|i| grey(f.color_at((i as f64 + 0.5) / 100.0)))
            .collect();
        greys.dedup();
        assert_eq!(greys, vec![0, 85, 170, 255]);
        // Band `i` runs from i/n to (i+1)/n and has the colour at i/(n-1).
        assert_eq!(grey(f.color_at(0.1)), 0);
        assert_eq!(grey(f.color_at(0.3)), 85);
        assert_eq!(grey(f.color_at(0.6)), 170);
        assert_eq!(grey(f.color_at(0.9)), 255);
        assert_eq!(grey(f.color_at(1.0)), 255);
        // Two stops per band, the edges hard.
        let r = f.resolved_stops();
        assert_eq!(r.len(), 8);
        assert!((r[1].pos - 0.25).abs() < 1e-12 && (r[2].pos - 0.25).abs() < 1e-12);
        assert_eq!((grey(r[1].color), grey(r[2].color)), (0, 85));
        let same = |f: &Fountain, r: &[Stop]| {
            for k in 0..=200 {
                let t = (k as f64 + 0.37) / 201.0;
                assert_eq!(
                    f.color_at(t).to_rgb8(),
                    sample_stops(r, t).to_rgb8(),
                    "{f:?} at {t}"
                );
            }
        };
        same(&f, &r);
        // Smooth from 256 up; never fewer than two bands.
        f.steps = Fountain::SMOOTH_STEPS;
        assert!(!f.stepped() && !f.has_blend_options());
        assert_eq!(grey(f.color_at(0.5)), 128);
        f.steps = 0;
        assert_eq!(f.band_count(), 2);
        assert_eq!(grey(f.color_at(0.49)), 0);
        assert_eq!(grey(f.color_at(0.51)), 255);
        // The bands sit inside each cycle of a repeat, and inverted.
        let mut s = black_white();
        s.steps = 2;
        s.repeat = FountainRepeat::Repeat;
        s.repeats = 2;
        s.invert = true;
        for t in [0.1, 0.6] {
            assert_eq!(grey(s.color_at(t)), 255);
        }
        for t in [0.4, 0.9] {
            assert_eq!(grey(s.color_at(t)), 0);
        }
        let r = s.resolved_stops();
        assert_eq!(r.len(), 8, "{r:?}");
        same(&s, &r);
        // The plain version for formats that know only stops.
        let p = s.to_plain(64);
        assert!(!p.has_blend_options() && p.runs_once());
        assert_eq!(p.stops, r);
    }

    /// Red to blue clockwise (hue increasing) passes through green, the
    /// other way through magenta; straight it is the RGB mix (#1208).
    #[test]
    fn hue_blends_go_round_the_wheel() {
        let red = Color::rgb8(255, 0, 0);
        let blue = Color::rgb8(0, 0, 255);
        let mut f = Fountain::two(FountainKind::Linear, red, blue, 0.0);
        assert_eq!(f.color_at(0.5).to_rgb8(), [128, 0, 128]);
        f.blend_direction = ColorBlend::Clockwise;
        assert!(f.has_blend_options());
        assert_eq!(f.color_at(0.5).to_rgb8(), [0, 255, 0]);
        assert_eq!(f.color_at(0.25).to_rgb8(), [255, 255, 0]);
        assert_eq!(f.color_at(0.0).to_rgb8(), [255, 0, 0]);
        assert_eq!(f.color_at(1.0).to_rgb8(), [0, 0, 255]);
        // Between the sampled pieces the colour stays close to the wheel.
        let [r, g, b] = f.color_at(0.125).to_rgb8();
        assert!(
            r == 255 && (g as i32 - 128).abs() <= 3 && b == 0,
            "{r} {g} {b}"
        );
        f.blend_direction = ColorBlend::CounterClockwise;
        assert_eq!(f.color_at(0.5).to_rgb8(), [255, 0, 255]);
        // The resolved stops carry the path for shaders and exports.
        let r = f.resolved_stops();
        assert_eq!(r.len(), Fountain::MID_STEPS + 1);
        assert_eq!(sample_stops(&r, 0.5).to_rgb8(), [255, 0, 255]);
        // Greys have no hue: black to white stays grey either way.
        let mut bw = black_white();
        bw.blend_direction = ColorBlend::Clockwise;
        assert_eq!(bw.color_at(0.5).to_rgb8(), [128, 128, 128]);
    }

    /// The smooth transition is a smoothstep over each blend: at a quarter
    /// of the way the colour is `3/16 - 2/64` of the way (#1208).
    #[test]
    fn smooth_transition_eases_each_blend() {
        assert!((smoothstep(0.25) - 0.15625).abs() < 1e-12);
        assert_eq!(smoothstep(0.5), 0.5);
        let mut f = black_white();
        f.smooth = true;
        assert!(f.has_blend_options());
        assert_eq!(grey(f.color_at(0.25)), (255.0 * 0.15625f64).round() as u8);
        assert_eq!(grey(f.color_at(0.5)), 128);
        assert_eq!(grey(f.color_at(0.75)), (255.0 * 0.84375f64).round() as u8);
        assert_eq!(grey(f.color_at(0.0)), 0);
        assert_eq!(grey(f.color_at(1.0)), 255);
        // Flat at the ends: darker than straight early on.
        assert!(grey(f.color_at(0.0625)) < grey(black_white().color_at(0.0625)));
        // With a midpoint off the middle the ease keeps it: half way at
        // the midpoint.
        f.stops[0].mid = 0.25;
        assert_eq!(grey(f.color_at(0.25)), 128);
    }

    /// The acceleration skews every blend's midpoint, on top of the
    /// stop's own: +100 pulls it to the start, -100 to the end (#1208).
    #[test]
    fn acceleration_skews_every_midpoint() {
        assert_eq!(Fountain::skewed_mid(0.5, 1.0), 0.0);
        assert_eq!(Fountain::skewed_mid(0.5, -1.0), 1.0);
        assert!((Fountain::skewed_mid(0.5, 0.5) - 0.25).abs() < 1e-12);
        assert!((Fountain::skewed_mid(0.2, -0.5) - 0.6).abs() < 1e-12);
        assert_eq!(Fountain::skewed_mid(0.3, 0.0), 0.3);
        let mut f = black_white();
        f.acceleration = 100;
        assert!(f.has_blend_options());
        // Past half way almost at once.
        assert!(
            grey(f.color_at(0.0625)) > 128,
            "{}",
            grey(f.color_at(0.0625))
        );
        assert_eq!(grey(f.color_at(0.0)), 0);
        f.acceleration = -100;
        assert!(
            grey(f.color_at(0.9375)) < 128,
            "{}",
            grey(f.color_at(0.9375))
        );
        assert_eq!(grey(f.color_at(1.0)), 255);
        // +50 halves each midpoint: half way at a quarter.
        f.acceleration = 50;
        assert_eq!(grey(f.color_at(0.25)), 128);
        // Combined with the stop's own midpoint (0.25 halved: 0.125).
        f.stops[0].mid = 0.25;
        assert_eq!(grey(f.color_at(0.125)), 128);
        // Out of range values count as the ends of the range.
        f.acceleration = 1000;
        assert_eq!(f.blend_mid(&f.stops[0]), 0.0);
        // The plain version carries it in the stops.
        f.acceleration = 50;
        let p = f.to_plain(64);
        assert!(!p.has_blend_options() && !p.has_midpoints());
        assert_eq!(grey(sample_stops(&p.stops, 0.125)), 128);
    }

    #[test]
    fn pattern_tiles_have_both_colours() {
        for t in PatternTile::ALL {
            let mut front = 0;
            for i in 0..20 {
                for j in 0..20 {
                    if t.front(i as f64 / 20.0, j as f64 / 20.0) {
                        front += 1;
                    }
                }
            }
            assert!(front > 0 && front < 400, "{t:?}");
        }
    }

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> crate::document::Shape {
        let mut s = crate::document::Shape::new(
            crate::ShapeId(1),
            crate::document::ShapeKind::Rect {
                rect: Rect::new(x0, y0, x1, y1),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = None;
        s
    }

    #[test]
    fn vector_pattern_from_shapes_moves_bounds_to_origin() {
        let shapes = vec![
            square(30.0, 40.0, 35.0, 45.0),
            square(35.0, 45.0, 40.0, 50.0),
        ];
        let Pattern::Vector { shapes, tile, .. } = Pattern::vector_from_shapes(&shapes) else {
            panic!("expected a vector pattern");
        };
        assert_eq!(tile, Size::new(10.0, 10.0));
        assert_eq!(shapes.len(), 2);
        let b = shapes[0].bounds();
        assert!((b.x0).abs() < 1e-9 && (b.y0).abs() < 1e-9, "{b:?}");
        assert!(
            (b.x1 - 5.0).abs() < 1e-9 && (b.y1 - 5.0).abs() < 1e-9,
            "{b:?}"
        );
        let b = shapes[1].bounds();
        assert!(
            (b.x1 - 10.0).abs() < 1e-9 && (b.y1 - 10.0).abs() < 1e-9,
            "{b:?}"
        );
    }

    #[test]
    fn vector_pattern_from_nothing_has_a_minimal_tile() {
        let Pattern::Vector { shapes, tile, .. } = Pattern::vector_from_shapes(&[]) else {
            panic!("expected a vector pattern");
        };
        assert!(shapes.is_empty());
        assert!(tile.width > 0.0 && tile.height > 0.0);
    }

    #[test]
    fn vector_pattern_round_trips_through_json() {
        let fill = Fill::Pattern(Pattern::vector_from_shapes(&[square(0.0, 0.0, 5.0, 5.0)]));
        let json = serde_json::to_string(&fill).unwrap();
        assert!(json.contains("\"pattern\":\"vector\""));
        let back: Fill = serde_json::from_str(&json).unwrap();
        assert_eq!(back, fill);
    }

    #[test]
    fn older_pattern_fills_still_load() {
        let json = r#"{"fill":"pattern","pattern":"twocolor","tile":"dots","front":{"model":"gray","v":0.0},"back":{"model":"gray","v":1.0},"size_mm":5.0}"#;
        let back: Fill = serde_json::from_str(json).unwrap();
        assert!(matches!(
            back,
            Fill::Pattern(Pattern::TwoColor {
                tile: PatternTile::Dots,
                ..
            })
        ));
        // A vector tile without shapes is still a valid (empty) tile.
        let json = r#"{"fill":"pattern","pattern":"vector","tile":{"width":4.0,"height":2.0}}"#;
        let back: Fill = serde_json::from_str(json).unwrap();
        assert!(
            matches!(back, Fill::Pattern(Pattern::Vector { ref shapes, .. }) if shapes.is_empty())
        );
    }

    fn checker(size_mm: f64) -> Pattern {
        Pattern::TwoColor {
            tile: PatternTile::Checker,
            front: Color::BLACK,
            back: Color::WHITE,
            size_mm,
            placement: PatternPlacement::default(),
        }
    }

    /// An untouched placement is left out of the file, so files written
    /// before placements read the same and stay byte for byte the same.
    #[test]
    fn default_placement_is_not_written_and_a_set_one_round_trips() {
        let fill = Fill::Pattern(checker(5.0));
        let json = serde_json::to_string(&fill).unwrap();
        assert!(!json.contains("placement"), "{json}");
        let back: Fill = serde_json::from_str(&json).unwrap();
        assert_eq!(back, fill);

        let placed = PatternPlacement {
            mirror_horizontal: true,
            offset_x: 2.5,
            rotation: 30.0,
            skew: -10.0,
            shift_percent: 50.0,
            shift: TileShift::Column,
            size: Some(Size::new(4.0, 6.0)),
            transform_with_object: true,
            ..PatternPlacement::default()
        };
        let fill = Fill::Pattern(checker(5.0).with_placement(placed));
        let json = serde_json::to_string(&fill).unwrap();
        assert!(json.contains("\"placement\""), "{json}");
        // Only the values set are written.
        assert!(json.contains("\"mirror_horizontal\":true") && !json.contains("mirror_vertical"));
        assert!(json.contains("\"shift\":\"column\"") && !json.contains("offset_y"));
        let back: Fill = serde_json::from_str(&json).unwrap();
        assert_eq!(back, fill);
    }

    #[test]
    fn tile_size_follows_the_kind_and_the_placement() {
        let mut p = checker(5.0);
        assert_eq!(p.tile_size(), Size::new(5.0, 5.0));
        p.set_tile_size(Size::new(8.0, 8.0));
        assert_eq!(p.tile_size(), Size::new(8.0, 8.0));
        assert!(p.placement().size.is_none(), "a square tile needs no size");
        p.set_tile_size(Size::new(8.0, 4.0));
        assert_eq!(p.tile_size(), Size::new(8.0, 4.0));
        assert_eq!(p.placement().size, Some(Size::new(8.0, 4.0)));

        let mut v = Pattern::vector_from_shapes(&[square(0.0, 0.0, 5.0, 5.0)]);
        v.set_tile_size(Size::new(10.0, 20.0));
        let Pattern::Vector {
            shapes,
            tile,
            placement,
        } = &v
        else {
            panic!("expected a vector pattern");
        };
        assert_eq!(*tile, Size::new(10.0, 20.0));
        assert!(placement.size.is_none());
        let b = shapes[0].bounds();
        assert!(
            (b.x1 - 10.0).abs() < 1e-9 && (b.y1 - 20.0).abs() < 1e-9,
            "{b:?}"
        );
    }

    #[test]
    fn default_placement_is_one_upright_tile_on_the_origin() {
        let p = PatternPlacement::default();
        let tile = Size::new(10.0, 5.0);
        assert!(!p.needs_cell());
        assert_eq!(p.cell(tile).as_coeffs(), [10.0, 0.0, 0.0, 5.0, 0.0, 0.0]);
        assert_eq!(p.tile_copies(tile), vec![Affine::IDENTITY]);
        assert_eq!(
            p.transform(tile).as_coeffs(),
            [10.0, 0.0, 0.0, 5.0, 0.0, 0.0]
        );
        assert_eq!(p.tile_uv(Point::new(12.5, 1.25), tile), (0.25, 0.25));
    }

    #[test]
    fn mirrored_tiles_flip_every_second_one() {
        let p = PatternPlacement {
            mirror_horizontal: true,
            mirror_vertical: true,
            ..PatternPlacement::default()
        };
        let tile = Size::new(10.0, 10.0);
        assert_eq!(p.cell_tiles(), (2, 2));
        assert_eq!(p.cell(tile).as_coeffs(), [20.0, 0.0, 0.0, 20.0, 0.0, 0.0]);
        let copies = p.tile_copies(tile);
        assert_eq!(copies.len(), 4);
        // The second column shows the tile flipped: its left edge is the
        // tile's right edge.
        let flipped = copies[1] * Point::new(0.0, 2.0);
        assert!(
            (flipped.x - 20.0).abs() < 1e-9 && (flipped.y - 2.0).abs() < 1e-9,
            "{flipped:?}"
        );
        // Sampling agrees: a point a quarter into the second column reads
        // the tile three quarters along.
        let (u, v) = p.tile_uv(Point::new(12.5, 2.5), tile);
        assert!(
            (u - 0.75).abs() < 1e-9 && (v - 0.25).abs() < 1e-9,
            "{u} {v}"
        );
        let (u, v) = p.tile_uv(Point::new(2.5, 12.5), tile);
        assert!(
            (u - 0.25).abs() < 1e-9 && (v - 0.75).abs() < 1e-9,
            "{u} {v}"
        );
    }

    #[test]
    fn a_row_offset_shifts_every_row_by_that_part_of_a_tile() {
        let p = PatternPlacement {
            shift_percent: 50.0,
            ..PatternPlacement::default()
        };
        let tile = Size::new(10.0, 10.0);
        assert!(p.needs_cell());
        // The block leans: its top edge starts half a tile along.
        assert_eq!(p.cell(tile).as_coeffs(), [10.0, 0.0, 5.0, 10.0, 0.0, 0.0]);
        // The second row's tile starts at x = 5: a point at x = 6 in that
        // row is a tenth into a tile.
        let (u, v) = p.tile_uv(Point::new(6.0, 15.0), tile);
        assert!((u - 0.1).abs() < 1e-9 && (v - 0.5).abs() < 1e-9, "{u} {v}");
        // The first row is not shifted.
        let (u, _) = p.tile_uv(Point::new(6.0, 5.0), tile);
        assert!((u - 0.6).abs() < 1e-9, "{u}");
        // Copies at both sides cover the leaning edges.
        assert_eq!(p.tile_copies(tile).len(), 4);

        let c = PatternPlacement {
            shift: TileShift::Column,
            ..p
        };
        assert_eq!(c.cell(tile).as_coeffs(), [10.0, 5.0, 0.0, 10.0, 0.0, 0.0]);
        let (u, v) = c.tile_uv(Point::new(15.0, 6.0), tile);
        assert!((u - 0.5).abs() < 1e-9 && (v - 0.1).abs() < 1e-9, "{u} {v}");
    }

    #[test]
    fn placement_moves_turns_and_skews_the_tiles() {
        let p = PatternPlacement {
            offset_x: 3.0,
            offset_y: 4.0,
            rotation: 90.0,
            ..PatternPlacement::default()
        };
        // The tile's bottom-right corner (10, 0) turned a quarter turn
        // counter-clockwise lands above the offset point.
        let q = p.transform(Size::new(10.0, 10.0)) * Point::new(1.0, 0.0);
        assert!(
            (q.x - 3.0).abs() < 1e-9 && (q.y - 14.0).abs() < 1e-9,
            "{q:?}"
        );
        let s = PatternPlacement {
            skew: 45.0,
            ..PatternPlacement::default()
        };
        // The top-left corner of a skewed tile leans one tile height right.
        let q = s.transform(Size::new(10.0, 10.0)) * Point::new(0.0, 1.0);
        assert!(
            (q.x - 10.0).abs() < 1e-9 && (q.y - 10.0).abs() < 1e-9,
            "{q:?}"
        );
    }

    #[test]
    fn sanitized_placement_has_usable_values() {
        let p = PatternPlacement {
            offset_x: f64::NAN,
            skew: 400.0,
            shift_percent: -20.0,
            size: Some(Size::new(0.0, 5.0)),
            ..PatternPlacement::default()
        }
        .sanitized();
        assert_eq!(p.offset_x, 0.0);
        assert_eq!(p.skew, 89.0);
        assert_eq!(p.shift_percent, 0.0);
        assert!(p.size.is_none());
    }
}

/// The outline of a preset head in the head's own frame: the tip at the
/// origin, the line's direction along +x (the body extends towards -x),
/// its left normal along +y, `size` the preset head size in millimetres
/// and `w` the outline width the thin parts are drawn with. A custom
/// head's own outline is scaled to `size`. `None` for no head.
fn head_outline(kind: &Arrowhead, size: f64, w: f64) -> Option<crate::geometry::BezPath> {
    use crate::geometry::{BezPath, Shape as _};
    let mut p = BezPath::new();
    match kind {
        Arrowhead::None => return None,
        Arrowhead::Custom { path: outline, .. } => {
            // Head space (unit tall, tip at the origin, body towards -x)
            // scaled to the preset size.
            p = Affine::scale(size) * outline.clone();
        }
        Arrowhead::Arrow => {
            p.move_to((0.0, 0.0));
            p.line_to((-size, size * 0.4));
            p.line_to((-size * 0.7, 0.0));
            p.line_to((-size, -size * 0.4));
            p.close_path();
        }
        Arrowhead::OpenArrow => {
            let t = w.max(0.2);
            p.move_to((0.0, 0.0));
            p.line_to((-size, size * 0.45));
            p.line_to((-size - t, size * 0.45 - t));
            p.line_to((-t * 1.4, 0.0));
            p.line_to((-size - t, -size * 0.45 + t));
            p.line_to((-size, -size * 0.45));
            p.close_path();
        }
        Arrowhead::Circle => {
            let r = size * 0.35;
            p = crate::geometry::Circle::new((-r, 0.0), r).to_path(0.01);
        }
        Arrowhead::Square => {
            let h = size * 0.35;
            let cx = -h;
            p.move_to((cx + h, h));
            p.line_to((cx - h, h));
            p.line_to((cx - h, -h));
            p.line_to((cx + h, -h));
            p.close_path();
        }
        Arrowhead::Bar => {
            let t = w.max(0.2);
            p.move_to((0.0, size * 0.5));
            p.line_to((-t, size * 0.5));
            p.line_to((-t, -size * 0.5));
            p.line_to((0.0, -size * 0.5));
            p.close_path();
        }
        Arrowhead::Diamond => {
            let h = size * 0.5;
            p.move_to((0.0, 0.0));
            p.line_to((-h, h * 0.6));
            p.line_to((-h * 2.0, 0.0));
            p.line_to((-h, -h * 0.6));
            p.close_path();
        }
    }
    Some(p)
}

/// The preset head size in millimetres for an outline `width`:
/// [`HEAD_SIZE_PER_WIDTH`] widths, hairlines counting as 0.25 mm, at least
/// 1 mm. The second value is the width the thin parts of a head are
/// drawn with.
pub fn head_size_for_width(width: f64) -> (f64, f64) {
    let w = if width <= Stroke::HAIRLINE + 1e-9 {
        0.25
    } else {
        width
    };
    ((w * HEAD_SIZE_PER_WIDTH).max(1.0), w)
}

/// Arrowhead outlines (page space) for an open path's ends, sized by the
/// outline width (hairlines use 0.25 mm) and each end's placement (size,
/// offset, mirror, rotation; the end takes the start's head and
/// placement with Share attributes on). Filled with the outline colour.
pub fn arrowhead_paths(
    path: &crate::geometry::BezPath,
    stroke: &Stroke,
) -> Vec<crate::geometry::BezPath> {
    use crate::geometry::{BezPath, PathEl, Vec2};
    let (size, w) = head_size_for_width(stroke.width);
    let mut pts: Vec<Point> = Vec::new();
    let mut closed = false;
    kurbo::flatten(path.elements().iter().copied(), 0.05, &mut |el| match el {
        PathEl::MoveTo(p) => {
            if pts.is_empty() {
                pts.push(p)
            }
        }
        PathEl::LineTo(p) => pts.push(p),
        PathEl::ClosePath => closed = true,
        _ => {}
    });
    if closed || pts.len() < 2 {
        return Vec::new();
    }
    // A head in its own frame, placed at `tip` with the line's outward
    // direction `dir`: x along the direction, y along its left normal.
    let head =
        |tip: Point, dir: Vec2, kind: &Arrowhead, at: &ArrowheadPlacement| -> Option<BezPath> {
            let dir = dir.normalize();
            if !dir.x.is_finite() {
                return None;
            }
            let n = Vec2::new(-dir.y, dir.x);
            let outline = head_outline(kind, size, w)?;
            Some(at.place(tip, dir, n) * outline)
        };
    let mut out = Vec::new();
    let n = pts.len();
    if let Some(p) = head(
        pts[0],
        pts[0] - pts[1.min(n - 1)],
        &stroke.start_arrow,
        &stroke.start_head,
    ) {
        out.push(p);
    }
    let (end_kind, end_at) = stroke.end_head_drawn();
    if let Some(p) = head(pts[n - 1], pts[n - 1] - pts[n - 2], end_kind, end_at) {
        out.push(p);
    }
    out
}

#[cfg(test)]
mod arrowhead_tests {
    use super::*;
    use crate::geometry::{BezPath, Shape as _};

    /// A triangle pointing right: tip at (30, 15), base from (10, 5) to (10, 25).
    fn triangle() -> BezPath {
        let mut p = BezPath::new();
        p.move_to((10.0, 5.0));
        p.line_to((30.0, 15.0));
        p.line_to((10.0, 25.0));
        p.close_path();
        p
    }

    #[test]
    fn custom_head_is_normalised_to_head_space() {
        let head = Arrowhead::from_shape_path(&triangle(), "Tri");
        let Arrowhead::Custom { path, name } = &head else {
            panic!("expected a custom head");
        };
        assert_eq!(name, "Tri");
        assert!(head.is_custom());
        assert_eq!(head.name(), "Tri");
        let b = path.bounding_box();
        assert!((b.height() - 1.0).abs() < 1e-9, "{b:?}");
        assert!(
            (b.y0 + 0.5).abs() < 1e-9 && (b.y1 - 0.5).abs() < 1e-9,
            "{b:?}"
        );
        assert!(b.x1.abs() < 1e-9, "tip must be at x = 0: {b:?}");
        assert!((b.x0 + 1.0).abs() < 1e-9, "20 wide over 20 tall: {b:?}");
    }

    #[test]
    fn placed_head_keeps_its_size_and_place() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((50.0, 0.0));
        // 20 wide and 20 tall, tip at (10, 0): with 10 units per outline
        // width it is two widths tall and reaches one width past the end
        // point.
        let mut placed = BezPath::new();
        placed.move_to((-10.0, -10.0));
        placed.line_to((10.0, 0.0));
        placed.line_to((-10.0, 10.0));
        placed.close_path();
        let head = Arrowhead::from_placed_path(&placed, "Tri", 10.0);
        assert_eq!(head.name(), "Tri");
        for width in [1.0, 0.5, 2.0] {
            let stroke = Stroke {
                end_arrow: head.clone(),
                ..Stroke::new(Color::BLACK, width)
            };
            let heads = arrowhead_paths(&line, &stroke);
            assert_eq!(heads.len(), 1);
            let b = heads[0].bounding_box();
            assert!((b.height() - 2.0 * width).abs() < 1e-9, "{width}: {b:?}");
            assert!((b.x1 - (50.0 + width)).abs() < 1e-9, "{width}: {b:?}");
            assert!((b.y0 + width).abs() < 1e-9, "{width}: {b:?}");
        }
        // Huge heads are capped at 100 outline widths.
        let Arrowhead::Custom { path, .. } = Arrowhead::from_placed_path(&triangle(), "Tri", 1e-6)
        else {
            panic!("expected a custom head");
        };
        assert!((path.bounding_box().height() - 100.0 / HEAD_SIZE_PER_WIDTH).abs() < 1e-9);
        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Arrowhead::from_placed_path(&triangle(), "Tri", bad),
                Arrowhead::None
            );
        }
        assert_eq!(
            Arrowhead::from_placed_path(&BezPath::new(), "empty", 5.0),
            Arrowhead::None
        );
    }

    #[test]
    fn degenerate_outlines_give_no_head() {
        assert_eq!(
            Arrowhead::from_shape_path(&BezPath::new(), "empty"),
            Arrowhead::None
        );
        let mut dot = BezPath::new();
        dot.move_to((3.0, 3.0));
        dot.line_to((3.0, 3.0));
        dot.close_path();
        assert_eq!(Arrowhead::from_shape_path(&dot, "dot"), Arrowhead::None);
        // A flat bar keeps a height taken from its width.
        let mut bar = BezPath::new();
        bar.move_to((0.0, 1.0));
        bar.line_to((4.0, 1.0));
        bar.close_path();
        assert!(Arrowhead::from_shape_path(&bar, "bar").is_custom());
    }

    #[test]
    fn presets_are_not_custom_and_all_lists_only_presets() {
        for a in Arrowhead::ALL.iter() {
            assert!(!a.is_custom(), "{a:?}");
        }
        assert_eq!(Arrowhead::ALL.len(), 7);
        assert_eq!(Arrowhead::Arrow.name(), "Arrow");
    }

    #[test]
    fn custom_head_round_trips_through_json_and_presets_stay_strings() {
        let stroke = Stroke {
            start_arrow: Arrowhead::Diamond,
            end_arrow: Arrowhead::from_shape_path(&triangle(), "Tri"),
            ..Stroke::new(Color::BLACK, 1.0)
        };
        let json = serde_json::to_string(&stroke).unwrap();
        assert!(json.contains("\"start_arrow\":\"diamond\""));
        assert!(json.contains("\"end_arrow\":{\"custom\":"));
        let back: Stroke = serde_json::from_str(&json).unwrap();
        assert_eq!(back, stroke);
        // Files written before custom heads existed still load.
        let old = r#"{"color":{"model":"gray","v":0.0},"width":1.0,"cap":"butt","join":"miter","dash":[],"scale_with_object":false,"behind_fill":false,"start_arrow":"none","end_arrow":"arrow","stretch":1.0,"nib_angle":0.0}"#;
        let back: Stroke = serde_json::from_str(old).unwrap();
        assert_eq!(back.end_arrow, Arrowhead::Arrow);
        // ... and keep the mitre limit and position they were drawn with.
        assert!((back.miter_ratio() - 4.0).abs() < 1e-9);
        assert_eq!(back.position, OutlinePosition::Center);
        assert!(!back.align_dashes);
    }

    #[test]
    fn position_mitre_limit_and_dash_alignment_round_trip() {
        let stroke = Stroke {
            position: OutlinePosition::Inside,
            miter_limit: 12.5,
            align_dashes: true,
            dash: vec![4.0, 2.0],
            ..Stroke::new(Color::BLACK, 1.0)
        };
        let json = serde_json::to_string(&stroke).unwrap();
        assert!(json.contains("\"position\":\"inside\""), "{json}");
        let back: Stroke = serde_json::from_str(&json).unwrap();
        assert_eq!(back, stroke);
        // A centred outline without alignment does not write the fields.
        let plain = serde_json::to_string(&Stroke::new(Color::BLACK, 1.0)).unwrap();
        assert!(!plain.contains("position") && !plain.contains("align_dashes"));
    }

    #[test]
    fn mitre_limit_angles_and_ratios_convert_both_ways() {
        let s = Stroke::new(Color::BLACK, 1.0);
        assert_eq!(s.miter_limit, 45.0);
        // 1 / sin(22.5 degrees)
        assert!(
            (s.miter_ratio() - 2.6131).abs() < 1e-3,
            "{}",
            s.miter_ratio()
        );
        for r in [1.5, 4.0, 10.0] {
            let a = Stroke::miter_angle_from_ratio(r);
            let back = Stroke {
                miter_limit: a,
                ..s.clone()
            };
            assert!((back.miter_ratio() - r).abs() < 1e-9, "{r}");
        }
        for bad in [f64::NAN, 0.5, f64::INFINITY] {
            assert_eq!(Stroke::miter_angle_from_ratio(bad), 45.0);
        }
        let odd = Stroke {
            miter_limit: f64::NAN,
            ..s
        };
        assert!(odd.miter_ratio().is_finite());
    }

    #[test]
    fn custom_head_is_placed_at_the_line_end_pointing_forward() {
        // A horizontal line from (0, 0) to (50, 0) with a custom end head.
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((50.0, 0.0));
        let stroke = Stroke {
            end_arrow: Arrowhead::from_shape_path(&triangle(), "Tri"),
            ..Stroke::new(Color::BLACK, 1.0)
        };
        let heads = arrowhead_paths(&line, &stroke);
        assert_eq!(heads.len(), 1);
        let b = heads[0].bounding_box();
        // Size is four widths = 4 mm: the head is 4 mm tall, centred on the
        // line, with its tip at the end point and its body behind it.
        assert!((b.x1 - 50.0).abs() < 1e-6, "{b:?}");
        assert!((b.x0 - 46.0).abs() < 1e-6, "{b:?}");
        assert!(
            (b.y0 + 2.0).abs() < 1e-6 && (b.y1 - 2.0).abs() < 1e-6,
            "{b:?}"
        );

        // The same head at the start points the other way.
        let stroke = Stroke {
            start_arrow: stroke.end_arrow.clone(),
            end_arrow: Arrowhead::None,
            ..Stroke::new(Color::BLACK, 1.0)
        };
        let heads = arrowhead_paths(&line, &stroke);
        assert_eq!(heads.len(), 1);
        let b = heads[0].bounding_box();
        assert!(b.x0.abs() < 1e-6 && (b.x1 - 4.0).abs() < 1e-6, "{b:?}");
    }

    fn line() -> BezPath {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((50.0, 0.0));
        line
    }

    fn end_head_bounds(stroke: &Stroke) -> Rect {
        let heads = arrowhead_paths(&line(), stroke);
        assert_eq!(heads.len(), 1, "{heads:?}");
        heads[0].bounding_box()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// #1210: the Arrowhead editor's settings move the head geometry.
    /// A 1 mm line's preset arrow is 4 mm long, 3.2 mm tall, tip at the
    /// end point: at 200 % it is twice as long and tall; an offset moves
    /// it along the line; mirrored along the line it faces back; a
    /// rotation turns it about its tip, counter-clockwise positive.
    #[test]
    fn head_size_offset_mirror_and_rotation_place_the_head() {
        let base = Stroke {
            end_arrow: Arrowhead::Arrow,
            ..Stroke::new(Color::BLACK, 1.0)
        };
        let b = end_head_bounds(&base);
        assert!(near(b.x0, 46.0) && near(b.x1, 50.0), "{b:?}");
        assert!(near(b.y0, -1.6) && near(b.y1, 1.6), "{b:?}");

        // 200 %: twice as long and tall, the tip where it was.
        let mut twice = base.clone();
        twice.end_head.size = 200.0;
        let b = end_head_bounds(&twice);
        assert!(near(b.x0, 42.0) && near(b.x1, 50.0), "{b:?}");
        assert!(near(b.y0, -3.2) && near(b.y1, 3.2), "{b:?}");

        // The size stays in 10 to 1000 %; a size that is not a number is 100 %.
        let mut huge = base.clone();
        huge.end_head.size = 5000.0;
        assert!(near(end_head_bounds(&huge).x0, 10.0));
        let mut nan = base.clone();
        nan.end_head.size = f64::NAN;
        assert!(near(end_head_bounds(&nan).x0, 46.0));

        // Offset: 3 mm out past the end, -3 mm back along the line.
        let mut out = base.clone();
        out.end_head.offset = 3.0;
        let b = end_head_bounds(&out);
        assert!(near(b.x0, 49.0) && near(b.x1, 53.0), "{b:?}");
        out.end_head.offset = -3.0;
        let b = end_head_bounds(&out);
        assert!(near(b.x0, 43.0) && near(b.x1, 47.0), "{b:?}");

        // Mirrored along the line: the head faces back along it, its tip
        // still at the end point.
        let mut back = base.clone();
        back.end_head.mirror_h = true;
        let b = end_head_bounds(&back);
        assert!(near(b.x0, 50.0) && near(b.x1, 54.0), "{b:?}");

        // Turned a quarter counter-clockwise: the body points down (-y).
        let mut turned = base.clone();
        turned.end_head.rotation = 90.0;
        let b = end_head_bounds(&turned);
        assert!(near(b.x0, 48.4) && near(b.x1, 51.6), "{b:?}");
        assert!(near(b.y0, -4.0) && near(b.y1, 0.0), "{b:?}");

        // The same settings on the start head, in the start's frame: the
        // offset moves it out past the start.
        let mut start = Stroke {
            start_arrow: Arrowhead::Arrow,
            ..Stroke::new(Color::BLACK, 1.0)
        };
        start.start_head.offset = 2.0;
        start.start_head.size = 50.0;
        let heads = arrowhead_paths(&line(), &start);
        let b = heads[0].bounding_box();
        assert!(near(b.x0, -2.0) && near(b.x1, 0.0), "{b:?}");
        assert!(near(b.y0, -0.8) && near(b.y1, 0.8), "{b:?}");
    }

    /// A head mirrored across the line is the mirror image of the plain
    /// one: an asymmetric custom head (one side only) changes sides.
    #[test]
    fn a_vertically_mirrored_head_is_the_mirror_image() {
        // A half arrow: tip at (20, 10), body above the line only.
        let mut half = BezPath::new();
        half.move_to((0.0, 10.0));
        half.line_to((20.0, 10.0));
        half.line_to((0.0, 20.0));
        half.close_path();
        // Head space is normalised to one unit tall over the whole box,
        // centred on y = 0, so the plain head lies on both sides; the
        // mirrored one must be its exact mirror image.
        let base = Stroke {
            end_arrow: Arrowhead::from_shape_path(&half, "Half"),
            ..Stroke::new(Color::BLACK, 1.0)
        };
        let plain = arrowhead_paths(&line(), &base);
        let mut mirrored = base.clone();
        mirrored.end_head.mirror_v = true;
        let flipped = arrowhead_paths(&line(), &mirrored);
        assert_eq!(plain.len(), 1);
        assert_eq!(flipped.len(), 1);
        let mirror = Affine::scale_non_uniform(1.0, -1.0);
        let want = mirror * plain[0].clone();
        let got = &flipped[0];
        let a: Vec<_> = want.elements().to_vec();
        let b: Vec<_> = got.elements().to_vec();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            match (x, y) {
                (crate::geometry::PathEl::MoveTo(p), crate::geometry::PathEl::MoveTo(q))
                | (crate::geometry::PathEl::LineTo(p), crate::geometry::PathEl::LineTo(q)) => {
                    assert!((*p - *q).hypot() < 1e-9, "{p:?} vs {q:?}");
                }
                (crate::geometry::PathEl::ClosePath, crate::geometry::PathEl::ClosePath) => {}
                other => panic!("{other:?}"),
            }
        }
        // Mirrored along and across: the point on the line's far side.
        let plain_b = plain[0].bounding_box();
        assert!(
            near(plain_b.x1, 50.0) && near(plain_b.x0, 42.0),
            "{plain_b:?}"
        );
    }

    /// Share attributes: the end is drawn with the start's head and its
    /// placement, whatever the end's own fields hold, and
    /// `share_start_head` copies them into the end's fields.
    #[test]
    fn share_attributes_gives_the_end_the_start_head() {
        let mut stroke = Stroke {
            start_arrow: Arrowhead::Diamond,
            end_arrow: Arrowhead::None,
            share_arrowheads: true,
            ..Stroke::new(Color::BLACK, 1.0)
        };
        stroke.start_head.size = 200.0;
        stroke.start_head.offset = 1.0;
        let heads = arrowhead_paths(&line(), &stroke);
        assert_eq!(heads.len(), 2, "both ends drawn");
        let s = heads[0].bounding_box();
        let e = heads[1].bounding_box();
        // A diamond is one size long: 8 mm at 200 %, its tip 1 mm out past
        // each end and its body back along the line.
        assert!(near(s.x0, -1.0) && near(s.x1, 7.0), "{s:?}");
        assert!(near(e.x0, 43.0) && near(e.x1, 51.0), "{e:?}");
        assert!(near(s.height(), e.height()));
        assert_eq!(
            stroke.end_head_drawn(),
            (&Arrowhead::Diamond, &stroke.start_head)
        );
        // Off: the end's own (none) is drawn again.
        stroke.share_arrowheads = false;
        assert_eq!(arrowhead_paths(&line(), &stroke).len(), 1);
        stroke.share_start_head();
        assert_eq!(stroke.end_arrow, Arrowhead::Diamond);
        assert_eq!(stroke.end_head, stroke.start_head);
        assert_eq!(arrowhead_paths(&line(), &stroke).len(), 2);
    }

    /// Placements and Share attributes round trip through JSON; untouched
    /// ones are not written, so files stay as they were, and files
    /// without them load with the defaults.
    #[test]
    fn head_placements_round_trip_and_defaults_are_not_written() {
        let plain = serde_json::to_string(&Stroke::new(Color::BLACK, 1.0)).unwrap();
        assert!(
            !plain.contains("start_head")
                && !plain.contains("end_head")
                && !plain.contains("share_arrowheads"),
            "{plain}"
        );
        let mut stroke = Stroke {
            start_arrow: Arrowhead::Arrow,
            share_arrowheads: true,
            ..Stroke::new(Color::BLACK, 1.0)
        };
        stroke.start_head = ArrowheadPlacement {
            size: 150.0,
            offset: -2.5,
            mirror_h: true,
            mirror_v: false,
            rotation: 30.0,
        };
        stroke.end_head.mirror_v = true;
        let json = serde_json::to_string(&stroke).unwrap();
        assert!(json.contains("\"start_head\":{\"size\":150.0,\"offset\":-2.5,\"mirror_h\":true,\"rotation\":30.0}"), "{json}");
        assert!(json.contains("\"end_head\":{\"mirror_v\":true}"), "{json}");
        assert!(json.contains("\"share_arrowheads\":true"), "{json}");
        let back: Stroke = serde_json::from_str(&json).unwrap();
        assert_eq!(back, stroke);
        let old: Stroke = serde_json::from_str(&plain).unwrap();
        assert!(old.start_head.is_default() && old.end_head.is_default());
        assert!(!old.share_arrowheads);
        assert_eq!(ArrowheadPlacement::default().scale(), 1.0);
    }
}
