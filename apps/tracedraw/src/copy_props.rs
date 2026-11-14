//! What Copy Properties From (Edit menu) and the Attributes Eyedropper
//! read from one object and write to others (#1550).
//!
//! The properties are read from the object that shows them: the object
//! clicked inside a group or a clip frame, not the group (which has no
//! fill or outline of its own, so copying from it took the fill and
//! outline away). A text whose characters all carry the same colour of
//! their own gives that colour as its fill. When the fill or the outline
//! goes to a text, the colours its characters carry of their own let go,
//! so the copied fill shows. A mesh fill, whose nodes lie in its
//! object's own space, is fitted to each object it goes to.

use tracedraw_core::{
    document::{Shape, ShapeKind, TextSpan},
    geometry::{Affine, Rect, Shape as _},
    Command, Fill, Stroke,
};

/// The object `s` shows its properties through: a group's frontmost
/// visible object (inside nested groups too), a clip frame's frame,
/// else `s` itself. Transforms are composed, so the result sits where
/// it is drawn.
pub fn properties_source(s: &Shape) -> Shape {
    match &s.kind {
        ShapeKind::Group { children } => match children.iter().rev().find(|c| c.visible) {
            Some(c) => {
                let mut c = c.clone();
                c.transform_by(s.transform);
                properties_source(&c)
            }
            None => s.clone(),
        },
        ShapeKind::ClipFrame { frame, .. } => {
            let mut f = (**frame).clone();
            f.transform_by(s.transform);
            f
        }
        _ => s.clone(),
    }
}

/// The characters of a text that are drawn (not its empty spans).
fn shown_spans(spans: &[TextSpan]) -> impl Iterator<Item = &TextSpan> {
    spans.iter().filter(|sp| !sp.text.is_empty())
}

/// The value every drawn character of `spans` carries, if they all
/// carry the same one.
fn common<T: Clone + PartialEq>(
    spans: &[TextSpan],
    field: impl Fn(&TextSpan) -> &Option<T>,
) -> Option<T> {
    let mut all = shown_spans(spans).map(field);
    let first = all.next()?.clone()?;
    all.all(|v| v.as_ref() == Some(&first)).then_some(first)
}

/// The fill `s` shows: the colour all the characters of a text carry of
/// their own, else the object's fill.
pub fn shown_fill(s: &Shape) -> Fill {
    if let ShapeKind::Text { spans, .. } = &s.kind {
        if let Some(f) = common(spans, |sp| &sp.fill) {
            return f;
        }
    }
    s.fill.clone()
}

/// The outline `s` shows: the outline all the characters of a text
/// carry of their own, else the object's outline.
pub fn shown_stroke(s: &Shape) -> Option<Stroke> {
    if let ShapeKind::Text { spans, .. } = &s.kind {
        if let Some(st) = common(spans, |sp| &sp.outline) {
            // An outline of no width takes the outline off.
            return (st.width > 0.0).then_some(st);
        }
    }
    s.stroke.clone()
}

/// The texts in `s` (itself, inside its groups, its clip frame's frame
/// and contents), each with its spans as `f` leaves them when that
/// changes them.
fn texts_changed(s: &Shape, f: &dyn Fn(&mut TextSpan), out: &mut Vec<Command>) {
    match &s.kind {
        ShapeKind::Text { .. } => {
            let mut kind = s.kind.clone();
            if let ShapeKind::Text { spans, .. } = &mut kind {
                spans.iter_mut().for_each(f);
            }
            if kind != s.kind {
                out.push(Command::SetShapeKind { shape: s.id, kind });
            }
        }
        ShapeKind::Group { children } => {
            for c in children {
                texts_changed(c, f, out);
            }
        }
        ShapeKind::ClipFrame { frame, contents } => {
            texts_changed(frame, f, out);
            for c in contents {
                texts_changed(c, f, out);
            }
        }
        _ => {}
    }
}

/// The objects a fill given to `s` reaches (a group passes it to all
/// its objects, a clip frame to its frame), with their transforms
/// composed.
fn fill_leaves(s: &Shape, out: &mut Vec<Shape>) {
    match &s.kind {
        ShapeKind::Group { children } => {
            for c in children {
                let mut c = c.clone();
                c.transform_by(s.transform);
                fill_leaves(&c, out);
            }
        }
        ShapeKind::ClipFrame { frame, .. } => {
            let mut f = (**frame).clone();
            f.transform_by(s.transform);
            out.push(f.clone());
            fill_leaves(&f, out);
        }
        _ => out.push(s.clone()),
    }
}

/// The box of `s` in its own space, when it has an area.
fn local_box(s: &Shape) -> Option<Rect> {
    let b = s.local_path().bounding_box();
    (b.width() > 1e-9 && b.height() > 1e-9 && b.area().is_finite()).then_some(b)
}

/// The commands that give `targets` the fill `fill`, read from `src`:
/// the characters of the texts among them let their own colours go (in
/// that order, first), and a mesh fill is fitted to each object's box.
pub fn fill_commands(fill: &Fill, src: &Shape, targets: &[Shape]) -> Vec<Command> {
    let mut cmds = Vec::new();
    for t in targets {
        texts_changed(t, &|sp| sp.fill = None, &mut cmds);
    }
    let Fill::Mesh(mesh) = fill else {
        cmds.push(Command::SetFill {
            shapes: targets.iter().map(|t| t.id).collect(),
            fill: fill.clone(),
        });
        return cmds;
    };
    let from = local_box(src);
    let mut leaves = Vec::new();
    for t in targets {
        fill_leaves(t, &mut leaves);
    }
    for leaf in leaves {
        let fitted = match (from, local_box(&leaf)) {
            (Some(a), Some(b)) => mesh.transformed(
                Affine::translate((b.x0, b.y0))
                    * Affine::scale_non_uniform(b.width() / a.width(), b.height() / a.height())
                    * Affine::translate((-a.x0, -a.y0)),
            ),
            _ => mesh.clone(),
        };
        cmds.push(Command::SetFill {
            shapes: vec![leaf.id],
            fill: Fill::Mesh(fitted),
        });
    }
    cmds
}

/// The commands that give `targets` the mesh fill `mesh` when it is new
/// to them: its nodes, wherever the fill was made (over the page box of
/// the object, or a box of its own), are fitted to each object's own
/// box, the space its fill is drawn and edited in (#2506). An object
/// with a mesh already takes `mesh` as it is (an edit of that mesh).
/// None when no target needs fitting.
pub fn new_mesh_commands(mesh: &tracedraw_core::Mesh, targets: &[Shape]) -> Vec<Command> {
    let mut leaves = Vec::new();
    for t in targets {
        fill_leaves(t, &mut leaves);
    }
    let Some(from) = mesh_box(mesh) else {
        return Vec::new();
    };
    if !leaves.iter().any(|l| !matches!(l.fill, Fill::Mesh(_))) {
        return Vec::new();
    }
    let mut cmds = Vec::new();
    for t in targets {
        texts_changed(t, &|sp| sp.fill = None, &mut cmds);
    }
    for leaf in leaves {
        let fitted = match (&leaf.fill, local_box(&leaf)) {
            (Fill::Mesh(_), _) | (_, None) => mesh.clone(),
            (_, Some(b)) => mesh.transformed(
                Affine::translate((b.x0, b.y0))
                    * Affine::scale_non_uniform(
                        b.width() / from.width(),
                        b.height() / from.height(),
                    )
                    * Affine::translate((-from.x0, -from.y0)),
            ),
        };
        cmds.push(Command::SetFill {
            shapes: vec![leaf.id],
            fill: Fill::Mesh(fitted),
        });
    }
    cmds
}

/// The box of a mesh's nodes.
fn mesh_box(m: &tracedraw_core::Mesh) -> Option<Rect> {
    let mut it = m.nodes.iter().map(|n| n.pos);
    let first = it.next()?;
    let b = it.fold(Rect::from_points(first, first), |r, p| r.union_pt(p));
    (b.width() > 1e-9 && b.height() > 1e-9).then_some(b)
}

/// The commands that let the characters of the texts in `targets` drop
/// the outlines they carry of their own, so the object's outline shows.
pub fn clear_character_outlines(targets: &[Shape]) -> Vec<Command> {
    let mut cmds = Vec::new();
    for t in targets {
        texts_changed(t, &|sp| sp.outline = None, &mut cmds);
    }
    cmds
}

/// The kind of `s` once `cmds` ran: the last kind they give it, else
/// its own.
pub fn kind_after(cmds: &[Command], s: &Shape) -> ShapeKind {
    cmds.iter()
        .rev()
        .find_map(|cmd| match cmd {
            Command::SetShapeKind { shape, kind } if *shape == s.id => Some(kind.clone()),
            _ => None,
        })
        .unwrap_or_else(|| s.kind.clone())
}

/// `span` with the character formatting of `from`: font, size, weight
/// and slant, underline and strikethrough, letter spacing, OpenType
/// features and variable font axes. Its text and its colours stay.
pub fn with_character_format(span: &TextSpan, from: &TextSpan) -> TextSpan {
    TextSpan {
        font_family: from.font_family.clone(),
        size_pt: from.size_pt,
        bold: from.bold,
        italic: from.italic,
        underline: from.underline,
        strikethrough: from.strikethrough,
        tracking_pct: from.tracking_pct,
        features: from.features.clone(),
        variations: from.variations.clone(),
        ..span.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::{geometry::Point, Color, ShapeId};

    fn rect(id: u64, x: f64) -> Shape {
        Shape::new(
            ShapeId(id),
            ShapeKind::Rect {
                rect: Rect::new(x, 0.0, x + 10.0, 10.0),
                radius: 0.0,
                corners: None,
            },
        )
    }

    #[test]
    fn a_group_shows_its_frontmost_object() {
        let mut back = rect(1, 0.0);
        back.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        let mut front = rect(2, 5.0);
        front.fill = Fill::Solid(Color::rgb8(0, 0, 255));
        let mut group = Shape::new(
            ShapeId(3),
            ShapeKind::Group {
                children: vec![back, front],
            },
        );
        group.transform = Affine::translate((100.0, 0.0));
        let s = properties_source(&group);
        assert_eq!(s.id, ShapeId(2));
        assert_eq!(s.fill, Fill::Solid(Color::rgb8(0, 0, 255)));
        assert!((s.bounds().x0 - 105.0).abs() < 1e-9);
    }

    /// A mesh fill's nodes, in the source's own space, are fitted to
    /// the box of the object it goes to.
    #[test]
    fn a_mesh_fill_is_fitted_to_the_object_it_goes_to() {
        let src = rect(1, 0.0);
        let mut far = rect(2, 0.0);
        far.kind = ShapeKind::Rect {
            rect: Rect::new(100.0, 50.0, 140.0, 70.0),
            radius: 0.0,
            corners: None,
        };
        let mesh = tracedraw_core::style::Mesh::new(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            2,
            2,
            Color::rgb8(10, 20, 30),
        );
        let cmds = fill_commands(&Fill::Mesh(mesh), &src, std::slice::from_ref(&far));
        let [Command::SetFill {
            shapes,
            fill: Fill::Mesh(m),
        }] = cmds.as_slice()
        else {
            panic!("one mesh fill: {cmds:?}");
        };
        assert_eq!(shapes, &vec![ShapeId(2)]);
        let xs: Vec<f64> = m.nodes.iter().map(|n| n.pos.x).collect();
        let ys: Vec<f64> = m.nodes.iter().map(|n| n.pos.y).collect();
        let lo = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
        let hi = |v: &[f64]| v.iter().cloned().fold(f64::MIN, f64::max);
        assert!((lo(&xs) - 100.0).abs() < 1e-9 && (hi(&xs) - 140.0).abs() < 1e-9);
        assert!((lo(&ys) - 50.0).abs() < 1e-9 && (hi(&ys) - 70.0).abs() < 1e-9);
    }

    #[test]
    fn a_text_shows_the_colour_its_characters_carry() {
        let red = Fill::Solid(Color::rgb8(255, 0, 0));
        let mut span = TextSpan::new("Hi", "Sans", 12.0);
        span.fill = Some(red.clone());
        let mut t = Shape::new(
            ShapeId(1),
            ShapeKind::Text {
                spans: vec![span.clone(), TextSpan::new("", "Sans", 12.0)],
                origin: Point::ZERO,
                frame: None,
                align: Default::default(),
                para: Default::default(),
                on_path: None,
            },
        );
        assert_eq!(shown_fill(&t), red);
        // Characters of two colours: the object's fill.
        if let ShapeKind::Text { spans, .. } = &mut t.kind {
            let mut other = span.clone();
            other.fill = Some(Fill::Solid(Color::rgb8(0, 255, 0)));
            spans.push(other);
        }
        assert_eq!(shown_fill(&t), t.fill);
    }
}
