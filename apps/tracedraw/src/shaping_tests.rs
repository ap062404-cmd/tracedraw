//! Weld, Trim, Intersect and the other shaping operations on the inputs
//! people use them on (#555): overlapping rectangles, ellipses, curves,
//! groups, text and text converted to curves, objects with holes, and a
//! target behind the source. Each result is measured: its area against
//! the areas worked out by hand, or, for letters, against the areas of
//! the other operations (Weld = A + B - Intersect, Trim = A - Intersect).

use crate::app::App;
use crate::ops::Shaping;
use tracedraw_core::document::{Shape, ShapeKind};
use tracedraw_core::geometry::{BezPath, Point, Rect, Shape as _};
use tracedraw_core::shaping::Op;
use tracedraw_core::{Color, Fill, ShapeId};

fn filled(app: &mut App, kind: ShapeKind, c: Color) -> ShapeId {
    let id = app.new_shape(kind).expect("shape");
    app.run(tracedraw_core::Command::SetFill {
        shapes: vec![id],
        fill: Fill::Solid(c),
    });
    id
}

fn rect(app: &mut App, x0: f64, y0: f64, x1: f64, y1: f64) -> ShapeId {
    filled(
        app,
        ShapeKind::Rect {
            rect: Rect::new(x0, y0, x1, y1),
            radius: 0.0,
            corners: None,
        },
        Color::rgb8(255, 0, 0),
    )
}

fn curve(app: &mut App, path: BezPath) -> ShapeId {
    filled(
        app,
        ShapeKind::Path { path, closed: true },
        Color::rgb8(0, 0, 255),
    )
}

fn polygon(pts: &[(f64, f64)]) -> BezPath {
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

/// The area a path fills with the even-odd rule.
fn area_of(p: &BezPath) -> f64 {
    tracedraw_core::shaping::overlay(p, &BezPath::new(), Op::Weld)
        .area()
        .abs()
}

/// The area an object fills, read as the shaping operations read it.
fn shape_area(s: &Shape) -> f64 {
    match &s.kind {
        // The pieces of a cut group overlap where its objects did: what
        // they cover together.
        ShapeKind::Group { children } => area_of(
            &children
                .iter()
                .map(|c| c.page_path())
                .reduce(|a, b| tracedraw_core::shaping::overlay(&a, &b, Op::Weld))
                .unwrap_or_default(),
        ),
        _ => area_of(&s.page_path()),
    }
}

/// The objects on the page now that were not there before.
fn new_objects(app: &App, before: &[ShapeId]) -> Vec<Shape> {
    app.doc()
        .all_layers()
        .flat_map(|l| &l.shapes)
        .filter(|s| !before.contains(&s.id))
        .cloned()
        .collect()
}

fn top_ids(app: &App) -> Vec<ShapeId> {
    app.doc()
        .all_layers()
        .flat_map(|l| &l.shapes)
        .map(|s| s.id)
        .collect()
}

/// Run `op` on `selection` (the target last) in a fresh copy of what
/// `build` draws, and give the area of what it made, with the objects it
/// made. `None` when it made nothing.
fn run(
    build: &dyn Fn(&mut App) -> Vec<ShapeId>,
    op: Shaping,
    order: &dyn Fn(&[ShapeId]) -> Vec<ShapeId>,
) -> Option<(f64, Vec<Shape>, App)> {
    let mut app = App::headless();
    let ids = build(&mut app);
    let before = top_ids(&app);
    app.select(order(&ids));
    app.shaping(op);
    let made = new_objects(&app, &before);
    if made.is_empty() {
        return None;
    }
    let area = made.iter().map(shape_area).sum();
    Some((area, made, app))
}

fn as_built(ids: &[ShapeId]) -> Vec<ShapeId> {
    ids.to_vec()
}

fn reversed(ids: &[ShapeId]) -> Vec<ShapeId> {
    ids.iter().rev().copied().collect()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Weld, Trim and Intersect of the two objects `build` draws (the second
/// selected last, the target), and Front Minus Back and Back Minus Front,
/// against the areas expected.
fn check(name: &str, build: &dyn Fn(&mut App) -> Vec<ShapeId>, expected: [f64; 5], tol: f64) {
    let ops = [
        Shaping::Weld,
        Shaping::Trim,
        Shaping::Intersect,
        Shaping::FrontMinusBack,
        Shaping::BackMinusFront,
    ];
    for (op, want) in ops.into_iter().zip(expected) {
        let got = run(build, op, &as_built).map(|r| r.0);
        assert!(
            got.is_some_and(|a| near(a, want, tol)),
            "{name} {op:?}: {got:?}, expected {want}"
        );
    }
}

/// The red rectangle (10, 10)-(40, 40) behind the blue one (30, 30)-(60,
/// 60), overlapping by 10 x 10.
fn two_rects(app: &mut App) -> Vec<ShapeId> {
    vec![
        rect(app, 10.0, 10.0, 40.0, 40.0),
        rect(app, 30.0, 30.0, 60.0, 60.0),
    ]
}

#[test]
fn overlapping_rectangles() {
    check(
        "rectangles",
        &two_rects,
        [1700.0, 800.0, 100.0, 800.0, 800.0],
        1e-3,
    );
}

/// The target behind the source: the back rectangle picked last is cut
/// by the one in front, which stays (Trim), and the result takes the
/// back one's place in the selection and its fill.
#[test]
fn a_target_behind_the_source() {
    for (op, want) in [
        (Shaping::Trim, 800.0),
        (Shaping::Weld, 1700.0),
        (Shaping::Intersect, 100.0),
    ] {
        let (area, made, app) = run(&two_rects, op, &reversed).expect("a result");
        assert!(near(area, want, 1e-3), "{op:?}: {area}");
        assert_eq!(made.len(), 1, "{op:?}");
        let b = made[0].page_path().bounding_box();
        if op == Shaping::Trim {
            // The back rectangle less the corner the front one covers.
            assert!(near(b.x0, 10.0, 1e-6) && near(b.x1, 40.0, 1e-6), "{b:?}");
            assert_eq!(top_ids(&app).len(), 2, "the front rectangle stays");
        }
        assert_eq!(app.selection, vec![made[0].id]);
    }
}

#[test]
fn ellipses() {
    // Circles of radius 10, their centres 10 apart.
    let build = |app: &mut App| {
        let mut ids = Vec::new();
        for cx in [0.0, 10.0] {
            ids.push(filled(
                app,
                ShapeKind::Ellipse {
                    rect: Rect::new(cx - 10.0, -10.0, cx + 10.0, 10.0),
                    arc: None,
                },
                Color::rgb8(0, 160, 0),
            ));
        }
        ids
    };
    let circle = std::f64::consts::PI * 100.0;
    let lens = 200.0 * (0.5f64).acos() - 5.0 * 300.0f64.sqrt();
    check(
        "ellipses",
        &build,
        [
            2.0 * circle - lens,
            circle - lens,
            lens,
            circle - lens,
            circle - lens,
        ],
        0.5,
    );
}

#[test]
fn curves() {
    // A triangle and a square drawn as curves.
    let build = |app: &mut App| {
        vec![
            curve(app, polygon(&[(0.0, 0.0), (40.0, 0.0), (0.0, 40.0)])),
            curve(
                app,
                polygon(&[(10.0, 10.0), (50.0, 10.0), (50.0, 50.0), (10.0, 50.0)]),
            ),
        ]
    };
    check(
        "curves",
        &build,
        [2200.0, 1400.0, 200.0, 1400.0, 600.0],
        1e-3,
    );
    // A curve with Bezier segments: a circle drawn as a curve, and a
    // rectangle over its right half.
    let build = |app: &mut App| {
        let c = kurbo::Circle::new((0.0, 0.0), 10.0).to_path(1e-4);
        vec![curve(app, c), rect(app, 0.0, -20.0, 20.0, 20.0)]
    };
    let half = std::f64::consts::PI * 50.0;
    check(
        "a circle as a curve",
        &build,
        [half + 800.0, 800.0 - half, half, 800.0 - half, half],
        0.5,
    );
}

/// An object with a hole: a square ring, crossed by a bar that runs over
/// its hole.
#[test]
fn an_object_with_a_hole() {
    let build = |app: &mut App| {
        let mut ring = polygon(&[(0.0, 0.0), (30.0, 0.0), (30.0, 30.0), (0.0, 30.0)]);
        ring.extend(
            polygon(&[(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 20.0)])
                .elements()
                .iter()
                .copied(),
        );
        vec![curve(app, ring), rect(app, 15.0, -5.0, 25.0, 35.0)]
    };
    // Ring 800; bar 400, of which 250 on the ring, 50 in the hole and 100
    // outside.
    check("a ring", &build, [950.0, 150.0, 250.0, 150.0, 550.0], 1e-3);
    // The ring as the target (picked last): it is what Trim cuts.
    let (area, made, _) = run(&build, Shaping::Trim, &reversed).expect("a result");
    assert!(near(area, 550.0, 1e-3), "{area}");
    // Still a ring: its hole is there.
    let p = made[0].page_path();
    assert!(
        tracedraw_core::shaping::overlay(
            &p,
            &Rect::new(11.0, 11.0, 14.0, 19.0).to_path(0.01),
            Op::Intersect
        )
        .elements()
        .is_empty(),
        "{p:?}"
    );
}

/// A group of two rectangles apart, and a bar across both.
#[test]
fn groups() {
    let build = |app: &mut App| {
        let a = rect(app, 0.0, 0.0, 10.0, 10.0);
        let b = rect(app, 20.0, 0.0, 30.0, 10.0);
        app.select(vec![a, b]);
        app.group_selection();
        let g = app.selection[0];
        let bar = rect(app, 5.0, 2.0, 25.0, 8.0);
        vec![g, bar]
    };
    // Group 200, bar 120, sharing 60.
    check("a group", &build, [260.0, 60.0, 60.0, 60.0, 140.0], 1e-3);
    // The group as the target: Trim cuts each of its objects, the pieces
    // a group.
    let (area, made, _) = run(&build, Shaping::Trim, &reversed).expect("a result");
    assert!(near(area, 140.0, 1e-3), "{area}");
    assert!(matches!(made[0].kind, ShapeKind::Group { .. }));
}

/// A group whose objects overlap: what it covers is their union, with
/// no hole where they overlap.
#[test]
fn a_group_of_overlapping_objects() {
    let build = |app: &mut App| {
        let a = rect(app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(app, 10.0, 10.0, 30.0, 30.0);
        app.select(vec![a, b]);
        app.group_selection();
        let g = app.selection[0];
        // A rectangle over the side of the second.
        let c = rect(app, 25.0, 0.0, 35.0, 15.0);
        vec![g, c]
    };
    // Group 700, the rectangle 150, sharing 5 x 5.
    check(
        "a group of overlapping objects",
        &build,
        [825.0, 125.0, 25.0, 125.0, 675.0],
        1e-3,
    );
}

fn text(app: &mut App, at: Point, s: &str) -> ShapeId {
    tracedraw_text::install();
    app.start_text(at, None);
    app.text_insert(s);
    let id = app.text_edit.as_ref().expect("editing").shape;
    app.finish_text();
    app.run(tracedraw_core::Command::SetFill {
        shapes: vec![id],
        fill: Fill::Solid(Color::rgb8(200, 30, 30)),
    });
    id
}

/// Text, as it is and converted to curves, with a rectangle over part of
/// it: the operations agree with one another (Weld = A + B - Intersect,
/// Trim = A - Intersect, the Minus operations the same), and the
/// counters of the letters stay holes.
#[test]
fn text_and_text_converted_to_curves() {
    for converted in [false, true] {
        let build = move |app: &mut App| {
            let t = text(app, Point::new(10.0, 100.0), "OBA");
            if converted {
                app.select(vec![t]);
                app.convert_to_curves();
            }
            let t = app.selection.first().copied().unwrap_or(t);
            let b = app.doc().find_shape(t).expect("text").bounds();
            // The left half of the letters and more above and below.
            let r = rect(app, b.x0 - 5.0, b.y0 - 5.0, b.center().x, b.y1 + 5.0);
            // The text in front, picked last: the target.
            app.run(tracedraw_core::Command::OrderRelative {
                shapes: vec![t],
                reference: r,
                in_front: true,
            });
            vec![r, t]
        };
        let mut app = App::headless();
        let ids = build(&mut app);
        let letters = shape_area(app.doc().find_shape(ids[1]).expect("text"));
        let bar = shape_area(app.doc().find_shape(ids[0]).expect("rect"));
        assert!(letters > 10.0, "{letters}");
        let inter = run(&build, Shaping::Intersect, &as_built)
            .expect("an intersection")
            .0;
        assert!(inter > 1.0 && inter < letters, "{inter} of {letters}");
        let weld = run(&build, Shaping::Weld, &as_built).expect("a weld").0;
        let trim = run(&build, Shaping::Trim, &as_built).expect("a trim").0;
        let fmb = run(&build, Shaping::FrontMinusBack, &as_built)
            .expect("front minus back")
            .0;
        let bmf = run(&build, Shaping::BackMinusFront, &as_built)
            .expect("back minus front")
            .0;
        let tol = 0.01 * letters;
        let name = if converted { "curves" } else { "text" };
        assert!(
            near(weld, letters + bar - inter, tol),
            "{name}: weld {weld}"
        );
        assert!(near(trim, letters - inter, tol), "{name}: trim {trim}");
        assert!(near(fmb, letters - inter, tol), "{name}: front {fmb}");
        assert!(near(bmf, bar - inter, tol), "{name}: back {bmf}");
        // The A, right of the rectangle, keeps its counter.
        let (_, made, _) = run(&build, Shaping::Trim, &as_built).expect("a trim");
        let parts = tracedraw_core::effects::subpaths(&made[0].page_path()).len();
        assert!(parts >= 2, "{name}: {parts} parts");
    }
}

/// Create Boundary of open lines that cross and close round an area: one
/// closed object, the area they enclose filled, with no holes (#2548).
#[test]
fn boundary_of_open_lines_is_one_solid_object() {
    let line = |app: &mut App, a: (f64, f64), b: (f64, f64)| {
        let mut p = BezPath::new();
        p.move_to(a);
        p.line_to(b);
        app.new_shape(ShapeKind::Path {
            path: p,
            closed: false,
        })
        .expect("line")
    };
    let build = |app: &mut App| {
        vec![
            line(app, (-10.0, 0.0), (110.0, 0.0)),
            line(app, (-10.0, 100.0), (110.0, 100.0)),
            line(app, (0.0, -10.0), (0.0, 110.0)),
            line(app, (100.0, -10.0), (100.0, 110.0)),
        ]
    };
    let (area, made, _) = run(&build, Shaping::Boundary, &as_built).expect("a boundary");
    assert_eq!(made.len(), 1);
    let s = &made[0];
    assert!(
        matches!(s.kind, ShapeKind::Path { closed: true, .. }),
        "{:?}",
        s.kind
    );
    // The square the lines close round, filled, with the ends that stick
    // out: no hole in the middle, nothing scattered.
    assert!(area > 9_900.0 && area < 10_600.0, "{area}");
    let contours = s
        .page_path()
        .elements()
        .iter()
        .filter(|e| matches!(e, tracedraw_core::geometry::PathEl::MoveTo(_)))
        .count();
    assert_eq!(contours, 1, "one outer edge");
}

/// The objects of `group` (one level), in their order.
fn children_of(app: &App, group: ShapeId) -> Vec<Shape> {
    match &app.doc().find_shape(group).expect("group").kind {
        ShapeKind::Group { children } => children.clone(),
        other => panic!("not a group: {other:?}"),
    }
}

/// A group of two overlapping squares and a third one apart, the group
/// moved off its place (its own transform), on a layer with an object
/// above it.
fn grouped_squares(app: &mut App) -> (ShapeId, [ShapeId; 3], ShapeId) {
    let a = rect(app, 0.0, 0.0, 20.0, 20.0);
    let b = rect(app, 10.0, 10.0, 30.0, 30.0);
    let c = rect(app, 50.0, 0.0, 60.0, 10.0);
    app.select(vec![a, b, c]);
    app.group_selection();
    let group = app.selection[0];
    app.select(vec![group]);
    app.transform_selection(tracedraw_core::geometry::Affine::translate((5.0, 7.0)));
    let above = rect(app, 200.0, 200.0, 210.0, 210.0);
    (group, [a, b, c], above)
}

/// Weld, Trim, Intersect, the Minus operations and Boundary on two
/// objects of a group (selected inside it) leave the result in the group,
/// where the objects were, the group's other object kept; the result is
/// where the objects were on the page (#2640).
#[test]
fn shaping_objects_of_a_group_leaves_the_result_in_the_group() {
    for (op, area) in [
        (Shaping::Trim, 400.0 - 100.0),
        (Shaping::Intersect, 100.0),
        (Shaping::FrontMinusBack, 400.0 - 100.0),
        (Shaping::BackMinusFront, 400.0 - 100.0),
        (Shaping::Weld, 700.0),
        (Shaping::Boundary, 700.0),
    ] {
        let mut app = App::headless();
        let (group, [a, b, c], above) = grouped_squares(&mut app);
        let layer_before = top_ids(&app);
        app.select(vec![a, b]);
        app.shaping(op);
        assert_eq!(
            top_ids(&app),
            layer_before,
            "{op:?}: nothing added to the layer"
        );
        let kids = children_of(&app, group);
        assert!(
            kids.iter().any(|k| k.id == c),
            "{op:?}: the third object stays"
        );
        // What was made (Trim keeps its cutter, as on a layer).
        let made: Vec<&Shape> = kids.iter().filter(|k| ![a, b, c].contains(&k.id)).collect();
        assert!(!made.is_empty(), "{op:?}: {kids:?}");
        let ids: Vec<ShapeId> = made.iter().map(|k| k.id).collect();
        // The result as it shows on the page: the group's move counts.
        let placed = app.doc().shape_on_page(made[0].id).expect("placed");
        let got: f64 = made
            .iter()
            .map(|k| shape_area(&app.doc().shape_on_page(k.id).expect("placed")))
            .sum();
        assert!(near(got, area, 1.0), "{op:?}: area {got}");
        let bb = placed.bounds();
        assert!(
            bb.x0 >= 4.0 && bb.y0 >= 6.0 && bb.x1 <= 36.0,
            "{op:?}: {bb:?}"
        );
        // The selection is the result, and the object above stays above.
        assert_eq!(app.selection, ids, "{op:?}");
        assert_eq!(top_ids(&app).last(), Some(&above), "{op:?}");
        // One undo step brings the two objects back.
        app.undo();
        let back: Vec<ShapeId> = children_of(&app, group).iter().map(|k| k.id).collect();
        assert_eq!(back, vec![a, b, c], "{op:?}");
    }
}

/// Combine on objects of a group (selected inside it) makes one curve of
/// them inside the group, where they were; the group keeps its other
/// object (#2640: Combine did nothing inside a group).
#[test]
fn combine_inside_a_group_keeps_the_result_in_the_group() {
    let mut app = App::headless();
    let (group, [a, b, c], above) = grouped_squares(&mut app);
    let layer_before = top_ids(&app);
    app.select(vec![a, b]);
    app.combine();
    assert_eq!(top_ids(&app), layer_before);
    let kids = children_of(&app, group);
    assert_eq!(kids.len(), 2, "{kids:?}");
    assert_eq!(kids[1].id, c, "the third object stays above");
    let combined = app.doc().shape_on_page(kids[0].id).expect("placed");
    let ShapeKind::Path { path, .. } = &combined.kind else {
        panic!("{:?}", combined.kind);
    };
    let subpaths = path
        .elements()
        .iter()
        .filter(|e| matches!(e, tracedraw_core::geometry::PathEl::MoveTo(_)))
        .count();
    assert_eq!(subpaths, 2);
    let bb = combined.bounds();
    assert!(
        (bb.x0 - 5.0).abs() < 1e-6 && (bb.y0 - 7.0).abs() < 1e-6,
        "{bb:?}"
    );
    assert!(
        (bb.x1 - 35.0).abs() < 1e-6 && (bb.y1 - 37.0).abs() < 1e-6,
        "{bb:?}"
    );
    assert_eq!(app.selection, vec![kids[0].id]);
    assert_eq!(top_ids(&app).last(), Some(&above));
    let _ = (a, b);
    app.undo();
    let back: Vec<ShapeId> = children_of(&app, group).iter().map(|k| k.id).collect();
    assert_eq!(back, vec![a, b, c]);
}
