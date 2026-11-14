//! Strokes of the shaping brushes (Smooth, Smear, Twirl, Attract and
//! Repel, Smudge, Roughen): a drag over the selected objects reshapes the
//! curves under the nib, step by step, the whole stroke one undo step.
//! Brushes change only what is selected: with nothing selected a drag
//! does nothing and a click selects (#271). Objects that are
//! not curves are converted first. The settings come from the property bar
//! (`ui::brush_bars`).
//!
//! Twirl and Attract and Repel go on while the button is held still.
//! Held in one place, Attract and Repel work from the outline as the
//! hold found it ([`StrokeBase`]) with the dose held so far, so they
//! pull or push at a steady rate and come to rest once the dose is full;
//! a move starts afresh from the outline as it is. Roughen works out the
//! whole stroke from the outline it started from and the way the nib
//! went, so going over the same place again adds no nodes.
//!
//! The nib is sized on the drawing too: a drag that starts with Shift
//! held sizes it instead of brushing (right grows it, left shrinks it,
//! the circle showing where the press was), and the wheel over the
//! drawing sizes it a notch at a time with Shift held (Alt held when the
//! wheel scrolls instead of zooming, where Shift+wheel scrolls sideways).

use crate::app::{App, Drag};
use crate::tools::Tool;
use crate::ui::brush_bars::BrushSettings;
use egui::{Modifiers, PointerButton, Response};
use tracedraw_core::{
    effects,
    geometry::{BezPath, Point},
    Command, ShapeId, ShapeKind,
};

/// How far Twirl and Attract and Repel go per second while the button is
/// held still, as a pointer move (mm).
pub const HOLD_MM_PER_S: f64 = 10.0;

/// How far Twirl turns per second while the button is down, as a pointer
/// move (mm): at the bar's 50 % (0.05 rad per mm) one whole turn a
/// second at the nib's centre, as CorelDRAW's Twirl winds the outline
/// round while it is held. It went on as if the pointer moved 10 mm a
/// second, a turn every 12 s, and read as slow (#2640).
pub const TWIRL_MM_PER_S: f64 = std::f64::consts::TAU / 0.05;

/// Attract and Repel's dose for each millimetre the pointer moves (or,
/// held still, [`HOLD_MM_PER_S`] a second), times the bar's rate, over
/// the nib's radius: at 50 % speed (rate 0.3) a 10 mm nib reaches its
/// whole effect after 17 mm of moves, or 1.7 s held still. Over the
/// radius, so the pull or push goes as many millimetres a second
/// whatever the nib's size.
pub const ATTRACT_DOSE: f64 = 2.0;

/// What a brush stroke works from (see the module notes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StrokeBase {
    /// The selected curves on the page as the hold (Attract and Repel)
    /// or the stroke (Roughen) found them.
    pub base: Vec<(ShapeId, BezPath)>,
    /// Attract and Repel's dose (-1 to 1) held so far on `base`.
    pub dose: f64,
    /// Twirl: the turn (radians) held so far on `base`.
    pub angle: f64,
    /// Roughen: where the nib went in this stroke.
    pub swath: Vec<Point>,
}

/// The smallest and the largest nib radius (mm), as the property bar
/// takes them.
pub const MIN_RADIUS: f64 = 0.5;
pub const MAX_RADIUS: f64 = 500.0;

/// How much one wheel notch grows (or, turned the other way, shrinks)
/// the nib.
pub const WHEEL_STEP: f64 = 1.1;

/// The nib being sized by a Shift+drag: the press was at `center` (page)
/// and `start_x` (screen points across), with a nib of `start_radius`
/// (mm). The edge of the circle follows the pointer across.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushResize {
    pub center: Point,
    pub start_x: f32,
    pub start_radius: f64,
}

/// Whether the wheel turned with `mods` sizes the nib: with Shift while
/// the wheel zooms (Shift+wheel does nothing else then), with Alt while
/// the wheel scrolls (Shift+wheel scrolls sideways then, Alt+wheel
/// nothing else). Ctrl keeps its zoom or scroll.
pub fn wheel_resizes(wheel_zooms: bool, mods: Modifiers) -> bool {
    let ctrl = mods.ctrl || mods.command;
    if wheel_zooms {
        mods.shift && !ctrl && !mods.alt
    } else {
        mods.alt && !ctrl && !mods.shift
    }
}

/// The circle of the nib being sized, at the press: drawn over the
/// drawing while a Shift+drag sizes it, black on a white halo.
pub fn draw_resize(app: &App, painter: &egui::Painter) {
    let Some(r) = app.brush_resize else {
        return;
    };
    let px = app.brush_radius as f32 * app.view.zoom;
    if !px.is_finite() || px <= 0.0 {
        return;
    }
    let c = app.view.to_screen(r.center);
    painter.circle_stroke(c, px, egui::Stroke::new(3.0, egui::Color32::WHITE));
    painter.circle_stroke(c, px, egui::Stroke::new(1.0, egui::Color32::BLACK));
}

impl App {
    /// The nib's radius set to `mm` (kept between [`MIN_RADIUS`] and
    /// [`MAX_RADIUS`]); the status bar says the new size.
    pub fn set_brush_radius(&mut self, mm: f64) {
        if !mm.is_finite() {
            return;
        }
        self.brush_radius = mm.clamp(MIN_RADIUS, MAX_RADIUS);
        let size = format!(
            "{:.1} {}",
            self.units.from_mm(self.brush_radius),
            self.units.short()
        );
        self.status = crate::i18n::trf("status.brush_size", &[("size", &size)]);
    }

    /// The wheel turned `notches` (up positive) over the drawing with the
    /// sizing modifier held: each notch up grows the nib by
    /// [`WHEEL_STEP`], each notch down shrinks it as much.
    pub fn brush_wheel(&mut self, notches: f32) {
        if !notches.is_finite() || notches == 0.0 {
            return;
        }
        let n = f64::from(notches).clamp(-20.0, 20.0);
        self.set_brush_radius(self.brush_radius * WHEEL_STEP.powf(n));
    }

    /// A Shift+drag begins sizing the nib at `center` (page), the pointer
    /// at `x` (screen points across). No stroke begins.
    pub fn begin_brush_resize(&mut self, center: Point, x: f32) {
        self.brush_resize = Some(BrushResize {
            center,
            start_x: x,
            start_radius: self.brush_radius,
        });
        self.drag = Drag::None;
    }

    /// The sizing drag moved to `x` (screen points across): the circle's
    /// edge follows the pointer, so the nib grows as far as the pointer
    /// went right and shrinks as far as it went left.
    pub fn brush_resize_to(&mut self, x: f32) {
        let Some(r) = self.brush_resize else {
            return;
        };
        let zoom = f64::from(self.view.zoom);
        if !(zoom.is_finite() && zoom > 0.0) || !x.is_finite() {
            return;
        }
        let px = r.start_radius * zoom + f64::from(x - r.start_x);
        self.set_brush_radius(px / zoom);
    }

    /// A stroke begins at `p`. Brushes change only what is selected:
    /// with nothing selected the stroke does nothing and the status bar
    /// says to select an object first (a click selects, see
    /// [`crate::tools::Tool::click_selects`]).
    pub fn brush_begin(&mut self, p: Point) {
        self.brush_stepped = false;
        self.brush_stroke = StrokeBase::default();
        if self.selection.is_empty() {
            self.status = crate::i18n::tr("status.brush_select_first");
            self.drag = Drag::None;
            return;
        }
        // Brushes work on curves. A group or clip frame is never brushed,
        // so it keeps its envelope and objects as they are.
        let shapes = self
            .selected_shapes()
            .into_iter()
            .filter(|s| {
                !matches!(
                    s.kind,
                    ShapeKind::Group { .. } | ShapeKind::ClipFrame { .. }
                )
            })
            .collect();
        self.convert_shapes_to_curves(shapes, true);
        self.drag = Drag::Node { last: p };
        self.brush_stepped = false;
    }

    /// The stroke moves on to `p`, `dt` seconds after the last step;
    /// `swap` (Alt or Shift held) swaps Attract and Repel and Twirl's
    /// turn. Twirl and Attract and Repel go on while the button is held
    /// still, as if the pointer moved [`HOLD_MM_PER_S`]. Held in one
    /// place, Attract and Repel work from the outline the hold found with
    /// the dose held so far, so they stop once the dose is full. Roughen
    /// works the whole stroke out from the outline it began with and the
    /// nib's way.
    pub fn brush_step(&mut self, p: Point, swap: bool, dt: f64) {
        let Drag::Node { last } = self.drag else {
            return;
        };
        let delta = p - last;
        let radius = self.brush_radius;
        let b = self.brush;
        let held = if dt.is_finite() {
            dt.clamp(0.0, 0.1)
        } else {
            0.0
        };
        let motion = delta.hypot() + HOLD_MM_PER_S * held;
        let turn = if swap { -1.0 } else { 1.0 };
        if self.tool == Tool::AttractRepel {
            // A move starts afresh from the outline as it is now.
            if delta.hypot() > 1e-9 || self.brush_stroke.base.is_empty() {
                self.brush_stroke.base = self.brush_page_paths();
                self.brush_stroke.dose = 0.0;
            }
            let before = self.brush_stroke.dose;
            let dose =
                (before + motion * b.attract_rate(swap) * ATTRACT_DOSE / radius).clamp(-1.0, 1.0);
            if !dose.is_finite() || dose == before {
                // Attract and Repel at their whole effect: at rest.
                self.drag = Drag::Node { last: p };
                return;
            }
            self.brush_stroke.dose = dose;
        }
        if self.tool == Tool::Twirl {
            // Held in one place, the turn so far is worked out afresh from
            // the outline the hold found, so a hold of several turns is
            // one exact twirl, not many small ones each fitted again
            // (#2640); a move starts afresh from the outline as it is.
            if delta.hypot() > 1e-9 || self.brush_stroke.base.is_empty() {
                self.brush_stroke.base = self.brush_page_paths();
                self.brush_stroke.angle = 0.0;
            }
            let step = b.twirl_rate() * turn * (delta.hypot() + TWIRL_MM_PER_S * held);
            if !step.is_finite() || step == 0.0 {
                self.drag = Drag::Node { last: p };
                return;
            }
            self.brush_stroke.angle += step;
        }
        if self.tool == Tool::Roughen {
            if self.brush_stroke.base.is_empty() {
                self.brush_stroke.base = self.brush_page_paths();
            }
            if self.brush_stroke.swath.last() != Some(&p) {
                self.brush_stroke.swath.push(p);
            }
        }
        let dose = self.brush_stroke.dose;
        let mut cmds = Vec::new();
        for s in self.selected_shapes() {
            let ShapeKind::Path { path, closed } = &s.kind else {
                continue;
            };
            // The nib, its reach and the settings' millimetres are page
            // measures, so the brush works on the outline as it lies on
            // the page and the result is taken back into the object's own
            // space; a scaled object (text converted to curves, a resized
            // drawing) is reshaped within the nib as drawn, not within a
            // nib scaled with it.
            let page_path = s.transform * path;
            let base = self
                .brush_stroke
                .base
                .iter()
                .find(|(id, _)| *id == s.id)
                .map(|(_, b)| b);
            let new_page = match (self.tool, base) {
                (Tool::Smear, _) => effects::smear(
                    &page_path,
                    p,
                    delta * BrushSettings::pressure(b.smear_pressure),
                    radius,
                ),
                (Tool::Twirl, Some(from)) => {
                    effects::twirl(from, p, self.brush_stroke.angle, radius)
                }
                (Tool::AttractRepel, Some(from)) => effects::attract(from, p, radius, dose),
                (Tool::Smudge, _) => effects::smudge(
                    &page_path,
                    last,
                    p,
                    radius,
                    BrushSettings::pressure(b.smudge_pressure),
                ),
                (Tool::Roughen, Some(from)) => effects::roughen_stroke(
                    from,
                    &self.brush_stroke.swath,
                    radius,
                    self.roughen_amount,
                    b.roughen_spacing(),
                ),
                (Tool::Smooth, _) => effects::smooth(&page_path, p, radius, b.smooth_tolerance()),
                _ => continue,
            };
            if new_page == page_path {
                continue;
            }
            let new_path = s.transform.inverse() * new_page;
            cmds.push(Command::SetShapeKind {
                shape: s.id,
                kind: ShapeKind::Path {
                    path: new_path,
                    closed: *closed,
                },
            });
        }
        self.drag = Drag::Node { last: p };
        if cmds.is_empty() {
            return;
        }
        let label = match self.tool {
            Tool::Smear => "Smear",
            Tool::Twirl => "Twirl",
            Tool::AttractRepel => "Attract",
            Tool::Smudge => "Smudge",
            Tool::Roughen => "Roughen",
            _ => "Smooth",
        };
        // One undo step for the whole stroke.
        if self.brush_stepped && self.engine.undo_label() == Some(label) {
            let _ = self.engine.undo();
        }
        if self.engine.run_batch(label, &cmds).is_ok() {
            self.brush_stepped = true;
        }
    }

    /// The selected curves as they lie on the page.
    fn brush_page_paths(&self) -> Vec<(ShapeId, BezPath)> {
        self.selected_shapes()
            .into_iter()
            .filter_map(|s| match &s.kind {
                ShapeKind::Path { path, .. } => Some((s.id, s.transform * path)),
                _ => None,
            })
            .collect()
    }

    /// The brushes' pointer input. A drag that starts with Shift held
    /// when the button went down sizes the nib (see
    /// [`App::begin_brush_resize`]); Shift pressed later in a stroke swaps
    /// Attract and Repel and Twirl's turn.
    pub fn brush_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        let held = matches!(self.tool, Tool::Twirl | Tool::AttractRepel);
        // Whether Shift was held when the button went down: the drag
        // starts a few points later.
        let shift_id = egui::Id::new("brush_press_shift");
        let press_shift = response.ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::PointerButton {
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(modifiers.shift),
                _ => None,
            })
        });
        if let Some(shift) = press_shift {
            response.ctx.data_mut(|d| d.insert_temp(shift_id, shift));
        }
        let shift_at_press = response
            .ctx
            .data(|d| d.get_temp::<bool>(shift_id))
            .unwrap_or(mods.shift);
        let x = response
            .interact_pointer_pos()
            .map_or_else(|| self.view.to_screen(p).x, |s| s.x);
        if response.drag_started_by(PointerButton::Primary)
            && shift_at_press
            && self.brush_resize.is_none()
        {
            // From where the button went down: the drag starts a few
            // points further on.
            self.drag = Drag::None;
            let origin = response.ctx.input(|i| i.pointer.press_origin());
            match origin {
                Some(o) => self.begin_brush_resize(self.view.to_page(o), o.x),
                None => self.begin_brush_resize(p, x),
            }
            self.brush_resize_to(x);
        }
        if self.brush_resize.is_some() {
            if response.dragged_by(PointerButton::Primary) {
                self.brush_resize_to(x);
            } else {
                self.brush_resize = None;
            }
            return;
        }
        // A press let go without a drag ends its stroke too.
        if held
            && matches!(self.drag, Drag::Node { .. })
            && response.ctx.input(|i| i.pointer.primary_released())
            && !response.drag_stopped()
        {
            self.drag = Drag::None;
            return;
        }
        // Twirl and Attract and Repel work from the press, the button held
        // still: they waited for the pointer to move far enough to count
        // as a drag (live test of 0.41).
        let pressed = held
            && !shift_at_press
            && response.is_pointer_button_down_on()
            && response.ctx.input(|i| i.pointer.primary_pressed());
        if response.drag_started_by(PointerButton::Primary) || pressed {
            if !matches!(self.drag, Drag::Node { .. }) {
                self.brush_begin(p);
            }
        }
        let down_here = held
            && response.is_pointer_button_down_on()
            && response.ctx.input(|i| i.pointer.primary_down())
            && matches!(self.drag, Drag::Node { .. });
        if response.dragged_by(PointerButton::Primary) || down_here {
            // Roughen adds its teeth once per spacing along the way, not
            // at every frame over the same stretch, which piled up nodes
            // in a tangle (live test of 0.41).
            if self.tool == Tool::Roughen {
                if let Drag::Node { last } = self.drag {
                    if (p - last).hypot() < self.brush.roughen_spacing() * 0.5 {
                        return;
                    }
                }
            }
            let dt = if held {
                // Keep going while the button is held still.
                response.ctx.request_repaint();
                response.ctx.input(|i| i.stable_dt) as f64
            } else {
                0.0
            };
            self.brush_step(p, mods.alt || mods.shift, dt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::{Rect, ShapeId};

    fn square(app: &mut App) -> ShapeId {
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("shape");
        app.select(vec![id]);
        id
    }

    fn bounds(app: &App, id: ShapeId) -> Rect {
        app.doc().shape_on_page(id).expect("shape").bounds()
    }

    /// A stroke is one undo step whatever its length; the next stroke is
    /// another. Smear's pressure scales how far the curve follows.
    #[test]
    fn a_stroke_is_one_step_and_follows_the_settings() {
        let mut app = App::headless();
        let id = square(&mut app);
        app.tool = Tool::Smear;
        app.brush_radius = 10.0;
        app.brush_begin(Point::new(40.0, 20.0));
        let depth = app.engine.history_labels().0.len();
        for x in [42.0, 44.0, 46.0] {
            app.brush_step(Point::new(x, 20.0), false, 0.0);
        }
        assert_eq!(app.engine.history_labels().0.len(), depth + 1);
        let full = bounds(&app, id).x1;
        assert!(full > 42.5, "{full}");
        app.undo();
        // Half the pressure: half as far.
        app.brush.smear_pressure = 50;
        app.brush_begin(Point::new(40.0, 20.0));
        for x in [42.0, 44.0, 46.0] {
            app.brush_step(Point::new(x, 20.0), false, 0.0);
        }
        let half = bounds(&app, id).x1;
        assert!(half > 40.5 && half < full - 0.5, "{half} vs {full}");
    }

    /// Brushes change only the selected objects: a stroke over a
    /// selected square and an unselected one beside it reshapes the
    /// selected one alone, and with nothing selected a stroke changes
    /// nothing and says to select an object first (#271).
    #[test]
    fn brushes_change_only_the_selected_objects() {
        for tool in [Tool::Smear, Tool::Roughen, Tool::Smudge, Tool::Twirl] {
            let mut app = App::headless();
            let a = square(&mut app);
            let b = app
                .new_shape(ShapeKind::Rect {
                    rect: Rect::new(40.0, 0.0, 80.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("shape");
            app.select(vec![a]);
            app.tool = tool;
            app.brush_radius = 10.0;
            let kind_b = app.doc().find_shape(b).expect("b").kind.clone();
            // Along the edge both squares share.
            app.brush_begin(Point::new(40.0, 5.0));
            for y in [10.0, 15.0, 20.0, 25.0, 30.0] {
                app.brush_step(Point::new(40.0 + y / 10.0, y), false, 0.05);
            }
            assert!(
                matches!(
                    app.doc().find_shape(a).expect("a").kind,
                    ShapeKind::Path { .. }
                ),
                "{tool:?}: the selected square is brushed"
            );
            assert_eq!(
                app.doc().find_shape(b).expect("b").kind,
                kind_b,
                "{tool:?}: the other square is left alone"
            );
            // Nothing selected: nothing changes.
            app.select(Vec::new());
            let doc = app.doc().clone();
            app.brush_begin(Point::new(60.0, 5.0));
            for y in [10.0, 20.0, 30.0] {
                app.brush_step(Point::new(62.0, y), false, 0.05);
            }
            assert_eq!(app.doc().pages, doc.pages, "{tool:?}");
            assert!(app.selection.is_empty());
            assert_eq!(app.status, crate::i18n::tr("status.brush_select_first"));
        }
    }

    /// Attract pulls and Repel pushes; Shift swaps them. Twirl turns the
    /// other way when set clockwise.
    #[test]
    fn attract_repel_and_twirl_follow_their_modes() {
        let mut app = App::headless();
        let id = square(&mut app);
        app.tool = Tool::AttractRepel;
        app.brush_radius = 8.0;
        // How far out the corner by the nib, (40, 40), now is (x + y).
        let stroke = |app: &mut App, shift: bool| {
            app.brush_begin(Point::new(36.0, 36.0));
            app.brush_step(Point::new(37.0, 36.0), shift, 0.0);
            app.brush_step(Point::new(36.0, 36.0), shift, 0.0);
            // Along the outline, curves flattened: the brushes fit the
            // stretch in reach with curves.
            let mut reach = f64::MIN;
            if let ShapeKind::Path { path, .. } = &app.doc().find_shape(id).expect("shape").kind {
                tracedraw_core::geometry::flatten(path, 0.01, &mut |e| match e {
                    tracedraw_core::geometry::PathEl::MoveTo(q)
                    | tracedraw_core::geometry::PathEl::LineTo(q) => reach = reach.max(q.x + q.y),
                    _ => {}
                });
            }
            app.undo();
            reach
        };
        assert!(stroke(&mut app, false) < 80.0);
        assert!(stroke(&mut app, true) > 80.0);
        app.brush.repel = true;
        assert!(stroke(&mut app, false) > 80.0);
        assert!(stroke(&mut app, true) < 80.0);
        // Twirl near the top right corner: the corner goes up one way and
        // across the other.
        app.tool = Tool::Twirl;
        app.brush_radius = 15.0;
        let twirl = |app: &mut App| {
            app.brush_begin(Point::new(35.0, 35.0));
            for x in [36.0, 38.0, 40.0, 42.0] {
                app.brush_step(Point::new(x, 35.0), false, 0.0);
            }
            let b = bounds(app, id);
            app.undo();
            b
        };
        let ccw = twirl(&mut app);
        app.brush.twirl_clockwise = true;
        let cw = twirl(&mut app);
        assert_ne!(ccw, cw);
        // Held still, Twirl still turns.
        let before = bounds(&app, id);
        app.brush_begin(Point::new(35.0, 35.0));
        for _ in 0..5 {
            app.brush_step(Point::new(35.0, 35.0), false, 0.1);
        }
        assert_ne!(bounds(&app, id), before);
    }
}

#[cfg(test)]
mod hold_tests {
    use super::*;
    use tracedraw_core::Rect;

    /// Held still on the right side of a square for four seconds at the
    /// bar's 50 %, Twirl winds the side round the nib's centre about four
    /// times, as CorelDRAW's does (#2640: it turned a little over half a
    /// turn in that time and left a single hook). The outline comes back
    /// a smooth spiral of a bounded number of curves, and each held step
    /// is quick, since it is one twirl of the outline the hold found.
    #[test]
    fn a_held_twirl_winds_the_outline_round_several_times() {
        let mut app = App::headless();
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("shape");
        app.select(vec![id]);
        app.tool = Tool::Twirl;
        app.brush_radius = 15.0;
        let at = Point::new(40.0, 20.0);
        app.brush_begin(at);
        let t = std::time::Instant::now();
        for _ in 0..240 {
            app.brush_step(at, false, 1.0 / 60.0);
        }
        let per_step = t.elapsed().as_secs_f64() / 240.0;
        let page = app.doc().shape_on_page(id).expect("shape").page_path();
        // How often the outline crosses the ray from the nib's centre
        // going up: a spiral of n turns on each side of the centre
        // crosses it about 2 n times.
        let mut crossings = 0;
        let mut prev: Option<Point> = None;
        tracedraw_core::geometry::flatten(&page, 0.01, &mut |e| match e {
            tracedraw_core::geometry::PathEl::MoveTo(q) => prev = Some(q),
            tracedraw_core::geometry::PathEl::LineTo(q) => {
                if let Some(o) = prev {
                    if (o.x < at.x) != (q.x < at.x) {
                        let y = o.y + (q.y - o.y) * (at.x - o.x) / (q.x - o.x);
                        if y > at.y && y < at.y + 15.0 {
                            crossings += 1;
                        }
                    }
                }
                prev = Some(q);
            }
            _ => {}
        });
        assert!(
            crossings >= 6,
            "{crossings} crossings: wound round too little"
        );
        assert!(
            page.elements().len() < 400,
            "{} elements",
            page.elements().len()
        );
        assert!(per_step < 0.25, "{per_step} s a step");
    }

    /// Held still over the top side of a square, Attract and Repel move
    /// the side from the first step on, at a steady rate, and come to
    /// rest once the dose is full (the live test of 0.41: the objects
    /// kept moving and never settled); Repel pushes the side out to at
    /// most the nib's edge, Attract pulls it in.
    #[test]
    fn attract_and_repel_come_to_rest_while_held() {
        for repel in [false, true] {
            let mut app = App::headless();
            let id = app
                .new_shape(ShapeKind::Rect {
                    rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("shape");
            app.select(vec![id]);
            app.tool = Tool::AttractRepel;
            app.brush.repel = repel;
            app.brush_radius = 8.0;
            let at = Point::new(20.0, 36.0);
            app.brush_begin(at);
            let mut revs = Vec::new();
            let mut tops = Vec::new();
            for _ in 0..200 {
                app.brush_step(at, false, 1.0 / 60.0);
                revs.push(app.doc().shape_on_page(id).expect("shape").page_path());
                // Where the side crosses x = 20.
                let page = app.doc().shape_on_page(id).expect("shape").page_path();
                let mut side = f64::NAN;
                let mut prev: Option<Point> = None;
                tracedraw_core::geometry::flatten(&page, 0.01, &mut |e| match e {
                    tracedraw_core::geometry::PathEl::MoveTo(q) => prev = Some(q),
                    tracedraw_core::geometry::PathEl::LineTo(q) => {
                        if let Some(a) = prev {
                            let crosses = (a.x - 20.0) * (q.x - 20.0) <= 0.0 && a.x != q.x;
                            if crosses && a.y.max(q.y) > 20.0 {
                                side = a.y + (q.y - a.y) * (20.0 - a.x) / (q.x - a.x);
                            }
                        }
                        prev = Some(q);
                    }
                    _ => {}
                });
                tops.push(side);
            }
            assert!(
                (tops[2] - tops[0]).abs() > 0.02,
                "repel {repel}: moving from the start: {:?}",
                &tops[..3]
            );
            // A steady rate: about as far in the second tenth of a
            // second as in the first.
            let (a, b) = ((tops[6] - tops[0]).abs(), (tops[12] - tops[6]).abs());
            assert!(
                a > 0.0 && (a - b).abs() < 0.5 * a,
                "repel {repel}: {a} then {b}"
            );
            assert_eq!(revs[150], revs[199], "repel {repel}: at rest");
            let end = tops[199];
            if repel {
                // 4 mm from the centre: out to 5 mm, within the nib.
                assert!((end - 41.0).abs() < 0.15, "pushed to {end}");
            } else {
                // In to 2 mm from the centre.
                assert!((end - 38.0).abs() < 0.15, "pulled to {end}");
            }
        }
    }
}

#[cfg(test)]
mod page_space_tests {
    use super::*;
    use tracedraw_core::geometry::Affine;
    use tracedraw_core::{Rect, ShapeId};

    fn square(app: &mut App) -> ShapeId {
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("shape");
        app.select(vec![id]);
        id
    }

    /// Points along the object's outline on the page, half a millimetre
    /// apart at most (a straight side gives points all along it).
    fn page_points(app: &App, id: ShapeId) -> Vec<Point> {
        let s = app.doc().shape_on_page(id).expect("shape");
        let ShapeKind::Path { path, .. } = &s.kind else {
            panic!("not a curve");
        };
        let mut verts = Vec::new();
        tracedraw_core::geometry::flatten(&(s.transform * path), 0.01, &mut |el| match el {
            tracedraw_core::geometry::PathEl::MoveTo(q)
            | tracedraw_core::geometry::PathEl::LineTo(q) => verts.push(q),
            _ => {}
        });
        let mut pts = Vec::new();
        for w in verts.windows(2) {
            let n = ((w[1] - w[0]).hypot() / 0.5).ceil().max(1.0) as usize;
            for k in 0..n {
                pts.push(w[0].lerp(w[1], k as f64 / n as f64));
            }
        }
        pts.extend(verts.last().copied());
        pts
    }

    /// The nib is measured on the page: on a square scaled five times, a
    /// smear with a 10 mm nib at the middle of the right side moves the
    /// side within 10 mm of the pointer, not within 50 mm, and by the
    /// pointer's move, not by five times it.
    #[test]
    fn the_nib_is_measured_on_the_page_whatever_the_object_scale() {
        let mut app = App::headless();
        let id = square(&mut app);
        app.transform_selection(Affine::scale(5.0));
        app.tool = Tool::Smear;
        app.brush_radius = 10.0;
        app.brush_begin(Point::new(200.0, 100.0));
        app.brush_step(Point::new(202.0, 100.0), false, 0.0);
        let pts = page_points(&app, id);
        // The side well away from the nib (and from the corners, which
        // the refit rounds a little) is where it was.
        let far = pts
            .iter()
            .filter(|q| (5.0..=80.0).contains(&q.y) || (120.0..=195.0).contains(&q.y))
            .map(|q| q.x)
            .fold(0.0, f64::max);
        assert!((far - 200.0).abs() < 1e-6, "far side moved to {far}");
        // What the new curve was fitted through stays on the old side
        // just past the nib.
        let edge = pts
            .iter()
            .filter(|q| !(85.0..=115.0).contains(&q.y))
            .map(|q| q.x)
            .fold(0.0, f64::max);
        assert!(edge < 200.5, "edge of the nib at {edge}");
        let near = pts.iter().map(|q| q.x).fold(0.0, f64::max);
        assert!(near > 201.0 && near < 202.5, "near side at {near}");
    }

    /// Smooth's tolerance is in page millimetres too: a stroke over a
    /// scaled, jagged outline leaves the part outside the nib exactly as
    /// it was.
    #[test]
    fn smooth_leaves_the_outline_outside_the_nib_alone() {
        let mut app = App::headless();
        let id = square(&mut app);
        app.transform_selection(Affine::scale(5.0));
        app.tool = Tool::Smooth;
        app.brush_radius = 10.0;
        app.brush_begin(Point::new(200.0, 100.0));
        app.brush_step(Point::new(201.0, 100.0), false, 0.0);
        let after = page_points(&app, id);
        // The corners and the far sides are still where they were.
        for q in [
            Point::new(0.0, 0.0),
            Point::new(200.0, 0.0),
            Point::new(200.0, 200.0),
            Point::new(0.0, 200.0),
            Point::new(200.0, 60.0),
            Point::new(200.0, 140.0),
        ] {
            let near = |pts: &[Point]| {
                pts.iter()
                    .map(|r| (*r - q).hypot())
                    .fold(f64::INFINITY, f64::min)
            };
            assert!(near(&after) < 1e-3, "{q:?} left by {}", near(&after));
        }
    }
}
