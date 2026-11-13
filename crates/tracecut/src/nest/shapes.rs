//! Shape nesting: objects placed by their true outlines, holes allowed.
//!
//! Each object becomes a region (its closed outlines flattened, open
//! curves by their boxes, holes filled unless parts may go into them),
//! grown by half the spacing with round corners and simplified within a
//! quarter of the tolerance; the growth carries 0.3 x the tolerance more
//! so the simplification never brings two objects closer than the
//! spacing. Each allowed turn of each region is drawn on a grid of cells
//! (a cell is set when the region touches it).
//!
//! Bottom-left fill: objects go largest first. For each sheet in turn and
//! each turn of the object, the grid gives the lowest, then leftmost,
//! cell position where no set cells meet; from there the object slides
//! down and left in true geometry (exact polygon overlap tests, a
//! binary search over up to three cells) until it touches. The turn whose
//! top ends lowest wins; an object that fits on no sheet opens a new one.
//!
//! The improvement loop then swaps two objects of the order at random
//! (seeded, so the same seed gives the same layout), nests again and keeps
//! the new order when the layout is no worse: fewer objects left out,
//! fewer sheets, less length on the last sheet, then a lower sum of tops.
//! It runs a set number of rounds and stops early when the caller says
//! time is up.

use super::poly::{bounds, overlap, Grid, Region};
use super::{Frame, Layout, Placement, Rng, SheetSpec};
use crate::subpaths;
use tracedraw_core::geometry::{flatten, Affine, BezPath, PathEl, Point, Rect, Shape as _, Vec2};
use tracedraw_core::shaping;

/// The turns an object may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Rotations {
    /// As it is.
    None,
    /// 0 and 180 degrees.
    Half,
    /// Every 90 degrees.
    #[default]
    Quarter,
    /// Every 45 degrees.
    Eighth,
}

impl Rotations {
    pub const ALL: [Rotations; 4] = [
        Rotations::None,
        Rotations::Half,
        Rotations::Quarter,
        Rotations::Eighth,
    ];

    /// The angles, degrees counter-clockwise.
    pub fn angles(self) -> Vec<f64> {
        match self {
            Rotations::None => vec![0.0],
            Rotations::Half => vec![0.0, 180.0],
            Rotations::Quarter => vec![0.0, 90.0, 180.0, 270.0],
            Rotations::Eighth => (0..8).map(|i| i as f64 * 45.0).collect(),
        }
    }
}

/// What shape nesting does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeSettings {
    pub sheet: SheetSpec,
    pub rotations: Rotations,
    /// Smaller objects may go into the holes of larger ones.
    pub holes: bool,
    /// Accuracy of the outlines (mm), 0.01 to 50.
    pub tolerance: f64,
    /// Rounds of the improvement loop (0: none).
    pub rounds: usize,
    pub seed: u64,
}

impl Default for ShapeSettings {
    fn default() -> Self {
        ShapeSettings {
            sheet: SheetSpec::default(),
            rotations: Rotations::Quarter,
            holes: true,
            tolerance: 0.5,
            rounds: 100,
            seed: 1,
        }
    }
}

/// One turn of an object.
#[derive(Debug, Clone)]
struct Orient {
    /// The grown region with its box's corner at (0, 0).
    region: Region,
    w: f64,
    h: f64,
    runs: Vec<Vec<(usize, usize)>>,
    /// Page to this local frame.
    local: Affine,
    angle: f64,
}

#[derive(Debug, Clone)]
struct Part {
    area: f64,
    key: f64,
    orients: Vec<Orient>,
}

/// Where one object went: sheet, turn and position in the usable area.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spot {
    sheet: usize,
    orient: usize,
    pos: Vec2,
}

/// How good a layout is: lower is better, compared in order.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Score {
    left_out: usize,
    sheets: usize,
    /// Sum of occupied bounding-box areas; lower scores favour compact groups.
    occupied_area: f64,
    last_length: f64,
    tops: f64,
}

impl Score {
    fn better_or_equal(&self, o: &Score) -> bool {
        use std::cmp::Ordering::*;
        let ord = self
            .left_out
            .cmp(&o.left_out)
            .then(self.sheets.cmp(&o.sheets))
            .then(self.occupied_area.total_cmp(&o.occupied_area))
            .then(self.last_length.total_cmp(&o.last_length))
            .then(self.tops.total_cmp(&o.tops));
        matches!(ord, Less | Equal)
    }
}

struct Attempt {
    spots: Vec<Option<Spot>>,
    score: Score,
}

const EPS: f64 = 1e-7;

/// The usable area of every sheet and the grid cell.
#[derive(Debug, Clone, Copy)]
struct Area {
    cw: f64,
    /// Infinite for a roll.
    ch: f64,
    cell: f64,
}

impl Area {
    fn new_grid(&self) -> Grid {
        let cols = (self.cw / self.cell).ceil().clamp(1.0, 1e6) as usize + 1;
        let rows = if self.ch.is_finite() {
            (self.ch / self.cell).ceil().clamp(1.0, 1e6) as usize + 2
        } else {
            usize::MAX
        };
        Grid::new(self.cell, cols, rows)
    }
}

/// The closed rings of one path in the normalised frame: closed
/// subpaths flattened, then open ones by their boxes (grown a little so
/// they have an area), with how many of the rings are closed subpaths.
fn rings_of(path: &BezPath, norm: Affine, tol: f64) -> (Vec<Vec<Point>>, usize) {
    let mut rings = Vec::new();
    let mut boxes = Vec::new();
    for sp in subpaths(path) {
        let p = norm * sp.path.clone();
        if sp.closed {
            let mut ring: Vec<Point> = Vec::new();
            flatten(&p, tol, &mut |el| match el {
                PathEl::MoveTo(q) | PathEl::LineTo(q) => ring.push(q),
                _ => {}
            });
            if ring.len() > 1 && ring.first() == ring.last() {
                ring.pop();
            }
            rings.push(ring);
        } else {
            let b = p.bounding_box().inflate(0.01, 0.01);
            boxes.push(vec![
                Point::new(b.x0, b.y0),
                Point::new(b.x1, b.y0),
                Point::new(b.x1, b.y1),
                Point::new(b.x0, b.y1),
            ]);
        }
    }
    let closed = rings.len();
    rings.extend(boxes);
    (rings, closed)
}

fn rings_to_path(rings: &[Vec<Point>]) -> BezPath {
    let mut p = BezPath::new();
    for r in rings {
        let mut it = r.iter();
        if let Some(first) = it.next() {
            p.move_to(*first);
            for q in it {
                p.line_to(*q);
            }
            p.close_path();
        }
    }
    p
}

/// The closed rings of a path, curves flattened within `tol` (a curve
/// read by its end points alone would cut across the outside of its
/// bulge and leave part of the outline out of the region).
fn path_to_rings(p: &BezPath, tol: f64) -> Vec<Vec<Point>> {
    let mut out = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    flatten(p, tol, &mut |el| match el {
        PathEl::MoveTo(q) => {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(q);
        }
        PathEl::LineTo(q) => cur.push(q),
        PathEl::ClosePath if !cur.is_empty() => out.push(std::mem::take(&mut cur)),
        _ => {}
    });
    if !cur.is_empty() {
        out.push(cur);
    }
    for r in &mut out {
        if r.len() > 1 && r.first() == r.last() {
            r.pop();
        }
    }
    out
}

/// Thin a closed ring within `tol`.
fn simplify_ring(r: &[Point], tol: f64) -> Vec<Point> {
    let Some(first) = r.first() else {
        return Vec::new();
    };
    let mut closed = r.to_vec();
    closed.push(*first);
    let mut s = tracedraw_core::geometry::simplify(&closed, tol);
    s.pop();
    s
}

/// The grown region of an object, before turning: each path a piece of
/// its own (even-odd inside it), the pieces welded.
fn grown_region(
    paths: &[BezPath],
    norm: Affine,
    s: &ShapeSettings,
    d: f64,
    tol: f64,
) -> Option<Region> {
    let mut all: Vec<Point> = Vec::new();
    let mut union: Option<BezPath> = None;
    let mut add = |piece: BezPath| {
        union = Some(match union.take() {
            None => piece,
            Some(u) => shaping::overlay(&u, &piece, shaping::Op::Weld),
        });
    };
    for path in paths {
        let (mut rings, closed) = rings_of(path, norm, tol / 20.0);
        all.extend(
            rings
                .iter()
                .flatten()
                .copied()
                .filter(|p| p.x.is_finite() && p.y.is_finite()),
        );
        // The boxes of open curves are pieces of their own: read
        // even-odd with the closed outlines, a box inside one would cut
        // a hole other parts could be nested into.
        let boxes = rings.split_off(closed.min(rings.len()));
        let mut region = Region::new(rings);
        if !s.holes {
            region = region.outer_only();
        }
        if !region.is_empty() {
            add(rings_to_path(&region.rings));
        }
        for b in boxes {
            let b = Region::new(vec![b]);
            if !b.is_empty() {
                add(rings_to_path(&b.rings));
            }
        }
    }
    if all.is_empty() {
        return None;
    }
    let union = union.unwrap_or_default();
    // The offset as the polygon it is worked out as: fitted curves read
    // by their end points cut the outside of every bulge off the region.
    let grown = shaping::offset_polygon(&union, d);
    let simple: Vec<Vec<Point>> = path_to_rings(&grown, tol / 20.0)
        .iter()
        .map(|r| simplify_ring(r, tol / 4.0))
        .collect();
    let region = Region::new(simple);
    if !region.is_empty() && region.area() > 1e-9 {
        return Some(region);
    }
    // Flat or empty outlines: their box, grown.
    let b = bounds(all.iter()).inflate(d.max(EPS), d.max(EPS));
    Some(Region::new(vec![vec![
        Point::new(b.x0, b.y0),
        Point::new(b.x1, b.y0),
        Point::new(b.x1, b.y1),
        Point::new(b.x0, b.y1),
    ]]))
}

fn prepare(
    paths: &[BezPath],
    norm: Affine,
    s: &ShapeSettings,
    d: f64,
    tol: f64,
    cell: f64,
) -> Option<Part> {
    let base = grown_region(paths, norm, s, d, tol)?;
    let key = base.area();
    let area = paths
        .iter()
        .map(|p| crate::measure::measure_object(std::slice::from_ref(p)).area)
        .filter(|a| a.is_finite())
        .sum::<f64>();
    let mut orients = Vec::new();
    for angle in s.rotations.angles() {
        let rot = Affine::rotate(angle.to_radians());
        let rings: Vec<Vec<Point>> = base
            .rings
            .iter()
            .map(|r| r.iter().map(|p| rot * *p).collect())
            .collect();
        let b = bounds(rings.iter().flatten());
        let shift = -b.origin().to_vec2();
        let region = Region::new(rings).moved(shift);
        if region.is_empty() {
            continue;
        }
        let (w, h) = (b.width(), b.height());
        let mut g = Grid::new(
            cell,
            (w / cell).ceil().clamp(1.0, 1e6) as usize + 2,
            usize::MAX,
        );
        g.paint(&region);
        orients.push(Orient {
            region,
            w,
            h,
            runs: g.runs(),
            local: Affine::translate(shift) * rot * norm,
            angle,
        });
    }
    (!orients.is_empty()).then_some(Part { area, key, orients })
}

#[derive(Debug, Clone)]
struct SheetState {
    grid: Grid,
    placed: Vec<Region>,
}

fn fits(o: &Orient, pos: Vec2, sheet: &SheetState, area: &Area) -> bool {
    if pos.x < -EPS || pos.y < -EPS || pos.x + o.w > area.cw + EPS || pos.y + o.h > area.ch + EPS {
        return false;
    }
    let bb = o.region.bbox + pos;
    let near: Vec<&Region> = sheet
        .placed
        .iter()
        .filter(|r| {
            r.bbox.x0 <= bb.x1 && bb.x0 <= r.bbox.x1 && r.bbox.y0 <= bb.y1 && bb.y0 <= r.bbox.y1
        })
        .collect();
    if near.is_empty() {
        return true;
    }
    let moved = o.region.moved(pos);
    !near.iter().any(|r| overlap(r, &moved))
}

/// The farthest `d` in `0..=max` that `ok` accepts (0 when none is).
fn slide(max: f64, ok: &mut dyn FnMut(f64) -> bool) -> f64 {
    if max <= EPS {
        return 0.0;
    }
    if ok(max) {
        return max;
    }
    let (mut lo, mut hi) = (0.0, max);
    for _ in 0..12 {
        let mid = (lo + hi) / 2.0;
        if ok(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The lowest, then leftmost, place for `o` on `sheet`, slid down and
/// left until it touches.
fn find_spot(o: &Orient, sheet: &SheetState, area: &Area) -> Option<Vec2> {
    if o.w > area.cw + EPS || o.h > area.ch + EPS {
        return None;
    }
    let c = area.cell;
    let max_kx = ((area.cw - o.w) / c + 1e-9).floor().max(0.0) as usize;
    let max_ky = if area.ch.is_finite() {
        ((area.ch - o.h) / c + 1e-9).floor().max(0.0) as usize
    } else {
        sheet.grid.used_rows()
    };
    let mut found = None;
    'rows: for ky in 0..=max_ky {
        let mut kx = 0usize;
        while kx <= max_kx {
            match sheet.grid.conflict(&o.runs, kx, ky) {
                Some(next) => kx = next.max(kx + 1),
                None => {
                    let pos = Vec2::new(kx as f64 * c, ky as f64 * c);
                    if fits(o, pos, sheet, area) {
                        found = Some(pos);
                        break 'rows;
                    }
                    kx += 1;
                }
            }
        }
    }
    let mut pos = found?;
    // Slide down and left in true geometry.
    let reach = 3.0 * c;
    for _ in 0..8 {
        let mut moved = false;
        let d = slide(reach.min(pos.y), &mut |d| {
            fits(o, pos - Vec2::new(0.0, d), sheet, area)
        });
        if d > 1e-6 {
            pos.y -= d;
            moved = true;
        }
        let d = slide(reach.min(pos.x), &mut |d| {
            fits(o, pos - Vec2::new(d, 0.0), sheet, area)
        });
        if d > 1e-6 {
            pos.x -= d;
            moved = true;
        }
        if !moved {
            break;
        }
    }
    Some(pos)
}

/// The best place for `part` on `sheet`: its turn and position.
fn best_on(part: &Part, sheet: &SheetState, area: &Area) -> Option<(usize, Vec2)> {
    let mut best: Option<(usize, Vec2, (f64, f64))> = None;
    for (oi, o) in part.orients.iter().enumerate() {
        if let Some(pos) = find_spot(o, sheet, area) {
            let score = (pos.y + o.h, pos.x);
            let better = best.as_ref().is_none_or(|(_, _, s)| {
                score.0 < s.0 - EPS || (score.0 <= s.0 + EPS && score.1 < s.1 - EPS)
            });
            if better {
                best = Some((oi, pos, score));
            }
        }
    }
    best.map(|(oi, pos, _)| (oi, pos))
}

/// Nest the parts in `order`. `None` when `out_of_time` says stop.
fn run(
    parts: &[Option<Part>],
    order: &[usize],
    area: &Area,
    roll: bool,
    out_of_time: &mut Option<&mut dyn FnMut() -> bool>,
) -> Option<Attempt> {
    let mut sheets: Vec<SheetState> = Vec::new();
    let mut spots: Vec<Option<Spot>> = vec![None; parts.len()];
    for &i in order {
        if let Some(stop) = out_of_time.as_mut() {
            if stop() {
                return None;
            }
        }
        let Some(Some(part)) = parts.get(i) else {
            continue;
        };
        // Do not blindly use the first sheet that accepts this part.
        // Compare the increase in each sheet's occupied bounding rectangle,
        // so a part can fill a useful gap on a later sheet instead of making
        // the first sheet wider/taller and leaving fragmented offcuts.
        let mut spot: Option<(usize, usize, Vec2, (f64, f64, f64, f64, f64, usize))> = None;
        for (si, sheet) in sheets.iter().enumerate() {
            let Some((oi, pos)) = best_on(part, sheet, area) else {
                continue;
            };
            let (used_right, used_top) = sheet.placed.iter().fold(
                (0.0_f64, 0.0_f64),
                |(right, top), placed| {
                    (right.max(placed.bbox.x1), top.max(placed.bbox.y1))
                },
            );
            let Some(orient) = part.orients.get(oi) else {
                continue;
            };
            let right = used_right.max(pos.x + orient.w);
            let top = used_top.max(pos.y + orient.h);
            let score = (right * top, top, right, pos.y, pos.x, si);
            if spot.as_ref().is_none_or(|(_, _, _, best)| score < *best) {
                spot = Some((si, oi, pos, score));
            }
        }
        let mut spot = spot.map(|(si, oi, pos, _)| (si, oi, pos));
        if spot.is_none() && (!roll || sheets.is_empty()) {
            let fresh = SheetState {
                grid: area.new_grid(),
                placed: Vec::new(),
            };
            if let Some((oi, pos)) = best_on(part, &fresh, area) {
                sheets.push(fresh);
                spot = Some((sheets.len() - 1, oi, pos));
            }
        }
        let Some((si, oi, pos)) = spot else {
            continue;
        };
        let (Some(sheet), Some(o)) = (sheets.get_mut(si), part.orients.get(oi)) else {
            continue;
        };
        let placed = o.region.moved(pos);
        sheet.grid.paint(&placed);
        sheet.placed.push(placed);
        if let Some(s) = spots.get_mut(i) {
            *s = Some(Spot {
                sheet: si,
                orient: oi,
                pos,
            });
        }
    }
    let n_sheets = sheets.len();
    let mut last_length: f64 = 0.0;
    let mut tops = 0.0;
    let mut left_out = 0;
    let mut sheet_extents = vec![(0.0_f64, 0.0_f64); n_sheets];
    for (i, s) in spots.iter().enumerate() {
        let Some(Some(part)) = parts.get(i) else {
            continue;
        };
        match s {
            None => left_out += 1,
            Some(s) => {
                let h = part.orients.get(s.orient).map_or(0.0, |o| o.h);
                tops += s.pos.y + h;
                if let (Some(extent), Some(orient)) = (
                    sheet_extents.get_mut(s.sheet),
                    part.orients.get(s.orient),
                ) {
                    extent.0 = extent.0.max(s.pos.x + orient.w);
                    extent.1 = extent.1.max(s.pos.y + orient.h);
                }
                if s.sheet + 1 == n_sheets {
                    last_length = last_length.max(s.pos.y + h);
                }
            }
        }
    }
    Some(Attempt {
        spots,
        score: Score {
            left_out,
            sheets: n_sheets,
            occupied_area: sheet_extents.iter().map(|(w, h)| w * h).sum(),
            last_length,
            tops,
        },
    })
}

/// Place each object, given by its page-space outlines, by its true
/// shape. `out_of_time` is asked between objects during the improvement
/// loop; the first layout is always finished.
pub fn nest_shapes(
    parts: &[Vec<BezPath>],
    s: &ShapeSettings,
    out_of_time: &mut dyn FnMut() -> bool,
) -> Layout {
    let mut layout = Layout {
        placements: vec![None; parts.len()],
        ..Default::default()
    };
    let Some(spec) = s.sheet.sane() else {
        return layout;
    };
    let tol = if s.tolerance.is_finite() {
        s.tolerance.clamp(0.01, 50.0)
    } else {
        0.5
    };
    let d = spec.spacing / 2.0 + 0.3 * tol;
    // The grown regions hold each outline grown by at least half the
    // spacing (the rest of `d` covers flattening and simplifying).
    let inset = spec.margin - spec.spacing / 2.0;
    let (across, along) = spec.across_along();
    let cw = across - 2.0 * inset;
    let ch = along - 2.0 * inset;
    if !(cw > EPS && ch > EPS) {
        return layout;
    }
    let area = Area {
        cw,
        ch,
        // Keep the search grid fine enough for print layouts. At 320 mm,
        // the old 256-column cap forced 1.25 mm placement steps and skipped
        // narrow but usable gaps. 1024 columns reduce that to about 0.31 mm.
        cell: tol.max(cw / 1024.0),
    };
    let frame = Frame::new(&spec);
    let norm = frame.to_norm();
    let prepared: Vec<Option<Part>> = parts
        .iter()
        .map(|p| prepare(p, norm, s, d, tol, area.cell))
        .collect();
    let mut order: Vec<usize> = (0..parts.len())
        .filter(|i| prepared[*i].is_some())
        .collect();
    let key = |i: usize| {
        prepared
            .get(i)
            .and_then(|p| p.as_ref())
            .map_or(0.0, |p| p.key)
    };
    order.sort_by(|a, b| key(*b).total_cmp(&key(*a)).then(a.cmp(b)));

    let Some(mut best) = run(&prepared, &order, &area, spec.roll, &mut None) else {
        return layout;
    };
    let n = order.len();
    let mut rng = Rng::new(s.seed);
    if n >= 2 {
        for _ in 0..s.rounds {
            if out_of_time() {
                layout.stopped_by_time = true;
                break;
            }
            let mut cand = order.clone();
            let i = rng.below(n);
            let mut j = rng.below(n - 1);
            if j >= i {
                j += 1;
            }
            if rng.below(3) == 0 {
                // Move one object to another place in the order.
                let x = cand.remove(i);
                cand.insert(j.min(cand.len()), x);
            } else {
                cand.swap(i, j);
            }
            let attempt = run(
                &prepared,
                &cand,
                &area,
                spec.roll,
                &mut Some(&mut *out_of_time),
            );
            layout.rounds += 1;
            let Some(attempt) = attempt else {
                layout.stopped_by_time = true;
                break;
            };
            if attempt.score.better_or_equal(&best.score) {
                best = attempt;
                order = cand;
            }
        }
    }

    let mut extents = Vec::new();
    for (i, spot) in best.spots.iter().enumerate() {
        let (Some(spot), Some(Some(part))) = (spot, prepared.get(i)) else {
            continue;
        };
        let Some(o) = part.orients.get(spot.orient) else {
            continue;
        };
        let transform = frame.place(spot.sheet, inset, spot.pos, o.local);
        if let Some(p) = layout.placements.get_mut(i) {
            *p = Some(Placement {
                sheet: spot.sheet,
                transform,
                angle: o.angle,
            });
        }
        let to_sheet = frame.to_sheet(spot.sheet) * transform;
        let far = parts
            .get(i)
            .and_then(|p| placed_bounds(p, to_sheet))
            .map_or(inset + spot.pos.y + o.h, |b| b.y1);
        extents.push((spot.sheet, far));
        layout.parts_area += part.area;
    }
    layout.finish(&spec, &extents);
    layout
}

/// The page-space box of `paths` after `t` (for callers and tests).
pub fn placed_bounds(paths: &[BezPath], t: Affine) -> Option<Rect> {
    paths
        .iter()
        .map(|p| (t * p.clone()).bounding_box())
        .reduce(|a, b| a.union(b))
}

#[cfg(test)]
mod tests {
    use super::super::rects::{nest_rects, RectItem, RectSettings};
    use super::*;
    use tracedraw_core::geometry::{Circle, Rect};

    fn poly(pts: &[(f64, f64)]) -> BezPath {
        let mut p = BezPath::new();
        for (i, q) in pts.iter().enumerate() {
            if i == 0 {
                p.move_to(*q);
            } else {
                p.line_to(*q);
            }
        }
        p.close_path();
        p
    }

    fn square(x: f64, y: f64, s: f64) -> BezPath {
        poly(&[(x, y), (x + s, y), (x + s, y + s), (x, y + s)])