//! tracedraw-tracecut: the geometry of TraceCut, the tools for cutting
//! plotters.
//!
//! Pure geometry over page-space outlines, independent of the user
//! interface. The application turns the results into commands.
//!
//! - [`measure`]: outline length, net, outer and hole area.
//! - [`cleanup`]: duplicate objects and edges that would be cut twice.
//! - [`crossings`]: places where outlines cross themselves or each other.
//! - [`cut`]: plotter output, weeding, bridges, cut order, registration and crop marks.
//! - [`nest`]: placing objects on sheets and rolls with the least waste.
//! - [`calc`]: cutting time and cost.
//! - [`cnc`]: toolpaths and G-code for CNC routers and mills.
//! - [`eyelets`]: eyelet (grommet) marks along an edge.
//! - [`boxes`]: the panels of a box with finger joints.
//! - [`tray`]: the flat pattern of a tray folded from composite panel.
//!
//! Units are millimetres, Y up, like the rest of the model.

pub mod boxes;
pub mod calc;
pub mod cleanup;
pub mod cnc;
pub mod crossings;
pub mod cut;
pub mod eyelets;
pub mod measure;
pub mod nest;
pub mod tray;

use tracedraw_core::geometry::{Affine, BezPath, PathEl, Point};
use tracedraw_core::live::Effect;
use tracedraw_core::{Shape, ShapeKind};

/// The outlines a cutter would follow for `shape`, in page space:
/// one path per drawn object, groups opened, a clip frame by its frame
/// only (its contents are cut off by it). An object whose outline a live
/// effect reshapes (an envelope, a perspective, a distortion) gives the
/// outline as it is drawn, not the one it had before the effect: the
/// cutter follows what shows, and nesting keeps what shows apart
/// (#2227). `shape` must already carry its page transform (see
/// `Document::shape_on_page`).
pub fn outlines(shape: &Shape) -> Vec<BezPath> {
    let mut out = Vec::new();
    collect(shape, Affine::IDENTITY, 0, &mut out);
    out
}

/// How deep groups are opened: a drawing nested deeper than this is not
/// followed further.
const MAX_DEPTH: usize = 64;

fn collect(shape: &Shape, parent: Affine, depth: usize, out: &mut Vec<BezPath>) {
    if depth > MAX_DEPTH {
        return;
    }
    if shape.effects.iter().any(Effect::warps_outline) {
        // The outline as the effects leave it; what they add (contours,
        // blends, shadows) is not the object's own outline.
        let mut drawn = tracedraw_core::live::evaluate(shape).main;
        drawn.effects.clear();
        collect(&drawn, parent, depth + 1, out);
        return;
    }
    let t = parent * shape.transform;
    match &shape.kind {
        ShapeKind::Group { children } => {
            for c in children {
                collect(c, t, depth + 1, out);
            }
        }
        ShapeKind::ClipFrame { frame, .. } => collect(frame, t, depth + 1, out),
        _ => {
            let p = t * shape.local_path();
            if !p.elements().is_empty() {
                out.push(p);
            }
        }
    }
}

/// One subpath: its elements from the move on, and whether it is closed
/// (by a close element, or by ending where it starts).
#[derive(Debug, Clone)]
pub(crate) struct Subpath {
    pub path: BezPath,
    pub closed: bool,
}

/// Split a path into its subpaths. A subpath without segments is dropped.
pub(crate) fn subpaths(path: &BezPath) -> Vec<Subpath> {
    let mut out: Vec<Subpath> = Vec::new();
    let mut cur = BezPath::new();
    let mut start = Point::ZERO;
    let mut last = Point::ZERO;
    let mut segs = 0usize;
    let mut closed = false;
    let flush = |cur: &mut BezPath, segs: &mut usize, closed: &mut bool, out: &mut Vec<Subpath>| {
        if *segs > 0 {
            out.push(Subpath {
                path: std::mem::take(cur),
                closed: *closed,
            });
        } else {
            *cur = BezPath::new();
        }
        *segs = 0;
        *closed = false;
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                flush(&mut cur, &mut segs, &mut closed, &mut out);
                cur.move_to(p);
                start = p;
                last = p;
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                if cur.elements().is_empty() {
                    cur.move_to(last);
                    start = last;
                }
                cur.push(*el);
                segs += 1;
                last = p;
            }
            PathEl::ClosePath => {
                if segs > 0 {
                    cur.close_path();
                    closed = true;
                }
                last = start;
                flush(&mut cur, &mut segs, &mut closed, &mut out);
            }
        }
    }
    flush(&mut cur, &mut segs, &mut closed, &mut out);
    for s in &mut out {
        if !s.closed {
            let first = s.path.elements().first().and_then(|e| e.end_point());
            let end = s.path.elements().last().and_then(|e| e.end_point());
            if let (Some(a), Some(b)) = (first, end) {
                if (a - b).hypot() < 1e-6 {
                    s.closed = true;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::geometry::Rect;
    use tracedraw_core::ShapeId;

    #[test]
    fn groups_open_into_their_members_in_page_space() {
        let a = Shape::new(
            ShapeId(1),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            },
        );
        let mut b = a.clone();
        b.id = ShapeId(2);
        b.transform = Affine::translate((20.0, 0.0));
        let mut g = Shape::new(
            ShapeId(3),
            ShapeKind::Group {
                children: vec![a, b],
            },
        );
        g.transform = Affine::translate((0.0, 5.0));
        let o = outlines(&g);
        assert_eq!(o.len(), 2);
        let b1 = tracedraw_core::geometry::Shape::bounding_box(&o[1]);
        assert!((b1.x0 - 20.0).abs() < 1e-9 && (b1.y0 - 5.0).abs() < 1e-9);
    }

    /// #2227: an enveloped object is cut and nested by the outline it
    /// shows. Its frame's top right corner pulled far out, the outline
    /// reaches there too, alone or inside a group.
    #[test]
    fn an_enveloped_object_gives_the_outline_it_draws() {
        use tracedraw_core::geometry::Shape as _;
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut a = Shape::new(
            ShapeId(1),
            ShapeKind::Rect {
                rect,
                radius: 0.0,
                corners: None,
            },
        );
        let mut frame = tracedraw_core::envelope::Frame::around(rect);
        let corner = frame
            .nodes
            .iter()
            .position(|n| (*n - Point::new(40.0, 20.0)).hypot() < 1e-9)
            .expect("corner");
        frame.nodes[corner] = Point::new(90.0, 70.0);
        a.effects = vec![Effect::from_envelope(
            &frame,
            false,
            tracedraw_core::envelope::Mapping::Putty,
        )];
        a.transform = Affine::translate((100.0, 0.0));
        let drawn = a.visual_bounds();
        assert!(drawn.x1 > 180.0 && drawn.y1 > 60.0, "{drawn:?}");
        let o = outlines(&a);
        assert_eq!(o.len(), 1);
        let b = o[0].bounding_box();
        assert!(
            (b.x1 - drawn.x1).abs() < 0.01 && (b.y1 - drawn.y1).abs() < 0.01,
            "{b:?} vs {drawn:?}"
        );
        let mut g = Shape::new(ShapeId(2), ShapeKind::Group { children: vec![a] });
        g.transform = Affine::translate((0.0, 5.0));
        let o = outlines(&g);
        assert_eq!(o.len(), 1);
        let b = o[0].bounding_box();
        assert!(
            (b.x1 - drawn.x1).abs() < 0.01 && (b.y1 - drawn.y1 - 5.0).abs() < 0.01,
            "{b:?} vs {drawn:?}"
        );
    }

    #[test]
    fn subpaths_know_whether_they_close() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((1.0, 0.0));
        p.line_to((0.0, 0.0));
        p.move_to((5.0, 5.0));
        p.line_to((6.0, 5.0));
        p.move_to((9.0, 9.0));
        let s = subpaths(&p);
        assert_eq!(s.len(), 2);
        assert!(s[0].closed);
        assert!(!s[1].closed);
    }
}
