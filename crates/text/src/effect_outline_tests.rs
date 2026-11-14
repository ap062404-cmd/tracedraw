//! Live effects on letters (#2156): the block shadow and the contour of
//! text follow the letters' outlines, with no spikes or shards past the
//! distance asked for. Letters have holes, sharp corners and curves, which
//! the effects must all handle.

use super::*;
use tracedraw_core::document::{Shape, ShapeKind};
use tracedraw_core::id::ShapeId;
use tracedraw_core::live::{evaluate, Effect};
use tracedraw_core::shaping::{FillRule, Join};
use tracedraw_core::style::Fill;
use tracedraw_core::Color;

const SELAWIK_BOLD: &[u8] = include_bytes!("../../../apps/tracedraw/assets/fonts/Selawik-Bold.ttf");

/// `text` laid out glyph by glyph (no kerning) in `font`, its capitals
/// `cap` mm tall, as one path in page space (y up).
fn word_in(font: &[u8], text: &str, cap: f64) -> BezPath {
    let face = ttf_parser::Face::parse(font, 0).expect("font");
    let cap_units = face.capital_height().unwrap_or(1400) as f64;
    let scale = cap / cap_units;
    let mut sink = OutlineSink {
        path: BezPath::new(),
        scale,
        x: 0.0,
        y: 0.0,
    };
    for c in text.chars() {
        let Some(g) = face.glyph_index(c) else {
            continue;
        };
        face.outline_glyph(g, &mut sink);
        sink.x += face.glyph_hor_advance(g).unwrap_or(0) as f64 * scale;
    }
    sink.path
}

/// Points along a path, every element flattened finely.
fn samples(path: &BezPath) -> Vec<Point> {
    let mut pts = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), 0.005, |el| match el {
        kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => pts.push(p),
        _ => {}
    });
    pts
}

/// The straight edges of a path's flattened outline.
fn edges(path: &BezPath) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut last = None;
    kurbo::flatten(path.elements().iter().copied(), 0.002, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if let (Some(a), Some(s)) = (last, start) {
                out.push((a, s));
            }
            start = Some(p);
            last = Some(p);
        }
        kurbo::PathEl::LineTo(p) => {
            if let Some(a) = last {
                out.push((a, p));
            }
            last = Some(p);
        }
        kurbo::PathEl::ClosePath => {
            if let (Some(a), Some(s)) = (last, start) {
                out.push((a, s));
            }
            last = start;
        }
        _ => {}
    });
    if let (Some(a), Some(s)) = (last, start) {
        out.push((a, s));
    }
    out
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - a.lerp(b, t)).hypot()
}

fn boundary_distance(p: Point, edges: &[(Point, Point)]) -> f64 {
    edges
        .iter()
        .map(|&(a, b)| segment_distance(p, a, b))
        .fold(f64::INFINITY, f64::min)
}

/// Whether `p` lies inside the letters (the nonzero rule, as text fills).
fn inside(p: Point, edges: &[(Point, Point)]) -> bool {
    let mut wind = 0i32;
    for &(a, b) in edges {
        if a.y <= p.y {
            if b.y > p.y && (b - a).cross(p - a) > 0.0 {
                wind += 1;
            }
        } else if b.y <= p.y && (b - a).cross(p - a) < 0.0 {
            wind -= 1;
        }
    }
    wind != 0
}

fn shape_of(path: BezPath) -> Shape {
    let mut s = Shape::new(ShapeId(1), ShapeKind::Path { path, closed: true });
    s.fill_rule = FillRule::NonZero;
    s
}

const QUICKSAND: &[u8] = include_bytes!("../testdata/Quicksand-VF.ttf");

/// The words of the report in a static face, and in a variable face whose
/// letters are drawn as overlapping strokes (V, N, A, K, X, W), with the
/// letters' area as the nonzero rule fills it.
/// With each, an inside offset its strokes are wide enough for: that of
/// the report for the bold face, less for the light one.
fn words() -> Vec<(&'static str, BezPath, BezPath, f64)> {
    let mut out = Vec::new();
    for (name, font, inset) in [
        ("Selawik Bold", SELAWIK_BOLD, 1.67),
        ("Quicksand", QUICKSAND, 0.6),
    ] {
        let w = word_in(font, "VINSIDEAKXW", 35.0);
        let clean = tracedraw_core::shaping::simplify(&w);
        out.push((name, w, clean, inset));
    }
    // The variable face really has overlapping strokes.
    let mv = |p: &BezPath| {
        p.elements()
            .iter()
            .filter(|e| matches!(e, kurbo::PathEl::MoveTo(_)))
            .count()
    };
    assert!(mv(&out[1].2) < mv(&out[1].1));
    out
}

/// Every point of the block shadow of letters lies on the letters slid
/// along the shadow's offset, and the slid letters are all covered: no
/// shard reaches out from one letter to the next (#2156: depth 4.23 mm,
/// direction -2.4 degrees, letters 35 mm tall).
#[test]
fn block_shadow_of_letters_stays_on_the_slid_letters() {
    let angle = (-2.4f64).to_radians();
    let v = kurbo::Vec2::new(4.23 * angle.cos(), 4.23 * angle.sin());
    for (name, w, clean, _) in words() {
        let e = edges(&clean);
        let mut s = shape_of(w);
        s.effects.push(Effect::BlockShadow {
            offset: v,
            color: Color::BLACK,
            gap: 0.0,
        });
        let ev = evaluate(&s);
        assert_eq!(ev.below.len(), 1, "{name}");
        let shadow = ev.below[0].page_path();
        let off_letters = |p: Point| {
            (0..=60)
                .map(|k| {
                    let q = p - v * (k as f64 / 60.0);
                    if inside(q, &e) {
                        0.0
                    } else {
                        boundary_distance(q, &e)
                    }
                })
                .fold(f64::INFINITY, f64::min)
        };
        for p in samples(&shadow) {
            let d = off_letters(p);
            assert!(d < 0.03, "{name}: {p:?} lies {d} mm off the slid letters");
        }
        let se = edges(&shadow);
        for t in [0.0, 0.5, 1.0] {
            for p in samples(&clean).into_iter().step_by(7) {
                let q = p + v * t;
                assert!(
                    inside(q, &se) || boundary_distance(q, &se) < 0.03,
                    "{name}: {q:?} is not covered"
                );
            }
        }
    }
}

fn contour(outside: bool, offset: f64, join: Join) -> Effect {
    Effect::Contour {
        steps: 1,
        offset,
        outside,
        to_center: false,
        fill_to: Color::WHITE,
        outline_to: None,
        color_blend: 0,
        corners: Some(join),
        accel_objects: 0.0,
        accel_colors: 0.0,
    }
}

/// The inside contour of letters keeps the distance asked for from the
/// letters' outline: round corners exactly, mitred ones no nearer, bevelled
/// ones inside the letters (a bevel across the corner of a narrow notch
/// comes near the notch's tip, as it should). The overlapping strokes of a variable
/// face left contours round their overlaps, drawn as black triangles in
/// the V and N and a bar in the E (#2156).
#[test]
fn inside_contour_of_letters_keeps_its_distance() {
    for (name, w, clean, d) in words() {
        let e = edges(&clean);
        for join in [Join::Round, Join::Miter, Join::Bevel] {
            let mut s = shape_of(w.clone());
            s.fill = Fill::Solid(Color::BLACK);
            s.effects.push(contour(false, d, join));
            let ev = evaluate(&s);
            assert_eq!(ev.above.len(), 1, "{name} {join:?}");
            for p in samples(&ev.above[0].page_path()) {
                let at = boundary_distance(p, &e);
                assert!(
                    inside(p, &e),
                    "{name} {join:?}: {p:?} is outside the letters"
                );
                let ok = match join {
                    Join::Round => (at - d).abs() < 0.06,
                    Join::Miter => at > d - 0.06,
                    Join::Bevel => at > 0.02,
                };
                assert!(ok, "{name} {join:?}: {p:?} lies {at} mm in");
            }
        }
    }
}

/// The outside contour of letters keeps its distance too.
#[test]
fn outside_contour_of_letters_keeps_its_distance() {
    let d = 1.0;
    for (name, w, clean, _) in words() {
        let e = edges(&clean);
        for join in [Join::Round, Join::Miter] {
            let mut s = shape_of(w.clone());
            s.effects.push(contour(true, d, join));
            let ev = evaluate(&s);
            assert_eq!(ev.below.len(), 1, "{name} {join:?}");
            for p in samples(&ev.below[0].page_path()) {
                let at = boundary_distance(p, &e);
                assert!(
                    !inside(p, &e),
                    "{name} {join:?}: {p:?} is inside the letters"
                );
                let ok = match join {
                    Join::Round => (at - d).abs() < 0.06,
                    _ => at > d - 0.06,
                };
                assert!(ok, "{name} {join:?}: {p:?} lies {at} mm out");
            }
        }
    }
}
