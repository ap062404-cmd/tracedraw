//! Edit > Invert Selection and Edit > Select Similar (#2002): selection
//! commands that pick objects of the current page from what is selected.
//! Each one is a step of its own on the Undo list: Undo right after it,
//! with the drawing unchanged since, puts the selection back as it was
//! (no step of the drawing is undone), and Redo makes it again.

use crate::app::App;
use tracedraw_core::{PageId, Shape, ShapeId, ShapeKind};

/// What Select Similar compares with the selected objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Similar {
    /// The same fill (colour, gradient, pattern or none).
    Fill,
    /// The same outline (width, colour, style), or no outline.
    Outline,
    /// Text with the same font and size.
    TextFormat,
    /// The same kind of object (rectangles, curves, artistic text...).
    Kind,
}

impl Similar {
    pub const ALL: [Similar; 4] = [
        Similar::Fill,
        Similar::Outline,
        Similar::TextFormat,
        Similar::Kind,
    ];

    /// The menu item's key.
    pub fn key(self) -> &'static str {
        match self {
            Similar::Fill => "menu.edit.similar_fill",
            Similar::Outline => "menu.edit.similar_outline",
            Similar::TextFormat => "menu.edit.similar_text_format",
            Similar::Kind => "menu.edit.similar_kind",
        }
    }

    /// The Edit > Repeat on Similar item's key.
    pub fn repeat_key(self) -> &'static str {
        match self {
            Similar::Fill => "menu.edit.repeat_similar_fill",
            Similar::Outline => "menu.edit.repeat_similar_outline",
            Similar::TextFormat => "menu.edit.repeat_similar_text_format",
            Similar::Kind => "menu.edit.repeat_similar_kind",
        }
    }
}

/// The fonts and sizes a text's characters use (families compared
/// without case, sizes to a hundredth of a point).
fn text_formats(shape: &Shape) -> Vec<(String, i64)> {
    let ShapeKind::Text { spans, .. } = &shape.kind else {
        return Vec::new();
    };
    let mut out: Vec<(String, i64)> = Vec::new();
    for s in spans {
        if s.text.trim().is_empty() && spans.len() > 1 {
            continue;
        }
        let f = (
            s.font_family.trim().to_lowercase(),
            (s.size_pt * 100.0).round() as i64,
        );
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// The kind of object, as Select Similar > Object Type reads it: the
/// shape kind, artistic and paragraph text apart.
fn kind_of(shape: &Shape) -> (std::mem::Discriminant<ShapeKind>, bool) {
    let paragraph = matches!(&shape.kind, ShapeKind::Text { frame: Some(_), .. });
    (std::mem::discriminant(&shape.kind), paragraph)
}

/// The objects of `shape` Select Similar compares: the shape itself, or
/// for a group the objects in it at any depth, the groups themselves
/// left out. A group has no fill or outline of its own to compare (its
/// own fill is none), so compared as one object it matched every object
/// without a fill and none of the objects it holds (#2203).
/// For Object Type (`groups`) a group is an object of its kind as well,
/// kept beside the objects in it.
fn members<'a>(shape: &'a Shape, groups: bool, out: &mut Vec<&'a Shape>) {
    match &shape.kind {
        ShapeKind::Group { children } => {
            if groups {
                out.push(shape);
            }
            for c in children {
                members(c, groups, out);
            }
        }
        _ => out.push(shape),
    }
}

/// Whether `candidate` is like `reference` in the way `how` asks.
pub fn is_similar(reference: &Shape, candidate: &Shape, how: Similar) -> bool {
    match how {
        Similar::Fill => reference.fill == candidate.fill,
        Similar::Outline => reference.stroke == candidate.stroke,
        Similar::TextFormat => {
            let want = text_formats(reference);
            !want.is_empty() && text_formats(candidate).iter().any(|f| want.contains(f))
        }
        Similar::Kind => kind_of(reference) == kind_of(candidate),
    }
}

/// One selection command on the Undo list: the selection before and
/// after it, on `page`, at the drawing's `revision`.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionStep {
    /// The command's menu key, for the Undo and Redo labels.
    pub key: &'static str,
    pub before: Vec<ShapeId>,
    pub after: Vec<ShapeId>,
    pub page: PageId,
    pub revision: u64,
}

impl SelectionStep {
    /// The step's name on the Undo and Redo items: "Invert Selection",
    /// "Select Similar (Fill)".
    pub fn label(&self) -> String {
        if Similar::ALL.iter().any(|h| h.key() == self.key) {
            format!(
                "{} ({})",
                crate::i18n::tr("menu.edit.select_similar"),
                crate::i18n::tr(self.key)
            )
        } else {
            crate::i18n::tr(self.key)
        }
    }
}

/// The selection steps that can be undone and redone, newest last.
#[derive(Debug, Clone, Default)]
pub struct SelectionSteps {
    pub undo: Vec<SelectionStep>,
    pub redo: Vec<SelectionStep>,
}

impl SelectionSteps {
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

impl App {
    /// The objects Select All takes: those of the current page's unlocked
    /// layers (the contents only while a clip frame's are edited).
    pub fn selectable_ids(&self) -> Vec<ShapeId> {
        self.page_layers()
            .into_iter()
            .filter(|l| !l.locked)
            .flat_map(|l| &l.shapes)
            .map(|s| s.id)
            .filter(|id| !self.outside_clip_edit(*id))
            .collect()
    }

    /// The top-level objects of the page Select All takes, as shapes.
    fn selectable_shapes(&self) -> Vec<&Shape> {
        self.page_layers()
            .into_iter()
            .filter(|l| !l.locked)
            .flat_map(|l| &l.shapes)
            .filter(|s| !self.outside_clip_edit(s.id))
            .collect()
    }

    /// Edit > Invert Selection: every object Select All would take that
    /// is not selected now, and none of those that are. One undo step.
    pub fn invert_selection(&mut self) {
        let ids: Vec<ShapeId> = self
            .selectable_ids()
            .into_iter()
            .filter(|id| !self.selection.contains(id))
            .collect();
        self.selection_command("menu.edit.invert_selection", ids);
    }

    /// Edit > Select Similar: every object of the page Select All would
    /// take, and every object in its groups, that is like one of the
    /// selected objects in the way `how` asks (the selected ones among
    /// them; a selected group stands for the objects in it). Nothing
    /// selected: nothing happens. One undo step.
    pub fn select_similar(&mut self, how: Similar) {
        if self.selection.is_empty() {
            return;
        }
        let selected: Vec<Shape> = self.selected_shapes();
        let ids = self.similar_ids(&selected, how);
        if ids.is_empty() {
            return;
        }
        self.selection_command(how.key(), ids);
    }

    /// The objects of the page Select All would take, and the objects in
    /// their groups, that are like one of `shapes` in the way `how` asks
    /// (`shapes` among them when they are on the page).
    pub(crate) fn similar_ids(&self, shapes: &[Shape], how: Similar) -> Vec<ShapeId> {
        // Object Type compares a selected group as a group; the other ways
        // compare the objects in it.
        let kind = how == Similar::Kind;
        let mut refs = Vec::new();
        for s in shapes {
            if kind {
                refs.push(s);
            } else {
                members(s, false, &mut refs);
            }
        }
        // The objects in groups count too, each on its own (#2203).
        let mut candidates = Vec::new();
        for s in self.selectable_shapes() {
            members(s, kind, &mut candidates);
        }
        candidates
            .into_iter()
            .filter(|c| refs.iter().any(|r| is_similar(r, c, how)))
            .map(|c| c.id)
            .collect()
    }

    /// Edit > Repeat on Similar: what Edit > Repeat would repeat, done
    /// again to every object of the page like the objects it was done to
    /// (as they were before it) in the way `how` asks, those objects and
    /// the objects in them left out. A move, turn or resize, or a copy, is
    /// repeated on each object on its own (a turn about each object's own
    /// centre); fills, outlines, opacity and Recolor on all of them at
    /// once. One undo step; the objects it was done to become the
    /// selection. The number of objects.
    pub fn repeat_on_similar(&mut self, how: Similar) -> usize {
        let entries = self.repeat_group_entries();
        let Some(first) = entries.first() else {
            return 0;
        };
        let refs: Vec<Shape> = if first.refs.is_empty() {
            first
                .before
                .iter()
                .filter_map(|id| self.doc().find_shape(*id).cloned())
                .collect()
        } else {
            first.refs.clone()
        };
        // What was done already: the objects of every action of the group,
        // before and after (a copy), and everything inside them.
        let mut done: Vec<ShapeId> = Vec::new();
        for e in &entries {
            for id in e.before.iter().chain(&e.after) {
                if let Some(s) = self.doc().find_shape(*id) {
                    let mut inside = Vec::new();
                    members(s, true, &mut inside);
                    done.extend(inside.iter().map(|m| m.id));
                }
                done.push(*id);
            }
        }
        let mut group: Vec<crate::app::Repeatable> = Vec::new();
        for e in &entries {
            match &e.action {
                crate::app::Repeatable::Group(actions) => group.extend(actions.iter().cloned()),
                a => group.push(a.clone()),
            }
        }
        let targets: Vec<ShapeId> = self
            .similar_ids(&refs, how)
            .into_iter()
            .filter(|id| !done.contains(id))
            .collect();
        if targets.is_empty() {
            self.status = crate::i18n::tr("status.repeat_on_similar_none");
            return 0;
        }
        // An object inside another target goes with it (a group's members
        // matched with the group by Object Type).
        let targets: Vec<ShapeId> = targets
            .iter()
            .copied()
            .filter(|id| {
                !targets.iter().any(|other| {
                    other != id
                        && self.doc().find_shape(*other).is_some_and(|s| {
                            let mut inside = Vec::new();
                            members(s, true, &mut inside);
                            inside.iter().any(|m| m.id == *id && m.id != s.id)
                        })
                })
            })
            .collect();
        let each = group.iter().any(|a| {
            !matches!(a, crate::app::Repeatable::Command(c)
                if !matches!(**c, tracedraw_core::Command::TransformShapes { .. }))
        });
        let mark = self.engine.begin_group();
        let mut after = Vec::new();
        if each {
            for id in &targets {
                self.select(vec![*id]);
                for a in &group {
                    self.replay(a.clone());
                }
                after.extend(self.selection.iter().copied());
            }
        } else {
            self.select(targets.clone());
            for a in &group {
                self.replay(a.clone());
            }
            after = self.selection.clone();
        }
        self.engine.end_group(mark, "Repeat on Similar");
        self.select(after);
        // Edit > Repeat after it repeats the same actions again.
        if self.engine.last_step_id() != mark {
            self.note_repeatable_from(crate::app::Repeatable::Group(group), targets.clone());
        }
        let n = targets.len();
        self.status = crate::i18n::trf("status.repeat_on_similar_done", &[("n", &n.to_string())]);
        n
    }

    /// Select `ids` as the selection command `key`, kept on the Undo list.
    fn selection_command(&mut self, key: &'static str, ids: Vec<ShapeId>) {
        if !self.has_document() || ids == self.selection {
            return;
        }
        let step = SelectionStep {
            key,
            before: self.selection.clone(),
            after: ids.clone(),
            page: self.page,
            revision: self.engine.revision(),
        };
        // Earlier steps stay only while nothing changed since them.
        if !self.selection_step_current() {
            self.selection_steps.clear();
        }
        self.select(ids);
        self.selection_steps.undo.push(step);
        self.selection_steps.redo.clear();
        self.status = crate::i18n::trf(
            "status.n_objects_selected",
            &[("n", &self.selection.len().to_string())],
        );
    }

    /// The newest selection step still matches the drawing and the
    /// selection (nothing changed since it was made or undone).
    fn selection_step_current(&self) -> bool {
        let fits = |s: &SelectionStep, sel: &Vec<ShapeId>| {
            s.revision == self.engine.revision() && s.page == self.page && *sel == self.selection
        };
        match (
            self.selection_steps.undo.last(),
            self.selection_steps.redo.last(),
        ) {
            (Some(u), _) => fits(u, &u.after),
            (None, Some(r)) => fits(r, &r.before),
            (None, None) => false,
        }
    }

    /// The selection step Undo would take back now, if any.
    fn undoable_selection_step(&self) -> Option<&SelectionStep> {
        let s = self.selection_steps.undo.last()?;
        (s.revision == self.engine.revision() && s.page == self.page && s.after == self.selection)
            .then_some(s)
    }

    /// The selection step Redo would make again now, if any.
    fn redoable_selection_step(&self) -> Option<&SelectionStep> {
        let s = self.selection_steps.redo.last()?;
        (s.revision == self.engine.revision() && s.page == self.page && s.before == self.selection)
            .then_some(s)
    }

    /// Undo of a selection command: the selection as it was before it.
    /// False (and the steps forgotten) when the drawing or the selection
    /// changed since, so Undo goes on to the drawing's steps.
    pub fn undo_selection_step(&mut self) -> bool {
        if self.undoable_selection_step().is_none() {
            self.selection_steps.undo.clear();
            return false;
        }
        let Some(step) = self.selection_steps.undo.pop() else {
            return false;
        };
        self.select(step.before.clone());
        self.status = crate::i18n::trf("status.undo_n", &[("l", &step.label())]);
        self.selection_steps.redo.push(step);
        true
    }

    /// Redo of a selection command undone just before.
    pub fn redo_selection_step(&mut self) -> bool {
        if self.redoable_selection_step().is_none() {
            self.selection_steps.redo.clear();
            return false;
        }
        let Some(step) = self.selection_steps.redo.pop() else {
            return false;
        };
        self.select(step.after.clone());
        self.status = crate::i18n::trf("status.redo_n", &[("l", &step.label())]);
        self.selection_steps.undo.push(step);
        true
    }

    /// What Edit > Undo takes back, as its menu shows it: a selection
    /// command, else the drawing's last step.
    pub fn undo_label(&self) -> Option<String> {
        match self.undoable_selection_step() {
            Some(s) => Some(s.label()),
            None => self.engine.undo_label().map(str::to_string),
        }
    }

    /// What Edit > Redo makes again, as its menu shows it.
    pub fn redo_label(&self) -> Option<String> {
        match self.redoable_selection_step() {
            Some(s) => Some(s.label()),
            None => self.engine.redo_label().map(str::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::geometry::Rect;
    use tracedraw_core::{Color, TextSpan};
    use tracedraw_core::{Fill, Stroke};

    fn rect(app: &mut App, x: f64, fill: Fill, stroke: Option<Stroke>) -> ShapeId {
        app.new_shape_styled(
            ShapeKind::Rect {
                rect: Rect::new(x, 10.0, x + 10.0, 20.0),
                radius: 0.0,
                corners: None,
            },
            |s| {
                s.fill = fill;
                s.stroke = stroke;
            },
        )
        .expect("rect")
    }

    fn ellipse(app: &mut App, x: f64, fill: Fill) -> ShapeId {
        app.new_shape_styled(
            ShapeKind::Ellipse {
                rect: Rect::new(x, 40.0, x + 10.0, 50.0),
                arc: None,
            },
            |s| s.fill = fill,
        )
        .expect("ellipse")
    }

    fn text(app: &mut App, x: f64, font: &str, size: f64) -> ShapeId {
        app.new_shape(ShapeKind::Text {
            spans: vec![TextSpan::new("Hello", font, size)],
            origin: tracedraw_core::geometry::Point::new(x, 80.0),
            frame: None,
            align: Default::default(),
            para: Default::default(),
            on_path: None,
        })
        .expect("text")
    }

    fn red() -> Fill {
        Fill::Solid(Color::rgb8(255, 0, 0))
    }

    fn blue() -> Fill {
        Fill::Solid(Color::rgb8(0, 0, 255))
    }

    fn sorted(mut v: Vec<ShapeId>) -> Vec<ShapeId> {
        v.sort();
        v
    }

    /// A drawing with two red rectangles (one with a thick outline), a
    /// blue rectangle, a red ellipse and three texts.
    struct Scene {
        app: App,
        red_a: ShapeId,
        red_b: ShapeId,
        blue_r: ShapeId,
        red_e: ShapeId,
        arial_12: ShapeId,
        arial_12b: ShapeId,
        serif_12: ShapeId,
    }

    fn scene() -> Scene {
        let mut app = App::headless();
        for id in app.selectable_ids() {
            app.select(vec![id]);
            app.delete_selection();
        }
        let thin = Some(Stroke::default());
        let thick = Some(Stroke {
            width: 2.0,
            ..Stroke::default()
        });
        let red_a = rect(&mut app, 0.0, red(), thin.clone());
        let red_b = rect(&mut app, 20.0, red(), thick);
        let blue_r = rect(&mut app, 40.0, blue(), thin);
        let red_e = ellipse(&mut app, 0.0, red());
        let arial_12 = text(&mut app, 0.0, "Arial", 12.0);
        let arial_12b = text(&mut app, 50.0, "arial", 12.0);
        let serif_12 = text(&mut app, 100.0, "Serif", 12.0);
        app.select(Vec::new());
        Scene {
            app,
            red_a,
            red_b,
            blue_r,
            red_e,
            arial_12,
            arial_12b,
            serif_12,
        }
    }

    #[test]
    fn invert_selection_takes_the_others_and_undo_puts_it_back() {
        let mut s = scene();
        let app = &mut s.app;
        let all = sorted(app.selectable_ids());
        assert_eq!(all.len(), 7);
        app.select(vec![s.red_a, s.blue_r]);
        let revision = app.engine.revision();
        app.invert_selection();
        assert_eq!(
            sorted(app.selection.clone()),
            sorted(
                all.iter()
                    .copied()
                    .filter(|id| *id != s.red_a && *id != s.blue_r)
                    .collect()
            )
        );
        // Nothing in the drawing changed: a selection step only.
        assert_eq!(app.engine.revision(), revision);
        assert_eq!(
            app.undo_label(),
            Some(crate::i18n::tr("menu.edit.invert_selection"))
        );
        app.undo();
        assert_eq!(app.selection, vec![s.red_a, s.blue_r]);
        assert_eq!(app.engine.revision(), revision, "no drawing step undone");
        app.redo();
        assert_eq!(app.selection.len(), 5);
        // Inverting twice gives the selection back.
        app.invert_selection();
        assert_eq!(
            sorted(app.selection.clone()),
            sorted(vec![s.red_a, s.blue_r])
        );
        // Nothing selected: everything.
        app.select(Vec::new());
        app.invert_selection();
        assert_eq!(sorted(app.selection.clone()), all);
    }

    #[test]
    fn select_similar_fill_outline_text_and_kind() {
        let mut s = scene();
        let app = &mut s.app;
        app.select(vec![s.red_a]);
        app.select_similar(Similar::Fill);
        assert_eq!(
            sorted(app.selection.clone()),
            sorted(vec![s.red_a, s.red_b, s.red_e])
        );
        app.select(vec![s.red_a]);
        app.select_similar(Similar::Outline);
        assert!(app.selection.contains(&s.blue_r), "same thin outline");
        assert!(!app.selection.contains(&s.red_b), "a thicker outline");
        app.select(vec![s.arial_12]);
        app.select_similar(Similar::TextFormat);
        assert_eq!(
            sorted(app.selection.clone()),
            sorted(vec![s.arial_12, s.arial_12b]),
            "the same font (any case) and size"
        );
        // An object without text finds no text formatting: no change.
        app.select(vec![s.red_a]);
        app.select_similar(Similar::TextFormat);
        assert_eq!(app.selection, vec![s.red_a]);
        app.select(vec![s.blue_r]);
        app.select_similar(Similar::Kind);
        assert_eq!(
            sorted(app.selection.clone()),
            sorted(vec![s.red_a, s.red_b, s.blue_r])
        );
        app.select(vec![s.serif_12]);
        app.select_similar(Similar::Kind);
        assert_eq!(app.selection.len(), 3, "the three artistic texts");
        // Several selected: like any of them.
        app.select(vec![s.blue_r, s.red_e]);
        app.select_similar(Similar::Fill);
        assert_eq!(
            sorted(app.selection.clone()),
            sorted(vec![s.red_a, s.red_b, s.blue_r, s.red_e])
        );
    }

    /// #2203: yellow and purple squares, two of them grouped, and two
    /// outlines without a fill. The objects in the group were never
    /// compared, and the group itself (whose own fill is none) matched the
    /// outlines.
    #[test]
    fn select_similar_reaches_into_groups_and_compares_what_they_hold() {
        let mut app = App::headless();
        for id in app.selectable_ids() {
            app.select(vec![id]);
            app.delete_selection();
        }
        let yellow = || Fill::Solid(Color::rgb8(255, 255, 0));
        let purple = || Fill::Solid(Color::rgb8(128, 0, 128));
        let line = Some(Stroke::default());
        let y1 = rect(&mut app, 0.0, yellow(), line.clone());
        let y2 = rect(&mut app, 20.0, yellow(), line.clone());
        let p1 = rect(&mut app, 40.0, purple(), line.clone());
        let y3 = rect(&mut app, 60.0, yellow(), line.clone());
        let p2 = rect(&mut app, 80.0, purple(), line.clone());
        let o1 = rect(&mut app, 100.0, Fill::None, line.clone());
        let o2 = rect(&mut app, 120.0, Fill::None, line.clone());
        let e = ellipse(&mut app, 140.0, yellow());
        app.select(vec![y3, p2]);
        app.group_selection();
        let g = app.selection[0];
        let mut run = |sel: Vec<ShapeId>, how: Similar| {
            app.select(sel);
            app.select_similar(how);
            sorted(app.selection.clone())
        };
        // A yellow square, at the top or in the group: every yellow object.
        let yellows = sorted(vec![y1, y2, y3, e]);
        assert_eq!(run(vec![y1], Similar::Fill), yellows);
        assert_eq!(run(vec![y3], Similar::Fill), yellows);
        // An outline without a fill: the other outlines, not the group.
        assert_eq!(run(vec![o1], Similar::Fill), sorted(vec![o1, o2]));
        // The group stands for its yellow and purple squares.
        assert_eq!(
            run(vec![g], Similar::Fill),
            sorted(vec![y1, y2, p1, y3, p2, e])
        );
        // Object Type: rectangles in the group too; a group finds groups.
        assert_eq!(
            run(vec![y1], Similar::Kind),
            sorted(vec![y1, y2, p1, y3, p2, o1, o2])
        );
        assert_eq!(run(vec![g], Similar::Kind), vec![g]);
    }

    #[test]
    fn selection_steps_undo_in_order_and_end_with_a_drawing_change() {
        let mut s = scene();
        let app = &mut s.app;
        app.select(vec![s.red_a]);
        app.select_similar(Similar::Fill);
        app.select_similar(Similar::Kind);
        assert_eq!(app.selection.len(), 4, "the rectangles and the ellipse");
        assert_eq!(
            app.undo_label(),
            Some(format!(
                "{} ({})",
                crate::i18n::tr("menu.edit.select_similar"),
                crate::i18n::tr("menu.edit.similar_kind")
            ))
        );
        app.undo();
        assert_eq!(app.selection.len(), 3);
        let fill = Some(format!(
            "{} ({})",
            crate::i18n::tr("menu.edit.select_similar"),
            crate::i18n::tr("menu.edit.similar_fill")
        ));
        assert_eq!(app.undo_label(), fill);
        app.undo();
        assert_eq!(app.selection, vec![s.red_a]);
        assert_eq!(app.redo_label(), fill);
        app.redo();
        assert_eq!(app.selection.len(), 3);
        // A change to the drawing: Undo takes back the drawing's step,
        // the selection steps are gone.
        let shapes = app.selectable_ids().len();
        app.select(vec![s.red_a]);
        app.invert_selection();
        app.delete_selection();
        assert_eq!(app.selectable_ids().len(), 1);
        assert_ne!(
            app.undo_label(),
            Some(crate::i18n::tr("menu.edit.invert_selection"))
        );
        app.undo();
        assert_eq!(app.selectable_ids().len(), shapes, "the delete undone");
        // A selection made by hand after the step: Undo is the drawing's.
        app.select(vec![s.red_a]);
        app.invert_selection();
        app.select(vec![s.red_e]);
        assert_ne!(
            app.undo_label(),
            Some(crate::i18n::tr("menu.edit.invert_selection"))
        );
    }

    #[test]
    fn the_menu_items_run_by_key() {
        let mut s = scene();
        let ctx = crate::theme::ui_context();
        let app = &mut s.app;
        app.select(vec![s.red_a]);
        let mut ran = false;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ran = crate::ui::menus::run_command(app, ui.ctx(), "menu.edit.similar_fill");
        });
        out.textures_delta.clear();
        assert!(ran);
        assert_eq!(app.selection.len(), 3);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ran = crate::ui::menus::run_command(app, ui.ctx(), "menu.edit.invert_selection");
        });
        out.textures_delta.clear();
        assert!(ran);
        assert_eq!(app.selection.len(), 4);
        assert!(!app.selection.contains(&s.red_a));
        let _ = (s.red_b, s.red_e);
    }

    fn fill_of(app: &App, id: ShapeId) -> Option<Fill> {
        app.doc().find_shape(id).map(|s| s.fill.clone())
    }

    /// A fill change of one red rectangle, repeated on the objects with
    /// the fill it had (red), not on the blue one; one undo step; the
    /// objects it was done to become the selection.
    #[test]
    fn repeat_on_similar_fill_uses_the_fill_before_the_change() {
        let mut s = scene();
        let app = &mut s.app;
        let green = Fill::Solid(Color::rgb8(0, 200, 0));
        app.select(vec![s.red_a]);
        app.apply_fill(green.clone());
        let steps = app.engine.history_labels().0.len();
        assert_eq!(app.repeat_on_similar(Similar::Fill), 2);
        assert_eq!(fill_of(app, s.red_b), Some(green.clone()));
        assert_eq!(fill_of(app, s.red_e), Some(green.clone()));
        assert_eq!(fill_of(app, s.blue_r), Some(blue()));
        assert_eq!(app.engine.history_labels().0.len(), steps + 1);
        assert_eq!(
            sorted(app.selection.clone()),
            sorted(vec![s.red_b, s.red_e])
        );
        app.undo();
        assert_eq!(fill_of(app, s.red_b), Some(red()));
        assert_eq!(fill_of(app, s.red_a), Some(green), "the first change stays");
    }

    /// By object type: a move of one rectangle moves the other
    /// rectangles, each by the same distance; the ellipse stays.
    #[test]
    fn repeat_on_similar_kind_moves_each_object() {
        let mut s = scene();
        let app = &mut s.app;
        let x = |app: &App, id| {
            app.doc()
                .find_shape(id)
                .map(|s| s.bounds().x0)
                .unwrap_or(0.0)
        };
        let (b0, blue0, e0) = (x(app, s.red_b), x(app, s.blue_r), x(app, s.red_e));
        app.select(vec![s.red_a]);
        app.run(tracedraw_core::Command::TransformShapes {
            shapes: vec![s.red_a],
            transform: tracedraw_core::geometry::Affine::translate((5.0, 0.0)),
        });
        assert_eq!(app.repeat_on_similar(Similar::Kind), 2);
        assert!((x(app, s.red_b) - b0 - 5.0).abs() < 1e-6);
        assert!((x(app, s.blue_r) - blue0 - 5.0).abs() < 1e-6);
        assert!((x(app, s.red_e) - e0).abs() < 1e-6);
    }

    /// A Recolor of one object repeats on the others with its colours,
    /// and nothing like it leaves the drawing as it was.
    #[test]
    fn repeat_on_similar_recolor_and_nothing_similar() {
        let mut s = scene();
        let app = &mut s.app;
        app.select(vec![s.blue_r]);
        let r = tracedraw_core::recolor::Recolor {
            hue: 90.0,
            ..Default::default()
        };
        assert!(app.recolor_shapes(vec![s.blue_r], r));
        assert_eq!(app.repeat_on_similar(Similar::Fill), 0, "no other blue");
        app.select(vec![s.red_a]);
        assert!(app.recolor_shapes(vec![s.red_a], r));
        assert_eq!(app.repeat_on_similar(Similar::Fill), 2);
        assert_eq!(fill_of(app, s.red_b), fill_of(app, s.red_a));
        assert_ne!(fill_of(app, s.red_b), Some(red()));
    }
}
