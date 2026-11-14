//! Extrusions turned in space, lit by several lights and bevelled.
//!
//! The body is a prism: the object's outline at depth 0 and a copy at
//! depth `L` (the length of the offset for parallel bodies, the depth
//! share of the distance from the outline's middle to the vanishing
//! point for the others). Back bodies run away from the viewer (z = -u),
//! front bodies toward the viewer (z = +u). A bevel cuts the near cap's
//! edge: the near cap shrinks by the bevel width and a band slopes from
//! it to the outline over the same depth.
//!
//! A parallel body runs in space along its offset across and as far in
//! depth (a point at depth `u` sits at `p + vector * u / L`), and is seen
//! straight on, so the offset is what shows. The other bodies are seen
//! from the vanishing point: a point at depth share `t` (its depth `u`
//! over `L`, whatever the turn) goes to `q + (V - q) * s` with `s = t d / (t d + 1 -
//! d)` (small: a perspective that puts the copy where the flat body has
//! it) or `q + (q - V) * d t` (big), `q` its place across. The prism
//! turns about the middle of the body by `rotation` (degrees about x,
//! then y, then z). Untouched, this is the flat body.
//!
//! A face shows when its outward side faces the viewer: the projected
//! face, ordered with its outward side up, runs counterclockwise. Faces
//! are drawn farthest first (by their middle's z, then as the flat body
//! orders them). Light: the first light keeps the flat formula with its
//! elevation, `0.55 + 0.45 (k (n.l + 1) / 2 + 1 - k)`; every further
//! light adds `0.3 k max(0, n.l)`; the result is kept within 0.2 to 1.

use crate::document::{Shape, ShapeKind};
use crate::extrude::{Body, Coloring, ExtrudeKind, Faces};
use crate::geometry::{Affine, BezPath, Point, Vec2};
use crate::style::{lerp_color, Fill};
use crate::Color;
use serde::{Deserialize, Serialize};

/// A light on an extrusion: where it comes from and how strong it is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Light {
    /// Degrees, 0 = from the right, counterclockwise.
    pub angle: f64,
    /// Degrees above the drawing's plane, toward the viewer (0 to 90).
    pub elevation: f64,
    /// 0 to 100.
    pub intensity: f64,
}

/// The extrusion's turn in space, its first light's elevation and the
/// lights after the first.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Solid {
    /// Degrees about x (the top tips away), y (the right side turns away)
    /// and z (counterclockwise), applied in that order.
    #[serde(default)]
    pub rotation: [f64; 3],
    /// The first light's elevation, degrees.
    #[serde(default)]
    pub elevation: f64,
    /// Further lights (at most [`MAX_EXTRA_LIGHTS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<Light>,
}

/// Lights after the first.
pub const MAX_EXTRA_LIGHTS: usize = 2;

impl Solid {
    /// Nothing turned, the first light in the plane, no further light.
    pub fn is_plain(&self) -> bool {
        self.rotation.iter().all(|a| *a == 0.0) && self.elevation == 0.0 && self.lights.is_empty()
    }

    pub fn turned(&self) -> bool {
        self.rotation.iter().any(|a| a.is_finite() && *a != 0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct V3 {
    x: f64,
    y: f64,
    z: f64,
}

impl V3 {
    fn new(x: f64, y: f64, z: f64) -> Self {
        V3 { x, y, z }
    }
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn cross(self, o: V3) -> V3 {
        V3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn unit(self) -> V3 {
        let l = self.dot(self).sqrt();
        if l > 1e-12 {
            V3::new(self.x / l, self.y / l, self.z / l)
        } else {
            V3::new(0.0, 0.0, 1.0)
        }
    }
}

/// Rotation matrix for degrees about x, then y, then z.
fn rotation(r: [f64; 3]) -> [[f64; 3]; 3] {
    let f = |a: f64| if a.is_finite() { a.to_radians() } else { 0.0 };
    let (ax, ay, az) = (f(r[0]), f(r[1]), f(r[2]));
    let rx = [
        [1.0, 0.0, 0.0],
        [0.0, ax.cos(), -ax.sin()],
        [0.0, ax.sin(), ax.cos()],
    ];
    let ry = [
        [ay.cos(), 0.0, ay.sin()],
        [0.0, 1.0, 0.0],
        [-ay.sin(), 0.0, ay.cos()],
    ];
    let rz = [
        [az.cos(), -az.sin(), 0.0],
        [az.sin(), az.cos(), 0.0],
        [0.0, 0.0, 1.0],
    ];
    let mul = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| {
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        m
    };
    // x first, so it is the rightmost.
    mul(rz, mul(ry, rx))
}

fn apply(m: &[[f64; 3]; 3], v: V3) -> V3 {
    V3::new(
        m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    )
}

fn area2(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum()
}

fn scale(c: Color, k: f64) -> Color {
    let [r, g, b] = c.to_rgb8();
    Color::rgb8(
        (r as f64 * k).round().clamp(0.0, 255.0) as u8,
        (g as f64 * k).round().clamp(0.0, 255.0) as u8,
        (b as f64 * k).round().clamp(0.0, 255.0) as u8,
    )
}

/// The loop moved inward by `b`, each corner along its bisector (at most
/// `3 b`). Loops are oriented with the inside on the left.
fn inset(pts: &[Point], b: f64) -> Vec<Point> {
    let n = pts.len();
    if n < 3 || b <= 0.0 {
        return pts.to_vec();
    }
    let left = |a: Point, c: Point| {
        let e = c - a;
        let l = e.hypot();
        if l < 1e-12 {
            Vec2::ZERO
        } else {
            Vec2::new(-e.y, e.x) / l
        }
    };
    (0..n)
        .map(|i| {
            let p = pts[i];
            let n1 = left(pts[(i + n - 1) % n], p);
            let n2 = left(p, pts[(i + 1) % n]);
            let d = 1.0 + n1.dot(n2);
            let mut off = if d > 1e-6 {
                (n1 + n2) * (b / d)
            } else {
                n2 * b
            };
            if off.hypot() > 3.0 * b {
                off = off.normalize() * 3.0 * b;
            }
            p + off
        })
        .collect()
}

/// Twice the signed area of the triangle `a`, `b`, `c`: positive with `c`
/// on the left of the way from `a` to `b`.
fn side(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// One edge of a closed loop for [`loops_cross`]: its box, its ends, its
/// loop and its place in it.
struct LoopEdge {
    lo: Point,
    hi: Point,
    a: Point,
    b: Point,
    of: usize,
    at: usize,
    n: usize,
}

/// Whether an edge of the closed `loops` crosses another edge, of its own
/// loop or of another one (edges that only touch do not count).
fn loops_cross(loops: &[Vec<Point>]) -> bool {
    let mut edges: Vec<LoopEdge> = Vec::new();
    for (of, pts) in loops.iter().enumerate() {
        let n = pts.len();
        for at in 0..n {
            let (a, b) = (pts[at], pts[(at + 1) % n]);
            edges.push(LoopEdge {
                lo: Point::new(a.x.min(b.x), a.y.min(b.y)),
                hi: Point::new(a.x.max(b.x), a.y.max(b.y)),
                a,
                b,
                of,
                at,
                n,
            });
        }
    }
    // Sorted by their left end, an edge is only tried against those that
    // start before it ends.
    edges.sort_by(|p, q| p.lo.x.total_cmp(&q.lo.x));
    for (k, e) in edges.iter().enumerate() {
        for f in edges.iter().skip(k + 1) {
            if f.lo.x > e.hi.x {
                break;
            }
            if f.lo.y > e.hi.y || f.hi.y < e.lo.y {
                continue;
            }
            // Neighbours on a loop share a corner.
            if e.of == f.of {
                let d = e.at.abs_diff(f.at);
                if d <= 1 || d + 1 == e.n {
                    continue;
                }
            }
            if side(e.a, e.b, f.a) * side(e.a, e.b, f.b) < 0.0
                && side(f.a, f.b, e.a) * side(f.a, f.b, e.b) < 0.0
            {
                return true;
            }
        }
    }
    false
}

/// Halvings [`fitting_bevel`] makes: what it finds is within a thousandth
/// of the bevel asked for of the widest one that fits.
const BEVEL_FIT_STEPS: usize = 10;

/// The widest bevel up to `b` the outline takes: inset by it, the closed
/// subpaths keep their sense, cross neither themselves nor one another
/// and still enclose an area. A bevel wider than a letter's stem, or than
/// the wall between an outline and its hole, folded the near cap over
/// itself (#2205).
fn fitting_bevel(subs: &[(Vec<Point>, bool, bool)], b: f64) -> f64 {
    let loops: Vec<&Vec<Point>> = subs
        .iter()
        .filter(|(pts, closed, _)| *closed && pts.len() >= 3)
        .map(|(pts, _, _)| pts)
        .collect();
    if loops.is_empty() || b <= 0.0 {
        return b;
    }
    let filled: f64 = loops.iter().map(|pts| area2(pts)).sum();
    let fits = |w: f64| {
        let ins: Vec<Vec<Point>> = loops.iter().map(|pts| inset(pts, w)).collect();
        ins.iter()
            .zip(&loops)
            .all(|(i, pts)| area2(i) * area2(pts) > 0.0)
            && ins.iter().map(|i| area2(i)).sum::<f64>() * filled > 0.0
            && !loops_cross(&ins)
    };
    if fits(b) {
        return b;
    }
    // An outline that crosses itself as drawn has no bevel that fits: it
    // keeps the one asked for.
    let own: Vec<Vec<Point>> = loops.iter().map(|pts| (*pts).clone()).collect();
    if loops_cross(&own) {
        return b;
    }
    let (mut lo, mut hi) = (0.0, b);
    for _ in 0..BEVEL_FIT_STEPS {
        let mid = (lo + hi) / 2.0;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// What orders the faces of a parallel body: its run in space, across the
/// page (x and y, each over the run's length across squared) and in depth.
type Run = Option<(f64, f64, f64)>;

/// The run of a parallel `body` turned by `m`: none for a body with a
/// vanishing point and for one that runs straight at the viewer.
fn run_of(body: &Body, m: &[[f64; 3]; 3], dir: f64, len: f64) -> Run {
    if !body.kind.parallel() {
        return None;
    }
    let r = apply(m, V3::new(body.vector.x, body.vector.y, dir * len));
    let across = r.x * r.x + r.y * r.y;
    (across > 1e-9).then_some((r.x / across, r.y / across, r.z))
}

/// How near the point `q` of a face is, for the order the faces are drawn
/// in (larger is nearer). Two points drawn at one place of the page lie
/// on two lines that run as the body does, so a point's depth is taken
/// back along its line to where the line crosses the page's origin: the
/// key is the same along the whole run of a wall, and for two points
/// drawn at one place it differs by what their depths differ by. The
/// plain z of a wall differs by the body's depth from one end to the
/// other, so its mean put long walls in the wrong order (#2206).
fn depth_key(run: Run, q: V3) -> f64 {
    match run {
        Some((ax, ay, rz)) => q.z - (q.x * ax + q.y * ay) * rz,
        None => q.z,
    }
}

/// A face that shows: its z key, flat key, shape and drawn corners.
type ShownFace = (f64, f64, Shape, [Point; 4]);

struct Ctx<'a> {
    body: &'a Body,
    m: [[f64; 3]; 3],
    pivot: V3,
    len: f64,
    /// -1 for back bodies (away from the viewer), +1 for front ones.
    dir: f64,
}

impl Ctx<'_> {
    /// The turned point of `p` at depth `u`.
    fn turn(&self, p: Point, u: f64) -> V3 {
        // A parallel body runs along its offset across and as far in
        // depth (seen straight on, the offset is what shows).
        let across = if self.body.kind.parallel() {
            self.body.vector * (u / self.len)
        } else {
            Vec2::ZERO
        };
        let v = V3::new(p.x + across.x, p.y + across.y, self.dir * u).sub(self.pivot);
        apply(&self.m, v).add(self.pivot)
    }

    /// The turned point as if the body ran straight back: what a face's
    /// light is worked from, so an untouched body is lit as the flat one.
    fn turn_straight(&self, p: Point, u: f64) -> V3 {
        let v = V3::new(p.x, p.y, self.dir * u).sub(self.pivot);
        apply(&self.m, v).add(self.pivot)
    }

    /// Where the turned point `q` of depth `u` is drawn. The depth share
    /// is the point's own (`u / L`, 0 at the object and 1 at the copy),
    /// not where the turn puts it along z: the body is only as deep as
    /// its share of the way to the vanishing point, so with that point
    /// near the object a turn moved z by many depths and the faces ran
    /// far past the object (#2207).
    fn project(&self, q: V3, u: f64) -> Point {
        let t = (u / self.len).clamp(0.0, 1.0);
        let p = Point::new(q.x, q.y);
        let d = self.body.depth.clamp(0.0, 0.99);
        match self.body.kind {
            ExtrudeKind::BackParallel | ExtrudeKind::FrontParallel => p,
            ExtrudeKind::SmallBack | ExtrudeKind::SmallFront => {
                let den = (t * d + 1.0 - d).max(0.05);
                p + (self.body.vanishing - p) * (t * d / den)
            }
            ExtrudeKind::BigBack | ExtrudeKind::BigFront => p + (p - self.body.vanishing) * (d * t),
        }
    }
}

/// Twice the signed area of a projected polygon.
fn projected_area(pts: &[Point]) -> f64 {
    area2(pts)
}

/// How bright a face with outward normal `n` (viewer space) is.
fn lit(n: V3, lights: &[(V3, f64)]) -> f64 {
    let mut v = 0.0;
    for (i, (l, k)) in lights.iter().enumerate() {
        if i == 0 {
            v += 0.55 + 0.45 * (k * (n.dot(*l) + 1.0) / 2.0 + (1.0 - k));
        } else {
            v += 0.3 * k * n.dot(*l).max(0.0);
        }
    }
    if lights.is_empty() {
        v = 1.0;
    }
    v.clamp(0.2, 1.0)
}

fn light_dir(angle: f64, elevation: f64) -> V3 {
    let a = if angle.is_finite() {
        angle.to_radians()
    } else {
        0.0
    };
    let e = if elevation.is_finite() {
        elevation.clamp(-90.0, 90.0).to_radians()
    } else {
        0.0
    };
    V3::new(a.cos() * e.cos(), a.sin() * e.cos(), e.sin())
}

fn poly(pts: &[Point], close: bool) -> BezPath {
    let mut p = BezPath::new();
    for (i, q) in pts.iter().enumerate() {
        if i == 0 {
            p.move_to(*q);
        } else {
            p.line_to(*q);
        }
    }
    if close {
        p.close_path();
    }
    p
}

/// The body of `shape` turned, lit and bevelled as `body.solid` and
/// `body.bevel` say. `subs` are the outline's flattened subpaths with
/// whether each closes and is a hole, `outline` the same subpaths with
/// their curves.
pub(crate) fn faces(
    body: &Body,
    shape: &Shape,
    subs: &[(Vec<Point>, bool, bool)],
    outline: &BezPath,
) -> Faces {
    let mut out = Faces::default();
    let mut lo = Point::new(f64::INFINITY, f64::INFINITY);
    let mut hi = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (pts, _, _) in subs {
        for p in pts {
            lo = Point::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Point::new(hi.x.max(p.x), hi.y.max(p.y));
        }
    }
    if !(lo.x.is_finite() && hi.x.is_finite()) {
        return out;
    }
    let c = lo.midpoint(hi);
    let len = if body.kind.parallel() {
        body.vector.hypot()
    } else {
        body.depth.clamp(0.0, 0.99) * (body.vanishing - c).hypot()
    };
    if !(len.is_finite() && len > 1e-6) {
        return out;
    }
    let dir = if body.kind.front() { 1.0 } else { -1.0 };
    let ctx = Ctx {
        body,
        m: rotation(body.solid.rotation),
        pivot: if body.kind.parallel() {
            V3::new(
                c.x + body.vector.x / 2.0,
                c.y + body.vector.y / 2.0,
                dir * len / 2.0,
            )
        } else {
            V3::new(c.x, c.y, dir * len / 2.0)
        },
        len,
        dir,
    };
    let run = run_of(body, &ctx.m, dir, len);
    let half_min = ((hi.x - lo.x).min(hi.y - lo.y) / 2.0).max(0.0);
    let bevel = if body.bevel.is_finite() {
        body.bevel.clamp(0.0, (0.9 * len).min(0.9 * half_min))
    } else {
        0.0
    };
    let bevel = fitting_bevel(subs, bevel);
    let turned = body.solid.turned();
    let replace_main = turned || bevel > 0.0;

    let base = shape.fill.preview_color().unwrap_or(Color::WHITE);
    let (from, to) = match body.coloring {
        Coloring::Object => (base, lerp_color(base, Color::BLACK, 0.5)),
        Coloring::Solid(c) => (c, lerp_color(c, Color::BLACK, 0.5)),
        Coloring::Shade(a, b) => (a, b),
    };
    let k = |i: f64| {
        if i.is_finite() {
            (i / 100.0).clamp(0.0, 1.0)
        } else {
            0.5
        }
    };
    let mut lights = vec![(
        light_dir(body.light_angle, body.solid.elevation),
        k(body.light_intensity),
    )];
    for l in body
        .solid
        .lights
        .iter()
        .take(crate::extrude3d::MAX_EXTRA_LIGHTS)
    {
        lights.push((light_dir(l.angle, l.elevation), k(l.intensity)));
    }

    let template = || {
        let mut s = shape.clone();
        s.effects.clear();
        s.shadow = None;
        s.transform = Affine::IDENTITY;
        if let Some(st) = &mut s.stroke {
            st.width = st.width.min(0.1);
            st.start_arrow = Default::default();
            st.end_arrow = Default::default();
        }
        s
    };
    let outline_stroke = template().stroke;

    // Rings: (depth, the loop at that depth) for each subpath; the near
    // cap's ring is inset by the bevel.
    let near_u = if dir < 0.0 { 0.0 } else { len };
    // Each face: (z key, flat key, shape).
    let mut drawn: Vec<(f64, f64, Shape)> = Vec::new();
    let flat_key = |m: Point| -> f64 {
        let to_v = (m - body.vanishing).hypot();
        match body.kind {
            ExtrudeKind::BackParallel => -m.to_vec2().dot(body.vector),
            ExtrudeKind::FrontParallel => m.to_vec2().dot(body.vector),
            ExtrudeKind::SmallBack | ExtrudeKind::BigFront => to_v,
            ExtrudeKind::BigBack | ExtrudeKind::SmallFront => -to_v,
        }
    };
    let corner = 20f64.to_radians().cos();
    let mut near_loops: Vec<(Vec<Point>, bool)> = Vec::new();
    let mut far_loops: Vec<(Vec<Point>, bool)> = Vec::new();
    let mut near_area = 0.0;
    let mut far_area = 0.0;
    let mut far_z = 0.0;
    let mut cap_points = 0usize;
    for (pts, closed, _hole) in subs {
        let closed = *closed;
        let n = pts.len();
        if n < 2 {
            continue;
        }
        let b = if closed { bevel } else { 0.0 };
        let inner = if b > 0.0 { inset(pts, b) } else { pts.clone() };
        // Bands from the object's side to the copy's.
        let bands: Vec<(f64, &Vec<Point>, f64, &Vec<Point>)> = if b > 0.0 {
            if dir < 0.0 {
                vec![(0.0, &inner, b, pts), (b, pts, len, pts)]
            } else {
                vec![(0.0, pts, len - b, pts), (len - b, pts, len, &inner)]
            }
        } else {
            vec![(0.0, pts, len, pts)]
        };
        let edges = if closed { n } else { n - 1 };
        for (u0, r0, u1, r1) in bands {
            // Outward faces: [a0, a1, b1, b0] for back bodies (a0 nearer),
            // the other way round for front ones.
            let mut shown: Vec<Option<ShownFace>> = Vec::with_capacity(edges);
            for i in 0..edges {
                let j = (i + 1) % n;
                let (a0, b0) = (ctx.turn(r0[i], u0), ctx.turn(r0[j], u0));
                let (a1, b1) = (ctx.turn(r1[i], u1), ctx.turn(r1[j], u1));
                // Each corner with its depth.
                let (quad, depths) = if dir < 0.0 {
                    ([a0, a1, b1, b0], [u0, u1, u1, u0])
                } else {
                    ([a0, b0, b1, a1], [u0, u0, u1, u1])
                };
                let proj: [Point; 4] = [
                    ctx.project(quad[0], depths[0]),
                    ctx.project(quad[1], depths[1]),
                    ctx.project(quad[2], depths[2]),
                    ctx.project(quad[3], depths[3]),
                ];
                if !proj.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
                    shown.push(None);
                    continue;
                }
                let area = projected_area(&proj);
                // Open subpaths show both sides.
                if closed && area <= 1e-12 {
                    shown.push(None);
                    continue;
                }
                let straight = {
                    let (a0, b0) = (ctx.turn_straight(r0[i], u0), ctx.turn_straight(r0[j], u0));
                    let (a1, b1) = (ctx.turn_straight(r1[i], u1), ctx.turn_straight(r1[j], u1));
                    if dir < 0.0 {
                        [a0, a1, b1, b0]
                    } else {
                        [a0, b0, b1, a1]
                    }
                };
                let mut normal = straight[1]
                    .sub(straight[0])
                    .cross(straight[3].sub(straight[0]))
                    .unit();
                if area < 0.0 {
                    normal = V3::new(-normal.x, -normal.y, -normal.z);
                }
                let light = lit(normal, &lights);
                let mut s = template();
                s.kind = ShapeKind::Path {
                    path: poly(&proj, true),
                    closed: true,
                };
                let side_from = from;
                let (fill, edge) = match body.coloring {
                    // The bevel band is one colour; the sides shade.
                    Coloring::Shade(..) if u1 - u0 > b || b <= 0.0 => {
                        let m0 = proj[0].midpoint(proj[3]);
                        let m1 = proj[1].midpoint(proj[2]);
                        let (start, end) = if dir < 0.0 { (m0, m1) } else { (m1, m0) };
                        let run = end - start;
                        let ang = run.y.atan2(run.x).to_degrees();
                        (
                            Fill::linear(scale(side_from, light), scale(to, light), ang),
                            scale(lerp_color(side_from, to, 0.5), light),
                        )
                    }
                    _ => (
                        Fill::Solid(scale(side_from, light)),
                        scale(side_from, light),
                    ),
                };
                s.fill = fill;
                s.stroke = Some(crate::Stroke::hairline(edge));
                let z = quad.iter().map(|q| depth_key(run, *q)).sum::<f64>() / 4.0;
                let mid = proj[0].midpoint(proj[2]);
                shown.push(Some((z, flat_key(mid), s, proj)));
            }
            // Runs of faces between corners, outlined as one surface.
            let dirv = |i: usize| (pts[(i + 1) % n] - pts[i]).normalize();
            let starts = |i: usize| -> bool {
                if shown[i].is_none() {
                    return false;
                }
                let prev = match (i, closed) {
                    (0, true) => Some(edges - 1),
                    (0, false) => None,
                    _ => Some(i - 1),
                };
                match prev {
                    None => true,
                    Some(p) => shown[p].is_none() || dirv(p).dot(dirv(i)) < corner,
                }
            };
            let mut runs: Vec<Vec<usize>> = Vec::new();
            match (0..edges).find(|&i| starts(i)) {
                None if edges > 0 && shown.iter().all(|f| f.is_some()) => {
                    runs.push((0..edges).collect())
                }
                None => {}
                Some(f) => {
                    let order: Vec<usize> = if closed {
                        (0..edges).map(|k| (f + k) % edges).collect()
                    } else {
                        (f..edges).collect()
                    };
                    let mut cur: Vec<usize> = Vec::new();
                    for i in order {
                        if (shown[i].is_none() || starts(i)) && !cur.is_empty() {
                            runs.push(std::mem::take(&mut cur));
                        }
                        if shown[i].is_some() {
                            cur.push(i);
                        }
                    }
                    if !cur.is_empty() {
                        runs.push(cur);
                    }
                }
            }
            for run in runs {
                let mut z = 0.0;
                let mut fk = 0.0;
                let mut first_edge = Vec::new();
                let mut second_edge = Vec::new();
                for (pos, &i) in run.iter().enumerate() {
                    let Some((fz, f, s, proj)) = &shown[i] else {
                        continue;
                    };
                    z += fz;
                    fk += f;
                    drawn.push((*fz, *f, s.clone()));
                    // proj: back [a0, a1, b1, b0]; front [a0, b0, b1, a1].
                    let (a_near, a_far, b_near, b_far) = if dir < 0.0 {
                        (proj[0], proj[1], proj[3], proj[2])
                    } else {
                        (proj[0], proj[3], proj[1], proj[2])
                    };
                    if pos == 0 {
                        first_edge.push(a_near);
                        second_edge.push(a_far);
                    }
                    first_edge.push(b_near);
                    second_edge.push(b_far);
                }
                if let Some(st) = &outline_stroke {
                    let mut ring = first_edge.clone();
                    ring.extend(second_edge.iter().rev());
                    let mut o = template();
                    o.kind = ShapeKind::Path {
                        path: poly(&ring, true),
                        closed: true,
                    };
                    o.fill = Fill::None;
                    o.stroke = Some(st.clone());
                    let cnt = run.len().max(1) as f64;
                    // Just after its faces.
                    drawn.push((z / cnt + 1e-9, fk / cnt - 1e-9, o));
                }
            }
        }
        // Caps.
        let near_ring: &Vec<Point> = &inner;
        let near: Vec<V3> = near_ring.iter().map(|p| ctx.turn(*p, near_u)).collect();
        let far_u = if dir < 0.0 { len } else { 0.0 };
        let far: Vec<V3> = pts.iter().map(|p| ctx.turn(*p, far_u)).collect();
        let near_p: Vec<Point> = near.iter().map(|q| ctx.project(*q, near_u)).collect();
        let far_p: Vec<Point> = far.iter().map(|q| ctx.project(*q, far_u)).collect();
        if closed {
            near_area += projected_area(&near_p);
            far_area -= projected_area(&far_p);
        }
        far_z += far.iter().map(|q| q.z).sum::<f64>();
        cap_points += n;
        near_loops.push((near_p, closed));
        far_loops.push((far_p, closed));
    }
    let any_closed = subs.iter().any(|(_, c, _)| *c);
    let cnt = cap_points.max(1) as f64;
    let cap_path = |loops: &[(Vec<Point>, bool)]| {
        let mut p = BezPath::new();
        for (pts, closed) in loops {
            for el in poly(pts, *closed).elements() {
                p.push(*el);
            }
        }
        p
    };
    // A cap lies at one depth, where the turn and the view move and
    // scale the outline and bend nothing: it is the outline's own curves
    // taken there, with the outline's nodes (#2208). The near cap of a
    // bevelled body is the outline drawn in, kept as its flattened ring.
    let far_u = if dir < 0.0 { len } else { 0.0 };
    let cap_map = |u: f64| -> Option<Affine> {
        let at = |p: Point| ctx.project(ctx.turn(p, u), u);
        let (o, x, y) = (
            at(c),
            at(c + Vec2::new(1.0, 0.0)),
            at(c + Vec2::new(0.0, 1.0)),
        );
        let (ex, ey) = (x - o, y - o);
        let all = [o.x, o.y, ex.x, ex.y, ey.x, ey.y];
        if !all.iter().all(|v| v.is_finite()) {
            return None;
        }
        Some(Affine::new([ex.x, ex.y, ey.x, ey.y, o.x, o.y]) * Affine::translate(-c.to_vec2()))
    };
    let cap_curves = |u: f64| -> Option<BezPath> {
        if outline.elements().is_empty() {
            return None;
        }
        cap_map(u).map(|to_cap| to_cap * outline.clone())
    };
    // The far cap, darkest; the near cap looks like the object.
    let far_shows = any_closed && (far_area > 1e-12 || !turned);
    if far_shows {
        let mut far = template();
        far.kind = ShapeKind::Path {
            path: cap_curves(far_u).unwrap_or_else(|| cap_path(&far_loops)),
            closed: true,
        };
        far.fill = Fill::Solid(to);
        // A parallel body's far cap is behind every wall that shows or,
        // facing the viewer, in front of them all.
        let far_key = match run {
            Some(_) if far_area > 1e-12 => f64::INFINITY,
            Some(_) => f64::NEG_INFINITY,
            None => far_z / cnt,
        };
        drawn.push((far_key, f64::INFINITY, far));
    }
    let near_shows = !any_closed || near_area > 1e-12 || !turned;
    let mut near = shape.clone();
    near.effects.clear();
    near.shadow = None;
    near.transform = Affine::IDENTITY;
    // The near cap is the outline taken to its depth: the object's mesh
    // goes there with it (#2337).
    crate::extrude::place_paint(
        &mut near,
        cap_map(near_u).unwrap_or(Affine::IDENTITY) * shape.transform,
    );
    let near_curves = if bevel > 0.0 {
        None
    } else {
        cap_curves(near_u)
    };
    near.kind = ShapeKind::Path {
        path: near_curves.unwrap_or_else(|| cap_path(&near_loops)),
        closed: any_closed,
    };
    // Far first: smaller z, then the larger flat key.
    drawn.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    let faces: Vec<Shape> = drawn.into_iter().map(|(_, _, s)| s).collect();
    if replace_main {
        // The object itself is the near cap (back bodies) or the far one
        // (front bodies), drawn here in its place among the faces.
        let mut hidden = shape.clone();
        hidden.effects.clear();
        hidden.kind = ShapeKind::Path {
            path: BezPath::new(),
            closed: false,
        };
        hidden.fill = Fill::None;
        hidden.stroke = None;
        hidden.transform = Affine::IDENTITY;
        out.main = Some(hidden);
        out.above = faces;
        if near_shows {
            // Nearest: last (back bodies' near cap is the object; front
            // bodies' near cap is the copy looking like it).
            out.above.push(near);
        }
    } else if body.kind.front() {
        out.above = faces;
        out.above.push(near);
    } else {
        out.below = faces;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extrude::Coloring;
    use crate::geometry::Rect;
    use crate::ShapeId;

    fn square() -> Shape {
        let mut s = Shape::new(
            ShapeId(1),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(200, 0, 0));
        s.stroke = None;
        s
    }

    fn body(kind: ExtrudeKind, solid: Solid, bevel: f64) -> Body {
        Body {
            kind,
            vector: Vec2::new(10.0, 10.0),
            vanishing: Point::new(60.0, 60.0),
            depth: 0.25,
            coloring: Coloring::Object,
            light_angle: 135.0,
            light_intensity: 50.0,
            bevel,
            solid,
        }
    }

    fn filled(v: &[Shape]) -> Vec<&Shape> {
        v.iter().filter(|s| s.fill != Fill::None).collect()
    }

    #[test]
    fn untouched_with_a_raised_light_it_draws_as_the_flat_body() {
        let solid = Solid {
            elevation: 30.0,
            ..Default::default()
        };
        let f = body(ExtrudeKind::BackParallel, solid, 0.0).faces(&square());
        assert!(f.main.is_none());
        // The far face and the two sides toward the vector.
        assert_eq!(filled(&f.below).len(), 3, "{:?}", f.below.len());
        let far = f.below[0].bounds();
        assert!((far.x0 - 10.0).abs() < 1e-6 && (far.y0 - 10.0).abs() < 1e-6);
    }

    /// #2208: the caps of a turned body keep the outline's curves and
    /// nodes (a turned circle's face had a node for each flattened edge).
    #[test]
    fn the_caps_of_a_turned_body_keep_the_outline_curves() {
        use kurbo::{ParamCurveNearest, PathEl};
        let mut c = square();
        c.kind = ShapeKind::Ellipse {
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
            arc: None,
        };
        let nodes = c.page_path().elements().len();
        let solid = Solid {
            rotation: [-17.0, 10.0, 5.0],
            ..Default::default()
        };
        for kind in [
            ExtrudeKind::BackParallel,
            ExtrudeKind::FrontParallel,
            ExtrudeKind::SmallBack,
            ExtrudeKind::BigFront,
        ] {
            let curved = body(kind, solid.clone(), 0.0).faces(&c);
            // The same body from the circle as a polygon of its flattened
            // points: its caps are where the curved ones must lie.
            let mut flat = BezPath::new();
            crate::geometry::flatten(&c.page_path(), 0.05, &mut |el| flat.push(el));
            let mut polygon = c.clone();
            polygon.kind = ShapeKind::Path {
                path: flat,
                closed: true,
            };
            let lines = body(kind, solid.clone(), 0.0).faces(&polygon);
            let cap = |f: &Faces| f.above.last().expect("the near cap").page_path();
            let (near, want) = (cap(&curved), cap(&lines));
            assert_eq!(near.elements().len(), nodes, "{kind:?}");
            assert!(want.elements().len() > 3 * nodes, "{kind:?}");
            for el in want.elements() {
                let (PathEl::MoveTo(p) | PathEl::LineTo(p)) = el else {
                    continue;
                };
                let d = near
                    .segments()
                    .map(|s| s.nearest(*p, 1e-9).distance_sq.sqrt())
                    .fold(f64::INFINITY, f64::min);
                assert!(d < 0.05, "{kind:?}: {p:?} is {d} off the cap");
            }
            // The far cap too, where it shows.
            let far_nodes = |f: &Faces| {
                f.above
                    .iter()
                    .chain(&f.below)
                    .filter(|s| {
                        s.fill == Fill::Solid(lerp_color(Color::rgb8(200, 0, 0), Color::BLACK, 0.5))
                    })
                    .map(|s| s.page_path().elements().len())
                    .max()
            };
            if let Some(n) = far_nodes(&curved) {
                assert_eq!(n, nodes, "{kind:?}: the far cap");
            }
        }
    }

    #[test]
    fn turning_shows_the_side_turned_toward_the_viewer() {
        // Turned 40 degrees about y: the right side turns away, so the
        // left side shows on a back body running straight back.
        let mut b = body(
            ExtrudeKind::BackParallel,
            Solid {
                rotation: [0.0, 40.0, 0.0],
                ..Default::default()
            },
            0.0,
        );
        b.vector = Vec2::new(0.0, 10.0);
        let f = b.faces(&square());
        assert!(f.main.is_some(), "the object is drawn among the faces");
        assert!(f.below.is_empty());
        let cap = f.above.last().expect("near cap");
        // The near cap is the turned square: narrower than 20.
        let cb = cap.bounds();
        assert!(cb.width() < 19.0 && cb.width() > 10.0, "{cb:?}");
        assert_eq!(cap.fill, Fill::Solid(Color::rgb8(200, 0, 0)));
        // A side face shows to the left of the cap.
        let sides: Vec<&Shape> = filled(&f.above[..f.above.len() - 1]);
        assert!(
            sides.iter().any(|s| s.bounds().x0 < cb.x0 - 1.0),
            "{sides:?}"
        );
    }

    #[test]
    fn a_half_turn_hides_the_object_and_shows_the_far_cap() {
        let b = body(
            ExtrudeKind::BackParallel,
            Solid {
                rotation: [0.0, 180.0, 0.0],
                ..Default::default()
            },
            0.0,
        );
        let f = b.faces(&square());
        // The far cap, darkest, is the last thing drawn; the object's face
        // looks away.
        let last = f.above.last().expect("faces");
        assert_eq!(last.fill, Fill::Solid(Color::rgb8(100, 0, 0)));
    }

    #[test]
    fn a_bevel_shrinks_the_near_cap_and_adds_a_band() {
        let f = body(ExtrudeKind::BackParallel, Solid::default(), 2.0).faces(&square());
        let cap = f.above.last().expect("near cap").bounds();
        assert!(
            (cap.width() - 16.0).abs() < 1e-6 && (cap.x0 - 2.0).abs() < 1e-6,
            "{cap:?}"
        );
        // Band faces join the cap to the outline one bevel deep, drawn
        // along the vector by 2 / |v| of it: x = 0 + 10 * 2 / 14.14.
        let reach = 10.0 * 2.0 / 200f64.sqrt();
        assert!(
            filled(&f.above)
                .iter()
                .any(|s| (s.bounds().x0 - reach).abs() < 1e-6),
            "{:?}",
            f.above.iter().map(|s| s.bounds()).collect::<Vec<_>>()
        );
    }

    /// #2205: a bevel wider than a letter's stems folded the near cap over
    /// itself.
    #[test]
    fn a_bevel_wider_than_the_outline_takes_is_narrowed_to_fit() {
        let pts = |v: &[(f64, f64)]| -> Vec<Point> {
            v.iter().map(|(x, y)| Point::new(*x, *y)).collect()
        };
        // A letter K, 20 high, its stems 4 wide.
        let k = pts(&[
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 8.0),
            (10.0, 0.0),
            (15.0, 0.0),
            (7.5, 10.5),
            (15.0, 20.0),
            (10.0, 20.0),
            (4.0, 12.5),
            (4.0, 20.0),
            (0.0, 20.0),
        ]);
        let subs = vec![(k.clone(), true, false)];
        assert!(loops_cross(&[inset(&k, 2.0)]), "as asked, it folds");
        for b in [1.0, 2.0, 3.0, 5.0] {
            let w = fitting_bevel(&subs, b);
            assert!(w > 0.3 && w < b, "{b}: {w}");
            let ins = inset(&k, w);
            assert!(area2(&ins) * area2(&k) > 0.0, "{b}: {w}");
            assert!(!loops_cross(&[ins]), "{b}: {w}");
        }
        // A bevel that fits stays as asked.
        assert_eq!(fitting_bevel(&subs, 0.5), 0.5);
        // A frame with a wall 4 wide: the cap and its hole would swap
        // places past a bevel of 2.
        let frame = vec![
            (
                pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)]),
                true,
                false,
            ),
            (
                pts(&[(4.0, 4.0), (4.0, 16.0), (16.0, 16.0), (16.0, 4.0)]),
                true,
                true,
            ),
        ];
        let w = fitting_bevel(&frame, 3.0);
        assert!(w > 1.9 && w < 2.0, "{w}");
        // An open outline has no bevel to narrow.
        assert_eq!(fitting_bevel(&[(k, false, false)], 3.0), 3.0);
    }

    /// #2206: with a long offset and a turn, walls were drawn in the order
    /// of their mean z, which differs by the body's depth along a wall, so
    /// far walls came out over near ones.
    #[test]
    fn walls_of_a_turned_parallel_body_are_ordered_where_they_overlap() {
        fn inside(q: &[Point; 4], p: Point) -> bool {
            let (mut pos, mut neg) = (0, 0);
            for i in 0..4 {
                let s = side(q[i], q[(i + 1) % 4], p);
                if s > 1e-9 {
                    pos += 1;
                } else if s < -1e-9 {
                    neg += 1;
                }
            }
            pos == 4 || neg == 4
        }
        // z of the quad's plane at `p`, unless it is seen edge-on.
        fn z_at(q: &[V3; 4], p: Point) -> Option<f64> {
            let n = q[1].sub(q[0]).cross(q[3].sub(q[0]));
            (n.z.abs() > 1e-9).then(|| q[0].z - (n.x * (p.x - q[0].x) + n.y * (p.y - q[0].y)) / n.z)
        }
        // A letter K 34 high, its offset and turn as in the report.
        let k: Vec<Point> = [
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 8.0),
            (10.0, 0.0),
            (15.0, 0.0),
            (7.5, 10.5),
            (15.0, 20.0),
            (10.0, 20.0),
            (4.0, 12.5),
            (4.0, 20.0),
            (0.0, 20.0),
        ]
        .iter()
        .map(|(x, y)| Point::new(x * 1.7, y * 1.7))
        .collect();
        let rot = [-17.0, 10.0, -8.0];
        // Both offsets of the report; the walls of a K overlap on the page
        // in a few pairs at each, and the mean z misorders all of them.
        let (mut pairs, mut by_z, mut by_key) = (0, 0, 0);
        for v in [Vec2::new(16.384, 10.656), Vec2::new(10.776, 37.191)] {
            let solid = Solid {
                rotation: rot,
                ..Default::default()
            };
            let mut b = body(ExtrudeKind::BackParallel, solid, 0.0);
            b.vector = v;
            let len = b.vector.hypot();
            let ctx = Ctx {
                body: &b,
                m: rotation(rot),
                pivot: V3::new(
                    12.75 + b.vector.x / 2.0,
                    17.0 + b.vector.y / 2.0,
                    -len / 2.0,
                ),
                len,
                dir: -1.0,
            };
            let run = run_of(&b, &ctx.m, -1.0, len);
            assert!(run.is_some());
            // The walls that show: mean z, key, corners in space and drawn.
            let mut walls: Vec<(f64, f64, [V3; 4], [Point; 4])> = Vec::new();
            for i in 0..k.len() {
                let j = (i + 1) % k.len();
                let quad = [
                    ctx.turn(k[i], 0.0),
                    ctx.turn(k[i], len),
                    ctx.turn(k[j], len),
                    ctx.turn(k[j], 0.0),
                ];
                let proj = [
                    ctx.project(quad[0], 0.0),
                    ctx.project(quad[1], len),
                    ctx.project(quad[2], len),
                    ctx.project(quad[3], 0.0),
                ];
                if projected_area(&proj) <= 1e-12 {
                    continue;
                }
                let z = quad.iter().map(|q| q.z).sum::<f64>() / 4.0;
                let key = quad.iter().map(|q| depth_key(run, *q)).sum::<f64>() / 4.0;
                walls.push((z, key, quad, proj));
            }
            // Pairs that overlap on the page and are drawn with the farther
            // one over the nearer, by the mean z and by the key.
            for (ia, a) in walls.iter().enumerate() {
                for c in walls.iter().skip(ia + 1) {
                    // How much nearer `a` is than `c` where both are drawn.
                    let mut nearer: Option<(f64, f64)> = None;
                    for (src, other) in [(&a.3, &c.3), (&c.3, &a.3)] {
                        for s in 1..12 {
                            for t in 1..12 {
                                let (s, t) = (s as f64 / 12.0, t as f64 / 12.0);
                                let p = src[0].lerp(src[1], s).lerp(src[3].lerp(src[2], s), t);
                                if !inside(other, p) {
                                    continue;
                                }
                                if let (Some(za), Some(zc)) = (z_at(&a.2, p), z_at(&c.2, p)) {
                                    let (lo, hi) = nearer.unwrap_or((za - zc, za - zc));
                                    nearer = Some((lo.min(za - zc), hi.max(za - zc)));
                                }
                            }
                        }
                    }
                    let Some((lo, hi)) = nearer else {
                        continue;
                    };
                    pairs += 1;
                    // `a` nearer by more than 0.05 somewhere, yet drawn first
                    // (the smaller key), or the other way round.
                    let wrong =
                        |ka: f64, kc: f64| (hi > 0.05 && ka < kc) || (lo < -0.05 && ka > kc);
                    by_z += usize::from(wrong(a.0, c.0));
                    by_key += usize::from(wrong(a.1, c.1));
                }
            }
        }
        assert!(pairs >= 5, "{pairs} pairs overlap");
        assert!(by_z >= 3, "the mean z misorders {by_z} of {pairs}");
        assert_eq!(by_key, 0, "of {pairs} pairs");
        // A body with a vanishing point keeps z.
        let small = body(ExtrudeKind::SmallBack, Solid::default(), 0.0);
        assert!(run_of(&small, &rotation(rot), -1.0, 20.0).is_none());
        assert_eq!(depth_key(None, V3::new(3.0, 4.0, 5.0)), 5.0);
    }

    #[test]
    fn further_lights_brighten_the_faces_they_reach() {
        let one = body(ExtrudeKind::BackParallel, Solid::default(), 0.0);
        let two = body(
            ExtrudeKind::BackParallel,
            Solid {
                lights: vec![Light {
                    angle: 0.0,
                    elevation: 0.0,
                    intensity: 100.0,
                }],
                ..Default::default()
            },
            0.0,
        );
        let red = |v: &[Shape]| -> Vec<u8> {
            filled(v)
                .iter()
                .filter_map(|s| match s.fill {
                    Fill::Solid(c) => Some(c.to_rgb8()[0]),
                    _ => None,
                })
                .collect()
        };
        // Both bodies draw their far face first.
        let mut a = red(one.faces(&square()).below.get(1..).unwrap_or_default());
        let lit = two.faces(&square()).below;
        let mut b = red(lit.get(1..).unwrap_or_default());
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a.len(), b.len());
        // Never darker, and the face toward the second light brighter.
        assert!(b.iter().zip(&a).all(|(x, y)| x >= y));
        assert!(b.iter().zip(&a).any(|(x, y)| x > y), "{a:?} {b:?}");
    }

    #[test]
    fn broken_values_never_panic() {
        for kind in ExtrudeKind::ALL {
            for r in [[f64::NAN, 0.0, 0.0], [0.0, 90.0, 0.0], [1e9, -1e9, 3.0]] {
                let mut b = body(
                    kind,
                    Solid {
                        rotation: r,
                        elevation: f64::INFINITY,
                        lights: vec![
                            Light {
                                angle: f64::NAN,
                                elevation: 0.0,
                                intensity: f64::NAN,
                            };
                            5
                        ],
                    },
                    f64::NAN,
                );
                let _ = b.faces(&square());
                b.bevel = 1e9;
                let _ = b.faces(&square());
            }
        }
    }

    /// A letter K, 34 high.
    fn letter_k() -> Shape {
        let pts: Vec<Point> = [
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 8.0),
            (10.0, 0.0),
            (15.0, 0.0),
            (7.5, 10.5),
            (15.0, 20.0),
            (10.0, 20.0),
            (4.0, 12.5),
            (4.0, 20.0),
            (0.0, 20.0),
        ]
        .iter()
        .map(|(x, y)| Point::new(x * 1.7, y * 1.7))
        .collect();
        let mut s = Shape::new(
            ShapeId(1),
            ShapeKind::Path {
                path: poly(&pts, true),
                closed: true,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(200, 0, 0));
        s.stroke = None;
        s
    }

    fn corners(s: &Shape) -> Vec<Point> {
        s.page_path()
            .elements()
            .iter()
            .filter_map(|el| el.end_point())
            .collect()
    }

    /// #2207: with the vanishing point near the object, a turned body's
    /// faces ran far past it.
    #[test]
    fn a_turned_body_stays_by_the_object_with_the_vanishing_point_near() {
        let shape = letter_k();
        let bb = shape.bounds();
        let c = bb.center();
        let turn = [-22.0, 7.0, -11.0];
        let d = 0.2;
        for kind in [
            ExtrudeKind::BigFront,
            ExtrudeKind::BigBack,
            ExtrudeKind::SmallFront,
            ExtrudeKind::SmallBack,
        ] {
            // Just outside the object's corner, then on the object.
            for off in [
                Vec2::new(bb.width() / 2.0 + 5.0, bb.height() / 2.0 + 5.0),
                Vec2::new(3.0, 2.0),
            ] {
                let mut b = body(
                    kind,
                    Solid {
                        rotation: turn,
                        ..Default::default()
                    },
                    0.0,
                );
                b.vanishing = c + off;
                b.depth = d;
                let f = b.faces(&shape);
                assert!(f.below.is_empty() && !f.above.is_empty(), "{kind:?}");
                // The turned prism stays within `reach` of the middle,
                // and the vanishing point moves a point at most the depth
                // share of its distance to it.
                let len = d * off.hypot();
                let reach = (bb.width() / 2.0).hypot(bb.height() / 2.0).hypot(len / 2.0);
                let most = reach + d * (reach + off.hypot()) + 1e-6;
                for s in &f.above {
                    for p in corners(s) {
                        assert!((p - c).hypot() <= most, "{kind:?} {off:?}: {p:?}");
                    }
                }
                // The near cap is the turned copy, moved by the vanishing
                // point as the flat body's copy is: all the way (depth
                // share 1) for front bodies, not at all for back ones.
                let m = rotation(turn);
                let dir = if kind.front() { 1.0 } else { -1.0 };
                let pivot = V3::new(c.x, c.y, dir * len / 2.0);
                let u = if kind.front() { len } else { 0.0 };
                let t = u / len;
                let want: Vec<Point> = corners(&shape)
                    .into_iter()
                    .map(|p| {
                        let q = apply(&m, V3::new(p.x, p.y, dir * u).sub(pivot)).add(pivot);
                        let q = Point::new(q.x, q.y);
                        match kind {
                            ExtrudeKind::BigFront | ExtrudeKind::BigBack => {
                                q + (q - b.vanishing) * (d * t)
                            }
                            _ => q + (b.vanishing - q) * (t * d / (t * d + 1.0 - d)),
                        }
                    })
                    .collect();
                let cap = corners(f.above.last().expect("near cap"));
                assert_eq!(cap.len(), want.len(), "{kind:?}");
                for (got, want) in cap.iter().zip(&want) {
                    assert!((*got - *want).hypot() < 1e-6, "{kind:?} {off:?}");
                }
            }
        }
    }

    /// A turned parallel body is the turned prism seen straight on: no
    /// corner of a face leaves the ball the prism turns in (#2207).
    #[test]
    fn a_turned_parallel_body_keeps_every_corner_in_its_reach() {
        let shape = letter_k();
        let bb = shape.bounds();
        let vector = Vec2::new(10.776, 37.191);
        let c = bb.center() + vector / 2.0;
        for bevel in [0.0, 1.0] {
            for kind in [ExtrudeKind::BackParallel, ExtrudeKind::FrontParallel] {
                let mut b = body(
                    kind,
                    Solid {
                        rotation: [-17.0, 10.0, -8.0],
                        ..Default::default()
                    },
                    bevel,
                );
                b.vector = vector;
                let f = b.faces(&shape);
                assert!(f.above.len() > 3, "{kind:?}");
                let len = vector.hypot();
                // The prism's corners farthest from its middle.
                let reach = corners(&shape)
                    .into_iter()
                    .flat_map(|p| [p, p + vector])
                    .map(|p| (p - c).hypot().hypot(len / 2.0))
                    .fold(0.0, f64::max);
                for s in &f.above {
                    for p in corners(s) {
                        assert!((p - c).hypot() <= reach + 1e-6, "{kind:?} {bevel}: {p:?}");
                    }
                }
            }
        }
    }
}
