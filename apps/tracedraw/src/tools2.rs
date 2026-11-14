//! Second wave of tools: Contour, Crop, Knife, Spiral, Common Shapes,
//! Brush Strokes (calligraphic), Parallel Dimension, Connector, Table,
//! and Convert Outline To Object. These produce static results; see
//! docs/features.md.

use crate::app::{App, Drag};
use crate::tools::Tool;
use egui::{Modifiers, PointerButton, Response};
use tracedraw_core::{
    document::{Shape, ShapeKind},
    geometry::{BezPath, Point, Rect, Shape as _, Vec2},
    Command, Fill, ShapeId, Stroke,
};

pub use crate::common_shapes::CommonShape;

/// Archimedean or logarithmic spiral inscribed in `rect`.
pub fn spiral_path(rect: Rect, revolutions: u32, logarithmic: bool) -> BezPath {
    let c = rect.center();
    let (rx, ry) = (rect.width() / 2.0, rect.height() / 2.0);
    let turns = revolutions.max(1) as f64;
    let steps = (turns * 48.0) as usize;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let a = t * turns * std::f64::consts::TAU;
        let r = if logarithmic {
            (t * 4.0).exp() / 4.0f64.exp()
        } else {
            t
        };
        pts.push(Point::new(c.x + rx * r * a.cos(), c.y + ry * r * a.sin()));
    }
    tracedraw_core::geometry::smooth_path(&pts, false)
}

/// A grid of `rows` x `cols` cells covering `rect`: the outer frame and
/// the inner lines (previews of Graph Paper and Table).
pub fn grid_path(rect: Rect, rows: u32, cols: u32) -> BezPath {
    let mut p = rect.to_path(0.01);
    let (rows, cols) = (rows.max(1), cols.max(1));
    for c in 1..cols {
        let x = rect.x0 + rect.width() * c as f64 / cols as f64;
        p.move_to((x, rect.y0));
        p.line_to((x, rect.y1));
    }
    for r in 1..rows {
        let y = rect.y0 + rect.height() * r as f64 / rows as f64;
        p.move_to((rect.x0, y));
        p.line_to((rect.x1, y));
    }
    p
}

/// The Action Lines tool's speed lines across `rect`: parallel (ending at the
/// right edge, random lengths) or radial (around the centre). The lengths
/// come from a fixed pseudo-random sequence, so the live preview and the
/// object created on release are the same.
pub fn action_lines_path(rect: Rect, lines: u32, radial: bool) -> BezPath {
    let n = lines.clamp(2, 500) as usize;
    let mut path = BezPath::new();
    let mut seed: u32 = 0x9E37_79B9 ^ n as u32;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 1000) as f64 / 1000.0
    };
    if radial {
        let c = rect.center();
        let rmax = (rect.width().min(rect.height())) / 2.0;
        for i in 0..n {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            let dir = Vec2::new(a.cos(), a.sin());
            let r0 = rmax * (0.25 + 0.35 * rnd());
            let r1 = rmax * (0.75 + 0.25 * rnd());
            path.move_to(c + dir * r0);
            path.line_to(c + dir * r1);
        }
    } else {
        for i in 0..n {
            let y = rect.y0 + rect.height() * (i as f64 + 0.5) / n as f64;
            let len = rect.width() * (0.3 + 0.7 * rnd());
            path.move_to(Point::new(rect.x1 - len, y));
            path.line_to(Point::new(rect.x1, y));
        }
    }
    path
}

impl App {
    pub fn tools2_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        match self.tool {
            Tool::Knife if self.knife.mode == crate::knife::KnifeMode::Freehand => {
                if response.drag_started_by(PointerButton::Primary) {
                    self.drag = Drag::Freehand { points: vec![p] };
                }
                if response.dragged_by(PointerButton::Primary) {
                    if let Drag::Freehand { points } = &mut self.drag {
                        if points
                            .last()
                            .map(|l| (*l - p).hypot() > 0.2)
                            .unwrap_or(true)
                        {
                            points.push(p);
                        }
                    }
                }
            }
            Tool::Knife => {
                let p = self.snap_point(p);
                if response.drag_started_by(PointerButton::Primary) {
                    self.drag = Drag::Box {
                        start: p,
                        current: p,
                        from_center: false,
                    };
                }
                if response.dragged_by(PointerButton::Primary) {
                    if let Drag::Box { current, .. } = &mut self.drag {
                        *current = p;
                    }
                }
            }
            // Cells, lines and edge pointers of a table; elsewhere a drag
            // draws a new table (`table_tool.rs`).
            Tool::Table => self.table_tool_input(response, p, mods),
            Tool::Spiral | Tool::CommonShapes => self.box_like_input(response, p, mods),
            Tool::BrushStrokes | Tool::PainterlyBrush => {
                if self.tool == Tool::PainterlyBrush && response.clicked_by(PointerButton::Primary)
                {
                    // A click picks the object to paint with the bar.
                    match self.hit_test(p) {
                        Some(id) => self.select(vec![id]),
                        None => self.selection.clear(),
                    }
                }
                if response.drag_started_by(PointerButton::Primary) {
                    self.drag = Drag::Freehand { points: vec![p] };
                    self.pen_stroke = crate::pen::PenStroke::start(self.pen_pressure);
                }
                if response.dragged_by(PointerButton::Primary) {
                    if let Drag::Freehand { points } = &mut self.drag {
                        if points
                            .last()
                            .map(|l| (*l - p).hypot() > 0.3)
                            .unwrap_or(true)
                        {
                            points.push(p);
                            let n = points.len();
                            self.pen_stroke.record(n, self.pen_pressure);
                        }
                    }
                }
            }
            Tool::ParallelDimension => self.dimension_input(response, p),
            Tool::Connector => {
                if response.drag_started_by(PointerButton::Primary) {
                    if let Some(id) = self.hit_test_inside(p) {
                        self.drag = Drag::Connector {
                            from: id,
                            start: p,
                            current: p,
                        };
                    }
                }
                if response.dragged_by(PointerButton::Primary) {
                    if let Drag::Connector { current, .. } = &mut self.drag {
                        *current = p;
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn box_like_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        let p = self.snap_point(p);
        if response.drag_started_by(PointerButton::Primary) {
            self.drag = Drag::Box {
                start: p,
                current: p,
                from_center: mods.shift,
            };
            self.box_regular = false;
        }
        if response.dragged_by(PointerButton::Primary) {
            self.box_regular = mods.ctrl;
            // Ctrl keeps a shape of My shapes in its own proportions.
            let aspect = match self.tool {
                Tool::CommonShapes => self.picked_my_shape().map(|m| m.aspect),
                _ => None,
            };
            if let Drag::Box {
                start,
                current,
                from_center,
            } = &mut self.drag
            {
                let mut q = p;
                if let Some(a) = aspect.filter(|_| mods.ctrl) {
                    q = crate::my_shapes::keep_proportions(a, *start, q);
                } else if mods.ctrl {
                    let d = q - *start;
                    let m = d.x.abs().max(d.y.abs());
                    q = *start + Vec2::new(m * d.x.signum(), m * d.y.signum());
                }
                *from_center = mods.shift;
                *current = q;
            }
        }
    }

    /// The object a box drag with the current tool would create, as page
    /// space outlines, so the shape itself follows the pointer while
    /// dragging (shape tools never show an empty box). `None` when the drag does not create an object outline
    /// (crop, zoom, text frames).
    pub fn creation_preview(
        &self,
        start: Point,
        current: Point,
        from_center: bool,
    ) -> Option<BezPath> {
        use tracedraw_core::geometry::{ellipse_arc_path, ellipse_path, polygon_path};
        let r = App::box_rect(start, current, from_center);
        let path = match self.tool {
            Tool::Rectangle => self.rect_corners.path(r, (1.0, 1.0)),
            Tool::Ellipse => match &self.ellipse_arc {
                None => ellipse_path(r),
                Some(a) => ellipse_arc_path(r, a.start_deg, a.end_deg, a.pie),
            },
            Tool::Polygon => polygon_path(
                self.polygon_model_rect(r, self.polygon_points, 0.0),
                self.polygon_points,
                0.0,
            ),
            Tool::Star if self.star_complex => tracedraw_core::geometry::complex_star_path(
                self.polygon_model_rect(r, self.complex_points, 0.0),
                self.complex_points,
                self.complex_sharpness,
            ),
            Tool::Star => polygon_path(
                self.polygon_model_rect(r, self.polygon_points, self.star_sharpness),
                self.polygon_points,
                self.star_sharpness,
            ),
            Tool::Spiral => spiral_path(r, self.spiral_revolutions, self.spiral_logarithmic),
            Tool::CommonShapes => match self.picked_my_shape() {
                Some(mine) => mine.path(r),
                None => self.common_shape.path(r),
            },
            Tool::GraphPaper => grid_path(r, self.graph_rows, self.graph_cols),
            Tool::ActionLines => {
                action_lines_path(r, self.action_lines_count, self.action_lines_radial)
            }
            Tool::Table => grid_path(r, self.table_rows, self.table_cols),
            Tool::Knife => {
                let mut p = BezPath::new();
                p.move_to(start);
                p.line_to(current);
                p
            }
            _ => return None,
        };
        Some(path)
    }

    /// Box drags for the second-wave tools, called from `end_drag`.
    pub fn finish_tools2_box(&mut self, start: Point, current: Point) -> bool {
        let rect = Rect::from_points(start, current);
        match self.tool {
            Tool::Spiral => {
                if rect.width() > 0.1 && rect.height() > 0.1 {
                    let path = spiral_path(rect, self.spiral_revolutions, self.spiral_logarithmic);
                    if let Some(id) = self.new_shape(ShapeKind::Path {
                        path,
                        closed: false,
                    }) {
                        self.select(vec![id]);
                    }
                }
                true
            }
            Tool::CommonShapes => {
                if rect.width() > 0.1 && rect.height() > 0.1 {
                    // A shape of My shapes is filled when its outline was
                    // closed, as the objects it was made from.
                    let (path, closed) = match self.picked_my_shape() {
                        Some(mine) => (mine.path(rect), mine.closed),
                        None => (self.common_shape.path(rect), true),
                    };
                    if let Some(id) = self.new_shape(ShapeKind::Path { path, closed }) {
                        self.select(vec![id]);
                    }
                }
                true
            }
            Tool::Table => {
                if rect.width() > 1.0 && rect.height() > 1.0 {
                    self.create_table(rect);
                }
                true
            }
            Tool::Knife => {
                if (current - start).hypot() > 0.5 {
                    self.knife(start, current);
                }
                true
            }
            Tool::SegmentDelete => {
                // A click whose pointer slipped a few pixels is still a
                // click: when its little box reaches nothing, the outline
                // near the press goes as a click would take it.
                let zoom = self.view.zoom.max(1e-6) as f64;
                let small = rect.width() * zoom < 8.0 && rect.height() * zoom < 8.0;
                if !self.delete_virtual_segments_in(rect) && small {
                    self.delete_virtual_segment_at(start);
                }
                true
            }
            _ => false,
        }
    }

    /// The Connector tool's drop: a straight connector to the object under
    /// the drop.
    pub fn finish_connector(&mut self, from: ShapeId, start: Point, at: Point) {
        if let Some(to) = self.hit_test_inside(at).filter(|id| *id != from) {
            self.connect(
                from,
                start,
                to,
                at,
                crate::connectors::ConnectorKind::Straight,
            );
        }
    }

    pub fn create_table(&mut self, rect: Rect) {
        let (rows, cols) = (self.table_rows.max(1), self.table_cols.max(1));
        let mut table = tracedraw_core::Table::new(rect, rows, cols, self.table_border.clone());
        table.cell_fill = self.table_fill.clone();
        if let Some(id) = self.new_shape(ShapeKind::Table(table)) {
            self.select(vec![id]);
        }
    }

    /// Ctrl+Shift+Q: replace an outline by a filled object of its shape.
    pub fn convert_outline_to_object(&mut self) {
        let shapes = self.selected_shapes();
        let Some(layer) = self.active_layer() else {
            return;
        };
        let mut cmds = Vec::new();
        let mut ids = Vec::new();
        for s in shapes {
            let Some(st) = &s.stroke else { continue };
            let Some(out) = outline_object_path(&s) else {
                continue;
            };
            let id = self.engine.new_shape_id();
            let mut o = Shape::new(
                id,
                ShapeKind::Path {
                    path: out,
                    closed: true,
                },
            );
            o.fill = Fill::Solid(st.color);
            o.stroke = None;
            o.name = Some("Outline".into());
            // The new object takes the outline's place: right above its
            // object in that object's layer, so the stacking order stays.
            let placed = self.doc().locate(s.id).ok();
            cmds.push(Command::AddShape {
                layer: placed.map(|(l, _)| l).unwrap_or(layer),
                shape: o,
            });
            if placed.is_some() {
                cmds.push(Command::OrderRelative {
                    shapes: vec![id],
                    reference: s.id,
                    in_front: true,
                });
            }
            // An object whose fill draws nothing (no fill, or an open
            // curve) would stay behind invisible: the outline replaces it
            // (#999).
            if fill_draws(&s, self.doc().metadata.fill_open_curves) {
                cmds.push(Command::SetStroke {
                    shapes: vec![s.id],
                    stroke: None,
                });
            } else {
                cmds.push(Command::DeleteShapes { shapes: vec![s.id] });
            }
            ids.push(id);
        }
        if !cmds.is_empty() {
            let _ = self.engine.run_batch("Convert Outline To Object", &cmds);
            self.select(ids);
        }
    }
}

/// Whether the fill of `s` draws anything: it has one, and an open curve
/// (or an ellipse made an arc) fills only its closed subpaths unless the
/// document fills open curves
/// (`fill_open`), as the renderer does.
fn fill_draws(s: &Shape, fill_open: bool) -> bool {
    if matches!(s.fill, Fill::None) {
        return false;
    }
    // A curve left open, or an ellipse made an arc.
    fill_open
        || !s.kind.is_open_curve()
        || !tracedraw_render::closed_subpaths(&s.local_path())
            .elements()
            .is_empty()
}

/// How far (mm) the curves of a converted outline may stray from the
/// outline drawn: the offset of each curve is fitted within it, and a
/// band (variable or calligraphic outline) is fitted to it.
const OUTLINE_FIT_TOLERANCE: f64 = 0.01;

/// The filled shape of a constant-width outline `width` mm wide along
/// `path` (page space), with the joins and ends of `st`. Each segment of
/// the path is offset to either side, so a curve stays a curve (fitted
/// within [`OUTLINE_FIT_TOLERANCE`], as few as that takes) and a line a
/// line, and the outline has about as many nodes as the path: it is not
/// flattened. Round ends and round corners are arcs of a circle around
/// the path's end or corner, with nodes where they meet the sides and at
/// each end's tip (#623). The pieces are united, so where the outline
/// overlaps itself it covers once. `None` when it draws nothing.
fn offset_outline(path: &BezPath, width: f64, st: &Stroke) -> Option<BezPath> {
    use tracedraw_core::shaping::{overlay_with, simplify, FillRule, Op};
    use tracedraw_core::{LineCap, LineJoin};
    let path = closed_where_they_meet(path);
    let join = match st.join {
        LineJoin::Miter => kurbo::Join::Miter,
        LineJoin::Round | LineJoin::Bevel => kurbo::Join::Bevel,
    };
    let cap = match st.cap {
        LineCap::Square => kurbo::Cap::Square,
        LineCap::Round | LineCap::Butt => kurbo::Cap::Butt,
    };
    let style = kurbo::Stroke::new(width)
        .with_join(join)
        .with_caps(cap)
        .with_miter_limit(st.miter_ratio());
    let body = kurbo::stroke(
        path.iter(),
        &style,
        &kurbo::StrokeOpts::default(),
        OUTLINE_FIT_TOLERANCE,
    );
    // Round ends and corners: the circle the outline's pen draws there.
    let half = width / 2.0;
    let mut discs = BezPath::new();
    if st.cap == LineCap::Round {
        for (p, t) in open_ends(&path) {
            disc(&mut discs, p, half, t);
        }
    }
    if st.join == LineJoin::Round {
        for (p, u, v) in path_turns(&path) {
            // A node where the arc leaves the side coming in, on the
            // outer side of the turn: the arc to the side going out is
            // one curve for a turn of up to a right angle.
            let from = if u.cross(v) > 0.0 {
                Vec2::new(u.y, -u.x)
            } else {
                Vec2::new(-u.y, u.x)
            };
            disc(&mut discs, p, half, from);
        }
    }
    let out = if discs.elements().is_empty() {
        simplify(&body)
    } else {
        overlay_with(&body, &discs, Op::Weld, FillRule::NonZero)
    };
    (!out.elements().is_empty()).then(|| with_closing_lines(&out))
}

/// `path` with every closed subpath drawn back to its start by a segment
/// of its own before it closes, so its last side is a line like the
/// others (a node at each corner, none left to the close).
fn with_closing_lines(path: &BezPath) -> BezPath {
    use tracedraw_core::geometry::PathEl;
    let mut out = BezPath::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => (start, last) = (p, p),
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => last = p,
            PathEl::ClosePath => {
                if (last - start).hypot() > 1e-9 {
                    out.line_to(start);
                }
                last = start;
            }
        }
        out.push(*el);
    }
    out
}

/// A circle of radius `r` around `c`, as four quarter curves turning
/// one way, with its nodes where the unit vector `from` points and a
/// quarter, a half and three quarters of a turn on from there.
fn disc(out: &mut BezPath, c: Point, r: f64, from: Vec2) {
    let len = from.hypot();
    if !(r > 1e-9 && len > 1e-12 && r.is_finite()) {
        return;
    }
    // The handle length of a quarter circle drawn as one cubic curve.
    let k = 4.0 / 3.0 * (std::f64::consts::PI / 8.0).tan() * r;
    let turn = |d: Vec2| Vec2::new(-d.y, d.x);
    let mut d = from / len;
    out.move_to(c + d * r);
    for _ in 0..4 {
        let e = turn(d);
        let (p0, p3) = (c + d * r, c + e * r);
        out.curve_to(p0 + turn(d) * k, p3 - turn(e) * k, p3);
        d = e;
    }
    out.close_path();
}

/// `path` with every subpath that ends where it starts closed: its
/// outline goes round without two ends meeting there (an ellipse drawn
/// open made a notch, #622).
fn closed_where_they_meet(path: &BezPath) -> BezPath {
    use tracedraw_core::geometry::PathEl;
    let mut out = BezPath::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let (mut segments, mut closed) = (0usize, true);
    let finish = |out: &mut BezPath, start: Point, last: Point, segments: usize, closed: bool| {
        if !closed && segments > 1 && (last - start).hypot() < 1e-6 {
            out.close_path();
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                finish(&mut out, start, last, segments, closed);
                (start, last, segments, closed) = (p, p, 0, false);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                last = p;
                segments += 1;
            }
            PathEl::ClosePath => closed = true,
        }
        out.push(*el);
    }
    finish(&mut out, start, last, segments, closed);
    out
}

/// The way a segment drawn through `pts` (its start, control points and
/// end) leaves its start and arrives at its end, as unit vectors; `None`
/// for a segment of no length.
fn segment_dirs(pts: &[Point]) -> Option<(Vec2, Vec2)> {
    let (&a, &b) = (pts.first()?, pts.last()?);
    let unit = |v: Vec2| (v.hypot() > 1e-9).then(|| v / v.hypot());
    let first = pts.iter().skip(1).find_map(|p| unit(*p - a))?;
    let end = pts.iter().rev().skip(1).find_map(|p| unit(b - *p))?;
    Some((first, end))
}

/// The ends of the open subpaths of `path`, each with the way its
/// subpath runs there (out of the start, on past the end).
fn open_ends(path: &BezPath) -> Vec<(Point, Vec2)> {
    use tracedraw_core::geometry::PathEl;
    let mut out = Vec::new();
    // The open subpath in hand: its start with the way it leaves it, and
    // its end with the way it arrives.
    let mut first: Option<(Point, Vec2)> = None;
    let mut end: Option<(Point, Vec2)> = None;
    let mut last = Point::ZERO;
    let flush = |out: &mut Vec<(Point, Vec2)>, first: &mut Option<_>, end: &mut Option<_>| {
        if let (Some((a, ta)), Some((b, tb))) = (first.take(), end.take()) {
            let ta: Vec2 = ta;
            out.push((a, -ta));
            out.push((b, tb));
        }
    };
    for el in path.elements() {
        let pts: Vec<Point> = match *el {
            PathEl::MoveTo(p) => {
                flush(&mut out, &mut first, &mut end);
                last = p;
                continue;
            }
            PathEl::ClosePath => {
                // A closed subpath has no ends.
                first = None;
                end = None;
                continue;
            }
            PathEl::LineTo(p) => vec![last, p],
            PathEl::QuadTo(a, p) => vec![last, a, p],
            PathEl::CurveTo(a, b, p) => vec![last, a, b, p],
        };
        if let Some((s, e)) = segment_dirs(&pts) {
            if first.is_none() {
                first = Some((last, s));
            }
            end = Some((pts[pts.len() - 1], e));
        }
        last = pts[pts.len() - 1];
    }
    flush(&mut out, &mut first, &mut end);
    out
}

/// The nodes of `path` where its direction turns: the end of one segment
/// leaves at another angle than the next one starts; with the way the
/// path comes in and goes on there (unit vectors).
fn path_turns(path: &BezPath) -> Vec<(Point, Vec2, Vec2)> {
    use tracedraw_core::geometry::PathEl;
    // Each segment's start and end with the directions there.
    let mut segs: Vec<(Point, Vec2, Point, Vec2)> = Vec::new();
    let mut subs: Vec<(usize, usize, bool)> = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let push = |segs: &mut Vec<(Point, Vec2, Point, Vec2)>, pts: &[Point]| {
        if let (Some((f, e)), Some(&a), Some(&b)) = (segment_dirs(pts), pts.first(), pts.last()) {
            segs.push((a, f, b, e));
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                subs.push((segs.len(), segs.len(), false));
                (start, last) = (p, p);
            }
            PathEl::LineTo(p) => {
                push(&mut segs, &[last, p]);
                last = p;
            }
            PathEl::QuadTo(a, p) => {
                push(&mut segs, &[last, a, p]);
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                push(&mut segs, &[last, a, b, p]);
                last = p;
            }
            PathEl::ClosePath => {
                push(&mut segs, &[last, start]);
                last = start;
                if let Some(sub) = subs.last_mut() {
                    sub.2 = true;
                }
            }
        }
        if let Some(sub) = subs.last_mut() {
            sub.1 = segs.len();
        }
    }
    let turns = |a: &(Point, Vec2, Point, Vec2), b: &(Point, Vec2, Point, Vec2)| {
        a.3.cross(b.1).atan2(a.3.dot(b.1)).abs() > 0.02
    };
    let mut out = Vec::new();
    for (from, to, closed) in subs {
        let Some(sub) = segs.get(from..to) else {
            continue;
        };
        out.extend(
            sub.windows(2)
                .filter(|w| turns(&w[0], &w[1]))
                .map(|w| (w[0].2, w[0].3, w[1].1)),
        );
        if let (Some(first), Some(end)) = (sub.first(), sub.last()) {
            let meets = closed || (first.0 - end.2).hypot() < 1e-6;
            if meets && turns(end, first) {
                out.push((first.0, end.3, first.1));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Effects tools: Blend, Extrude, Distort; brush tools: Smooth, Smear, Twirl;
// Freehand Pick lasso.
// ---------------------------------------------------------------------------

impl App {
    pub fn effects_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        match self.tool {
            Tool::Blend => {
                if response.drag_started_by(PointerButton::Primary) {
                    // The press point, not where the drag was noticed.
                    let press = response
                        .ctx
                        .input(|i| i.pointer.press_origin())
                        .map(|o| self.view.to_page(o))
                        .unwrap_or(p);
                    self.blend_handle_drag = None;
                    if self.begin_blend_handle(press) {
                        // A slider of the selected blend: no new blend.
                    } else if let Some(id) = self.hit_test_inside(p) {
                        self.drag = Drag::Connector {
                            from: id,
                            start: p,
                            current: p,
                        };
                    }
                }
                if response.dragged_by(PointerButton::Primary) {
                    if let Some(h) = self.blend_handle_drag {
                        self.drag_blend_handle_to(h, p, mods.ctrl || mods.command);
                    } else if let Drag::Connector { current, .. } = &mut self.drag {
                        *current = p;
                    }
                }
                if response.drag_stopped() {
                    self.finish_blend_end_drag(p);
                    self.blend_handle_drag = None;
                }
            }
            Tool::Smooth | Tool::Smear | Tool::Twirl => self.brush_input(response, p, mods),
            _ => {}
        }
    }
}

/// Even-odd point-in-polygon on a flattened path.
pub fn point_in_path(path: &BezPath, p: Point) -> bool {
    let pts = tracedraw_core::effects::resample(path, 256);
    let mut inside = false;
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (pts[i], pts[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y + 1e-12) + a.x
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(p: &BezPath) -> Rect {
        p.bounding_box()
    }

    /// Convert Outline to Object on a dashed line keeps the dashes: each
    /// becomes a closed shape and the gaps stay empty (#490).
    #[test]
    fn a_dashed_outline_becomes_one_shape_per_dash() {
        let mut app = App::headless();
        let mut line = BezPath::new();
        line.move_to((10.0, 10.0));
        line.line_to((70.0, 10.0));
        let id = app
            .new_shape(ShapeKind::Path {
                path: line,
                closed: false,
            })
            .expect("line");
        let mut st = Stroke::new(tracedraw_core::Color::BLACK, 1.0);
        // 4 on, 2 off, in widths: 10 dashes over 60 mm.
        st.dash = vec![4.0, 2.0];
        st.cap = tracedraw_core::LineCap::Butt;
        app.select(vec![id]);
        app.run(Command::SetStroke {
            shapes: vec![id],
            stroke: Some(st),
        });
        app.convert_outline_to_object();
        let out = app
            .doc()
            .find_shape(app.selection[0])
            .cloned()
            .expect("object");
        let ShapeKind::Path { path, .. } = &out.kind else {
            panic!("a curve");
        };
        let moves = path
            .elements()
            .iter()
            .filter(|e| matches!(e, kurbo::PathEl::MoveTo(_)))
            .count();
        assert_eq!(moves, 10, "one closed shape per dash");
        // The gap between the first two dashes (4 to 6 mm in) is empty.
        let pp = out.page_path();
        assert!(!kurbo::Shape::contains(&pp, Point::new(15.0, 10.0)));
        assert!(kurbo::Shape::contains(&pp, Point::new(12.0, 10.0)));
    }

    /// A combined curve of two crossing open lines (an "f": a stem and a
    /// cross bar) becomes one shape covering both strokes, the crossing
    /// filled under the even-odd rule curves are filled with, as it was
    /// drawn before (#997).
    #[test]
    fn crossing_subpaths_become_one_filled_shape() {
        let mut path = BezPath::new();
        path.move_to((20.0, 50.0));
        path.curve_to((30.0, 50.0), (28.0, 10.0), (40.0, 10.0));
        path.move_to((15.0, 30.0));
        path.line_to((45.0, 30.0));
        let kind = ShapeKind::Path {
            path,
            closed: false,
        };
        let (_, out) = outline_object(kind, 4.0, |st| {
            st.cap = tracedraw_core::LineCap::Round;
            st.join = tracedraw_core::LineJoin::Round;
        });
        let moves = out
            .elements()
            .iter()
            .filter(|e| matches!(e, kurbo::PathEl::MoveTo(_)))
            .count();
        assert_eq!(moves, 1, "one merged outline");
        let even_odd = |p: Point| out.winding(p) % 2 != 0;
        // Where the stem crosses the bar, and on each stroke away from it.
        let stem_at_bar = (0..=200)
            .map(|i| i as f64 / 200.0)
            .map(|t| {
                kurbo::ParamCurve::eval(
                    &kurbo::CubicBez::new((20.0, 50.0), (30.0, 50.0), (28.0, 10.0), (40.0, 10.0)),
                    t,
                )
            })
            .min_by(|a, b| (a.y - 30.0).abs().total_cmp(&(b.y - 30.0).abs()))
            .expect("point");
        assert!(even_odd(stem_at_bar), "the crossing is filled");
        assert!(even_odd(Point::new(17.0, 30.0)));
        assert!(even_odd(Point::new(43.0, 30.0)));
        assert!(even_odd(Point::new(21.0, 50.0)));
        assert!(!even_odd(Point::new(40.0, 40.0)));
    }

    /// Three rectangles stacked, the middle one with a 2 mm outline and
    /// `fill`: the app, the layer's ids bottom to top and the middle id.
    fn stacked_with_outline(fill: Fill) -> (App, Vec<ShapeId>, ShapeId) {
        let mut app = App::headless();
        let mut ids = Vec::new();
        for i in 0..3 {
            let x = 10.0 + 10.0 * i as f64;
            let id = app
                .new_shape(ShapeKind::Rect {
                    rect: Rect::new(x, 10.0, x + 30.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("rect");
            ids.push(id);
        }
        let mid = ids[1];
        app.run(Command::SetFill {
            shapes: vec![mid],
            fill,
        });
        app.run(Command::SetStroke {
            shapes: vec![mid],
            stroke: Some(Stroke::new(tracedraw_core::Color::BLACK, 2.0)),
        });
        app.select(vec![mid]);
        (app, ids, mid)
    }

    fn layer_ids(app: &App) -> Vec<ShapeId> {
        let layer = app.active_layer().expect("layer");
        app.doc()
            .layer(layer)
            .expect("layer")
            .shapes
            .iter()
            .map(|s| s.id)
            .collect()
    }

    /// An object without a fill is replaced by its outline: one object,
    /// where the original was in the stacking order, one undo step
    /// (#999).
    #[test]
    fn an_unfilled_object_is_replaced_by_its_outline() {
        let (mut app, ids, mid) = stacked_with_outline(Fill::None);
        app.convert_outline_to_object();
        let new = app.selection[0];
        assert_eq!(layer_ids(&app), vec![ids[0], new, ids[2]]);
        assert!(
            app.doc().find_shape(mid).is_none(),
            "no invisible object left"
        );
        app.undo();
        assert_eq!(layer_ids(&app), ids);
        let back = app.doc().find_shape(mid).expect("original back");
        assert!(back.stroke.is_some());
    }

    /// An open curve draws no fill: it is replaced as well (#999).
    #[test]
    fn an_open_curve_is_replaced_by_its_outline() {
        let mut app = App::headless();
        let mut line = BezPath::new();
        line.move_to((10.0, 10.0));
        line.curve_to((20.0, 0.0), (30.0, 20.0), (40.0, 10.0));
        let id = app
            .new_shape(ShapeKind::Path {
                path: line,
                closed: false,
            })
            .expect("line");
        app.run(Command::SetFill {
            shapes: vec![id],
            fill: Fill::Solid(tracedraw_core::Color::BLACK),
        });
        app.run(Command::SetStroke {
            shapes: vec![id],
            stroke: Some(Stroke::new(tracedraw_core::Color::BLACK, 2.0)),
        });
        app.select(vec![id]);
        app.convert_outline_to_object();
        assert_eq!(layer_ids(&app), app.selection);
        assert!(app.doc().find_shape(id).is_none());
    }

    /// A filled object keeps its fill, and the outline lies right above
    /// it, not on top of everything else.
    #[test]
    fn a_filled_object_keeps_its_fill_under_its_outline() {
        let fill = Fill::Solid(tracedraw_core::Color::BLACK);
        let (mut app, ids, mid) = stacked_with_outline(fill.clone());
        app.convert_outline_to_object();
        let new = app.selection[0];
        assert_eq!(layer_ids(&app), vec![ids[0], mid, new, ids[2]]);
        let kept = app.doc().find_shape(mid).expect("original");
        assert!(kept.stroke.is_none());
        assert_eq!(kept.fill, fill);
    }

    /// Ctrl+Shift+Q on an object of `kind` with a `width` mm black
    /// outline set up by `style`: the new object's path.
    fn outline_object(kind: ShapeKind, width: f64, style: impl Fn(&mut Stroke)) -> (App, BezPath) {
        let mut app = App::headless();
        let id = app.new_shape(kind).expect("shape");
        let mut st = Stroke::new(tracedraw_core::Color::BLACK, width);
        style(&mut st);
        app.select(vec![id]);
        app.run(Command::SetStroke {
            shapes: vec![id],
            stroke: Some(st),
        });
        app.convert_outline_to_object();
        let out = app
            .doc()
            .find_shape(app.selection[0])
            .map(|s| s.page_path())
            .expect("object");
        (app, out)
    }

    /// Lines and curves of a path.
    fn segments(path: &BezPath) -> (usize, usize) {
        let els = path.elements();
        let lines = els
            .iter()
            .filter(|e| matches!(e, kurbo::PathEl::LineTo(_)))
            .count();
        let curves = els
            .iter()
            .filter(|e| matches!(e, kurbo::PathEl::CurveTo(..)))
            .count();
        (lines, curves)
    }

    /// Convert Outline to Object on an ellipse keeps it round: two rings
    /// of a few curves each, not hundreds of short lines (#622).
    #[test]
    fn an_ellipse_outline_becomes_a_few_curves() {
        let ring = ShapeKind::Ellipse {
            rect: Rect::new(10.0, 10.0, 70.0, 50.0),
            arc: None,
        };
        let (_, out) = outline_object(ring, 4.0, |_| {});
        let (lines, curves) = segments(&out);
        assert_eq!(lines, 0, "{:?}", out.elements());
        assert!((8..=16).contains(&curves), "{curves} curves");
        // The ring is 4 mm wide around the ellipse.
        assert!(kurbo::Shape::contains(&out, Point::new(11.0, 30.0)));
        assert!(!kurbo::Shape::contains(&out, Point::new(13.5, 30.0)));
        assert!(!kurbo::Shape::contains(&out, Point::new(7.5, 30.0)));
        let b = out.bounding_box();
        assert!(
            (b.width() - 64.0).abs() < 0.05 && (b.height() - 44.0).abs() < 0.05,
            "{b:?}"
        );
    }

    /// A straight line with butt ends becomes the rectangle of its
    /// outline: four straight sides, four nodes (#622).
    #[test]
    fn a_straight_outline_becomes_a_rectangle() {
        let mut line = BezPath::new();
        line.move_to((10.0, 10.0));
        line.line_to((70.0, 10.0));
        let line = ShapeKind::Path {
            path: line,
            closed: false,
        };
        let (_, out) = outline_object(line, 2.0, |st| {
            st.cap = tracedraw_core::LineCap::Butt;
        });
        assert_eq!(segments(&out), (4, 0), "{:?}", out.elements());
    }

    /// A curved open path with a thick butt-ended outline, as in the
    /// report (an arc 87 mm across and 15 mm high, a 5.8 mm outline):
    /// four cusp nodes at the butt corners, each long side one curve and
    /// each end straight (#622).
    #[test]
    fn a_thick_butt_ended_arc_becomes_four_nodes_and_two_curves() {
        let mut arc = BezPath::new();
        arc.move_to((10.0, 30.0));
        arc.curve_to((35.0, 10.0), (72.0, 10.0), (97.0, 30.0));
        let arc = ShapeKind::Path {
            path: arc,
            closed: false,
        };
        let (_, out) = outline_object(arc, 5.8, |st| {
            st.cap = tracedraw_core::LineCap::Butt;
        });
        let (lines, curves) = segments(&out);
        assert_eq!(curves, 2, "{:?}", out.elements());
        assert!(lines <= 2, "{:?}", out.elements());
        let nodes = tracedraw_core::nodes::nodes(&out);
        assert_eq!(nodes.len(), 4, "{nodes:?}");
        for n in &nodes {
            assert_eq!(
                tracedraw_core::nodes::node_type_of_with(n, &Default::default()),
                tracedraw_core::nodes::NodeType::Cusp,
                "{n:?}"
            );
        }
        // The butt corners: 2.9 mm either side of each end, across it.
        for (p, t) in [
            (Point::new(10.0, 30.0), Vec2::new(25.0, -20.0)),
            (Point::new(97.0, 30.0), Vec2::new(25.0, 20.0)),
        ] {
            let t = t / t.hypot();
            let n = Vec2::new(-t.y, t.x) * 2.9;
            for q in [p + n, p - n] {
                assert!(
                    nodes.iter().any(|v| (v.pos - q).hypot() < 0.05),
                    "no node at {q:?}: {nodes:?}"
                );
            }
        }
    }

    /// Round ends and a round corner become real curves: each end two
    /// curves with a node at its tip, the corner one curve, the sides
    /// straight lines, eleven segments in all (#623).
    #[test]
    fn round_ends_and_corners_become_few_curves() {
        let mut line = BezPath::new();
        line.move_to((10.0, 10.0));
        line.line_to((40.0, 10.0));
        line.line_to((60.0, 30.0));
        let line = ShapeKind::Path {
            path: line,
            closed: false,
        };
        let (_, out) = outline_object(line, 4.0, |st| {
            st.cap = tracedraw_core::LineCap::Round;
            st.join = tracedraw_core::LineJoin::Round;
        });
        // Four sides, two curves per end, one round corner; the inner
        // corner is a plain corner.
        assert_eq!(segments(&out), (4, 5), "{:?}", out.elements());
        let nodes: Vec<Point> = out
            .elements()
            .iter()
            .filter_map(|e| e.end_point())
            .collect();
        let near = |q: Point| nodes.iter().any(|p| (*p - q).hypot() < 0.02);
        // The tips of the ends and where they meet the sides.
        let tip = Point::new(60.0, 30.0) + Vec2::new(2.0, 2.0) / 2f64.sqrt();
        for q in [
            Point::new(8.0, 10.0),
            tip,
            Point::new(10.0, 12.0),
            Point::new(10.0, 8.0),
        ] {
            assert!(near(q), "no node at {q:?}: {nodes:?}");
        }
    }

    /// A wide round end gets a node at its tip, right on it (#623).
    #[test]
    fn wide_round_ends_have_a_node_at_each_tip() {
        for width in [10.0, 17.0] {
            let (a, b) = (Point::new(20.0, 30.0), Point::new(130.0, 71.0));
            let mut line = BezPath::new();
            line.move_to(a);
            line.line_to(b);
            let line = ShapeKind::Path {
                path: line,
                closed: false,
            };
            let (_, out) = outline_object(line, width, |st| {
                st.cap = tracedraw_core::LineCap::Round;
            });
            let nodes: Vec<Point> = out
                .elements()
                .iter()
                .filter_map(|e| e.end_point())
                .collect();
            let t = (b - a) / (b - a).hypot();
            let half = width / 2.0;
            // The end is an arc of the pen's circle with a node at its tip.
            let within = 1e-3;
            for tip in [a - t * half, b + t * half] {
                let d = nodes
                    .iter()
                    .map(|p| (*p - tip).hypot())
                    .fold(f64::MAX, f64::min);
                assert!(d < within, "{width} mm: {d} from {tip:?}");
            }
            // Each end two curves, the sides straight.
            assert_eq!(segments(&out), (2, 4), "{width} mm: {:?}", out.elements());
        }
    }

    /// On a curved path too, each round end is two curves meeting at its
    /// tip, with nodes where it meets the sides (#623).
    #[test]
    fn round_ends_of_a_curve_meet_its_sides_at_nodes() {
        let mut arc = BezPath::new();
        arc.move_to((10.0, 10.0));
        arc.curve_to((20.0, 40.0), (50.0, 40.0), (60.0, 10.0));
        let arc = ShapeKind::Path {
            path: arc,
            closed: false,
        };
        let (_, out) = outline_object(arc, 4.0, |st| {
            st.cap = tracedraw_core::LineCap::Round;
        });
        let (lines, curves) = segments(&out);
        assert!(lines + curves <= 16, "{:?}", out.elements());
        let nodes: Vec<Point> = out
            .elements()
            .iter()
            .filter_map(|e| e.end_point())
            .collect();
        // The start runs along (1, 3): its end meets the sides 2 mm to
        // either side and has its tip 2 mm back.
        let t = Vec2::new(1.0, 3.0) / 10f64.sqrt();
        let n = Vec2::new(-t.y, t.x);
        let p = Point::new(10.0, 10.0);
        for q in [p + n * 2.0, p - n * 2.0, p - t * 2.0] {
            assert!(
                nodes.iter().any(|v| (*v - q).hypot() < 0.1),
                "no node at {q:?}: {nodes:?}"
            );
        }
    }

    /// A smooth curve of three Bezier segments (four nodes) with a 2 mm
    /// outline: Convert Outline to Object keeps it in curves, each side
    /// one curve per segment of the path, so the object has about twice
    /// the path's nodes, not dozens; with the same area as the outline
    /// drawn. A circle's ring is two circles of four nodes.
    #[test]
    fn a_curved_outline_keeps_about_as_many_nodes_as_its_path() {
        let mut smooth = BezPath::new();
        smooth.move_to((10.0, 10.0));
        smooth.curve_to((30.0, 60.0), (60.0, 60.0), (80.0, 30.0));
        smooth.curve_to((100.0, 0.0), (130.0, 0.0), (150.0, 30.0));
        smooth.curve_to((170.0, 60.0), (190.0, 60.0), (200.0, 40.0));
        let kind = ShapeKind::Path {
            path: smooth.clone(),
            closed: false,
        };
        let (_, out) = outline_object(kind.clone(), 2.0, |st| {
            st.cap = tracedraw_core::LineCap::Butt;
        });
        let nodes = tracedraw_core::nodes::nodes(&out).len();
        assert!(nodes <= 10, "{nodes} nodes: {:?}", out.elements());
        let (lines, curves) = segments(&out);
        assert_eq!(lines, 2, "the butt ends: {:?}", out.elements());
        assert!(curves >= 6, "{curves} curves");
        let mut st = Stroke::new(tracedraw_core::Color::BLACK, 2.0);
        st.cap = tracedraw_core::LineCap::Butt;
        let drawn = tracedraw_core::shaping::stroke_area(&smooth, &st);
        let (a, d) = (area(&out), area(&drawn));
        assert!((a - d).abs() < d * 0.002, "{a} vs {d}");
        // Round ends: two curves each.
        let (_, out) = outline_object(kind, 2.0, |st| {
            st.cap = tracedraw_core::LineCap::Round;
        });
        let nodes = tracedraw_core::nodes::nodes(&out).len();
        assert!(nodes <= 12, "{nodes} nodes: {:?}", out.elements());
        // A circle of four curves.
        let mut circle = BezPath::new();
        for el in kurbo::Shape::path_elements(&kurbo::Circle::new((100.0, 100.0), 40.0), 0.01) {
            circle.push(el);
        }
        assert_eq!(tracedraw_core::nodes::nodes(&circle).len(), 4);
        let ring = ShapeKind::Path {
            path: circle,
            closed: true,
        };
        let (_, out) = outline_object(ring, 2.0, |_| {});
        assert_eq!(segments(&out), (0, 8), "{:?}", out.elements());
        assert!(kurbo::Shape::contains(&out, Point::new(140.5, 100.0)));
        assert!(!kurbo::Shape::contains(&out, Point::new(138.5, 100.0)));
        assert!(!kurbo::Shape::contains(&out, Point::new(141.5, 100.0)));
    }

    /// The Shape tool on a converted outline shows corner widgets only
    /// where the outline has corners that round (the four butt corners
    /// of a smooth curve's outline), none on its smooth nodes; and on any
    /// curve no widget where rounding would change nothing that shows (a
    /// node where it barely turns).
    #[test]
    fn corner_widgets_show_only_where_they_round_something() {
        let mut smooth = BezPath::new();
        smooth.move_to((10.0, 10.0));
        smooth.curve_to((30.0, 60.0), (60.0, 60.0), (80.0, 30.0));
        smooth.curve_to((100.0, 0.0), (130.0, 0.0), (150.0, 30.0));
        let kind = ShapeKind::Path {
            path: smooth,
            closed: false,
        };
        let (mut app, _) = outline_object(kind, 3.0, |st| {
            st.cap = tracedraw_core::LineCap::Butt;
        });
        app.canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let page = app.page_rect();
        app.view.fit(page, app.canvas_rect);
        app.set_tool(Tool::Shape);
        assert_eq!(app.corner_widgets().len(), 4, "the butt corners");
        // A right angle, and a node where the line turns half a degree.
        let mut line = BezPath::new();
        line.move_to((20.0, 150.0));
        line.line_to((60.0, 150.0));
        let a = 0.5f64.to_radians();
        line.line_to((60.0 + 40.0 * a.cos(), 150.0 + 40.0 * a.sin()));
        line.line_to((100.0, 190.0));
        let id = app
            .new_shape(ShapeKind::Path {
                path: line,
                closed: false,
            })
            .expect("line");
        app.select(vec![id]);
        let widgets = app.corner_widgets();
        assert_eq!(widgets.len(), 1, "{widgets:?}");
        assert!(widgets[0].corner.at.x > 90.0, "the right angle's");
    }

    /// Area of a closed path, by its flattened polygon.
    fn area(path: &BezPath) -> f64 {
        let mut pts = Vec::new();
        let mut sum = 0.0;
        let mut close = |pts: &mut Vec<Point>| {
            for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)) {
                sum += a.x * b.y - b.x * a.y;
            }
            pts.clear();
        };
        tracedraw_core::geometry::flatten(path, 0.001, &mut |el| match el {
            kurbo::PathEl::MoveTo(p) => {
                close(&mut pts);
                pts.push(p);
            }
            kurbo::PathEl::LineTo(p) => pts.push(p),
            _ => {}
        });
        close(&mut pts);
        sum.abs() / 2.0
    }

    /// A thin outline (a hairline counts as 0.25 mm) with round ends and
    /// a round corner keeps its whole area and reach: the fine fans of
    /// its round parts are not merged away.
    #[test]
    fn a_thin_round_outline_keeps_its_shape() {
        let mut line = BezPath::new();
        line.move_to((10.0, 10.0));
        line.line_to((40.0, 10.0));
        line.line_to((60.0, 30.0));
        let line = ShapeKind::Path {
            path: line,
            closed: false,
        };
        let (_, out) = outline_object(line, 0.25, |st| {
            st.cap = tracedraw_core::LineCap::Round;
            st.join = tracedraw_core::LineJoin::Round;
        });
        let r = 0.125;
        let b = out.bounding_box();
        let want = Rect::new(10.0 - r, 10.0 - r, 60.0 + r, 30.0 + r);
        for (got, want) in [
            (b.x0, want.x0),
            (b.y0, want.y0),
            (b.x1, want.x1),
            (b.y1, want.y1),
        ] {
            assert!((got - want).abs() < 0.01, "{b:?} vs {want:?}");
        }
        // Two strokes meeting at 45 degrees: the rectangles, less their
        // overlap inside the corner, the round corner and the two ends.
        let turn = std::f64::consts::FRAC_PI_4;
        let len = 30.0 + 20.0 * 2f64.sqrt();
        let stroke = 2.0 * r * len - r * r * (turn / 2.0).tan()
            + r * r * turn / 2.0
            + std::f64::consts::PI * r * r;
        let a = area(&out);
        assert!((a - stroke).abs() < stroke * 0.005, "{a} vs {stroke}");
        // A straight hairline with round ends: not a flat sliver.
        let mut line = BezPath::new();
        line.move_to((10.0, 10.0));
        line.line_to((40.0, 10.0));
        let line = ShapeKind::Path {
            path: line,
            closed: false,
        };
        let (_, out) = outline_object(line, 0.0, |st| {
            st.cap = tracedraw_core::LineCap::Round;
        });
        let stroke = 2.0 * r * 30.0 + std::f64::consts::PI * r * r;
        let a = area(&out);
        assert!((a - stroke).abs() < stroke * 0.005, "{a} vs {stroke}");
    }

    /// A Variable Length outline converted to an object is a clean band:
    /// the eased width runs in a few curves, the butt ends are lines, not
    /// hundreds of short lines (#627).
    #[test]
    fn a_variable_outline_becomes_a_band_of_few_curves() {
        let width = |st: &mut Stroke| {
            st.widths = vec![tracedraw_core::WidthNode::symmetric(0.5, 10.0)];
        };
        let mut line = BezPath::new();
        line.move_to((10.0, 10.0));
        line.line_to((110.0, 10.0));
        let line = ShapeKind::Path {
            path: line,
            closed: false,
        };
        let (_, out) = outline_object(line, 2.0, width);
        // Each side two curves, eased from 1 mm to 5 mm and back; the
        // ends straight.
        assert_eq!(segments(&out), (2, 4), "{:?}", out.elements());
        let b = out.bounding_box();
        assert!((b.height() - 10.0).abs() < 0.01, "{b:?}");
        // Along an arch too, the same area as the band drawn.
        let mut arch = BezPath::new();
        arch.move_to((10.0, 10.0));
        arch.curve_to((30.0, 60.0), (80.0, 60.0), (110.0, 10.0));
        let mut st = Stroke::new(tracedraw_core::Color::BLACK, 2.0);
        width(&mut st);
        let drawn = tracedraw_core::shaping::outline_band(&arch, &st, 1.0).expect("band");
        let arch = ShapeKind::Path {
            path: arch,
            closed: false,
        };
        let (_, out) = outline_object(arch, 2.0, width);
        let (lines, curves) = segments(&out);
        assert!(lines == 2 && curves <= 16, "{lines} lines, {curves} curves");
        let (a, d) = (area(&out), area(&drawn));
        assert!((a - d).abs() < d * 0.002, "{a} vs {d}");
    }

    /// Every shape tool previews the object it is about to create, sized
    /// to the dragged box, while the drag is still in progress.
    #[test]
    fn shape_tools_preview_the_shape_being_drawn() {
        let mut app = App::headless();
        let (a, b) = (Point::new(10.0, 20.0), Point::new(60.0, 50.0));
        let full = Rect::from_points(a, b);
        for tool in [
            Tool::Rectangle,
            Tool::Ellipse,
            Tool::Polygon,
            Tool::Star,
            Tool::CommonShapes,
            Tool::GraphPaper,
            Tool::Table,
        ] {
            app.tool = tool;
            let p = app
                .creation_preview(a, b, false)
                .unwrap_or_else(|| panic!("{tool:?} has a preview"));
            let r = bounds(&p);
            assert!(
                (r.width() - full.width()).abs() < 0.5 && (r.height() - full.height()).abs() < 0.5,
                "{tool:?}: {r:?}"
            );
        }
        // The spiral fills the box but ends inside its corners.
        app.tool = Tool::Spiral;
        let r = bounds(&app.creation_preview(a, b, false).expect("spiral"));
        assert!(full.inflate(0.01, 0.01).contains_rect(r) && r.width() > full.width() * 0.8);
        // Shift: the box grows from the start point as its centre.
        app.tool = Tool::Rectangle;
        let r = bounds(&app.creation_preview(a, b, true).expect("rect"));
        assert!((r.center() - a).hypot() < 1e-6);
        // Crop and zoom boxes are not objects.
        app.tool = Tool::Crop;
        assert!(app.creation_preview(a, b, false).is_none());
    }

    #[test]
    fn knife_preview_follows_the_drag_direction() {
        let mut app = App::headless();
        app.tool = Tool::Knife;
        let (a, b) = (Point::new(10.0, 50.0), Point::new(60.0, 20.0));
        let p = app.creation_preview(a, b, false).expect("knife line");
        let els = p.elements();
        assert!(matches!(els[0], kurbo::PathEl::MoveTo(q) if q == a));
        assert!(matches!(els[1], kurbo::PathEl::LineTo(q) if q == b));
    }

    #[test]
    fn action_lines_preview_matches_the_created_object() {
        let r = Rect::new(0.0, 0.0, 80.0, 40.0);
        assert_eq!(
            action_lines_path(r, 12, false),
            action_lines_path(r, 12, false)
        );
        assert_eq!(action_lines_path(r, 12, true).elements().len(), 24);
        let g = grid_path(r, 4, 3);
        // Frame (5 elements) plus 2 vertical and 3 horizontal lines.
        assert_eq!(g.elements().len(), 5 + 2 * 2 + 3 * 2);
    }
}

/// The filled shape an object's outline draws, in page space: its band
/// (variable or calligraphic outlines), its dashes, or the stroke of each
/// subpath with its joins and caps, fitted to lines and curves. `None`
/// when the object has no outline or it draws nothing.
pub fn outline_object_path(s: &Shape) -> Option<BezPath> {
    let st = s.stroke.as_ref()?;
    let width = if st.width <= Stroke::HAIRLINE + 1e-9 {
        0.25
    } else {
        st.width
    };
    let pp = s.page_path();
    // A variable-width or calligraphic outline becomes its band;
    // a variable one from a finer flattening, as it is fitted to
    // curves below (#627).
    let band = if st.widths.is_empty() {
        tracedraw_core::shaping::outline_band(&pp, st, 1.0)
    } else {
        Some(tracedraw_core::shaping::variable_band_within(
            &pp, st.width, &st.widths, 0.005, st.cap,
        ))
    };
    if let Some(band) = band {
        if band.elements().is_empty() {
            return None;
        }
        // The band comes as many short lines: fit it to lines and
        // curves with as few nodes as keep its shape (#622).
        return Some(tracedraw_core::fit::fit_path(&band, OUTLINE_FIT_TOLERANCE));
    }
    // A dashed outline: each dash becomes a closed shape of its
    // own, the gaps kept (#490). The pattern is in widths, as the
    // renderer draws it.
    let pp = if st.dash.is_empty() {
        pp
    } else {
        let mut pattern: Vec<f64> = st.dash.iter().map(|d| (d * width).max(0.01)).collect();
        if pattern.len() % 2 == 1 {
            pattern = [pattern.clone(), pattern].concat();
        }
        kurbo::dash(pp.elements().iter().copied(), 0.0, &pattern).collect()
    };
    // The path's own segments offset to either side, curves kept as
    // curves; where subpaths of a combined curve cross, the pieces are
    // one shape, or the even-odd fill would leave a hole there (#997).
    offset_outline(&pp, width, st)
}
