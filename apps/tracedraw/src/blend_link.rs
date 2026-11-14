//! Blends keep their end object as an object of the drawing. The start
//! object holds the Blend effect, which names the end object (`end_id`)
//! and keeps a copy of it in page space for drawing the steps. After
//! every change of the drawing the copy is brought up to date, in the
//! same undo step, so moving, resizing or recolouring either object
//! redraws the steps; deleting the end object takes the blend away.
//!
//! The Blend bar and panel edit the blends of the selection: a selected
//! start object's, or the blend whose end object is selected.

use crate::app::App;
use tracedraw_core::{document::Shape, live::Effect, Command, ShapeId, ShapeKind};

/// The end object as the blend keeps it: in page space, its own effects
/// left out (a blend of a blend does not draw twice).
pub fn end_copy(s: &Shape) -> Shape {
    let mut e = s.clone();
    e.effects.clear();
    e
}

fn is_blend(e: &Effect) -> bool {
    matches!(e, Effect::Blend { .. })
}

/// `effects` with the blend replaced in place by `b` (removed with
/// `None`).
fn with_blend(effects: &[Effect], b: Option<Effect>) -> Vec<Effect> {
    let mut out = Vec::with_capacity(effects.len());
    let mut placed = false;
    for e in effects {
        if is_blend(e) {
            if !placed {
                out.extend(b.clone());
                placed = true;
            }
        } else {
            out.push(e.clone());
        }
    }
    if !placed {
        out.extend(b);
    }
    out
}

impl App {
    /// Whether `a` is drawn below `b` on the page: an earlier layer, or
    /// earlier in the order the page is painted, at any depth (inside
    /// groups and clip frames too, the frame before its contents). False
    /// when either is not on the page. Only top-level objects counted
    /// before, so a blend that was grouped or placed in a frame had its
    /// steps moved in front of its start object at the next change of
    /// the drawing (#2023, #2120).
    pub(crate) fn stacked_below(&self, a: ShapeId, b: ShapeId) -> bool {
        fn walk(s: &Shape, a: ShapeId, b: ShapeId, n: &mut usize, at: &mut [Option<usize>; 2]) {
            if s.id == a {
                at[0] = Some(*n);
            }
            if s.id == b {
                at[1] = Some(*n);
            }
            *n += 1;
            match &s.kind {
                ShapeKind::Group { children } => children.iter().for_each(|c| walk(c, a, b, n, at)),
                ShapeKind::ClipFrame { frame, contents } => {
                    walk(frame, a, b, n, at);
                    contents.iter().for_each(|c| walk(c, a, b, n, at));
                }
                _ => {}
            }
        }
        let mut n = 0;
        let mut at = [None, None];
        for l in self.page_layers() {
            for s in &l.shapes {
                walk(s, a, b, &mut n, &mut at);
            }
        }
        match at {
            [Some(pa), Some(pb)] => pa < pb,
            _ => false,
        }
    }

    /// Every object with a blend, and its blend.
    fn all_blends(&self) -> Vec<(ShapeId, Effect)> {
        fn walk(s: &Shape, out: &mut Vec<(ShapeId, Effect)>) {
            if let Some(e) = s.effects.iter().find(|e| is_blend(e)) {
                out.push((s.id, e.clone()));
            }
            match &s.kind {
                ShapeKind::Group { children } => children.iter().for_each(|c| walk(c, out)),
                ShapeKind::ClipFrame { frame, contents } => {
                    walk(frame, out);
                    contents.iter().for_each(|c| walk(c, out));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for l in self.doc().all_layers() {
            for s in &l.shapes {
                walk(s, &mut out);
            }
        }
        out
    }

    /// The blends of the selection: those of selected start objects and
    /// those whose end object is selected (start object, blend), each
    /// once.
    pub fn selected_blends(&self) -> Vec<(ShapeId, Effect)> {
        let mut out: Vec<(ShapeId, Effect)> = Vec::new();
        for (id, e) in self.all_blends() {
            let end = match &e {
                Effect::Blend { end_id, .. } => *end_id,
                _ => None,
            };
            let chosen = self.selection.contains(&id)
                || end.is_some_and(|end| self.selection.contains(&end));
            if chosen && !out.iter().any(|(o, _)| *o == id) {
                out.push((id, e));
            }
        }
        out
    }

    /// Change the selection's blends with `f` (`None` clears one), one
    /// undo step; a run of the same change on the same blends with
    /// nothing in between is one step.
    pub fn change_blends(&mut self, label: &'static str, f: impl Fn(&Effect) -> Option<Effect>) {
        let mut cmds = Vec::new();
        let mut ids = Vec::new();
        for (id, e) in self.selected_blends() {
            let next = f(&e);
            if next.as_ref() == Some(&e) {
                continue;
            }
            let Some(s) = self.doc().find_shape(id) else {
                continue;
            };
            ids.push(id);
            cmds.push(Command::SetEffects {
                shape: id,
                effects: with_blend(&s.effects, next),
            });
        }
        if cmds.is_empty() {
            return;
        }
        let key = (self.engine.revision(), ids.clone(), label.to_string());
        if self.engine.undo_label() == Some(label) && self.effect_edit_step.as_ref() == Some(&key) {
            let _ = self.engine.undo();
        }
        if let Err(e) = self.engine.run_batch(label, &cmds) {
            self.status = e.to_string();
            return;
        }
        self.effect_edit_step = Some((self.engine.revision(), ids, label.to_string()));
    }

    /// Blends follow their end objects (see the module notes). Runs once
    /// per change of the drawing; the changes join the step that made it.
    pub fn refresh_blends(&mut self) {
        let revision = self.engine.revision();
        if revision == self.blends_checked {
            return;
        }
        let mut cmds = Vec::new();
        for (id, e) in self.all_blends() {
            let Effect::Blend {
                end,
                end_id: Some(end_id),
                ..
            } = &e
            else {
                continue;
            };
            let Some(start) = self.doc().find_shape(id) else {
                continue;
            };
            let next = match self.doc().shape_on_page(*end_id) {
                // The end object is gone: so is the blend.
                None => None,
                Some(now) => {
                    // The copy lives where the start object's effects do:
                    // in the space of the group or frame holding it, which
                    // is the page for an object on a layer. Kept in page
                    // space, the steps of a grouped blend were drawn
                    // through the group's transform twice and stayed
                    // behind when the group moved (#2023).
                    let mut copy = end_copy(&now);
                    let parent = self
                        .doc()
                        .parent_transform(id)
                        .unwrap_or(tracedraw_core::Affine::IDENTITY);
                    let det = parent.determinant();
                    if det.is_finite()
                        && det.abs() > 1e-12
                        && parent != tracedraw_core::Affine::IDENTITY
                    {
                        copy.transform = parent.inverse() * copy.transform;
                    }
                    // The steps go between the two objects: below the
                    // start when the end is below it.
                    let below = self.stacked_below(*end_id, id);
                    let was_below =
                        matches!(&e, Effect::Blend { options, .. } if options.steps_below);
                    if copy == **end && below == was_below {
                        continue;
                    }
                    let mut b = e.clone();
                    if let Effect::Blend { end, options, .. } = &mut b {
                        **end = copy;
                        options.steps_below = below;
                    }
                    Some(b)
                }
            };
            cmds.push(Command::SetEffects {
                shape: id,
                effects: with_blend(&start.effects, next),
            });
        }
        for cmd in &cmds {
            if let Err(e) = self.engine.amend(cmd) {
                self.status = e.to_string();
                break;
            }
        }
        self.blends_checked = self.engine.revision();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::{
        geometry::{Affine, Rect},
        Color, Fill,
    };

    fn square(app: &mut App, x: f64, c: Color) -> ShapeId {
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(x, 0.0, x + 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("square");
        app.run(Command::SetFill {
            shapes: vec![id],
            fill: Fill::Solid(c),
        });
        id
    }

    fn steps_of(app: &App, id: ShapeId) -> Vec<Shape> {
        let s = app.doc().find_shape(id).cloned().expect("start");
        tracedraw_core::live::evaluate(&s).above
    }

    /// The steps sit between the two objects: with the end object below
    /// the start one (a front object blended to one behind it, as in
    /// #232) they are drawn below the start, from the end's side up, and
    /// they move above it when the end is brought to the front.
    #[test]
    fn the_steps_lie_between_the_two_objects() {
        let mut app = App::headless();
        // The end first (below), the start on top of it.
        let end = square(&mut app, 100.0, Color::rgb8(0, 160, 80));
        let start = square(&mut app, 0.0, Color::rgb8(250, 220, 0));
        app.select(vec![start, end]);
        app.blend_selection(3, 0.0, 0.0, 0.0);
        app.refresh_blends();
        let ev = |app: &App| {
            let s = app.doc().find_shape(start).cloned().expect("start");
            tracedraw_core::live::evaluate(&s)
        };
        let e = ev(&app);
        assert!(e.above.is_empty());
        assert_eq!(e.below.len(), 3);
        // Bottom to top: the step nearest the end first.
        let xs: Vec<f64> = e.below.iter().map(|s| s.bounds().center().x).collect();
        assert!(xs[0] > xs[1] && xs[1] > xs[2], "{xs:?}");
        // The end brought to the front: the steps go above the start, in
        // the same step as the reordering.
        let before = app.engine.history_labels().0.len();
        app.select(vec![end]);
        app.order(0);
        app.refresh_blends();
        assert_eq!(app.engine.history_labels().0.len(), before + 1);
        let e = ev(&app);
        assert!(e.below.is_empty());
        assert_eq!(e.above.len(), 3);
    }

    /// A blend made with the tool's defaults has 20 steps, enough for the
    /// depth of a front object blended to a copy just behind it to read
    /// as a smooth side rather than bands (#232).
    #[test]
    fn a_new_blend_takes_twenty_steps() {
        assert_eq!(crate::settings::ToolPrefs::default().blend_steps, 20);
        let mut app = App::headless();
        assert_eq!(app.blend_steps, 20);
        let end = square(&mut app, 30.0, Color::rgb8(0, 160, 80));
        let start = square(&mut app, 0.0, Color::rgb8(250, 220, 0));
        app.select(vec![start, end]);
        app.tool = crate::tools::Tool::Blend;
        let (a, b) = (
            app.doc()
                .find_shape(start)
                .expect("start")
                .bounds()
                .center(),
            app.doc().find_shape(end).expect("end").bounds().center(),
        );
        app.drag = crate::app::Drag::Connector {
            from: app.hit_test_inside(a).expect("start is hit"),
            start: a,
            current: b,
        };
        app.end_drag();
        app.refresh_blends();
        let s = app.doc().find_shape(start).cloned().expect("start");
        let e = tracedraw_core::live::evaluate(&s);
        assert_eq!(e.below.len() + e.above.len(), 20);
    }

    /// A blend grouped and the group moved: its steps go with the two
    /// objects and stay between them (#2023).
    #[test]
    fn a_grouped_blend_moves_with_its_group() {
        let mut app = App::headless();
        let a = square(&mut app, 0.0, Color::rgb8(255, 0, 0));
        let b = square(&mut app, 100.0, Color::rgb8(0, 0, 255));
        app.select(vec![a, b]);
        app.blend_selection(3, 0.0, 0.0, 0.0);
        app.refresh_blends();
        app.select(vec![a, b]);
        app.group_selection();
        app.refresh_blends();
        let group = app.selection[0];
        app.run(Command::TransformShapes {
            shapes: vec![group],
            transform: Affine::translate((0.0, 40.0)),
        });
        app.refresh_blends();
        // The middle step, through the group's transform, lies halfway
        // between the two moved squares.
        let parent = app.doc().parent_transform(a).expect("in the group");
        let mut mid = steps_of(&app, a)[1].clone();
        mid.transform = parent * mid.transform;
        let c = mid.bounds().center();
        assert!(
            (c.x - 55.0).abs() < 0.5 && (c.y - 45.0).abs() < 0.5,
            "{c:?}"
        );
    }

    /// A front object blended to one behind it, then grouped, recoloured
    /// as a group and placed inside a frame: the steps stay below the
    /// start object every time instead of jumping in front of it, where
    /// the grouped blend looked all in the end's colour (#2023, #2120).
    #[test]
    fn a_grouped_blend_keeps_its_steps_behind_the_front_object() {
        let mut app = App::headless();
        let end = square(&mut app, 30.0, Color::rgb8(0, 160, 80));
        let start = square(&mut app, 0.0, Color::rgb8(250, 220, 0));
        app.select(vec![start, end]);
        app.blend_selection(5, 0.0, 0.0, 0.0);
        app.refresh_blends();
        let ev = |app: &App| {
            let s = app.doc().find_shape(start).cloned().expect("start");
            tracedraw_core::live::evaluate(&s)
        };
        let below = |app: &App, what: &str| {
            let e = ev(app);
            assert!(e.above.is_empty(), "{what}: steps above the start");
            assert_eq!(e.below.len(), 5, "{what}");
        };
        below(&app, "made");
        app.select(vec![start, end]);
        app.group_selection();
        app.refresh_blends();
        below(&app, "grouped");
        let group = app.selection[0];
        app.run(Command::SetFill {
            shapes: vec![group],
            fill: Fill::Solid(Color::rgb8(200, 0, 0)),
        });
        app.refresh_blends();
        app.run(Command::SetFill {
            shapes: vec![start],
            fill: Fill::Solid(Color::rgb8(250, 220, 0)),
        });
        app.refresh_blends();
        below(&app, "group recoloured");
        app.run(Command::TransformShapes {
            shapes: vec![group],
            transform: Affine::translate((5.0, 5.0)),
        });
        app.refresh_blends();
        below(&app, "group moved");
        // The group placed inside a frame.
        let frame = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(-50.0, -50.0, 100.0, 100.0),
                radius: 0.0,
                corners: None,
            })
            .expect("frame");
        app.place_inside(vec![group], frame);
        app.refresh_blends();
        below(&app, "inside a frame");
    }

    #[test]
    fn the_end_object_stays_and_the_steps_follow_it() {
        let mut app = App::headless();
        let a = square(&mut app, 0.0, Color::rgb8(255, 0, 0));
        let b = square(&mut app, 100.0, Color::rgb8(0, 0, 255));
        app.select(vec![a, b]);
        app.blend_selection(3, 0.0, 0.0, 0.0);
        app.refresh_blends();
        // Both objects are there; the effect draws the three steps only.
        assert!(app.doc().find_shape(b).is_some());
        assert_eq!(steps_of(&app, a).len(), 3);
        let mid = steps_of(&app, a)[1].bounds().center();
        assert!((mid.x - 55.0).abs() < 0.5, "{mid:?}");
        // The end object moved: the steps follow, in the same step.
        let before = app.engine.history_labels().0.len();
        app.run(Command::TransformShapes {
            shapes: vec![b],
            transform: Affine::translate((0.0, 40.0)),
        });
        app.refresh_blends();
        assert_eq!(app.engine.history_labels().0.len(), before + 1);
        let mid = steps_of(&app, a)[1].bounds().center();
        assert!((mid.y - 25.0).abs() < 0.5, "{mid:?}");
        // Undo puts both back and needs no refresh (Redo stays).
        app.undo();
        app.refresh_blends();
        assert!(app.engine.redo_label().is_some());
        // Selecting the end object picks the blend for the bar.
        app.select(vec![b]);
        assert_eq!(app.selected_blends().len(), 1);
        app.change_blends("Blend", |e| {
            let mut e = e.clone();
            if let Effect::Blend { steps, .. } = &mut e {
                *steps = 7;
            }
            Some(e)
        });
        assert_eq!(steps_of(&app, a).len(), 7);
        // Deleting the end object takes the blend away.
        app.run(Command::DeleteShapes { shapes: vec![b] });
        app.refresh_blends();
        assert!(steps_of(&app, a).is_empty());
        let s = app.doc().find_shape(a).expect("start");
        assert!(!s.effects.iter().any(is_blend));
        // Clear Blend on a fresh pair leaves both objects.
        let c = square(&mut app, 200.0, Color::BLACK);
        app.select(vec![a, c]);
        app.blend_selection(2, 0.0, 0.0, 0.0);
        app.select(vec![c]);
        app.clear_blend();
        assert!(app.doc().find_shape(c).is_some());
        assert!(steps_of(&app, a).is_empty());
    }

    /// A blend of two texts saved as `.cdr` and opened again is still a
    /// blend: the end text is its end object, moving it redraws the
    /// steps, and the Blend panel finds it (#986).
    #[test]
    fn a_text_blend_survives_saving_and_opening_as_cdr() {
        let mut app = App::headless();
        let text = |app: &mut App, x: f64, c: Color| {
            let id = app
                .new_shape(ShapeKind::Text {
                    spans: vec![tracedraw_core::document::TextSpan::new("A", "Sans", 200.0)],
                    origin: tracedraw_core::geometry::Point::new(x, 100.0),
                    frame: None,
                    align: tracedraw_core::document::TextAlign::Left,
                    para: Default::default(),
                    on_path: None,
                })
                .expect("text");
            app.run(Command::SetFill {
                shapes: vec![id],
                fill: Fill::Solid(c),
            });
            id
        };
        let start = text(&mut app, 20.0, Color::rgb8(255, 240, 0));
        let end = text(&mut app, 90.0, Color::rgb8(0, 180, 80));
        app.select(vec![start, end]);
        app.blend_selection(20, 0.0, 0.0, 0.0);
        app.refresh_blends();
        assert_eq!(steps_of(&app, start).len(), 20);
        let bytes = tracedraw_cdr::write::document_to_cdr(app.doc());
        let (doc, _) = tracedraw_cdr::open_bytes(&bytes, "t").expect("read back");
        let mut back = App::headless();
        back.open_document(doc, None);
        let shapes = back.doc().pages[0].layers[0].shapes.clone();
        assert_eq!(shapes.len(), 2, "{shapes:?}");
        let (a, b) = (shapes[0].id, shapes[1].id);
        // The steps run from the start text to the end text, each a bit
        // further right than the one before. (Their exact places are
        // checked in the cdr crate: here other tests of the run carry
        // fonts in, which changes what "Sans" draws with meanwhile.)
        let again: Vec<Rect> = steps_of(&back, a).iter().map(|s| s.bounds()).collect();
        assert_eq!(again.len(), 20);
        let first = back.doc().find_shape(a).expect("start").bounds();
        let last = back.doc().find_shape(b).expect("end").bounds();
        assert!(
            first.x0 < again[0].x0 && again[19].x1 < last.x1,
            "{first:?} {again:?} {last:?}"
        );
        for w in again.windows(2) {
            assert!(w[0].x0 < w[1].x0, "{again:?}");
        }
        back.select(vec![b]);
        assert_eq!(back.selected_blends().len(), 1);
        back.run(Command::TransformShapes {
            shapes: vec![b],
            transform: Affine::translate((0.0, 40.0)),
        });
        back.refresh_blends();
        let moved: Vec<Rect> = steps_of(&back, a).iter().map(|s| s.bounds()).collect();
        assert!(moved[10].center().y > again[10].center().y + 15.0);
    }
}
