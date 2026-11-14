//! Pointer and keyboard handling on the canvas, per tool.

use crate::app::{App, CurveInProgress, Drag, Handle};

/// How near the drawing window's edge (interface points) a drag starts
/// scrolling the view (#2051).
pub const EDGE_SCROLL_BAND: f32 = 24.0;
/// The fastest the view scrolls during a drag, interface points a
/// second, with the pointer at the edge or past it.
pub const EDGE_SCROLL_SPEED: f32 = 900.0;

/// How far the view scrolls this frame (`dt` seconds) for a drag with the
/// pointer at `p` in the drawing window `rect`: toward each edge the
/// pointer is within [`EDGE_SCROLL_BAND`] of, faster the nearer, at most
/// [`EDGE_SCROLL_SPEED`]. Nothing in a window too small to have room for
/// the bands.
pub fn edge_scroll_step(rect: egui::Rect, p: egui::Pos2, dt: f32) -> egui::Vec2 {
    if !rect.is_positive()
        || rect.width() < 4.0 * EDGE_SCROLL_BAND
        || rect.height() < 4.0 * EDGE_SCROLL_BAND
    {
        return egui::Vec2::ZERO;
    }
    let axis = |lo: f32, hi: f32, v: f32| -> f32 {
        let toward_lo = ((lo + EDGE_SCROLL_BAND - v) / EDGE_SCROLL_BAND).clamp(0.0, 1.0);
        let toward_hi = ((v - (hi - EDGE_SCROLL_BAND)) / EDGE_SCROLL_BAND).clamp(0.0, 1.0);
        // Near the left (top) edge the drawing moves right (down) to show
        // what lies beyond it.
        (toward_lo - toward_hi) * EDGE_SCROLL_SPEED * dt
    };
    egui::Vec2::new(
        axis(rect.left(), rect.right(), p.x),
        axis(rect.top(), rect.bottom(), p.y),
    )
}

/// What a Pen or Bezier press would edit on the selected curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PenEdit {
    /// One of its nodes (curve, element index): dragged like with the
    /// Shape tool; with the Pen's automatic adding and deleting, a click
    /// deletes it.
    Node(tracedraw_core::ShapeId, usize),
    /// A segment (curve, segment index, place along it): a click adds a
    /// node there.
    Segment(tracedraw_core::ShapeId, usize, f64),
}
use crate::tools::Tool;
use egui::{Key, Modifiers, PointerButton, Response};
use tracedraw_core::{
    document::ShapeKind,
    geometry::{Affine, Point, Rect, Shape as _, Vec2},
    Command, Fill, ShapeId,
};

/// How far out of the selection box the rotate and skew arrows' centres
/// stand (rotate mode), interface points.
pub(crate) const ROTATE_HANDLE_OFFSET: f32 = 9.0;
/// How many groups, clip frames and symbols deep a click looks for the
/// object under it; deeper containers count anywhere in their box.
const MAX_HIT_DEPTH: usize = 64;
/// The rotate and skew arrows take a press this close to their centre.
const ROTATE_HANDLE_PX: f32 = 9.0;

/// The point of the selection box `b` a handle stands for (page space).
pub(crate) fn handle_point(b: Rect, h: Handle) -> Point {
    let c = b.center();
    match h {
        Handle::N => Point::new(c.x, b.y1),
        Handle::S => Point::new(c.x, b.y0),
        Handle::E => Point::new(b.x1, c.y),
        Handle::W => Point::new(b.x0, c.y),
        Handle::NE => Point::new(b.x1, b.y1),
        Handle::NW => Point::new(b.x0, b.y1),
        Handle::SE => Point::new(b.x1, b.y0),
        Handle::SW => Point::new(b.x0, b.y0),
    }
}
/// How near the centre of rotation the pointer grabs it, screen pixels.
const PIVOT_PX: f32 = 8.0;

/// `p` turned about `from` onto the nearest multiple of `step_deg`,
/// keeping how far it reaches along that direction (Ctrl while drawing
/// lines and curves).
pub fn constrain_from(from: Point, p: Point, step_deg: f64) -> Point {
    let d = p - from;
    if d.hypot() < 1e-9 || !(d.x.is_finite() && d.y.is_finite()) {
        return p;
    }
    let step = step_deg.max(0.1).to_radians();
    let a = (d.y.atan2(d.x) / step).round() * step;
    let dir = Vec2::new(a.cos(), a.sin());
    from + dir * d.dot(dir).max(0.0)
}

/// The skew a side handle in rotate mode makes when dragged by `d` (page
/// millimetres): the top and bottom handles slant the selection sideways,
/// the left and right ones up or down. The opposite side stays put (with
/// Shift the centre line); with Ctrl the slant goes in steps of
/// `constrain_deg`.
pub fn skew_transform(
    handle: Handle,
    b: Rect,
    d: Vec2,
    mods: crate::app::HandleMods,
    constrain_deg: f64,
) -> Option<Affine> {
    let horizontal = matches!(handle, Handle::N | Handle::S);
    if handle.is_corner() || b.width() <= 1e-9 || b.height() <= 1e-9 {
        return None;
    }
    // The fixed line and how far the dragged side is from it, signed.
    let (fixed, span, moved) = match handle {
        Handle::N => (
            if mods.from_center { b.center().y } else { b.y0 },
            b.y1,
            d.x,
        ),
        Handle::S => (
            if mods.from_center { b.center().y } else { b.y1 },
            b.y0,
            d.x,
        ),
        Handle::E => (
            if mods.from_center { b.center().x } else { b.x0 },
            b.x1,
            d.y,
        ),
        _ => (
            if mods.from_center { b.center().x } else { b.x1 },
            b.x0,
            d.y,
        ),
    };
    let span = span - fixed;
    if span.abs() < 1e-9 {
        return None;
    }
    let mut k = moved / span;
    if mods.steps {
        let step = constrain_deg.max(0.1).to_radians();
        let a = (k.atan() / step).round() * step;
        // Never a right angle: that slant has no inverse.
        k = a.clamp(-1.5, 1.5).tan();
    }
    if !k.is_finite() {
        return None;
    }
    Some(if horizontal {
        // x' = x + k (y - fixed)
        Affine::new([1.0, 0.0, k, 1.0, -k * fixed, 0.0])
    } else {
        // y' = y + k (x - fixed)
        Affine::new([1.0, k, 0.0, 1.0, 0.0, -k * fixed])
    })
}

/// Whether `events` hold a copy, a cut and a paste request (the forms the
/// windowing layer gives Ctrl+C, Ctrl+X and Ctrl+V), with the pasted text.
fn clipboard_events(events: &[egui::Event]) -> (bool, bool, Option<String>) {
    let mut out = (false, false, None);
    for e in events {
        match e {
            egui::Event::Copy => out.0 = true,
            egui::Event::Cut => out.1 = true,
            egui::Event::Paste(t) => out.2 = Some(t.clone()),
            _ => {}
        }
    }
    out
}

/// The paste shortcuts: Ctrl+V (Cmd+V), Shift+Insert and the Paste key.
fn is_paste_key(key: egui::Key, mods: Modifiers) -> bool {
    key == Key::Paste || (mods.command && key == Key::V) || (mods.shift && key == Key::Insert)
}

/// Whether a paste shortcut went up without a paste event or a press
/// before it. The windowing layer turns the shortcut into a paste event
/// only while the system clipboard holds text and drops the key press
/// otherwise (a picture or files copied in another application), so its
/// release is all that is left of it. `seen` keeps a paste event or press
/// across frames until the key goes up.
fn paste_without_text(events: &[egui::Event], seen: &mut bool) -> bool {
    let mut out = false;
    for e in events {
        match e {
            egui::Event::Paste(_) => *seen = true,
            egui::Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } if is_paste_key(*key, *modifiers) => {
                if *pressed {
                    *seen = true;
                } else {
                    out |= !*seen;
                    *seen = false;
                }
            }
            // V let go after Ctrl: not a paste, and the next one starts
            // afresh.
            egui::Event::Key {
                key: Key::V,
                pressed: false,
                ..
            } => *seen = false,
            _ => {}
        }
    }
    out
}

thread_local! {
    /// A right button pressed while the left one is held, reported by the
    /// windowing layer outside egui's events (see [`note_chorded_right_press`]).
    static CHORDED_RIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a mouse button event is the right button going down while the
/// left one is held: `button` and `buttons` as the browser's mouse events
/// give them (button 2 is the right one, bit 1 of `buttons` the left one
/// held).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn is_chorded_right_press(button: i16, buttons: u16) -> bool {
    button == 2 && buttons & 1 != 0
}

/// Note a right button pressed during a left drag. A browser reports a
/// button pressed while another is held as a pointer move, so egui never
/// sees it; the browser build listens for the mouse event itself
/// (`web.rs`) and the next frame takes the press as the right click that
/// makes a move, scale, rotation or skew leave the original.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn note_chorded_right_press() {
    CHORDED_RIGHT.with(|c| c.set(true));
}

/// The noted press, once.
fn take_chorded_right_press() -> bool {
    CHORDED_RIGHT.with(|c| c.replace(false))
}

thread_local! {
    /// The left button let go while the right one is still held, reported
    /// by the windowing layer outside egui's events (see
    /// [`note_chorded_left_release`]).
    static CHORDED_LEFT_UP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a mouse button event is the left button going up while the
/// right one is still held: `button` and `buttons` as the browser's mouse
/// events give them (button 0 is the left one, bit 2 of `buttons` the
/// right one held, bit 1 the left one).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn is_chorded_left_release(button: i16, buttons: u16) -> bool {
    button == 0 && buttons & 1 == 0 && buttons & 2 != 0
}

/// Note the left button let go while the right one is held. A browser
/// sends the pointer's release only when its last button goes up, so the
/// left release of a right click made during a drag never reached egui:
/// it took the left button for held, and the moved object stayed on the
/// pointer until Esc (#1334). The browser build listens for the mouse
/// event itself (`web.rs`); [`release_chorded_left`] gives egui the
/// release.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn note_chorded_left_release() {
    CHORDED_LEFT_UP.with(|c| c.set(true));
}

/// The raw input hook's part: a noted left release (see
/// [`note_chorded_left_release`]) becomes egui's release of the primary
/// button, at the latest pointer position, while egui still has it held.
pub fn release_chorded_left(ctx: &egui::Context, raw: &mut egui::RawInput) {
    if !CHORDED_LEFT_UP.with(|c| c.replace(false)) {
        return;
    }
    if !ctx.input(|i| i.pointer.primary_down()) {
        return;
    }
    let pos = raw
        .events
        .iter()
        .rev()
        .find_map(|e| match e {
            egui::Event::PointerMoved(p) => Some(*p),
            _ => None,
        })
        .or_else(|| ctx.pointer_latest_pos());
    let Some(pos) = pos else {
        return;
    };
    raw.events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: ctx.input(|i| i.modifiers),
    });
}

/// How far the outline of `s` reaches from its path on the page, as it
/// is drawn: half its width, all of it for a closed object whose outline
/// lies inside or outside the path, the widest side of a variable-width
/// outline's nodes; scaled by `to_page` (the object's transform to the
/// page) when the outline scales with the object, as the renderer draws
/// it; 0 without an outline. A click within this, plus the pick
/// distance, picks the object (#1678, #1935).
pub(crate) fn outline_reach(s: &tracedraw_core::Shape, to_page: Affine) -> f64 {
    let Some(stroke) = &s.stroke else {
        return 0.0;
    };
    let scale = if stroke.scale_with_object {
        let c = to_page.as_coeffs();
        (c[0] * c[3] - c[1] * c[2]).abs().sqrt()
    } else {
        1.0
    };
    let open = matches!(s.kind, ShapeKind::Path { closed: false, .. });
    let mut reach = match stroke.position {
        tracedraw_core::OutlinePosition::Center => stroke.width / 2.0,
        _ if open => stroke.width / 2.0,
        _ => stroke.width,
    };
    for n in &stroke.widths {
        reach = reach.max(n.left).max(n.right);
    }
    let r = reach * scale;
    if r.is_finite() {
        r.max(0.0)
    } else {
        0.0
    }
}

impl App {
    /// The Distribute keys, each pressed with Shift: the letters of the
    /// Align keys for the same edge or centre, P and A for the gaps.
    pub const DISTRIBUTE_KEYS: [(Key, crate::ops::Distribute); 8] = {
        use crate::ops::Distribute;
        [
            (Key::L, Distribute::Left),
            (Key::E, Distribute::CentersH),
            (Key::P, Distribute::SpacingH),
            (Key::R, Distribute::Right),
            (Key::T, Distribute::Top),
            (Key::C, Distribute::CentersV),
            (Key::A, Distribute::SpacingV),
            (Key::B, Distribute::Bottom),
        ]
    };

    pub fn hit_test(&self, p: Point) -> Option<ShapeId> {
        self.hit_test_where(p, |_| false)
    }

    /// The topmost visible object at `p` (on a visible, unlocked layer)
    /// when that object is locked: what a right click there unlocks.
    pub fn locked_at(&self, p: Point) -> Option<ShapeId> {
        if self.focus_shape_on_page().is_some() {
            return None;
        }
        let tol = self.object_pick_tol();
        self.page_layers()
            .into_iter()
            .rev()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.shapes.iter().rev())
            .find(|s| s.visible && !self.outside_clip_edit(s.id) && self.shape_hit(s, p, tol))
            .filter(|s| s.locked)
            .map(|s| s.id)
    }

    /// The topmost visible object at `p` to read from, locked or not and
    /// on a locked layer too (the eyedroppers: reading a colour or the
    /// attributes leaves the object as it is, #365). In focus mode, as
    /// [`Self::hit_test`].
    pub fn hit_test_to_read(&self, p: Point) -> Option<ShapeId> {
        if self.focus_shape_on_page().is_some() {
            return self.hit_test(p);
        }
        let tol = self.object_pick_tol();
        self.page_layers()
            .into_iter()
            .rev()
            .filter(|l| l.visible)
            .flat_map(|l| l.shapes.iter().rev())
            .find(|s| s.visible && self.shape_hit(s, p, tol))
            .map(|s| s.id)
    }

    /// The object [`Self::hit_test_to_read`] finds, opened down to the
    /// innermost part under `p`: a group's child, a clip frame's content
    /// (inside the frame) or the frame, their transforms composed. What a
    /// colour is read from.
    pub fn leaf_to_read(&self, p: Point) -> Option<tracedraw_core::Shape> {
        let id = self.hit_test_to_read(p)?;
        let (_, s) = self.doc().shape(id).ok()?;
        let tol = self.object_pick_tol();
        Some(self.leaf_at(s, p, tol).unwrap_or_else(|| s.clone()))
    }

    /// The innermost part of the object `id` under `p` (see
    /// [`Self::leaf_to_read`]), its transforms composed; `None` when
    /// `p` is on no part of it.
    pub fn leaf_of(&self, id: tracedraw_core::ShapeId, p: Point) -> Option<tracedraw_core::Shape> {
        let (_, s) = self.doc().shape(id).ok()?;
        self.leaf_at(s, p, self.object_pick_tol())
    }

    fn leaf_at(
        &self,
        s: &tracedraw_core::Shape,
        p: Point,
        tol: f64,
    ) -> Option<tracedraw_core::Shape> {
        let open = |children: &[tracedraw_core::Shape]| {
            children.iter().rev().filter(|c| c.visible).find_map(|c| {
                let mut c = c.clone();
                c.transform_by(s.transform);
                self.leaf_at(&c, p, tol)
            })
        };
        match &s.kind {
            ShapeKind::Group { children } => open(children),
            ShapeKind::ClipFrame { frame, contents } => {
                let mut f = (**frame).clone();
                f.transform_by(s.transform);
                if !f.page_path().contains(p) && !self.shape_hit(&f, p, tol) {
                    return None;
                }
                open(contents).or_else(|| self.shape_hit(&f, p, tol).then_some(f))
            }
            _ => self.shape_hit(s, p, tol).then(|| s.clone()),
        }
    }

    /// How far from an object's outline a click still picks it, page
    /// units: Options > Nodes and Handles, Object click distance (#1679).
    pub fn object_pick_tol(&self) -> f64 {
        self.settings.nodes.object_pick_px() / self.view.zoom.max(1e-6) as f64
    }

    /// The topmost object at `p` that `skip` does not rule out.
    pub fn hit_test_where(&self, p: Point, skip: impl Fn(ShapeId) -> bool) -> Option<ShapeId> {
        let tol = self.object_pick_tol();
        // Focus mode: only the parts of the object in focus (or itself).
        // A clip frame's parts are its contents, where the frame shows
        // them (#2157): a click in a focused clip frame took the clip
        // frame itself, so nothing inside it could be picked or edited.
        if let Some(f) = self.focus_shape_on_page() {
            return match &f.kind {
                ShapeKind::Group { .. } | ShapeKind::ClipFrame { .. } => self
                    .children_hit(&f, p, tol)
                    .into_iter()
                    .find(|c| !skip(c.id))
                    .map(|c| c.id),
                _ => (!skip(f.id) && self.shape_hit(&f, p, tol)).then_some(f.id),
            };
        }
        // Editing a symmetry group: its members are picked, not the
        // group, and never through a copy.
        let members = self.symmetry_members_on_page();
        for s in self
            .page_layers()
            .into_iter()
            .rev()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.shapes.iter().rev())
            .filter(|s| !s.locked && s.visible && !self.outside_clip_edit(s.id))
        {
            if let Some((g, inside)) = &members {
                if s.id == *g {
                    if let Some(c) = inside
                        .iter()
                        .rev()
                        .find(|c| !skip(c.id) && self.shape_hit(c, p, tol))
                    {
                        return Some(c.id);
                    }
                    continue;
                }
            }
            if !skip(s.id) && self.shape_hit(s, p, tol) {
                return Some(self.blend_end_under_steps(s, p, tol, &skip).unwrap_or(s.id));
            }
        }
        None
    }

    /// A press on the steps of the blend that `s` starts, away from `s`
    /// itself, over the blend's end object: that end object, as it is
    /// what shows there. The steps are drawn by the start object above
    /// the end and a text step is picked anywhere in its box, so the
    /// steps of a text blend covered nearly all of the end text, and a
    /// click on it picked the start text (the colour then went to the
    /// other end, #1739). Elsewhere on the steps the start is picked.
    /// The end object counts when it is a visible, unlocked object of
    /// the page, as the objects a click picks.
    fn blend_end_under_steps(
        &self,
        s: &tracedraw_core::Shape,
        p: Point,
        tol: f64,
        skip: &impl Fn(ShapeId) -> bool,
    ) -> Option<ShapeId> {
        let end = s.effects.iter().find_map(|e| match e {
            tracedraw_core::live::Effect::Blend {
                end_id: Some(end), ..
            } => Some(*end),
            _ => None,
        })?;
        if skip(end) {
            return None;
        }
        // The start object itself, without its steps, under the press:
        // it stays the pick.
        let mut own = s.clone();
        own.effects
            .retain(|e| !matches!(e, tracedraw_core::live::Effect::Blend { .. }));
        if self.shape_hit(&own, p, tol) {
            return None;
        }
        self.page_layers()
            .into_iter()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.shapes.iter())
            .find(|e| e.id == end)
            .filter(|e| {
                !e.locked && e.visible && !self.outside_clip_edit(e.id) && self.shape_hit(e, p, tol)
            })
            .map(|e| e.id)
    }

    /// What a click at `p` picks: the topmost object there, or an object
    /// whose drop shadow lies at `p` over whatever is beneath it (a
    /// shadow draws above the objects under its own), so a shadow is
    /// picked for editing with its object (#935). An object that casts a
    /// drop shadow of its own is never lost to another one's shadow over
    /// it: a click on it picks it, so picking one shadowed object after
    /// another never jumps to a neighbour. In focus mode and while a
    /// symmetry is edited, as [`Self::hit_test`].
    pub fn pick_at(&self, p: Point) -> Option<ShapeId> {
        let hit = self.hit_test(p);
        if self.focus_shape_on_page().is_some() || self.symmetry_edit.is_some() {
            return hit;
        }
        let shadowed = |id: ShapeId| {
            let top = self.doc().top_level_of(id).unwrap_or(id);
            self.doc()
                .find_shape(top)
                .is_some_and(|s| s.shadow.is_some_and(|sh| sh.opacity > 0.0))
        };
        if hit.is_some_and(shadowed) {
            return hit;
        }
        self.page_layers()
            .into_iter()
            .rev()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.shapes.iter().rev())
            .filter(|s| !s.locked && s.visible && !self.outside_clip_edit(s.id))
            .find_map(|s| {
                if Some(s.id) == hit {
                    hit
                } else {
                    crate::shadow_tool::shadow_covers(s, p).then_some(s.id)
                }
            })
            .or(hit)
    }

    /// `s` (in page space) is under `p` within `tol` page units: text,
    /// tables and bitmaps anywhere in their box; groups and symbols only
    /// where one of their objects is (#885), clip frames on their frame
    /// and where their contents show inside it; filled objects inside
    /// their shape (an open curve only when the document fills open
    /// curves, unfilled ones too with "Treat all objects as filled");
    /// otherwise near the outline.
    pub(crate) fn shape_hit(&self, s: &tracedraw_core::Shape, p: Point, tol: f64) -> bool {
        self.shape_hit_in(s, p, tol, 0)
    }

    /// [`Self::shape_hit`] for an object `depth` groups, clip frames or
    /// symbols deep. Past [`MAX_HIT_DEPTH`] a container counts anywhere in
    /// its box, so a file nesting symbols in themselves cannot recurse
    /// for ever.
    fn shape_hit_in(&self, s: &tracedraw_core::Shape, p: Point, tol: f64, depth: usize) -> bool {
        if let Some(own) = self.without_edited_symmetry(s) {
            return self.shape_hit_in(&own, p, tol, depth);
        }
        // A thick outline is picked anywhere it shows, out to the
        // tolerance beyond its visible edge, not only near its centre
        // line (#1678); a thin one within the tolerance as before.
        let half = outline_reach(s, s.transform);
        if !self
            .visual_bounds(s)
            .inflate(tol + half, tol + half)
            .contains(p)
        {
            return false;
        }
        if !s.effects.is_empty() {
            // What the object draws: its warped outline and the objects
            // its effects add.
            let ev = tracedraw_core::live::evaluate(s);
            // A lens shows over its whole inside, filled or not: a closed
            // shape with a Transparency lens lying over other objects is
            // picked (and its colour read) there, not the objects beneath
            // (#2549).
            if s.effects
                .iter()
                .any(|e| matches!(e, tracedraw_core::live::Effect::Lens(_)))
                && tracedraw_core::command::close_subpaths(&ev.main.page_path()).contains(p)
            {
                return true;
            }
            return std::iter::once(ev.main)
                .chain(ev.below)
                .chain(ev.above)
                .any(|mut part| {
                    part.effects.clear();
                    self.shape_hit_in(&part, p, tol, depth)
                });
        }
        // A dimension is picked on its lines, arrowheads and label only,
        // never in the empty part of its box: an object under it (the
        // circle a diameter is measured across) is picked there (#1493).
        // "Treat all objects as filled" leaves its lines as lines.
        if crate::dimensions::is_dimension(s) {
            return self.dimension_hit(s, p, tol);
        }
        if matches!(
            s.kind,
            ShapeKind::Text { .. } | ShapeKind::Table(_) | ShapeKind::Bitmap { .. }
        ) {
            return true;
        }
        // A group, clip frame or symbol is picked where one of its
        // objects is, not in the empty part of its box: a group lying on
        // top let nothing inside or behind it be clicked (#885).
        if matches!(
            s.kind,
            ShapeKind::Group { .. }
                | ShapeKind::ClipFrame { .. }
                | ShapeKind::SymbolInstance { .. }
        ) {
            if depth >= MAX_HIT_DEPTH {
                return true;
            }
            return self.container_hit(s, p, tol, depth + 1);
        }
        let path = s.page_path();
        // A curve left open, or an ellipse made an arc.
        let open = s.kind.is_open_curve();
        let filled =
            !matches!(s.fill, Fill::None) && (!open || self.doc().metadata.fill_open_curves);
        // The fill of an open curve shown runs across its open ends, as
        // drawn.
        let in_fill = filled
            && if open {
                tracedraw_core::command::close_subpaths(&path).contains(p)
            } else {
                path.contains(p)
            };
        if in_fill || (self.settings.treat_all_filled && path.contains(p)) {
            return true;
        }
        crate::canvas::distance_to_path(&path, p) <= (tol * 1.5).max(half + tol)
    }

    /// A part of the group, clip frame or symbol `s` (in page space) is
    /// under `p`: one of a group's visible objects; a clip frame's frame,
    /// or one of its contents where the frame shows it; the symbol's
    /// object. The parts are tested in page space with the container's
    /// transform; a group with none is tested without copying its parts.
    fn container_hit(&self, s: &tracedraw_core::Shape, p: Point, tol: f64, depth: usize) -> bool {
        let placed = |c: &tracedraw_core::Shape| {
            if s.transform == Affine::IDENTITY {
                self.shape_hit_in(c, p, tol, depth)
            } else {
                let mut c = c.clone();
                c.transform_by(s.transform);
                self.shape_hit_in(&c, p, tol, depth)
            }
        };
        match &s.kind {
            ShapeKind::Group { children } => {
                children.iter().rev().filter(|c| c.visible).any(placed)
            }
            ShapeKind::ClipFrame { frame, contents } => {
                let mut f = (**frame).clone();
                f.transform_by(s.transform);
                if self.shape_hit_in(&f, p, tol, depth) {
                    return true;
                }
                f.page_path().contains(p) && contents.iter().rev().filter(|c| c.visible).any(placed)
            }
            ShapeKind::SymbolInstance { index } => match self.doc().symbols.get(*index) {
                Some(def) => {
                    let mut c = def.shape.clone();
                    c.transform_by(s.transform);
                    self.shape_hit_in(&c, p, tol, depth)
                }
                None => true,
            },
            _ => true,
        }
    }

    /// A part of dimension `s` (in page space) is under `p`: its label
    /// anywhere in its box, its arrowheads inside, its lines near them.
    fn dimension_hit(&self, s: &tracedraw_core::Shape, p: Point, tol: f64) -> bool {
        let ShapeKind::Group { children } = &s.kind else {
            return false;
        };
        children.iter().filter(|c| c.visible).any(|c| {
            let mut c = c.clone();
            c.transform_by(s.transform);
            if matches!(c.kind, ShapeKind::Text { .. }) {
                return self.visual_bounds(&c).inflate(tol, tol).contains(p);
            }
            let path = c.page_path();
            let closed = !matches!(c.kind, ShapeKind::Path { closed: false, .. });
            (closed && !matches!(c.fill, Fill::None) && path.contains(p))
                || crate::canvas::distance_to_path(&path, p) <= tol * 1.5
        })
    }

    /// The children of a group (`g` in page space) that are under `p`, top
    /// first, each in page space; a child group counts only where one of
    /// its own objects is. The children of a clip frame are its contents,
    /// and only where they show: inside the frame.
    fn children_hit(
        &self,
        g: &tracedraw_core::Shape,
        p: Point,
        tol: f64,
    ) -> Vec<tracedraw_core::Shape> {
        let children = match &g.kind {
            ShapeKind::Group { children } => children,
            ShapeKind::ClipFrame { frame, contents } => {
                let mut f = (**frame).clone();
                f.transform_by(g.transform);
                if !f.page_path().contains(p) {
                    return Vec::new();
                }
                contents
            }
            _ => return Vec::new(),
        };
        children
            .iter()
            .rev()
            .filter(|c| c.visible && !c.locked)
            .map(|c| {
                let mut c = c.clone();
                c.transform_by(g.transform);
                c
            })
            .filter(|c| {
                if matches!(c.kind, ShapeKind::Group { .. }) {
                    !self.children_hit(c, p, tol).is_empty()
                } else {
                    self.shape_hit(c, p, tol)
                }
            })
            .collect()
    }

    /// The objects of selection group `n` that are still in the drawing.
    pub fn selection_group(&self, n: u8) -> Vec<ShapeId> {
        self.doc()
            .selection_groups
            .get(&n)
            .map(|ids| {
                ids.iter()
                    .copied()
                    .filter(|id| self.doc().find_shape(*id).is_some())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Store `ids` as selection group `n`.
    pub fn set_selection_group(&mut self, n: u8, ids: Vec<ShapeId>) {
        self.run(Command::SetSelectionGroup {
            number: n,
            shapes: ids,
        });
    }

    /// Select selection group `n` (its objects on the current page); when
    /// it is selected already, zoom to it.
    pub fn recall_selection_group(&mut self, n: u8) {
        // Objects on the layers the current page shows (one inside a group
        // counts by the group it is in).
        let shown: Vec<ShapeId> = self
            .page_layers()
            .into_iter()
            .flat_map(|l| l.shapes.iter().map(|s| s.id))
            .collect();
        let ids: Vec<ShapeId> = self
            .selection_group(n)
            .into_iter()
            .filter(|id| {
                let top = self.doc().top_level_of(*id).unwrap_or(*id);
                shown.contains(&top)
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let same =
            ids.len() == self.selection.len() && ids.iter().all(|id| self.selection.contains(id));
        if same {
            self.zoom_to_selection();
        } else {
            self.select(ids);
        }
    }

    /// Ctrl+click with the Pick tool on a clip frame on a layer: its
    /// contents are taken out for editing (Edit Contents), the object of
    /// the contents under the pointer selected (all of them when the
    /// click is off them). Nothing happens while contents are edited
    /// already, or off a clip frame; then `false` (#982).
    pub fn ctrl_click_edits_clip_frame(&mut self, p: Point) -> bool {
        if self.clip_frame_edit.is_some() {
            return false;
        }
        let chain = self.hit_chain(p);
        let Some(&clip) = chain.first() else {
            return false;
        };
        let doc = self.doc();
        let on_layer = doc.top_level_of(clip) == Some(clip);
        let is_clip = doc
            .find_shape(clip)
            .is_some_and(|s| matches!(s.kind, ShapeKind::ClipFrame { .. }));
        if !on_layer || !is_clip {
            return false;
        }
        let under = chain.get(1).copied();
        self.select(vec![clip]);
        self.edit_clip_frame();
        if self.clip_frame_edit.is_none() {
            return false;
        }
        if let Some(id) = under.filter(|id| self.doc().find_shape(*id).is_some()) {
            self.select(vec![id]);
        }
        true
    }

    /// Ctrl+click: one group level further in at `p` than what is selected
    /// there (the first level inside the topmost group when nothing of it
    /// is), down to the innermost object.
    pub fn hit_test_step(&self, p: Point) -> Option<ShapeId> {
        let chain = self.hit_chain(p);
        let next = match chain.iter().rposition(|id| self.selection.contains(id)) {
            Some(k) => k + 1,
            None => 1,
        };
        chain.get(next.min(chain.len().saturating_sub(1))).copied()
    }

    /// The topmost object at `p` (or the one whose drop shadow is there,
    /// see [`Self::pick_at`]) and the objects inside it under `p`, one
    /// group level further in each, down to the innermost one.
    fn hit_chain(&self, p: Point) -> Vec<ShapeId> {
        let Some(top) = self.pick_at(p) else {
            return Vec::new();
        };
        let tol = self.object_pick_tol();
        let mut chain = vec![top];
        let Ok((_, s)) = self.doc().shape(top) else {
            return chain;
        };
        let mut current = s.clone();
        while let Some(c) = self.children_hit(&current, p, tol).into_iter().next() {
            chain.push(c.id);
            current = c;
        }
        chain
    }

    /// Whether a Pick press at `p` on a guideline takes the guideline
    /// rather than what lies under it: objects win, as they are what is
    /// usually edited, but not a picture that is not selected, so a
    /// guideline over a photo moves (#2640: it could not be picked over
    /// an image); the rotation handles of a guideline in rotate mode win
    /// over everything.
    pub fn guide_wins_press(&self, p: Point) -> bool {
        if self.guide_rotate.is_some() {
            return true;
        }
        match self.hit_test(p) {
            None => true,
            Some(id) => {
                !self.selection.contains(&id)
                    && self
                        .doc()
                        .find_shape(id)
                        .is_some_and(|s| matches!(s.kind, ShapeKind::Bitmap { .. }))
            }
        }
    }

    /// Whether a Pick press at `p` lands on what is selected: `None`
    /// where no object is, else whether the topmost object there, or one
    /// of its parts under `p`, is in the selection (a drag then moves the
    /// selection as it is, see [`Self::press_target`]).
    pub fn over_selection(&self, p: Point) -> Option<bool> {
        if self.selection.is_empty() {
            return self.hit_test(p).map(|_| false);
        }
        let chain = self.hit_chain(p);
        (!chain.is_empty()).then(|| chain.iter().any(|id| self.selection.contains(id)))
    }

    /// Whether a selected object lies under `p`, also one hidden behind
    /// another object there (a right click on it keeps the selection and
    /// opens the selection's menu, #1684): the objects at `p` on the
    /// layers, or inside groups and clip frames for one chosen in there.
    pub fn selection_under(&self, p: Point) -> bool {
        if self.selection.is_empty() {
            return false;
        }
        self.hits_at(p, false)
            .iter()
            .chain(self.hits_at(p, true).iter())
            .any(|id| self.selection.contains(id))
    }

    /// What a Pick press at `p` takes: a selected object under `p` (an
    /// object chosen inside a group stays chosen, so a drag moves it alone,
    /// #849), else with `ctrl` the object one group level in, else the
    /// topmost object.
    pub fn press_target(&self, p: Point, ctrl: bool) -> Option<ShapeId> {
        let chain = self.hit_chain(p);
        if let Some(id) = chain.iter().rev().find(|id| self.selection.contains(id)) {
            return Some(*id);
        }
        if ctrl {
            self.hit_test_step(p)
        } else {
            chain.first().copied()
        }
    }

    /// Every object at `p`, top first: the objects on the layers, or with
    /// `deep` the innermost objects inside groups (Alt+click digs through
    /// these, Ctrl+Alt+click through the deep ones).
    pub fn hits_at(&self, p: Point, deep: bool) -> Vec<ShapeId> {
        let tol = self.object_pick_tol();
        let mut out = Vec::new();
        fn leaves(
            app: &App,
            s: &tracedraw_core::Shape,
            p: Point,
            tol: f64,
            out: &mut Vec<ShapeId>,
        ) {
            let inner = app.children_hit(s, p, tol);
            for c in &inner {
                leaves(app, c, p, tol, out);
            }
            // A clip frame is an object of its own under its contents.
            if inner.is_empty() || matches!(s.kind, ShapeKind::ClipFrame { .. }) {
                out.push(s.id);
            }
        }
        for s in self
            .page_layers()
            .into_iter()
            .rev()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.shapes.iter().rev())
            .filter(|s| {
                !s.locked && s.visible && !self.outside_clip_edit(s.id) && self.shape_hit(s, p, tol)
            })
        {
            if deep {
                leaves(self, s, p, tol, &mut out);
            } else {
                out.push(s.id);
            }
        }
        out
    }

    /// Add the objects `region` takes (wholly inside, or with `touching`
    /// those it touches) to the selection: on unlocked, visible layers,
    /// not locked or hidden. With `toggle` (Shift) a taken object that is
    /// selected already is deselected instead.
    pub fn select_in_region(
        &mut self,
        region: crate::select_region::Region,
        touching: bool,
        toggle: bool,
    ) {
        let doc = self.doc();
        let fill_open = doc.metadata.fill_open_curves;
        let all_filled = self.settings.treat_all_filled;
        let fill_counts = |s: &tracedraw_core::Shape| {
            all_filled || (!matches!(s.fill, Fill::None) && (fill_open || !s.kind.is_open_curve()))
        };
        let before = self.selection.clone();
        let mut ids = before.clone();
        // Editing a symmetry group: its members are taken, not the group.
        let members = self.symmetry_members_on_page();
        let mut candidates: Vec<tracedraw_core::Shape> = Vec::new();
        for s in self
            .page_layers()
            .into_iter()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| &l.shapes)
            .filter(|s| s.visible && !s.locked && !self.outside_clip_edit(s.id))
        {
            match &members {
                Some((g, inside)) if s.id == *g => candidates.extend(inside.iter().cloned()),
                _ => candidates.push(s.clone()),
            }
        }
        for i in crate::select_region::taken(region, &candidates, touching, &fill_counts) {
            let Some(s) = candidates.get(i) else {
                continue;
            };
            if !before.contains(&s.id) {
                if !ids.contains(&s.id) {
                    ids.push(s.id);
                }
            } else if toggle {
                ids.retain(|id| *id != s.id);
            }
        }
        self.select(ids);
        self.marquee_selection = Some(self.selection.clone());
    }

    /// A click that selects with a tool other than Pick (see
    /// [`Tool::click_selects`]): the object under `p` becomes the
    /// selection, Shift adds it or takes it away, Ctrl reaches one group
    /// level further in, and a click on empty space (without Shift) lets
    /// go of the selection. The tool stays.
    pub fn click_select(&mut self, p: Point, mods: Modifiers) {
        let hit = if mods.ctrl {
            self.hit_test_step(p)
        } else {
            self.pick_at(p)
        };
        match hit {
            Some(id) if mods.shift => {
                if let Some(i) = self.selection.iter().position(|s| *s == id) {
                    self.selection.remove(i);
                    self.rotate_mode = false;
                } else {
                    self.select_add(id);
                }
            }
            Some(id) => self.select(vec![id]),
            None if mods.shift => {}
            None => self.select(Vec::new()),
        }
    }

    /// Add `id` to the selection, taking out the groups holding it and the
    /// objects inside it, so nothing is selected twice over.
    pub fn select_add(&mut self, id: ShapeId) {
        let above = self.doc().ancestors(id);
        let doc = self.doc();
        let keep: Vec<ShapeId> = self
            .selection
            .iter()
            .copied()
            .filter(|s| *s != id && !above.contains(s) && !doc.ancestors(*s).contains(&id))
            .collect();
        self.selection = keep;
        self.selection.push(id);
        self.rotate_mode = false;
    }

    /// The selection handles' size, interface points: Options > Nodes
    /// and Handles > Handle size (Small 6, Medium 8, Large 10, the
    /// default; #1285).
    pub fn handle_size(&self) -> f32 {
        self.settings.nodes.handles.px()
    }

    /// How far out of the selection box the handles' centres stand,
    /// interface points: a gap of 1.5 points between handle and box.
    pub fn handle_offset(&self) -> f32 {
        self.handle_size() / 2.0 + 1.5
    }

    /// A press this close to a handle's centre takes it: within the
    /// handle, short of the selection box (the handles stand outside it).
    fn handle_reach(&self) -> f32 {
        (self.handle_size() / 2.0 + 1.0).max(4.0)
    }

    /// Screen-space handle positions for the selection box.
    pub fn handle_positions(&self, b: Rect) -> [(Handle, egui::Pos2); 8] {
        let off = if self.rotate_mode {
            ROTATE_HANDLE_OFFSET
        } else {
            self.handle_offset()
        };
        let r = self.view.rect_to_screen(b).expand(off);
        [
            (Handle::NW, r.left_top()),
            (Handle::N, r.center_top()),
            (Handle::NE, r.right_top()),
            (Handle::W, r.left_center()),
            (Handle::E, r.right_center()),
            (Handle::SW, r.left_bottom()),
            (Handle::S, r.center_bottom()),
            (Handle::SE, r.right_bottom()),
        ]
    }

    /// The centre of rotation of the selection: where it was moved to in
    /// rotate mode (now or the last time these objects were selected),
    /// else the middle of the selection.
    pub fn rotation_center(&self) -> Option<Point> {
        let b = self.selection_bounds()?;
        Some(self.moved_pivot(b).unwrap_or(b.center()))
    }

    /// The centre of rotation the user moved for the selection (page
    /// space), if one was moved.
    pub fn moved_rotation_center(&self) -> Option<Point> {
        self.moved_pivot(self.selection_bounds()?)
    }

    /// The centre of rotation moved for the selection, in page space.
    fn moved_pivot(&self, b: Rect) -> Option<Point> {
        let p = match self.selection.as_slice() {
            [] => return None,
            [one] => {
                let local = self.pivots.single.get(one)?;
                self.doc().shape_on_page(*one)?.transform * *local
            }
            many => {
                let mut key = many.to_vec();
                key.sort();
                let (u, v) = self.pivots.several.get(&key)?;
                Point::new(b.x0 + u * b.width(), b.y0 + v * b.height())
            }
        };
        (p.x.is_finite() && p.y.is_finite()).then_some(p)
    }

    /// Put the selection's centre of rotation at `p` (page space), kept
    /// with the object (or the objects) for the next time.
    pub fn set_pivot(&mut self, p: Point) {
        match self.selection.clone().as_slice() {
            [] => {}
            [one] => {
                let Some(t) = self.doc().shape_on_page(*one).map(|s| s.transform) else {
                    return;
                };
                let det = t.determinant();
                if det.is_finite() && det.abs() > 1e-12 {
                    self.pivots.single.insert(*one, t.inverse() * p);
                }
            }
            many => {
                let Some(b) = self.selection_bounds() else {
                    return;
                };
                let (w, h) = (b.width().max(1e-9), b.height().max(1e-9));
                let mut key = many.to_vec();
                key.sort();
                self.pivots
                    .several
                    .insert(key, ((p.x - b.x0) / w, (p.y - b.y0) / h));
            }
        }
    }

    /// The selection's centre of rotation goes back to its middle.
    pub fn reset_pivot(&mut self) {
        match self.selection.as_slice() {
            [] => {}
            [one] => {
                self.pivots.single.remove(one);
            }
            many => {
                let mut key = many.to_vec();
                key.sort();
                self.pivots.several.remove(&key);
            }
        }
    }

    /// The pointer is on the centre of rotation (rotate mode).
    pub fn pivot_at(&self, screen: egui::Pos2) -> bool {
        self.rotate_mode
            && self
                .rotation_center()
                .is_some_and(|c| self.view.to_screen(c).distance(screen) <= PIVOT_PX)
    }

    /// The pointer is on the selection's centre mark, the "x" (outside
    /// rotate mode): a drag there moves the selection, a click toggles the
    /// rotate and skew handles.
    pub fn center_mark_at(&self, screen: egui::Pos2) -> bool {
        !self.rotate_mode
            && matches!(self.tool, Tool::Pick | Tool::FreeformPick)
            && self
                .selection_bounds()
                .is_some_and(|b| self.view.to_screen(b.center()).distance(screen) <= PIVOT_PX)
    }

    /// Start moving the selection from `p` (page).
    /// The selection's own point under `p` that a drag in rotate mode
    /// turns the selection by: a node, a segment's midpoint or an
    /// ellipse's quadrant (as the pointer marks it), away from the centre
    /// of rotation (#933).
    pub(crate) fn grab_turn_point(&self, p: Point) -> Option<Point> {
        use crate::snap_points::SnapMode;
        if !self.rotate_mode || !matches!(self.tool, Tool::Pick | Tool::FreeformPick) {
            return None;
        }
        let t = self.selection_hover_target(p)?;
        if !matches!(
            t.mode,
            SnapMode::Node | SnapMode::Midpoint | SnapMode::Quadrant
        ) {
            return None;
        }
        let c = self.rotation_center()?;
        ((t.point - c).hypot() > self.snap_tolerance()).then_some(t.point)
    }

    /// Rotate mode: a press on a node of the selection (see
    /// [`App::grab_turn_point`]) starts turning it about the centre of
    /// rotation, the node following the pointer round its circle.
    fn start_grab_turn(&mut self, p: Point) -> bool {
        let Some(g) = self.grab_turn_point(p) else {
            return false;
        };
        let Some(c) = self.rotation_center() else {
            return false;
        };
        let a = (g - c).atan2();
        self.drag = Drag::Rotate {
            center: c,
            start_angle: a,
            current_angle: a,
            grab: Some(g),
        };
        true
    }

    pub(crate) fn start_move(&mut self, p: Point) {
        // A text fitted to a path slides along it instead.
        if self.fitted_text().is_some() {
            self.drag = Drag::AlongPath {
                start: p,
                current: p,
            };
            return;
        }
        self.drop_colour = None;
        self.drop_colour_target = None;
        self.drag = Drag::Move {
            start: p,
            last: p,
            total: Vec2::ZERO,
            start_bounds: self.selection_bounds().unwrap_or(Rect::ZERO),
            source: self.selection_snap_source(p),
            start_geometry: None,
        };
    }

    /// Where the next node of the curve being drawn goes for the pointer
    /// at `p`: with `ctrl` in steps of the constrain angle from the last
    /// node.
    /// The constrain key (Options > Tools > Curve tools; Ctrl at first)
    /// is among the modifiers held.
    pub fn constrain_held(&self, mods: &Modifiers) -> bool {
        self.settings.curves.constrain_key.held(mods)
    }

    pub fn curve_point(&self, p: Point, ctrl: bool) -> Point {
        match self.curve.as_ref().and_then(|c| c.nodes.last()) {
            Some((last, _)) if ctrl => constrain_from(*last, p, self.settings.constrain_angle),
            _ => p,
        }
    }

    /// The pointer is over the document palette.
    pub fn over_document_palette(&self) -> bool {
        self.doc_palette_rect
            .zip(self.pointer_screen)
            .is_some_and(|(r, p)| r.contains(p))
    }

    /// A move, scale, rotation or skew of the selection is being dragged.
    pub fn transforming(&self) -> bool {
        matches!(
            self.drag,
            Drag::Move { .. }
                | Drag::Scale { .. }
                | Drag::Rotate { .. }
                | Drag::Skew { .. }
                | Drag::MoveGuide { .. }
        ) && matches!(self.tool, Tool::Pick | Tool::FreeformPick)
            || (matches!(self.drag, Drag::Scale { .. }) && self.tool.resizes_by_handles())
    }

    /// The end of a move, scale, rotation or skew: on the selection, or on
    /// copies of it (which become the selection) after a right click
    /// during the drag.
    fn finish_transform(&mut self, t: Affine) {
        if std::mem::take(&mut self.drag_copy) {
            self.copy_selection_by(t);
            return;
        }
        self.transform_selection(t);
    }

    /// Space during a move leaves a copy where the selection is now.
    /// Space while the selection is moved, scaled, turned or slanted drops
    /// a copy where the outline is, and the drag goes on.
    fn stamp_on_space(&mut self, response: &Response) {
        // One copy a press: a key held sends its press again and again
        // (`repeat`), which left a trail of copies (#1502).
        let pressed = response.ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Space,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        });
        if !pressed {
            return;
        }
        let t = match &self.drag {
            Drag::Move { total, .. } => Some(Affine::translate(*total)),
            d @ (Drag::Scale { .. } | Drag::Rotate { .. } | Drag::Skew { .. }) => {
                self.preview_transform_of(d)
            }
            _ => None,
        };
        if let Some(t) = t {
            self.stamp_copies(t);
        }
    }

    /// Copies of the selection moved by `t`, left where they are while the
    /// drag goes on (Space during a move), as one undo step.
    pub fn stamp_copies(&mut self, t: Affine) {
        let Some(layer) = self.active_layer() else {
            return;
        };
        let mut cmds = Vec::new();
        for s in self.selected_shapes() {
            let id = self.engine.new_shape_id();
            let mut shape = crate::app::reid_pub(s, id, &mut self.engine);
            shape.transform_by(t);
            cmds.push(Command::AddShape { layer, shape });
        }
        if cmds.is_empty() {
            return;
        }
        if let Err(e) = self.engine.run_batch("Copy", &cmds) {
            self.status = e.to_string();
        }
    }

    /// A selection box or freehand selection of the Pick tools, or one of
    /// the Shape tool's over nodes, is being dragged.
    pub fn selecting(&self) -> bool {
        (matches!(self.drag, Drag::Marquee { .. } | Drag::Lasso { .. })
            && matches!(self.tool, Tool::Pick | Tool::FreeformPick))
            || matches!(self.drag, Drag::NodeMarquee { .. } | Drag::NodeLasso { .. })
    }

    /// A selection box (or freehand selection) runs from the press until
    /// the left button goes up, whatever the right button does meanwhile:
    /// a right press is ignored. egui takes the right press for the start
    /// of a new click or drag and never reported the left release as the
    /// end of the drag, so the box stayed on the canvas, frozen, until the
    /// next one replaced it (#1015). While the left button is held the box
    /// follows the latest pointer position even when egui reports no
    /// hover after the right press. The Shape tool's selection box and
    /// freehand selection over nodes go the same way: the right press
    /// opened the node menu during the drag and left the box on the
    /// canvas (#1848).
    /// While the right button is held as well, a selection box (of
    /// objects or of nodes) moves with the pointer instead of changing
    /// its size, and takes up its sizing again when the right button
    /// goes up (#2050).
    fn selection_drag_input(
        &mut self,
        response: &Response,
        hover: Option<egui::Pos2>,
        mods: Modifiers,
    ) {
        let (held, right) = response
            .ctx
            .input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
        let at = hover.or(if held { self.pointer_screen } else { None });
        if let Some(s) = at {
            let p = self.view.to_page(s);
            self.pointer_page = Some(p);
            let boxed = matches!(self.drag, Drag::Marquee { .. } | Drag::NodeMarquee { .. });
            if held && right && boxed {
                if let Some(last) = self.marquee_moved_from {
                    let d = p - last;
                    if let Drag::Marquee { start, current, .. }
                    | Drag::NodeMarquee { start, current } = &mut self.drag
                    {
                        *start += d;
                        *current += d;
                    }
                }
                self.marquee_moved_from = Some(p);
            } else if held {
                self.marquee_moved_from = None;
                // Shift pressed during a node marquee adds too (#2331).
                if mods.shift
                    && matches!(self.drag, Drag::NodeMarquee { .. } | Drag::NodeLasso { .. })
                {
                    self.node_lasso_keeps = true;
                }
                let step = 0.2 / self.view.zoom as f64;
                match &mut self.drag {
                    Drag::NodeMarquee { current, .. } => *current = p,
                    Drag::NodeLasso { points } => {
                        if points.last().is_none_or(|q| (*q - p).hypot() > step) {
                            points.push(p);
                        }
                    }
                    _ => self.pick_drag_to(p, mods),
                }
            }
        }
        if !held {
            self.marquee_moved_from = None;
            self.end_drag();
        }
    }

    /// Whether the drag running scrolls the view when the pointer nears
    /// the window's edge (#2051): moving and sizing objects, drawing,
    /// selecting, dragging nodes and handles, connectors and text frames.
    pub(crate) fn scrolls_at_edge(&self) -> bool {
        matches!(
            self.drag,
            Drag::Box { .. }
                | Drag::ThreePointBase { .. }
                | Drag::Move { .. }
                | Drag::Scale { .. }
                | Drag::Marquee { .. }
                | Drag::Lasso { .. }
                | Drag::Freehand { .. }
                | Drag::Node { .. }
                | Drag::Handle { .. }
                | Drag::KindNode { .. }
                | Drag::NodeMove { .. }
                | Drag::SegmentBend { .. }
                | Drag::PolygonEdge { .. }
                | Drag::NodeLasso { .. }
                | Drag::NodeMarquee { .. }
                | Drag::Connector { .. }
                | Drag::TextFrame { .. }
        )
    }

    /// Auto-scroll (#2051): while a drag that [`App::scrolls_at_edge`]
    /// has the left button held within [`EDGE_SCROLL_BAND`] of the
    /// drawing window's edge, or past it, the view scrolls toward that
    /// edge, faster the nearer it is, and the frame after is asked for so
    /// it goes on while the pointer rests. A few comparisons a frame; the
    /// drag then follows the pointer in the scrolled view.
    fn edge_scroll(&mut self, response: &Response) {
        if !self.scrolls_at_edge() {
            return;
        }
        let (held, dt) = response
            .ctx
            .input(|i| (i.pointer.primary_down(), i.stable_dt));
        let Some(p) = self.pointer_screen.filter(|_| held) else {
            return;
        };
        let step = edge_scroll_step(response.rect, p, dt.clamp(0.0, 0.05));
        if step != egui::Vec2::ZERO {
            self.view.pan(step);
            response.ctx.request_repaint();
        }
    }

    /// A scale of the selection by handle `h` of its box `b`, pressed at
    /// `p`: the opposite side or corner stays put.
    fn start_handle_scale(&mut self, h: Handle, b: Rect, p: Point, mods: Modifiers) {
        // The point of the box the handle stands for, and how far the
        // pointer is from it.
        let grab = p - handle_point(b, h);
        let anchor = match h {
            Handle::N => Point::new(b.center().x, b.y0),
            Handle::S => Point::new(b.center().x, b.y1),
            Handle::E => Point::new(b.x0, b.center().y),
            Handle::W => Point::new(b.x1, b.center().y),
            Handle::NE => Point::new(b.x0, b.y0),
            Handle::NW => Point::new(b.x1, b.y0),
            Handle::SE => Point::new(b.x0, b.y1),
            Handle::SW => Point::new(b.x1, b.y1),
        };
        self.drag = Drag::Scale {
            handle: h,
            anchor,
            start_bounds: b,
            current: p - grab,
            grab,
            mods: crate::app::HandleMods::of(mods),
            snapped: (false, false),
        };
    }

    pub(crate) fn handle_at(&self, screen: egui::Pos2) -> Option<Handle> {
        // A drop shadow chosen on its own shows its box, not the object's
        // handles (#2545).
        if self.picked_shadow().is_some() {
            return None;
        }
        let b = self.selection_bounds()?;
        let reach = if self.rotate_mode {
            ROTATE_HANDLE_PX
        } else {
            self.handle_reach()
        };
        self.handle_positions(b)
            .iter()
            .find(|(_, p)| p.distance(screen) <= reach)
            .map(|(h, _)| *h)
    }

    pub fn canvas_input(&mut self, response: &Response, mods: Modifiers) {
        self.freehand_end_tick(&response.ctx);
        self.snap_mark.set(None);
        self.clear_live_guides();
        self.pointer_screen = response.ctx.pointer_latest_pos();
        self.edge_scroll(response);
        // egui ends a drag when the right button of a right click goes up
        // and then hovers nothing while the left one is held: the
        // transform goes on with the latest pointer position.
        // A Pen or Bezier handle pulled out of the drawing window goes on
        // following the pointer, and its release ends it there (#1014).
        // A shape being drawn after a middle-button pan: egui counts no
        // drag of the held left button any more and hovers nothing (#1923).
        // A drag taken past the window's edge goes on with the pointer
        // while the view scrolls (#2051).
        let hover = response.hover_pos().or_else(|| {
            let held = response.ctx.input(|i| i.pointer.primary_down());
            let boxing = matches!(self.drag, Drag::Box { .. }) || self.scrolls_at_edge();
            if ((self.transforming() || boxing) && held) || self.curve_drag_running(response) {
                self.pointer_screen
            } else {
                None
            }
        });
        // A finger's tap: the windowing layer lets the pointer go with the
        // finger (the release comes with PointerGone), so the frame of the
        // click has no hover position and no tool saw the tap: the Text
        // tool placed no text and the keyboard never came (#1496, #1489).
        // Where the click happened stands in.
        let hover = hover.or_else(|| {
            let clicked = response.ctx.input(|i| i.pointer.any_click());
            response.interact_pointer_pos().filter(|_| clicked)
        });
        let pointer = hover.map(|h| self.view.to_page(h));
        self.pointer_page = pointer;
        // A colour picker's eyedropper waits for a click: the colour there
        // goes to it, and no tool sees the click (#814). Esc stops it.
        if crate::ui::color_picker::picking(&response.ctx) {
            response.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            if response.ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                crate::ui::color_picker::finish_pick(&response.ctx, None);
            } else if let (true, Some(p)) = (response.clicked(), pointer) {
                let c = self
                    .sample_color_at(p, false)
                    .or_else(|| self.sample_color_at(p, true))
                    .unwrap_or(tracedraw_core::Color::WHITE);
                crate::ui::color_picker::finish_pick(&response.ctx, Some(c));
            }
            return;
        }
        if self.selecting() {
            self.selection_drag_input(response, hover, mods);
            return;
        }
        let Some(p) = pointer else {
            let up = !response.ctx.input(|i| i.pointer.primary_down());
            let boxing = matches!(self.drag, Drag::Box { .. });
            let released = (self.transforming() || boxing) && up;
            if response.drag_stopped() || released {
                self.end_drag();
            }
            if up {
                self.end_curve_drag_outside();
            }
            return;
        };
        let mut screen = hover.unwrap_or_default();
        // A drag starts after the pointer moved a few pixels; hit-test at the
        // press position so small targets like nodes and handles are caught.
        let mut p = p;
        // (Not during a transform: a right press there restarts egui's drag
        // and its origin is that press.)
        if response.drag_started() && !self.transforming() {
            if let Some(origin) = response.ctx.input(|i| i.pointer.press_origin()) {
                screen = origin;
                p = self.view.to_page(origin);
            }
        }

        // Middle button always pans. A shape being drawn with the left
        // button held follows the pointer over the moving page (#1923).
        if response.dragged_by(PointerButton::Middle) {
            self.pan_drag(response.drag_delta(), &response.ctx);
            if matches!(self.drag, Drag::Box { .. })
                && response.ctx.input(|i| i.pointer.primary_down())
            {
                if let Some(s) = response.hover_pos() {
                    let now = self.snap_point(self.view.to_page(s));
                    self.box_follow(now, mods);
                }
            }
            return;
        }
        // File > Import's file waiting for a click, a drag or Space to
        // place it (#1936).
        if self.import_placement_input(response, p) {
            return;
        }
        // A right click during a move, scale, rotation or skew: the change
        // goes to copies on release, the originals stay (again: not).
        if self.transforming() {
            let right_pressed = response
                .ctx
                .input(|i| i.pointer.button_pressed(PointerButton::Secondary));
            if right_pressed || take_chorded_right_press() {
                self.drag_copy = !self.drag_copy;
                self.status = crate::i18n::tr(if self.drag_copy {
                    "status.drag_copy_on"
                } else {
                    "status.drag_copy_off"
                });
            }
            // The transform runs from the press until the left button goes
            // up, whatever the right button does meanwhile. egui takes a
            // press of either button for the start of a new click or drag:
            // after a right press it reports the left release as a click
            // (which picked, or emptied the selection on bare canvas,
            // before the copy was made) and a held right press as a new
            // drag. Neither reaches the tools here.
            if response.ctx.input(|i| i.pointer.primary_down()) {
                self.note_drop_keys(&response.ctx);
                self.pick_drag_to(p, mods);
                self.stamp_on_space(response);
            } else {
                self.end_drag();
            }
            return;
        } else if matches!(self.tool, Tool::Pick | Tool::FreeformPick)
            && self.right_drag_input(response, p)
        {
            // Right-button drags of objects end in the drop menu.
            return;
        }

        // The tab ruler over the paragraph frame being edited (#1852).
        if self.tool == Tool::Text && self.tab_ruler_input(response, screen) {
            return;
        }
        // With the Zoom tool a right click zooms out by default; with the
        // Pan tool when Options > Tools > Zoom/Pan says so.
        if response.secondary_clicked()
            && ((self.tool == Tool::Zoom && self.settings.zoom_right_click_out)
                || (self.tool == Tool::Pan && self.settings.pan_right_click_out))
        {
            self.view.zoom_at(screen, 0.5);
            self.end_zoom_one_shot();
            return;
        }
        // Right click: context menu (drawn by the UI at this position);
        // none while a transform is dragged.
        if response.secondary_clicked()
            && !self.pending_clip_frame
            && self.pending_text_link.is_none()
            && !self.transforming()
        {
            if self.pending_click(response, p) || self.coords_pick_input(response, p) {
                return;
            }
            self.open_context_menu(screen, p);
            return;
        }
        if self.pending_click(response, p) {
            return;
        }
        // The Coordinates docker waits for a click or a drag.
        if self.coords_pick_input(response, p) {
            return;
        }

        if self.pending_text_link.is_some() {
            // A drag that began on the tab: the release says where the
            // text flows, as a click there would.
            if self.text_link_from_tab {
                if response.drag_stopped_by(PointerButton::Primary) {
                    let now = self.pointer_page.unwrap_or(p);
                    self.finish_text_link(now, None);
                } else if !response.ctx.input(|i| i.pointer.primary_down()) {
                    // Let go without a drag: the next click says.
                    self.text_link_from_tab = false;
                }
                return;
            }
            if response.drag_started_by(PointerButton::Primary) {
                self.text_link_drag = Some(p);
            } else if response.drag_stopped_by(PointerButton::Primary) {
                if let Some(a) = self.text_link_drag.take() {
                    let now = self.pointer_page.unwrap_or(p);
                    self.finish_text_link(now, Some(Rect::from_points(a, now)));
                }
            } else if response.clicked_by(PointerButton::Primary) {
                self.finish_text_link(p, None);
            }
            if response.secondary_clicked() {
                self.cancel_text_link();
            }
            return;
        }
        if self.pending_clip_frame {
            if response.clicked_by(PointerButton::Primary) {
                self.pending_clip_frame = false;
                if let Some(frame) = self.hit_test(p) {
                    let contents: Vec<ShapeId> = self
                        .selection
                        .iter()
                        .copied()
                        .filter(|id| *id != frame)
                        .collect();
                    if contents.is_empty() {
                        self.status = crate::i18n::tr("status.clip_frame_select_contents");
                    } else {
                        self.place_inside(contents, frame);
                        self.select(vec![frame]);
                    }
                } else {
                    self.status = crate::i18n::tr("status.clip_frame_cancelled");
                }
            }
            if response.secondary_clicked() {
                self.pending_clip_frame = false;
            }
            return;
        }
        // Right after a shape is drawn its handles show with the drawing
        // tool still chosen: a drag from one resizes the selection, as
        // with the Pick tool, and a click on one leaves it as it is,
        // instead of starting a new shape or picking.
        if self.tool.resizes_by_handles()
            && matches!(self.drag, Drag::None)
            && self.three_point_base.is_none()
            && !self.rotate_mode
        {
            if let Some(h) = self.handle_at(screen) {
                if response.drag_started_by(PointerButton::Primary) {
                    let b = self.selection_bounds().unwrap_or(Rect::ZERO);
                    self.drag_copy = false;
                    self.start_handle_scale(h, b, p, mods);
                    if let Some(now) = self.pointer_page {
                        self.pick_drag_to(now, mods);
                    }
                    return;
                }
                if response.clicked_by(PointerButton::Primary) {
                    return;
                }
            }
        }
        // A frame's flow tab takes a press with any tool, as with the Pick
        // and Text tools (which look for it themselves, after their
        // handles): its text flows where the next click says (#1986).
        if !matches!(self.tool, Tool::Pick | Tool::FreeformPick | Tool::Text)
            && matches!(self.drag, Drag::None)
            && (response.clicked_by(PointerButton::Primary)
                || response.drag_started_by(PointerButton::Primary))
            && self.press_flow_tab(screen, response.drag_started_by(PointerButton::Primary))
        {
            return;
        }
        // A click with a tool whose click has no job of its own selects;
        // a drag goes on to the tool below.
        if self.tool.click_selects() && response.clicked_by(PointerButton::Primary) {
            // A click on a slider of the selected blend keeps the blend.
            if self.tool == Tool::Blend && self.blend_handle_at(p).is_some() {
                return;
            }
            self.click_select(p, mods);
            return;
        }
        match self.tool {
            Tool::Pick | Tool::FreeformPick => self.pick_input(response, p, screen, mods),
            Tool::Shape => {
                if !self.envelope_input(response, p) && !self.effect_node_input(response, p) {
                    self.shape_input(response, p, mods)
                }
            }
            Tool::FreeTransform
            | Tool::AttractRepel
            | Tool::Smudge
            | Tool::Roughen
            | Tool::VariableOutline
            | Tool::SegmentDelete
            | Tool::ShapeRecognition
            | Tool::Sketch
            | Tool::HorizontalVerticalDimension
            | Tool::AngularDimension
            | Tool::SegmentDimension
            | Tool::Callout
            | Tool::RightAngleConnector
            | Tool::RoundedConnector
            | Tool::AnchorEditing
            | Tool::BlockShadow
            | Tool::InnerShadow
            | Tool::Envelope
            | Tool::Extrude
            | Tool::Distort
            | Tool::AreaFill
            | Tool::MeshFill
            | Tool::OutlinePen
            | Tool::OutlineColor => self.tools3_input(response, p, mods),
            Tool::Zoom => self.zoom_input(response, p, screen, mods),
            Tool::Pan => {
                if response.dragged() {
                    self.pan_drag(response.drag_delta(), &response.ctx);
                }
            }
            t if t.is_box_tool() => self.box_input(response, p, mods),
            Tool::ThreePointRectangle | Tool::ThreePointEllipse | Tool::ThreePointCurve => {
                self.three_point_input(response, p, mods)
            }
            Tool::Freehand => self.freehand_input(response, p, mods),
            Tool::Bezier | Tool::Pen | Tool::Polyline | Tool::TwoPointLine | Tool::BSpline => {
                self.curve_input(response, p, mods)
            }
            Tool::Text => self.text_input(response, p, mods),
            Tool::ColorEyedropper | Tool::AttributesEyedropper => {
                if response.clicked() {
                    let attrs = self.tool == Tool::AttributesEyedropper;
                    // Ctrl samples again in Apply mode (#1302).
                    self.eyedropper_click(p, attrs, mods.shift, mods.ctrl);
                }
            }
            Tool::InteractiveFill => self.fill_input(response, p, mods),
            Tool::Crop => self.crop_input(response, p),
            Tool::Contour => self.contour_input(response, p),
            Tool::Knife
            | Tool::Spiral
            | Tool::CommonShapes
            | Tool::Table
            | Tool::BrushStrokes
            | Tool::PainterlyBrush
            | Tool::ParallelDimension
            | Tool::Connector => self.tools2_input(response, p, mods),
            Tool::Blend | Tool::Smooth | Tool::Smear | Tool::Twirl => {
                self.effects_input(response, p, mods)
            }
            Tool::DropShadow => {
                let mut first = false;
                if response.drag_started_by(PointerButton::Primary) {
                    // The press point, not where the drag was noticed.
                    let press = response
                        .ctx
                        .input(|i| i.pointer.press_origin())
                        .map(|o| self.view.to_page(o))
                        .unwrap_or(p);
                    if let Some(handle) = self.shadow_handle_at(press) {
                        self.drag = Drag::ShadowHandle {
                            handle,
                            grab: Vec2::ZERO,
                            begun: false,
                        };
                    } else if self.shadow_on_line(press) {
                        // On the line between the handles: the whole
                        // shadow moves with the pointer.
                        if let Some((_, sh, b)) = self.selected_shadow() {
                            self.drag = Drag::ShadowLine {
                                press,
                                end: sh.end_point(b),
                                begun: false,
                            };
                        }
                    } else if let Some(grab) = self.grab_shadow(press) {
                        // On the shadow itself: it moves with the pointer.
                        self.drag = Drag::ShadowHandle {
                            handle: crate::shadow_tool::ShadowHandle::End,
                            grab,
                            begun: false,
                        };
                    } else if let Some((id, _)) = self.shadow_drag_start(press) {
                        // On an object or just outside its edge. A press
                        // on one of several selected objects keeps them
                        // all: each takes the shadow (#2218).
                        if !self.selection.contains(&id) {
                            self.select(vec![id]);
                        }
                        self.drag = Drag::Shadow {
                            shape: id,
                            start: press,
                        };
                        first = true;
                    }
                }
                if response.dragged_by(PointerButton::Primary) {
                    match self.drag.clone() {
                        Drag::Shadow { shape, start } => {
                            self.drag_new_shadow(shape, start, p, first);
                            self.drag = Drag::Shadow { shape, start };
                        }
                        Drag::ShadowHandle {
                            handle,
                            grab,
                            begun,
                        } => {
                            self.drag_shadow_handle(handle, p + grab, begun);
                            self.drag = Drag::ShadowHandle {
                                handle,
                                grab,
                                begun: true,
                            };
                        }
                        Drag::ShadowLine { press, end, begun } => {
                            self.drag_shadow_line(press, end, p, mods.ctrl || mods.command, begun);
                            self.drag = Drag::ShadowLine {
                                press,
                                end,
                                begun: true,
                            };
                        }
                        _ => {}
                    }
                }
                if response.clicked_by(PointerButton::Primary) {
                    if self.shadow_handle_at(p).is_some() || self.shadow_on_line(p) {
                        return;
                    }
                    match self.pick_at(p) {
                        Some(id) => self.select(vec![id]),
                        None => self.select(Vec::new()),
                    }
                }
            }
            Tool::Transparency => {
                // The mask's nodes, as the Interactive Fill tool edits a
                // fill's: a double click on the line adds a node, on a node
                // removes it; a drag moves the start, the end, the skew, a
                // node or a midpoint slider (#448).
                if response.double_clicked_by(PointerButton::Primary) {
                    match self.fountain_handle_at(p) {
                        Some(crate::fill_tool::FountainHandle::Stop(i)) => {
                            self.remove_fountain_stop(i);
                            return;
                        }
                        Some(_) => return,
                        None => {
                            if let Some(t) = self.fountain_line_at(p) {
                                self.add_fountain_stop(t);
                                return;
                            }
                        }
                    }
                }
                if response.drag_started_by(PointerButton::Primary) {
                    // A node, a slider, or the line (the whole mask moves).
                    let press = response
                        .ctx
                        .input(|i| i.pointer.press_origin())
                        .map(|o| self.view.to_page(o))
                        .unwrap_or(p);
                    if self.begin_fountain_drag(p, press) {
                        return;
                    }
                    // The selection takes the mask; an unselected object
                    // pressed outside it is picked and takes it (#1661).
                    if let Some(shape) = self.transparency_drag_target(p) {
                        // The mask's start snaps (#2004).
                        let start = self.snap_point(p);
                        self.drag = Drag::FillGradient {
                            shape,
                            start,
                            current: start,
                        };
                    }
                }
                if response.dragged_by(PointerButton::Primary) {
                    if self.fountain_drag_to(p, mods.ctrl || mods.command || mods.shift) {
                        return;
                    }
                    self.fill_gradient_drag_to(p, mods);
                }
                if response.clicked() {
                    if let Some(h) = self.fountain_handle_at(p) {
                        if let Some(i) = self.fountain_stop_of(h) {
                            self.fill_node = Some(i);
                        }
                        return;
                    }
                    if self.fountain_line_at(p).is_some() {
                        return;
                    }
                    match self.hit_test(p) {
                        Some(id) => {
                            if !self.selection.contains(&id) {
                                self.select(vec![id]);
                            }
                        }
                        None => self.select(Vec::new()),
                    }
                }
            }
            Tool::Eraser => {
                // Drag erases a band; a click erases a dot (double-click
                // on an object deletes it whole).
                if response.double_clicked_by(PointerButton::Primary) {
                    if let Some(id) = self.hit_test(p) {
                        self.run(Command::DeleteShapes { shapes: vec![id] });
                        self.selection.retain(|s| *s != id);
                    }
                } else if response.drag_started_by(PointerButton::Primary) {
                    self.drag = Drag::Freehand { points: vec![p] };
                } else if response.dragged_by(PointerButton::Primary) {
                    if let Drag::Freehand { points } = &mut self.drag {
                        if points
                            .last()
                            .map(|l| (*l - p).hypot() > 0.3)
                            .unwrap_or(true)
                        {
                            points.push(p);
                        }
                    }
                } else if response.clicked_by(PointerButton::Primary) {
                    self.erase_along(&[p]);
                }
            }
            _ => {
                if response.clicked() {
                    self.status = crate::i18n::trf(
                        "status.tool_not_implemented",
                        &[("t", &self.tool.name())],
                    );
                }
            }
        }

        if response.drag_stopped_by(PointerButton::Primary) {
            self.end_drag();
        }
    }

    fn pick_input(&mut self, response: &Response, p: Point, screen: egui::Pos2, mods: Modifiers) {
        // Ctrl+click on a table chooses the cell under the pointer, and
        // with a cell chosen Shift+click adds another; any other click
        // lets the cells go (#2434).
        if response.clicked_by(PointerButton::Primary) {
            use crate::table_tool::CellPick;
            let chosen = self.table_edit.is_some();
            let how = if mods.command || mods.ctrl {
                Some(CellPick::One)
            } else if mods.shift && chosen {
                Some(CellPick::Toggle)
            } else {
                None
            };
            if how.is_some_and(|h| self.handle_at(screen).is_none() && self.pick_table_cell(p, h)) {
                return;
            }
            self.table_edit = None;
        }
        // A press on a frame's flow tab: where its text flows on is the
        // next click (#371).
        if response.clicked_by(PointerButton::Primary)
            || response.drag_started_by(PointerButton::Primary)
        {
            // A handle comes first: the tab never takes a drag of one.
            // A drag from the tab ends where the text flows.
            let from_tab = response.drag_started_by(PointerButton::Primary);
            if self.handle_at(screen).is_none() {
                if let Some(id) = self.flow_tab_at(screen) {
                    self.start_text_link(id);
                    self.text_link_from_tab = from_tab;
                    return;
                }
                if let Some(id) = self.flow_in_tab_at(screen) {
                    self.start_text_link_in(id);
                    self.text_link_from_tab = from_tab;
                    return;
                }
            }
        }
        // Resting over the selection (not over a handle or the centre of
        // rotation), the pointer shows the snap point or edge a drag
        // would carry, with its name.
        if matches!(self.drag, Drag::None)
            && response.hovered()
            && self.handle_at(screen).is_none()
            && !self.pivot_at(screen)
        {
            if let Some(t) = self.selection_hover_target(p) {
                self.snap_mark.set(Some(t));
            }
        }
        if response.drag_started_by(PointerButton::Primary) {
            self.drag_copy = false;
            // Editing a symmetry: its mirror lines move or turn.
            if self.symmetry_edit.is_some() && self.symmetry_press(p) {
                return;
            }
            // Rotate mode: the centre of rotation moves.
            if self.pivot_at(screen) {
                self.drag = Drag::Pivot;
                return;
            }
            if !self.rotate_mode && self.start_tracked_node_drag(p) {
                return;
            }
            if let Some(h) = self.handle_at(screen) {
                let b = self.selection_bounds().unwrap_or(Rect::ZERO);
                if self.rotate_mode {
                    if h.is_corner() {
                        let c = self.rotation_center().unwrap_or(b.center());
                        let a = (p - c).atan2();
                        self.drag = Drag::Rotate {
                            center: c,
                            start_angle: a,
                            current_angle: a,
                            grab: None,
                        };
                    } else {
                        self.drag = Drag::Skew {
                            handle: h,
                            start_bounds: b,
                            start: p,
                            current: p,
                            mods: crate::app::HandleMods::of(mods),
                        };
                    }
                } else {
                    self.start_handle_scale(h, b, p, mods);
                }
                // The drag starts once the pointer has gone a few pixels
                // from the press: the handle catches up with it now, not
                // in one jump at the next move (#383).
                if let Some(now) = self.pointer_page {
                    self.pick_drag_to(now, mods);
                }
                return;
            }
            // Rotate mode: a node of the selection (a midpoint, a quadrant)
            // turns it about the centre of rotation, carried onto other
            // objects' points and edges (#933).
            if !mods.shift && self.start_grab_turn(p) {
                return;
            }
            // The centre mark moves the selection, whatever lies under it.
            if self.center_mark_at(screen) && !mods.shift {
                self.start_move(p);
                return;
            }
            // Guidelines lie above the layers but objects win a press, as
            // they are what is usually edited; the rotation handles of a
            // guideline in rotate mode win over objects.
            if self.guide_wins_press(p) && self.press_guide(p, mods.shift) {
                return;
            }
            // Ctrl takes the object inside a group, unless the press is on
            // what is selected already (Ctrl then keeps the move straight);
            // an object chosen inside a group moves alone (#849).
            // Alt starts a selection by touching anywhere, also over an
            // object, selected or not (the object in front of a hidden
            // one, a background under the objects to pick), as a press
            // beside it would; a drag moves the selection without Alt
            // (#1341, #1350).
            let hit = if mods.alt {
                None
            } else {
                self.press_target(p, mods.ctrl)
            };
            match hit {
                Some(id) => {
                    if mods.shift {
                        if !self.selection.contains(&id) {
                            self.select_add(id);
                        }
                    } else if !self.selection.contains(&id) {
                        self.select(vec![id]);
                    }
                    self.start_move(p);
                }
                None => {
                    if !mods.shift {
                        self.select(Vec::new());
                    }
                    self.deselect_guides();
                    self.drag = if self.tool == Tool::FreeformPick {
                        Drag::Lasso {
                            points: vec![p],
                            touching: mods.alt != self.settings.marquee_touching,
                            toggle: mods.shift,
                        }
                    } else {
                        Drag::Marquee {
                            start: p,
                            current: p,
                            touching: mods.alt != self.settings.marquee_touching,
                            toggle: mods.shift,
                        }
                    };
                }
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            if matches!(self.drag, Drag::NodeMove { .. }) {
                self.node_move_to(p, mods);
                return;
            }
            self.note_drop_keys(&response.ctx);
            self.pick_drag_to(p, mods);
        }
        self.stamp_on_space(response);
        if response.clicked_by(PointerButton::Primary) {
            // A click on the centre of rotation keeps rotate mode.
            if self.handle_at(screen).is_some() || self.pivot_at(screen) {
                return;
            }
            // Editing a symmetry: a click on a mirror line shows or hides
            // the handles that turn the lines.
            if self.symmetry_edit.is_some() && self.symmetry_click(p) {
                return;
            }
            // A click on the centre mark: like a click on the selection.
            if self.center_mark_at(screen) && !mods.shift && !mods.alt {
                self.toggle_rotate_mode();
                return;
            }
            if (self.guide_rotate.is_some() || self.hit_test(p).is_none())
                && self.click_guide(p, mods.shift)
            {
                return;
            }
            self.deselect_guides();
            // Alt digs: the object beneath the selected one there, or,
            // with none of the selection under the pointer, beneath the
            // topmost one (the hidden object behind the one clicked,
            // #1350); after the bottom one, the top again (with Ctrl
            // among the objects inside groups).
            if mods.alt {
                let hits = self.hits_at(p, mods.ctrl);
                if !hits.is_empty() {
                    let from = match self.selection.as_slice() {
                        [one] => hits.iter().position(|h| h == one),
                        _ => None,
                    };
                    let id = hits[(from.unwrap_or(0) + 1) % hits.len()];
                    if mods.shift {
                        self.select_add(id);
                    } else {
                        self.select(vec![id]);
                    }
                    return;
                }
            }
            // Editing a clip frame's contents: a Ctrl+click off them
            // finishes (#1337).
            if mods.ctrl && !mods.shift && self.ctrl_click_finishes_clip_edit(p) {
                self.finish_clip_frame_edit();
                return;
            }
            // Ctrl+click on a clip frame: editing its contents (#982).
            if mods.ctrl && !mods.shift && self.ctrl_click_edits_clip_frame(p) {
                return;
            }
            // Ctrl: one group level further in.
            let hit = if mods.ctrl {
                self.hit_test_step(p)
            } else {
                self.pick_at(p)
            };
            match hit {
                Some(id) if mods.shift => {
                    if let Some(i) = self.selection.iter().position(|s| *s == id) {
                        self.selection.remove(i);
                        self.rotate_mode = false;
                    } else {
                        self.select_add(id);
                    }
                }
                Some(id) if !mods.ctrl && self.hit_test(p) != Some(id) => {
                    // On the drop shadow, away from the object: the shadow
                    // alone (#2545).
                    self.pick_shadow(id);
                }
                Some(id) => {
                    if self.picked_shadow() == Some(id) {
                        // The object of the chosen shadow: the object.
                        self.select(vec![id]);
                    } else if self.selection.contains(&id) {
                        // A click on what is selected: toggle the rotate and
                        // skew handles.
                        self.toggle_rotate_mode();
                    } else {
                        self.select(vec![id]);
                    }
                }
                None => self.select(Vec::new()),
            }
        }
        if response.double_clicked_by(PointerButton::Primary) {
            // The centre of rotation goes back to the middle.
            if self.pivot_at(screen) {
                self.reset_pivot();
                return;
            }
            // A drop shadow (away from its object, or over one it draws
            // above): the Drop Shadow tool on its object.
            if let Some(id) = self.pick_at(p).filter(|id| Some(*id) != self.hit_test(p)) {
                self.select(vec![id]);
                self.edit_shadow();
                return;
            }
            // The page border or its shadow: the page size options.
            let on_frame = response
                .interact_pointer_pos()
                .is_some_and(|s| crate::canvas::on_page_frame(self, s));
            if on_frame && self.hit_test(p).is_none() {
                self.open_page_options();
                return;
            }
            // A guideline where no object is: the Guidelines panel on it.
            if self.hit_test(p).is_none() {
                if let Some(i) = self.pick_guide_at(p) {
                    self.open_guide_properties(i);
                    return;
                }
            }
            if let Some(id) = self.hit_test(p) {
                // An object with a symmetry (or inside one): editing the
                // symmetry, as in the reference program.
                if self.symmetry_edit.is_none() {
                    let top = self.doc().top_level_of(id).unwrap_or(id);
                    let has = self.doc().find_shape(top).is_some_and(|s| {
                        s.effects
                            .iter()
                            .any(|e| matches!(e, tracedraw_core::live::Effect::Symmetry { .. }))
                    });
                    if has {
                        self.select(vec![top]);
                        self.edit_symmetry();
                        return;
                    }
                }
                // A dimension's label: editing it in place (#644).
                if self.edit_dimension_label_at(p) {
                    return;
                }
                // A symbol instance: editing its symbol's objects in
                // place (#1980).
                if self.clip_frame_edit.is_none()
                    && self
                        .doc()
                        .find_shape(id)
                        .is_some_and(|s| matches!(s.kind, ShapeKind::SymbolInstance { .. }))
                {
                    self.edit_symbol_instance(id);
                    return;
                }
                // A group or clip frame: into focus mode.
                if self.focus_on_double_click(id) {
                    self.focus_on(id);
                    // The part under the pointer comes selected (#2157):
                    // the double click left nothing selected, as if it
                    // had only deselected the object.
                    if let Some(part) = self.hit_test(p).filter(|part| *part != id) {
                        self.select(vec![part]);
                    }
                    return;
                }
                if let Some(s) = self.doc().find_shape(id).cloned() {
                    if matches!(s.kind, ShapeKind::Text { .. }) {
                        // Straight into editing that text, caret at the end.
                        self.select(vec![id]);
                        self.edit_selected_text();
                    } else {
                        self.select(vec![id]);
                        self.set_tool(Tool::Shape);
                    }
                }
            }
        }
    }

    /// A Pick tool drag goes on with the pointer at `p` (page) and the
    /// modifier keys `mods`.
    /// W and K held while the selection is moved: what a drop on a clip
    /// frame does (they are not tool shortcuts then).
    fn note_drop_keys(&mut self, ctx: &egui::Context) {
        if matches!(self.drag, Drag::Move { .. }) {
            // Alt adds to the frame as W does (#1337).
            self.drop_keys =
                ctx.input(|i| (i.key_down(Key::W) || i.modifiers.alt, i.key_down(Key::K)));
            // Alt copies the fill onto the object the selection is let go
            // over, Alt+Shift its outline (discussion #2066).
            let m = ctx.input(|i| i.modifiers);
            self.drop_colour = match (m.alt, m.shift) {
                (true, true) => Some(crate::drop_menu::DropAction::CopyOutline),
                (true, false) => Some(crate::drop_menu::DropAction::CopyFill),
                _ => None,
            };
        }
    }

    fn pick_drag_to(&mut self, p: Point, mods: Modifiers) {
        if matches!(self.drag, Drag::SymmetryLines { .. }) {
            self.symmetry_drag_to(p, mods.ctrl || mods.command);
            return;
        }
        if matches!(
            self.drag,
            Drag::MoveGuide { .. } | Drag::RotateGuide { .. } | Drag::GuidePivot
        ) {
            self.drag_guide_to(p);
            return;
        }
        // Once the pointer has left the snapping radius of where the move
        // began, the selection's start geometry attracts it (#1765); not
        // before, or every move begun on a node would stick to it.
        let away = match &self.drag {
            Drag::Move {
                start,
                start_geometry: None,
                ..
            } => {
                self.snap_enabled()
                    && self.snap.objects
                    && (p - *start).hypot() > self.snap_tolerance()
            }
            _ => false,
        };
        if away {
            let g = std::rc::Rc::new(self.selection_start_geometry());
            if let Drag::Move { start_geometry, .. } = &mut self.drag {
                *start_geometry = Some(g);
            }
        }
        let snapped_move = match &self.drag {
            Drag::Move {
                start,
                start_bounds,
                source,
                start_geometry,
                ..
            } => {
                // The pointer's way since the press, snapped; Ctrl (Cmd
                // on macOS) or Shift keeps it horizontal or vertical,
                // whichever is longer (#2480: Shift snapped it to objects
                // instead). Alt+Shift copies an outline on the drop, so
                // Shift with Alt does not.
                let mut d = p - *start;
                let straight = mods.ctrl || mods.command || (mods.shift && !mods.alt);
                let axis = straight.then_some(d.x.abs() >= d.y.abs());
                match axis {
                    Some(true) => d.y = 0.0,
                    Some(false) => d.x = 0.0,
                    None => {}
                }
                let mut t =
                    self.snap_move_from(*source, *start_bounds, d, start_geometry.as_deref());
                match axis {
                    Some(true) => t.y = 0.0,
                    Some(false) => t.x = 0.0,
                    None => {}
                }
                Some((p, t))
            }
            _ => None,
        };
        // Handles snap to other objects, guidelines, the grid and the
        // page, never to the selection's own points: the point of the box
        // a scale or skew handle stands for. The pointer turning the
        // selection (unless Ctrl turns it in steps) snaps to points only
        // (#933, see `snap_turn_point`).
        // A scale handle's moving side also stops on the points and lines
        // of other objects it meets, and a corner keeping the proportions
        // follows the axis that snapped (#2004).
        let mut scale_snapped = (false, false);
        let snapped_handle = match &self.drag {
            Drag::Scale { grab, .. } => {
                let (q, axes) = self.snap_scale_handle(p - *grab);
                scale_snapped = axes;
                q
            }
            Drag::Skew {
                handle,
                start_bounds,
                start,
                ..
            } => {
                let grab = *start - handle_point(*start_bounds, *handle);
                self.snap_point_excluding(p - grab, &self.selection) + grab
            }
            Drag::Rotate {
                center,
                grab: Some(g),
                ..
            } if !mods.ctrl => self.snap_turn_grab(*center, *g, p, &self.selection),
            Drag::Rotate { .. } if !mods.ctrl => self.snap_turn_point(p, &self.selection),
            _ => p,
        };
        match &mut self.drag {
            Drag::AlongPath { current, .. } => *current = p,
            Drag::Move { last, total, .. } => {
                if let Some((np, nt)) = snapped_move {
                    *last = np;
                    *total = nt;
                }
                // Over a clip frame that is not being moved: a drop
                // target (W adds to it, K replaces its contents).
                self.drop_frame = self.frame_under_pointer(p);
                // With Alt (Alt+Shift) held: the object that would take
                // the fill (outline), looked for only then.
                self.drop_colour_target = if self.drop_colour.is_some() {
                    let moving = self.selection.clone();
                    self.hit_test_where(p, |id| moving.contains(&id))
                } else {
                    None
                };
            }
            Drag::Marquee {
                start,
                current,
                touching,
                ..
            } => {
                // Ctrl keeps the marquee square.
                let mut q = p;
                if mods.ctrl || mods.command {
                    let d = p - *start;
                    let side = d.x.abs().max(d.y.abs());
                    q = *start + Vec2::new(side * d.x.signum(), side * d.y.signum());
                }
                *current = q;
                *touching = mods.alt != self.settings.marquee_touching;
            }
            Drag::Lasso {
                points, touching, ..
            } => {
                // Ctrl keeps the freehand selection a rectangle.
                if (mods.ctrl || mods.command) && !points.is_empty() {
                    let s = points[0];
                    *points = vec![s, Point::new(p.x, s.y), p, Point::new(s.x, p.y)];
                } else {
                    points.push(p);
                }
                *touching = mods.alt != self.settings.marquee_touching;
            }
            Drag::Scale {
                current,
                mods: m,
                snapped,
                ..
            } => {
                // The handle snaps to other objects, guidelines, the grid
                // and the page, never to the selection's own points.
                *current = snapped_handle;
                *snapped = scale_snapped;
                *m = crate::app::HandleMods::of(mods);
            }
            Drag::Skew {
                current, mods: m, ..
            } => {
                *current = snapped_handle;
                *m = crate::app::HandleMods::of(mods);
            }
            Drag::Pivot => {
                // It snaps: a corner of the selection, a node, the grid.
                let q = self.snap_pivot(p);
                self.set_pivot(q);
            }
            Drag::Rotate {
                center,
                start_angle,
                current_angle,
                ..
            } => {
                let mut a = (snapped_handle - *center).atan2();
                // Ctrl turns in steps of the constrain angle.
                if mods.ctrl {
                    let step = self.settings.constrain_angle.max(0.1).to_radians();
                    a = *start_angle + ((a - *start_angle) / step).round() * step;
                }
                *current_angle = a;
            }
            _ => {}
        }
    }

    fn shape_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        // On a table a click chooses the cell under the pointer, with
        // Ctrl one more or less and with Shift the block from the active
        // cell, as the Table tool chooses them (which edits the text on
        // a plain click); a click anywhere else lets the cells go
        // (#2436).
        if response.clicked_by(PointerButton::Primary) {
            use crate::table_tool::CellPick;
            let how = if mods.command || mods.ctrl {
                CellPick::Toggle
            } else if mods.shift {
                CellPick::Range
            } else {
                CellPick::One
            };
            if self.pick_table_cell(p, how) {
                return;
            }
            self.table_edit = None;
        }
        self.shape_tool_input(response, p, mods);
    }

    /// The table under page point `p`, if any.
    fn table_at(&self, p: Point) -> Option<ShapeId> {
        let id = self.hit_test(p)?;
        let (_, s) = self.doc().shape(id).ok()?;
        matches!(s.kind, ShapeKind::Table(_)).then_some(id)
    }

    fn text_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        // A frame's flow tabs link frames with the Text tool as with the
        // Pick tool: a click on one, then a click where the text flows,
        // or a drag from it there (#1914).
        if response.clicked_by(PointerButton::Primary)
            || response.drag_started_by(PointerButton::Primary)
        {
            let screen = self.view.to_screen(p);
            let from_tab = response.drag_started_by(PointerButton::Primary);
            if self.press_flow_tab(screen, from_tab) {
                return;
            }
        }
        // Inside the text being edited, the pointer places the caret and
        // drags a selection; elsewhere it starts a new text or edits
        // another one.
        // Looked up only when the button acts (a dimension's label is
        // inside its group, #644).
        let acts = response.drag_started_by(PointerButton::Primary)
            || response.clicked_by(PointerButton::Primary)
            || response.double_clicked_by(PointerButton::Primary);
        // On a table the Text tool edits the text of the cell under the
        // pointer, as the Table tool does (#2438).
        if acts && !response.double_clicked_by(PointerButton::Primary) {
            if let Some(id) = self.table_at(p) {
                if self.text_edit.is_some() {
                    self.finish_text();
                }
                let tf = self
                    .doc()
                    .shape(id)
                    .map(|(_, s)| s.transform)
                    .unwrap_or(Affine::IDENTITY);
                let local = tf.inverse() * p;
                let extend = mods.shift && self.table_edit.as_ref().is_some_and(|e| e.shape == id);
                if extend {
                    self.table_range_to(id, local);
                } else {
                    self.table_click(id, local, false);
                }
                self.select(vec![id]);
                self.drag = Drag::None;
                return;
            }
            // Anywhere else the cell being edited lets go.
            self.table_edit = None;
        }
        let on_edited = acts
            && self
                .text_edit
                .as_ref()
                .map(|te| te.shape)
                .filter(|id| {
                    self.hit_test(p) == Some(*id) || self.dimension_label_at(p) == Some(*id)
                })
                .is_some();
        if response.drag_started_by(PointerButton::Primary) {
            if on_edited {
                if let Some(idx) = self.text_hit_char(p) {
                    self.text_set_caret_at(idx, mods.shift);
                }
                self.drag = Drag::TextSelect;
            } else {
                if self.text_edit.is_some() {
                    self.finish_text();
                }
                self.drag = Drag::TextFrame {
                    start: p,
                    current: p,
                };
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            match &mut self.drag {
                Drag::TextFrame { current, .. } => *current = p,
                Drag::TextSelect => {
                    if let Some(idx) = self.text_hit_char(p) {
                        self.text_set_caret_at(idx, true);
                    }
                }
                _ => {}
            }
        }
        if response.double_clicked_by(PointerButton::Primary) && on_edited {
            if let Some(idx) = self.text_hit_char(p) {
                self.text_select_word_at(idx);
            }
            return;
        }
        if response.clicked_by(PointerButton::Primary) {
            if on_edited {
                if let Some(idx) = self.text_hit_char(p) {
                    self.text_set_caret_at(idx, mods.shift);
                }
                return;
            }
            // A dimension's label is edited in place (#644).
            if self.edit_dimension_label_at(p) {
                return;
            }
            match self.hit_test(p) {
                Some(id)
                    if matches!(
                        self.doc()
                            .shape(id)
                            .map(|(_, s)| matches!(s.kind, ShapeKind::Text { .. })),
                        Ok(true)
                    ) =>
                {
                    self.begin_text_edit(id);
                    if let Some(idx) = self.text_hit_char(p) {
                        self.text_set_caret_at(idx, false);
                    }
                }
                // On a path's outline the text starts on the path.
                _ if self.start_text_on_path_at(p) => {}
                _ => {
                    let p = self.snap_point(p);
                    self.start_text(p, None);
                }
            }
        }
    }

    fn zoom_input(&mut self, response: &Response, p: Point, screen: egui::Pos2, mods: Modifiers) {
        if response.drag_started_by(PointerButton::Primary) {
            self.drag = Drag::ZoomBox {
                start: p,
                current: p,
            };
        }
        if response.dragged_by(PointerButton::Primary) {
            if let Drag::ZoomBox { current, .. } = &mut self.drag {
                *current = p;
            }
        }
        if response.clicked_by(PointerButton::Primary) {
            self.view
                .zoom_at(screen, if mods.shift { 0.5 } else { 2.0 });
            self.end_zoom_one_shot();
        }
    }

    /// After a zoom with the Zoom tool picked by F2, the tool before it
    /// comes back.
    fn end_zoom_one_shot(&mut self) {
        if self.zoom_one_shot && self.tool == Tool::Zoom {
            let back = match self.previous_tool {
                Tool::Zoom => Tool::Pick,
                t => t,
            };
            if self.text_edit.is_some() {
                // F2 while editing a text (#1699): the editing goes on.
                self.zoom_one_shot = false;
                self.previous_tool = Tool::Zoom;
                self.tool = back;
            } else {
                self.set_tool(back);
            }
        }
    }

    fn box_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        let p = self.snap_point(p);
        let mut boxing = matches!(self.drag, Drag::Box { .. });
        // A middle press while the left button is held starts a new egui
        // drag too; that is a pan, not a new shape (#1923).
        if response.drag_started_by(PointerButton::Primary) && !boxing {
            // A new shape begins: what was selected lets go at once.
            self.select(Vec::new());
            self.drag = Drag::Box {
                start: p,
                current: p,
                from_center: mods.shift,
            };
            self.box_regular = false;
            boxing = true;
        }
        if !boxing {
            return;
        }
        // The shape follows the pointer while the left button is down and
        // is made when it goes up, read from the button itself: after a
        // middle-button pan egui no longer counts the left button's drag,
        // and would never report its end (#1923).
        if response.ctx.input(|i| i.pointer.primary_down()) {
            // The pointer now: on the frame the drag starts `p` is where
            // the button went down, the pointer already a few pixels away,
            // and the corner sat behind it (#655).
            let now = response
                .hover_pos()
                .map(|s| self.snap_point(self.view.to_page(s)))
                .unwrap_or(p);
            self.box_follow(now, mods);
        } else if !response.drag_stopped_by(PointerButton::Primary) {
            // (A drag egui still counts ends below, as every tool's does.)
            self.end_drag();
        }
        // A page frame comes from double-clicking the tool's toolbox
        // button, not from a double click on the drawing.
    }

    /// The far corner of the shape being drawn moves to `now`.
    fn box_follow(&mut self, now: Point, mods: Modifiers) {
        self.box_regular = mods.ctrl;
        if let Drag::Box {
            start,
            current,
            from_center,
        } = &mut self.drag
        {
            let mut q = now;
            if mods.ctrl {
                // Constrain to a square/circle.
                let d = q - *start;
                let m = d.x.abs().max(d.y.abs());
                q = *start + Vec2::new(m * d.x.signum(), m * d.y.signum());
            }
            // Shift draws from the centre (checked while dragging).
            *from_center = mods.shift;
            *current = q;
        }
    }

    /// 3-point rectangle, ellipse and curve: drag the base segment, then
    /// click the third point (Esc cancels, see `keyboard`). Ctrl keeps the
    /// base to steps of the constrain angle from where it starts, as the
    /// other line tools do (the curve's chord: the constrain key of the
    /// curve tools, Ctrl by default; #934). Held at the third click of the
    /// curve, it makes the curve a bow: the arc of the circle through the
    /// three points.
    fn three_point_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        let p = self.snap_point(p);
        let constrain = if self.tool == Tool::ThreePointCurve {
            self.constrain_held(&mods)
        } else {
            mods.ctrl || mods.command
        };
        let step = self.settings.constrain_angle;
        if self.three_point_base.is_some() {
            if response.clicked_by(PointerButton::Primary) {
                if let Some((a, b)) = self.three_point_base.take() {
                    self.finish_three_point(a, b, p, constrain);
                }
            }
            return;
        }
        if response.drag_started_by(PointerButton::Primary) {
            // A chord starting on an end node of the selected curve goes
            // on from it (the curve stays selected for the join); else
            // the selection is dropped, as a new object is about to be
            // drawn.
            if !(self.tool == Tool::ThreePointCurve && self.on_join_end(p)) {
                self.select(Vec::new());
            }
            self.drag = Drag::ThreePointBase {
                start: p,
                current: p,
            };
            self.three_point_circle = false;
        }
        if response.dragged_by(PointerButton::Primary) {
            // With Ctrl the 3-Point Ellipse draws a circle round the
            // press point, the line dragged its radius, and its turn in
            // steps of the constrain angle (#2550).
            let circle = self.tool == Tool::ThreePointEllipse && constrain;
            if let Drag::ThreePointBase { start, current } = &mut self.drag {
                *current = if circle {
                    let dir = constrain_from(*start, p, step) - *start;
                    let len = dir.hypot();
                    if len > 1e-9 {
                        *start + dir * ((p - *start).hypot() / len)
                    } else {
                        p
                    }
                } else if constrain {
                    constrain_from(*start, p, step)
                } else {
                    p
                };
                self.three_point_circle = circle;
            }
        }
    }

    /// A Freehand line of clicks ends once the double-click time after
    /// its last click has passed with no double-click (which makes that
    /// node a corner and goes on) and no button held (#2339). It is
    /// dropped when the line or the tool went another way meanwhile.
    fn freehand_end_tick(&mut self, ctx: &egui::Context) {
        let Some(at) = self.freehand_ends_at else {
            return;
        };
        if self.curve.is_none() || self.tool != Tool::Freehand {
            self.freehand_ends_at = None;
            return;
        }
        let (now, held) = ctx.input(|i| (i.time, i.pointer.primary_down()));
        if now >= at && !held {
            self.freehand_ends_at = None;
            self.finish_curve();
        } else {
            ctx.request_repaint_after_secs((at - now).max(0.0) as f32 + 0.01);
        }
    }

    fn freehand_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        // Resting before the first click: the pointer shows the point the
        // line would start at, as it does for the next node once the line
        // has begun (the preview snaps it then) (#1538).
        if self.tool == Tool::Freehand
            && self.curve.is_none()
            && matches!(self.drag, Drag::None)
            && response.hovered()
        {
            let _ = self.snap_point(p);
        }
        // A click, then another: a straight line between them (Ctrl keeps
        // it to steps of the constrain angle); Esc drops the first click.
        // As in CorelDRAW, a double-click instead of the second click
        // makes a corner and the line goes on to the next click; a single
        // click ends it (#2339). The line ends once the double-click time
        // has passed (see `freehand_end_tick`).
        // (egui counts the pair as a triple click when the click before
        // it came less than two double-click times earlier.)
        let double = response.double_clicked_by(PointerButton::Primary)
            || response.triple_clicked_by(PointerButton::Primary);
        if double && self.tool == Tool::Freehand {
            // The first click of the pair placed the node: it stays.
            self.freehand_ends_at = None;
            return;
        }
        if response.clicked_by(PointerButton::Primary) && self.tool == Tool::Freehand {
            // A click while the line waits to end: it ends, and this
            // click starts the next one.
            if self.freehand_ends_at.take().is_some() {
                self.finish_curve();
            }
            let p = self.curve_point(self.snap_point(p), self.constrain_held(&mods));
            let c = self.curve.get_or_insert_with(|| CurveInProgress {
                nodes: Vec::new(),
                smooth: false,
                dragging_handle: false,
                bspline: false,
                ..Default::default()
            });
            c.nodes.push((p, None));
            if c.nodes.len() >= 2 {
                let now = response.ctx.input(|i| i.time);
                let delay = response
                    .ctx
                    .options(|o| o.input_options.max_double_click_delay);
                self.freehand_ends_at = Some(now + delay);
                response.ctx.request_repaint_after_secs(delay as f32);
            }
            return;
        }
        if response.drag_started_by(PointerButton::Primary) {
            // A line of clicks still waiting to end ends before the drag.
            if self.freehand_ends_at.take().is_some() {
                self.finish_curve();
            }
            self.curve = None;
            self.drag = Drag::Freehand { points: vec![p] };
            self.pen_stroke = crate::pen::PenStroke::start(self.pen_pressure);
        }
        if response.dragged_by(PointerButton::Primary) {
            if let Drag::Freehand { points } = &mut self.drag {
                if points
                    .last()
                    .map(|l| (*l - p).hypot() > 0.2)
                    .unwrap_or(true)
                {
                    points.push(p);
                    let n = points.len();
                    self.pen_stroke.record(n, self.pen_pressure);
                }
            }
        }
    }

    /// The pressures a pen recorded along the freehand drag of `points`,
    /// when it had one at every point and Options > Tools > Curve tools
    /// lets the pressure set the width (#850).
    pub fn pen_pressures(&self, points: &[Point]) -> Option<&[f32]> {
        if !self.settings.curves.pen_pressure {
            return None;
        }
        self.pen_stroke.pressures(points.len())
    }

    /// Bezier/Pen: click places a node without a handle (the segment to
    /// it follows the handle of the node before and arrives straight),
    /// click-and-drag pulls out a smooth handle (C while
    /// pulling makes the node a cusp: only the outgoing handle follows;
    /// Alt moves the node). Clicking the last node again makes it a cusp.
    /// Polyline and 2-point line place straight segments. Double-click or
    /// Enter finishes; a press on the first node closes the curve, and
    /// dragging from it shapes the closing segment only.
    fn curve_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        // With no curve being drawn, the Pen and Bezier tools edit the
        // selected curve: a click on a segment adds a node, a node is
        // dragged like with the Shape tool.
        if self.pen_edit_input(response, p, mods) {
            return;
        }
        let raw = p;
        let p = self.snap_point(p);
        // Ctrl: the new node in steps of the constrain angle from the last
        // one (a dragged handle, from its node).
        let handle_from = self
            .curve
            .as_ref()
            .filter(|c| c.dragging_handle)
            .and_then(|c| c.nodes.last().map(|n| n.0));
        let constrain = self.constrain_held(&mods);
        let p = match handle_from {
            Some(node) if constrain => constrain_from(node, p, self.settings.constrain_angle),
            Some(_) => p,
            None => self.curve_point(p, constrain),
        };
        // B-Spline: each click (or the start of a drag) places a control
        // point; there are no handles to pull.
        let bspline = self.tool == Tool::BSpline;
        let smooth = matches!(self.tool, Tool::Bezier | Tool::Pen);
        if response.double_clicked_by(PointerButton::Primary) {
            self.finish_curve();
            return;
        }
        // Every press that changes the curve is a step Ctrl+Z takes back.
        if response.clicked_by(PointerButton::Primary)
            || response.drag_started_by(PointerButton::Primary)
        {
            self.curve_step();
        }
        let reach = self.curve_end_reach_px() / self.view.zoom as f64;
        let near = |q: Point| (q - raw).hypot() <= reach;
        let start_close = self
            .curve
            .as_ref()
            .is_some_and(|c| c.closes_at_first() && c.nodes.first().is_some_and(|n| near(n.0)));
        // The last node, or the one a curve going on from an end starts
        // at with that end's mirrored handle (#1138).
        let on_last = smooth
            && self.curve.as_ref().is_some_and(|c| {
                c.nodes
                    .last()
                    .is_some_and(|n| near(n.0) && (c.nodes.len() >= 2 || n.1.is_some()))
            });
        if response.clicked_by(PointerButton::Primary) {
            // A line going on from an end of the selected curve: a click
            // on that curve's other end closes it (#631).
            if let Some(end) = self.continuation_close_at(raw, reach) {
                if let Some(c) = &mut self.curve {
                    c.nodes.push((end, None));
                }
                self.finish_curve_as(true);
                return;
            }
            if start_close {
                // A click on the first node closes the curve with the
                // segment the preview showed: it leaves the last node
                // along its handle and arrives at the first node with no
                // handle (#359).
                if let Some(mut c) = self.curve.take() {
                    c.closing = None;
                    self.close_curve(c);
                }
                return;
            }
            if on_last {
                // Clicking the last node again makes it a cusp.
                if let Some(c) = &mut self.curve {
                    c.cusp_last();
                    c.dragging_handle = false;
                }
                return;
            }
            // While a curve is drawn the Pen's automatic adding and
            // deleting is off: a click on a node or a segment, of this
            // curve or another, places the next node there (#1143).
        }
        if response.drag_started_by(PointerButton::Primary) && start_close {
            // Dragging from the first node shapes the closing segment.
            if let Some(c) = &mut self.curve {
                c.closing = Some(raw);
                c.dragging_handle = false;
            }
            return;
        }
        // The Pen or Bezier tool's first press on an end node of an open
        // curve (selected or not) goes on from it as if it had never
        // stopped: the curve is selected, so the new line joins it, and
        // the next segment leaves along the mirror of the end's handle,
        // curving the preview (#630). A drag pulls a new handle instead.
        if smooth
            && self.curve.is_none()
            && (response.drag_started_by(PointerButton::Primary)
                || response.clicked_by(PointerButton::Primary))
        {
            if let Some((id, end, handle)) = self.open_end_at(raw) {
                if self.selection != [id] {
                    self.select(vec![id]);
                }
                self.curve = Some(CurveInProgress {
                    nodes: vec![(end, handle)],
                    smooth,
                    dragging_handle: response.drag_started_by(PointerButton::Primary),
                    ..Default::default()
                });
                return;
            }
        }
        if response.drag_started_by(PointerButton::Primary)
            || response.clicked_by(PointerButton::Primary)
        {
            let c = self.curve.get_or_insert_with(|| CurveInProgress {
                nodes: Vec::new(),
                smooth,
                dragging_handle: false,
                bspline,
                ..Default::default()
            });
            c.smooth = smooth;
            c.bspline = bspline;
            let drag = response.drag_started_by(PointerButton::Primary);
            // A click places a node without a handle: the segment to it
            // is the one the preview showed, shaped by the handle of the
            // node before (straight when that node has none), arriving
            // straight at the new node (#314).
            // A B-Spline point on the last one is not placed twice: the
            // curve would bend at its end where the preview showed none.
            let again = bspline && c.on_last_control(p);
            if (drag || !c.dragging_handle) && !again {
                c.nodes.push((p, None));
                // C or V held: a fixed point, which the curve passes
                // through with a corner (#2060).
                if bspline {
                    let fixed = response
                        .ctx
                        .input(|i| i.key_down(egui::Key::C) || i.key_down(egui::Key::V));
                    c.spline_fixed.resize(c.nodes.len() - 1, false);
                    c.spline_fixed.push(fixed);
                }
            } else if again && bspline {
                // C or V on the last point again makes it fixed.
                if response
                    .ctx
                    .input(|i| i.key_down(egui::Key::C) || i.key_down(egui::Key::V))
                {
                    c.spline_fixed.resize(c.nodes.len(), false);
                    if let Some(f) = c.spline_fixed.last_mut() {
                        *f = true;
                    }
                }
            }
            c.dragging_handle = drag;
            c.alt_from = None;
            if self.tool == Tool::TwoPointLine && c.nodes.len() == 2 {
                self.finish_curve();
            }
        }
        // The closing drag's handle, from the first node, kept to the
        // constrain angle like any other handle (#473).
        let closing_at = |c: &CurveInProgress, step: f64| match c.nodes.first() {
            Some(first) if constrain => constrain_from(first.0, raw, step),
            _ => raw,
        };
        let step = self.settings.constrain_angle;
        if response.dragged_by(PointerButton::Primary) {
            // C held (pressed before or during the pull): a cusp; it stays
            // one after C is let go.
            let c_pressed = response.ctx.input(|i| i.key_down(egui::Key::C));
            // Polyline: a drag draws freehand, a node every few pixels
            // along the pointer's way, as a click places a straight
            // segment (#2074).
            let polyline = self.tool == Tool::Polyline;
            let spacing = 2.0 / self.view.zoom.max(1e-6) as f64;
            if let Some(c) = &mut self.curve {
                if polyline && c.dragging_handle && c.closing.is_none() {
                    if c.nodes.last().is_none_or(|n| (n.0 - p).hypot() >= spacing) {
                        c.nodes.push((p, None));
                    }
                } else if c.closing.is_some() {
                    c.closing = Some(closing_at(c, step));
                } else if c.dragging_handle && smooth {
                    if c_pressed {
                        c.freeze_last_in();
                    }
                    if mods.alt {
                        // Alt: the node moves with its handles.
                        let from = c.alt_from.unwrap_or(p);
                        let d = p - from;
                        let i = c.nodes.len() - 1;
                        if let Some(last) = c.nodes.last_mut() {
                            last.0 += d;
                            last.1 = last.1.map(|h| h + d);
                        }
                        if let Some(Some(h)) = c.in_handles.get_mut(&i) {
                            *h += d;
                        }
                        c.alt_from = Some(p);
                    } else {
                        c.alt_from = None;
                        if let Some(last) = c.nodes.last_mut() {
                            last.1 = Some(p);
                        }
                    }
                }
            }
        }
        if response.drag_stopped_by(PointerButton::Primary) {
            if self.curve.as_ref().is_some_and(|c| c.closing.is_some()) {
                if let Some(mut c) = self.curve.take() {
                    c.closing = Some(closing_at(&c, step));
                    self.close_curve(c);
                }
                return;
            }
            // 2-point line: a drag ends the line where it is let go. It
            // waited for another press before, so on a touch screen,
            // with no pointer to follow after the finger left, the
            // second node landed wherever the next tap was (#2594).
            if self.tool == Tool::TwoPointLine
                && self
                    .curve
                    .as_ref()
                    .is_some_and(|c| c.dragging_handle && c.nodes.len() == 1)
            {
                let far = self
                    .curve
                    .as_ref()
                    .and_then(|c| c.nodes.first())
                    .is_some_and(|n| (n.0 - p).hypot() * self.view.zoom as f64 > 1.0);
                if far {
                    if let Some(c) = &mut self.curve {
                        c.nodes.push((p, None));
                    }
                    self.finish_curve();
                    return;
                }
            }
            if let Some(c) = &mut self.curve {
                c.dragging_handle = false;
                c.alt_from = None;
            }
        }
    }

    /// A handle of the curve being drawn is pulled (or its closing
    /// segment shaped) by the drawing window's own primary drag, this
    /// frame or in the release that ends it.
    fn curve_drag_running(&self, response: &Response) -> bool {
        self.curve
            .as_ref()
            .is_some_and(|c| c.dragging_handle || c.closing.is_some())
            && (response.dragged_by(PointerButton::Primary)
                || response.drag_stopped_by(PointerButton::Primary))
    }

    /// The button went up with the pointer nowhere over the drawing (out
    /// of the window): a handle pull of the curve being drawn ends where
    /// it was last seen, and a closing drag closes the curve. Otherwise
    /// the curve waited for a release it never saw, the preview stopped
    /// following the pointer and the next click only ended the pull
    /// instead of placing a node (#1014).
    pub(crate) fn end_curve_drag_outside(&mut self) {
        if self.curve.as_ref().is_some_and(|c| c.closing.is_some()) {
            if let Some(c) = self.curve.take() {
                self.close_curve(c);
            }
            return;
        }
        if let Some(c) = &mut self.curve {
            c.dragging_handle = false;
            c.alt_from = None;
        }
    }

    /// The curve being drawn, closed onto its first node, becomes a
    /// curve object and is selected.
    fn close_curve(&mut self, c: CurveInProgress) {
        let history = std::mem::take(&mut self.curve_steps.undo);
        self.curve_steps.clear();
        let path = c.path_closed(None, true);
        // The anchors stay in the history, the closing last (#360).
        let prior = if c.bspline {
            Vec::new()
        } else {
            Self::anchor_paths(&history, Some(&c))
        };
        if let Some(id) = self.new_curve_in_steps(
            prior,
            c.nodes.first().copied(),
            None,
            ShapeKind::Path {
                path: path.clone(),
                closed: true,
            },
        ) {
            if c.bspline {
                let points = c.nodes.iter().map(|n| n.0).collect();
                self.keep_spline(id, &path, points, c.spline_fixed_flags());
            }
            self.select(vec![id]);
        }
    }

    /// The Pen and Bezier tools with no curve being drawn and one curve
    /// selected: a click on one of its segments adds a node there, and a
    /// press on one of its nodes (not an end node, which continues the
    /// curve) drags it as the Shape tool does. True when it took the
    /// input.
    fn pen_edit_input(&mut self, response: &Response, p: Point, mods: Modifiers) -> bool {
        if self.pen_editing {
            self.shape_input(response, p, mods);
            // The drag ends with the button (`canvas_input` ends it).
            if response.drag_stopped() || !response.ctx.input(|i| i.pointer.primary_down()) {
                self.pen_editing = false;
            }
            return true;
        }
        if !matches!(self.tool, Tool::Bezier | Tool::Pen) || self.curve.is_some() {
            return false;
        }
        match self.pen_edit_at(p) {
            Some(PenEdit::Node(..)) if response.drag_started_by(PointerButton::Primary) => {
                self.pen_editing = true;
                self.shape_input(response, p, mods);
                true
            }
            Some(PenEdit::Node(id, index)) if response.clicked_by(PointerButton::Primary) => {
                if self.pen_deletes_nodes() {
                    self.pen_delete_node(id, index);
                }
                true
            }
            Some(PenEdit::Segment(id, index, t)) if response.clicked_by(PointerButton::Primary) => {
                if let Some(ShapeKind::Path { path, closed }) =
                    self.doc().find_shape(id).map(|s| s.kind.clone())
                {
                    let np = tracedraw_core::nodes::add_node(&path, index, t);
                    if self
                        .engine
                        .run_with_label(
                            &Command::SetShapeKind {
                                shape: id,
                                kind: ShapeKind::Path { path: np, closed },
                            },
                            "Add Node",
                        )
                        .is_ok()
                    {
                        self.node_selection = vec![(id, index)];
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// What a Pen or Bezier press at `p` would edit on the selected curve
    /// when no curve is being drawn: a node (not an end node of an open
    /// curve, where a new line continues it) or a segment.
    pub(crate) fn pen_edit_at(&self, p: Point) -> Option<PenEdit> {
        if !matches!(self.tool, Tool::Bezier | Tool::Pen) || self.curve.is_some() {
            return None;
        }
        let [id] = self.selection.as_slice() else {
            return None;
        };
        let shape = self.doc().shape_on_page(*id)?;
        let ShapeKind::Path { path, .. } = &shape.kind else {
            return None;
        };
        if self.on_join_end(p) {
            return None;
        }
        let tol = self.settings.nodes.pick_px() / self.view.zoom as f64;
        let lp = shape.transform.inverse() * p;
        if let Some(n) = tracedraw_core::nodes::nodes(path)
            .iter()
            .map(|n| (n.index, (n.pos - lp).hypot()))
            .filter(|(_, d)| *d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            return Some(PenEdit::Node(*id, n.0));
        }
        // The Pen adds nodes only with automatic adding and deleting on;
        // the Bezier tool always does.
        if self.tool == Tool::Pen && !self.settings.curves.pen_auto_add_delete {
            return None;
        }
        let (index, t, d) = tracedraw_core::nodes::nearest_segment(path, lp)?;
        (d <= tol).then_some(PenEdit::Segment(*id, index, t))
    }

    /// The Pen with automatic adding and deleting on: a click on a node
    /// of the selected curve deletes it.
    pub(crate) fn pen_deletes_nodes(&self) -> bool {
        self.tool == Tool::Pen && self.settings.curves.pen_auto_add_delete
    }

    /// Delete node `index` of curve `id` (one undo step), keeping at least
    /// two nodes.
    fn pen_delete_node(&mut self, id: tracedraw_core::ShapeId, index: usize) {
        let Some(ShapeKind::Path { path, closed }) =
            self.doc().find_shape(id).map(|s| s.kind.clone())
        else {
            return;
        };
        if tracedraw_core::nodes::nodes(&path).len() <= 2 {
            return;
        }
        let np = tracedraw_core::nodes::delete_node(&path, index);
        if self
            .engine
            .run_with_label(
                &Command::SetShapeKind {
                    shape: id,
                    kind: ShapeKind::Path { path: np, closed },
                },
                "Delete Node",
            )
            .is_ok()
        {
            self.node_selection.clear();
        }
    }

    fn fill_input(&mut self, response: &Response, p: Point, mods: Modifiers) {
        // A mesh fill's nodes are picked and dragged here too, as with the
        // Mesh Fill and Shape tools; a palette colour then goes to the
        // chosen nodes, the fill staying a mesh (#2506).
        if self.selected_mesh().is_some() && self.effect_node_input(response, p) {
            return;
        }
        // A double click on a colour node removes it; on the line, it adds
        // one there.
        if response.double_clicked_by(PointerButton::Primary) {
            match self.fountain_handle_at(p) {
                Some(crate::fill_tool::FountainHandle::Stop(i)) => {
                    self.remove_fountain_stop(i);
                    return;
                }
                Some(_) => return,
                None => {
                    if let Some(t) = self.fountain_line_at(p) {
                        self.add_fountain_stop(t);
                        return;
                    }
                }
            }
        }
        if response.drag_started_by(PointerButton::Primary) {
            // A node, a slider, or the line (the whole fill moves).
            let press = response
                .ctx
                .input(|i| i.pointer.press_origin())
                .map(|o| self.view.to_page(o))
                .unwrap_or(p);
            if self.begin_fountain_drag(p, press) {
                return;
            }
            if let Some(id) = self.hit_test_inside(p) {
                self.select(vec![id]);
                // The fill's start snaps (#2004).
                let start = self.snap_point(p);
                self.drag = Drag::FillGradient {
                    shape: id,
                    start,
                    current: start,
                };
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            if self.fountain_drag_to(p, mods.ctrl || mods.command || mods.shift) {
                return;
            }
            self.fill_gradient_drag_to(p, mods);
        }
        if response.clicked_by(PointerButton::Primary) {
            // A click on a node picks it (the node bar opens under it).
            if let Some(h) = self.fountain_handle_at(p) {
                if let Some(i) = self.fountain_stop_of(h) {
                    self.fill_node = Some(i);
                }
                return;
            }
            self.fill_node = None;
            let under = self.hit_test_inside(p);
            // A click on the selected fill's line keeps it, unless another
            // object lies on top there: that one is picked (#2508).
            let other = under.is_some_and(|id| !self.selection.contains(&id));
            if self.fountain_line_at(p).is_some() && !other {
                return;
            }
            if let Some(id) = under {
                self.fill_tool_click(id);
            }
        }
    }

    /// The gradient line of an Interactive Fill or Transparency drag
    /// follows the pointer to `p`; with Ctrl (Cmd) or Shift it turns in
    /// steps of the constrain angle about where the drag began (#1661,
    /// #2373).
    fn fill_gradient_drag_to(&mut self, p: Point, mods: Modifiers) {
        let step = self.settings.constrain_angle;
        // The end of the new fill's line snaps as the pointer does
        // (#2004); kept to the constrain angle, it does not.
        let constrained = mods.ctrl || mods.command || mods.shift;
        let q = if constrained || !matches!(self.drag, Drag::FillGradient { .. }) {
            p
        } else {
            self.snap_point(p)
        };
        if let Drag::FillGradient { start, current, .. } = &mut self.drag {
            *current = if constrained {
                constrain_from(*start, p, step)
            } else {
                q
            };
        }
    }

    /// The object a Transparency drag starting at `p` puts its mask on,
    /// picking it when it was not selected: with a selection, the
    /// selection (pressed on a selected object, even where another one
    /// covers it, or on no object); an unselected object pressed outside
    /// the selected ones is picked alone; with nothing selected, the
    /// object under the press (#1661).
    fn transparency_drag_target(&mut self, p: Point) -> Option<ShapeId> {
        let on_selected = self.selection.iter().copied().find(|id| {
            self.doc()
                .shape_on_page(*id)
                .is_some_and(|s| s.bounds().contains(p))
        });
        if let Some(id) = on_selected {
            return Some(id);
        }
        match self.hit_test(p) {
            Some(id) => {
                self.select(vec![id]);
                Some(id)
            }
            None => self.selection.first().copied(),
        }
    }

    pub fn end_drag(&mut self) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        // The page shows the drawing as it is now (#1259).
        self.node_drag_shown = None;
        if !matches!(drag, Drag::Move { .. }) {
            self.drop_frame = None;
        }
        match drag {
            Drag::Anchor { .. } => {}
            Drag::Box {
                start,
                current,
                from_center,
            } => {
                let r = App::box_rect(start, current, from_center);
                // The knife cuts along the dragged line, in its direction,
                // and Segment Delete needs the press (a slipped click is
                // taken there); shape tools take the normalised box.
                let (a, b) = if matches!(self.tool, Tool::Knife | Tool::SegmentDelete) {
                    (start, current)
                } else {
                    (Point::new(r.x0, r.y0), Point::new(r.x1, r.y1))
                };
                if !self.finish_tools2_box(a, b) {
                    self.create_box_shape(a, b);
                }
            }
            Drag::Connector { from, current, .. } if self.tool == Tool::Blend => {
                if let Some(to) = self.hit_test_inside(current).filter(|id| *id != from) {
                    self.selection = vec![from, to];
                    let (steps, o, c, r) = (
                        self.blend_steps,
                        self.blend_accel_objects,
                        self.blend_accel_colors,
                        self.blend_rotation,
                    );
                    self.blend_selection(steps, o, c, r);
                }
            }
            Drag::Connector {
                from,
                start,
                current,
            } if self.tool == Tool::RightAngleConnector => {
                self.finish_elbow_connector(from, start, current, false)
            }
            Drag::Connector {
                from,
                start,
                current,
            } if self.tool == Tool::RoundedConnector => {
                self.finish_elbow_connector(from, start, current, true)
            }
            Drag::Connector {
                from,
                start,
                current,
            } => self.finish_connector(from, start, current),
            Drag::Freehand { points } if self.tool == Tool::Eraser => self.erase_along(&points),
            Drag::Freehand { points } if self.tool == Tool::Knife => self.knife_cut(&points),
            Drag::Freehand { points } if self.tool == Tool::SegmentDelete => {
                // A click that slipped into a little stroke reaching
                // nothing still deletes as a click at the press.
                if !self.delete_virtual_segments_along(&points) {
                    if let Some(first) = points.first().copied() {
                        let zoom = self.view.zoom.max(1e-6) as f64;
                        let far = points.iter().any(|q| (*q - first).hypot() * zoom >= 8.0);
                        if !far {
                            self.delete_virtual_segment_at(first);
                        }
                    }
                }
            }
            Drag::Freehand { points } if self.tool == Tool::ShapeRecognition => {
                self.finish_shape_recognition(points)
            }
            Drag::Freehand { points } if self.tool == Tool::Sketch => self.finish_sketch(points),
            Drag::ThreePointBase { start, current } => {
                if std::mem::take(&mut self.three_point_circle)
                    && self.tool == Tool::ThreePointEllipse
                {
                    self.finish_three_point_circle(start, current);
                } else if (current - start).hypot() > 0.05 {
                    self.three_point_base = Some((start, current));
                }
            }
            Drag::TextSelect => {}
            Drag::TextFrame { start, current } => {
                let r = Rect::from_points(start, current);
                if r.width() > 2.0 && r.height() > 2.0 {
                    self.start_text(
                        Point::new(r.x0, r.y0),
                        Some(tracedraw_core::geometry::Size::new(r.width(), r.height())),
                    );
                }
            }
            // Objects let go on the Color Styles docker: a window makes
            // colour styles of their colours, and they stay where they
            // were (#2625).
            Drag::Move { .. } if self.color_styles_drop_target() => {
                self.drag_copy = false;
                self.drop_frame = None;
                self.open_color_styles_from_selection();
                self.end_page_drag_group();
            }
            // A picture let go on a palette bar: a window offers its
            // colours to that palette, and the picture stays where it was
            // (#2137).
            Drag::Move { .. } if self.picture_drop_target().is_some() => {
                self.drag_copy = false;
                self.drop_frame = None;
                if let Some(target) = self.picture_drop_target() {
                    self.open_picture_colors(target);
                }
                self.end_page_drag_group();
            }
            // Dropped on a page tab: the selection goes to that page, at
            // its place there (#1346).
            Drag::Move { .. } if self.page_tab_under_pointer().is_some() => {
                if let Some(i) = self.page_tab_under_pointer() {
                    self.drop_on_page_tab(i);
                }
            }
            // Dropped on the document palette: the selection's colours
            // join it and the objects stay where they were.
            Drag::Move { .. } if self.over_document_palette() => {
                self.drag_copy = false;
                self.palette_from_selection();
                self.end_page_drag_group();
            }
            // Let go over another object with Alt (Alt+Shift) held: it
            // takes the selection's fill (outline), as Copy Fill Here and
            // Copy Outline Here of the right-button drag do, and the
            // selection stays where it was. Alt alone over a clip frame
            // still puts the selection into it (#1337) (discussion #2066).
            Drag::Move { .. }
                if self.drop_colour_target.is_some()
                    && !(self.drop_colour == Some(crate::drop_menu::DropAction::CopyFill)
                        && self.drop_frame.is_some()) =>
            {
                let action = self.drop_colour.take();
                let target = self.drop_colour_target.take();
                self.drag_copy = false;
                self.drop_frame = None;
                if let (Some(action), Some(target)) = (action, target) {
                    self.copy_style_onto(target, action);
                }
                self.end_page_drag_group();
            }
            Drag::Move { total, .. } => {
                // Let go over another page of the Multipage View: the
                // objects become that page's, in the same undo step
                // (#2055).
                let to_page = (self.drop_frame.is_none() && self.page_drag_group.is_none())
                    .then(|| self.multipage_drop_page(total))
                    .flatten();
                let mark = to_page.map(|_| self.engine.begin_group());
                if total.hypot() > 1e-6 {
                    self.finish_transform(Affine::translate(total));
                }
                if let Some((idx, r)) = to_page {
                    self.give_selection_to_shown_page(idx, r);
                }
                if let Some(mark) = mark {
                    self.engine.end_group(mark, "Move to Page");
                }
                // Dropped on a clip frame with W: into it; with K: in
                // place of its contents. Without a key the objects just
                // land there, over the frame.
                let (w, k) = self.drop_keys;
                if let Some(frame) = self.drop_frame.take() {
                    let moved = self.selection.clone();
                    if k && !moved.is_empty() {
                        self.replace_clip_contents(moved, frame);
                    } else if w && !moved.is_empty() {
                        self.place_inside(moved, frame);
                        self.select(vec![frame]);
                    }
                }
                // A move that went to another page on the way: one step.
                self.end_page_drag_group();
            }
            Drag::AlongPath { start, current } => self.finish_along_path(start, current),
            Drag::Pivot => {}
            // A Free Transform drag applied its steps as the pointer
            // moved (a release off the canvas ends it here): nothing is
            // left to apply, its undo step is closed (#1342).
            Drag::Rotate { .. } if self.tool == Tool::FreeTransform => {
                self.end_free_transform_group();
            }
            Drag::Scale { .. } | Drag::Rotate { .. } | Drag::Skew { .. } => {
                if let Some(t) = self.preview_transform_of(&drag) {
                    self.finish_transform(t);
                }
            }
            Drag::Marquee {
                start,
                current,
                touching,
                toggle,
            } => {
                let r = Rect::from_points(start, current);
                if r.width() > 0.1 || r.height() > 0.1 {
                    self.select_in_region(crate::select_region::Region::Rect(r), touching, toggle);
                }
            }
            Drag::Lasso {
                points,
                touching,
                toggle,
            } => {
                if points.len() >= 3 {
                    self.select_in_region(
                        crate::select_region::Region::Polygon(&points),
                        touching,
                        toggle,
                    );
                }
            }
            Drag::Freehand { points } if self.tool == Tool::PainterlyBrush => {
                self.add_painterly_stroke(&points);
            }
            Drag::Freehand { points } if self.tool == Tool::BrushStrokes => {
                // A pen's pressure along the drag sets the width (#850).
                let pen = self
                    .pen_pressures(&points)
                    .and_then(|pr| crate::pen::PressureProfile::new(&points, pr));
                let shapes = self.media_shapes_pen(&points, pen.as_ref());
                if let Some(layer) = self.active_layer() {
                    let ids: Vec<ShapeId> = shapes.iter().map(|s| s.id).collect();
                    let cmds: Vec<Command> = shapes
                        .into_iter()
                        .map(|s| Command::AddShape { layer, shape: s })
                        .collect();
                    if !cmds.is_empty() {
                        let _ = self.engine.run_batch("Brush Strokes", &cmds);
                        self.selection = ids;
                    }
                }
            }
            Drag::Freehand { points } => {
                // Smoothing (Options > Tools > Curves, the property bar)
                // sets how far the stroke may be simplified, on screen.
                let tol = self.settings.curves.freehand_tolerance_px() / self.view.zoom as f64;
                let pts = tracedraw_core::geometry::simplify(&points, tol.max(0.02));
                if pts.len() >= 2 {
                    let path = tracedraw_core::geometry::smooth_path(&pts, false);
                    // Drawn with a pen: the outline follows its pressure
                    // along the stroke (#850); the mouse keeps it even.
                    let widths = self
                        .pen_pressures(&points)
                        .and_then(|pr| crate::pen::PressureProfile::new(&points, pr))
                        .zip(self.default_stroke())
                        .map(|(pen, st)| pen.freehand_widths(st.width))
                        .unwrap_or_default();
                    self.add_drawn_line_with_widths(path, widths);
                }
            }
            Drag::FillGradient {
                shape,
                start,
                current,
            } if self.tool == Tool::Transparency => {
                if (current - start).hypot() > 0.5 {
                    // The whole selection when the drag began on it (#1661).
                    let ids = if self.selection.contains(&shape) {
                        self.selection.clone()
                    } else {
                        vec![shape]
                    };
                    self.transparency_from_drag_on(&ids, start, current);
                }
            }
            Drag::FillGradient {
                shape,
                start,
                current,
            } => {
                if (current - start).hypot() > 0.5 {
                    self.fill_from_drag(shape, start, current);
                }
            }
            Drag::ZoomBox { start, current } => {
                let r = Rect::from_points(start, current);
                if r.width() > 0.5 && r.height() > 0.5 {
                    self.view.fit(r, self.canvas_rect);
                    self.end_zoom_one_shot();
                }
            }
            Drag::NodeMarquee { start, current } => self.finish_node_marquee(start, current),
            Drag::NodeLasso { points } => self.finish_node_lasso(&points),
            Drag::NodeMove { begun: true, .. } => self.join_dropped_end_node(),
            Drag::NodeMove { .. }
            | Drag::NodeTransform { .. }
            | Drag::SegmentBend { .. }
            | Drag::PolygonEdge { .. }
            | Drag::TextChars { .. }
            | Drag::TextSpacing { .. } => {}
            Drag::NewGuide { .. } => self.finish_guide_drag(),
            d @ (Drag::MoveGuide { .. } | Drag::RotateGuide { .. } | Drag::GuidePivot) => {
                // Released outside the window (no pointer): dropped off.
                let at = self
                    .pointer_page
                    .map(|p| self.view.to_screen(p))
                    .unwrap_or(egui::pos2(f32::NAN, f32::NAN));
                self.drag = d;
                self.finish_guide_move(at);
            }
            Drag::FountainHandle { .. }
            | Drag::SymmetryLines { .. }
            | Drag::ContourDrag { .. }
            | Drag::Shadow { .. }
            | Drag::ShadowHandle { .. }
            | Drag::ShadowLine { .. }
            | Drag::FountainLine { .. }
            | Drag::RulerOrigin { .. }
            | Drag::Node { .. }
            | Drag::KindNode { .. }
            | Drag::CornerWidget(_)
            | Drag::Handle { .. }
            | Drag::None => {}
        }
    }

    /// Transform the selection would get if the drag ended now (for preview
    /// and for commit). Translation is handled separately by `Drag::Move`.
    pub fn preview_transform_of(&self, drag: &Drag) -> Option<Affine> {
        match drag {
            Drag::Scale {
                handle,
                anchor,
                start_bounds,
                current,
                mods,
                snapped,
                ..
            } => {
                let b = *start_bounds;
                // From the centre (Shift) the handle's side is half as far
                // from the fixed point.
                let (anchor, half) = if mods.from_center {
                    (b.center(), 0.5)
                } else {
                    (*anchor, 1.0)
                };
                let (w, h) = ((b.width() * half).max(1e-6), (b.height() * half).max(1e-6));
                let (mut sx, mut sy) = (1.0, 1.0);
                match handle {
                    Handle::E | Handle::W => {
                        sx = (current.x - anchor.x) / (if *handle == Handle::E { w } else { -w })
                    }
                    Handle::N | Handle::S => {
                        sy = (current.y - anchor.y) / (if *handle == Handle::N { h } else { -h })
                    }
                    Handle::NE => {
                        sx = (current.x - anchor.x) / w;
                        sy = (current.y - anchor.y) / h;
                    }
                    Handle::NW => {
                        sx = (anchor.x - current.x) / w;
                        sy = (current.y - anchor.y) / h;
                    }
                    Handle::SE => {
                        sx = (current.x - anchor.x) / w;
                        sy = (anchor.y - current.y) / h;
                    }
                    Handle::SW => {
                        sx = (anchor.x - current.x) / w;
                        sy = (anchor.y - current.y) / h;
                    }
                }
                if handle.is_corner() && !self.rotate_mode && !mods.free {
                    // Corners scale proportionally (Alt: freely): by where
                    // the pointer falls along the box's diagonal, so the
                    // corner follows the pointer and a wide, low box (a
                    // line of text) does not grow with the short side's
                    // pace (#383). A corner snapped on one axis only (onto
                    // a guideline, a line, another object's side) scales
                    // by that axis, so it stays on what it snapped to
                    // whichever way the pointer drifts along it (#2004).
                    let (w2, h2) = (w * w, h * h);
                    let s = match snapped {
                        (true, false) => sx.abs(),
                        (false, true) => sy.abs(),
                        _ => (sx.abs() * w2 + sy.abs() * h2) / (w2 + h2),
                    };
                    sx = s * sx.signum();
                    sy = s * sy.signum();
                }
                if mods.steps {
                    // Ctrl: whole steps of 100 %, so crossing the other
                    // side gives the mirror image at the same size.
                    let step = |v: f64| v.signum() * v.abs().round().max(1.0);
                    sx = step(sx);
                    sy = step(sy);
                }
                if !(sx.is_finite() && sy.is_finite()) || sx.abs() < 1e-3 || sy.abs() < 1e-3 {
                    return None;
                }
                Some(
                    Affine::translate(anchor.to_vec2())
                        * Affine::scale_non_uniform(sx, sy)
                        * Affine::translate(-anchor.to_vec2()),
                )
            }
            Drag::Rotate {
                center,
                start_angle,
                current_angle,
                ..
            } => {
                let a = current_angle - start_angle;
                Some(
                    Affine::translate(center.to_vec2())
                        * Affine::rotate(a)
                        * Affine::translate(-center.to_vec2()),
                )
            }
            Drag::Skew {
                handle,
                start_bounds,
                start,
                current,
                mods,
            } => skew_transform(
                *handle,
                *start_bounds,
                *current - *start,
                *mods,
                self.settings.constrain_angle,
            ),
            _ => None,
        }
    }

    /// Pick `t` by its key. The Pick tool's key (Space) on the Pick tool
    /// goes back to the tool used before it, not during a drag, where
    /// Space drops a copy; true then, the key having done its work.
    fn pick_tool_by_key(&mut self, t: Tool, pointer_busy: bool) -> bool {
        if t == Tool::Pick && pointer_busy {
            return false;
        }
        if t == Tool::Pick && self.tool == Tool::Pick {
            if self.previous_tool != Tool::Pick {
                self.choose_tool(self.previous_tool);
            }
            return true;
        }
        self.choose_tool(t);
        false
    }

    /// The zoom keys while a text is being edited, which type nothing
    /// (#1699): F3 zooms out, F4 to fit all objects, Shift+F4 to the
    /// page, Shift+F2 to the selection, and F2 picks the Zoom tool for one
    /// zoom, after which the text is edited on where it was. Each key
    /// taken is consumed, so the typing does not see it; one the user
    /// gave another command or removed (Options > Customization >
    /// Commands) is left alone.
    pub(crate) fn text_view_keys(&mut self, ctx: &egui::Context) {
        let keys = [
            (Key::F2, Modifiers::SHIFT),
            (Key::F2, Modifiers::NONE),
            (Key::F3, Modifiers::NONE),
            (Key::F4, Modifiers::SHIFT),
            (Key::F4, Modifiers::NONE),
        ];
        for (key, mods) in keys {
            // Exact modifiers: Shift+F3 (Change Case) is not F3.
            let hit = ctx.input_mut(|i| {
                let before = i.events.len();
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. }
                        if *k == key && modifiers.matches_exact(mods))
                });
                i.events.len() != before
            });
            if !hit || self.builtin_key_overridden(ctx, key, mods) {
                continue;
            }
            match (key, mods.shift) {
                (Key::F2, true) => self.zoom_to_selection(),
                (Key::F2, false) => {
                    if self.tool != Tool::Zoom {
                        // Not `set_tool`, which would end the editing.
                        self.previous_tool = self.tool;
                        self.tool = Tool::Zoom;
                        self.zoom_one_shot = true;
                        self.flyout_open = None;
                    }
                }
                (Key::F3, _) => self.zoom_step(false),
                (Key::F4, true) => self.zoom_to_page(),
                _ => self.zoom_to_fit(),
            }
        }
    }

    /// The built-in shortcut `key` with `mods` is another command's by
    /// the user's choice, or was removed (Options > Customization >
    /// Commands).
    fn builtin_key_overridden(&mut self, ctx: &egui::Context, key: Key, mods: Modifiers) -> bool {
        let text = crate::shortcuts::format(mods, key);
        if self
            .settings
            .shortcuts
            .iter()
            .any(|(_, v)| crate::shortcuts::same(v, &text))
        {
            return true;
        }
        if !self.settings.shortcuts.iter().any(|(_, v)| v.is_empty()) {
            return false;
        }
        if self.shortcut_editor.entries.is_empty() {
            self.shortcut_editor.entries = crate::ui::menus::commands(self, ctx);
        }
        self.shortcut_editor.entries.iter().any(|e| {
            crate::shortcuts::same(&e.shortcut, &text)
                && self
                    .settings
                    .shortcuts
                    .iter()
                    .any(|(id, v)| id == &e.key && v.is_empty())
        })
    }

    pub fn keyboard(&mut self, ctx: &egui::Context) {
        // The pen's pressure, which the raw input hook read from this
        // frame's touch events, before the tools see the pointer (#850).
        self.pen_pressure = crate::pen::pressure(ctx);
        // A macro's message, input box, modal form or error owns the
        // keyboard (see ui/macros_ui.rs); Esc stops a running macro.
        if self.macros.modal() {
            return;
        }
        if self.macros.run.is_some()
            && !ctx.egui_wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            self.stop_macro();
            return;
        }
        // An open menu or list, or the menu bar with the keyboard, takes
        // the keys: its rows' access keys, arrows, Enter, Esc. The arrows
        // move its highlight, never the selection, a node or the caret
        // of the text being edited (#1839).
        if crate::ui::keynav::takes_keys(ctx) {
            return;
        }
        let dialog_open = !matches!(self.dialog, crate::ui::dialogs::Dialog::None);
        // Table cell typing has priority while a cell is active (not
        // behind a dialog, which owns the keyboard, nor while a field has
        // the keyboard: a font size typed into the property bar went into
        // the cell, in place of its selected text, #2328).
        if !dialog_open
            && !ctx.egui_wants_keyboard_input()
            && self.table_edit.is_some()
            && matches!(self.tool, Tool::Table | Tool::Text)
            && self.table_keyboard(ctx)
        {
            return;
        }
        // A field with the keyboard (the font search field of the
        // property bar, a docker's box) keeps what is typed in it, even
        // while a text is being edited (#342).
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // Text typing has priority; the zoom keys, which type nothing,
        // still zoom (#1699).
        if self.text_edit.is_some() && !dialog_open {
            // Shift+F3 while typing: Text > Change Case for the text being
            // edited, which the editing ends and selects first (#2059).
            if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3)) {
                let shape = self.text_edit.as_ref().map(|t| t.shape);
                self.finish_text();
                if let Some(shape) = shape {
                    self.select(vec![shape]);
                }
                self.dialog =
                    crate::ui::dialogs::Dialog::ChangeCase(crate::ops2::CaseMode::Sentence);
                return;
            }
            self.text_view_keys(ctx);
            self.text_keyboard(ctx);
            return;
        }
        // A modal dialog owns the keyboard: Esc closes it, the rest is
        // not interpreted as shortcuts.
        // A colour picker's eyedropper waiting for a click: Esc stops
        // it, and no dialog closes (#814).
        if crate::ui::color_picker::picking(ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            crate::ui::color_picker::finish_pick(ctx, None);
            return;
        }
        if !matches!(self.dialog, crate::ui::dialogs::Dialog::None) {
            // A command waiting for its shortcut (Options > Customization >
            // Commands): Esc stops waiting, other keys are the shortcut.
            if self.shortcut_editor.capture.is_some() {
                // Taken from the input so the dialog does not close too.
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                    self.shortcut_editor.capture = None;
                }
                return;
            }
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.dialog = crate::ui::dialogs::Dialog::None;
                // Esc is Cancel for the options dialogs.
                if let Some(s) = self.options_snapshot.take() {
                    s.restore(self);
                }
            }
            return;
        }
        // An import waiting for its place: Space or Enter centres it on
        // the page, Esc calls it off (#1936).
        if self.import_placement_keys(ctx) {
            return;
        }
        // Esc during a move, scale, rotation or skew cancels it: nothing
        // changes and the selection stays.
        if (self.pending_shaping.is_some() || self.pending_merge_frame)
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            self.pending_shaping = None;
            self.pending_merge_frame = false;
            self.status = crate::i18n::tr("status.cancelled");
            return;
        }
        if self.pending_text_link.is_some() && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.cancel_text_link();
            return;
        }
        if self.transforming() && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.drag = Drag::None;
            self.drag_copy = false;
            return;
        }
        let input = ctx.input(|i| i.clone());
        // Each key press with the modifiers held when it happened (a quick
        // Ctrl+E can be released before the frame that sees it). Shortcuts
        // match the modifiers exactly, so Ctrl+Shift+Z is not also Ctrl+Z;
        // only + and - ignore Shift, which some layouts need to type them.
        // A key held sends its press again and again (`repeat`); the
        // presses a held key makes are kept apart for what must happen
        // once a press, such as picking a tool (#1502).
        // In the browser version a key that stands in for one the browser
        // keeps (Alt+PgUp for Ctrl+PgUp, #1409) also counts as that key.
        // In the macOS desktop app so does the key that stands in for a
        // Command key macOS keeps (Alt+Shift+C for Convert to Curves'
        // Command+Q, the system's Quit), and such a Command key does
        // nothing here should it arrive.
        let web_keys = crate::shortcuts::web_keys();
        let presses: Vec<(Key, Modifiers)> = input
            .events
            .iter()
            .flat_map(|e| match e {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if crate::shortcuts::mac_kept_press(*key, *modifiers) {
                        return [None, None];
                    }
                    let kept = web_keys
                        .then(|| crate::shortcuts::browser_key_for(*key, *physical_key, *modifiers))
                        .flatten()
                        .or_else(|| crate::shortcuts::mac_key_for(*key, *modifiers))
                        .map(|(m, k)| (k, m));
                    [Some((*key, *modifiers)), kept]
                }
                _ => [None, None],
            })
            .flatten()
            .collect();
        let first_presses: Vec<(Key, Modifiers)> = input
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect();
        let matches = |presses: &[(Key, Modifiers)], k: Key, m: Modifiers| {
            presses.iter().any(|(key, mods)| {
                *key == k
                    && if matches!(k, Key::Plus | Key::Equals | Key::Minus) {
                        mods.matches_logically(m)
                    } else {
                        mods.matches_exact(m)
                    }
            })
        };
        let pressed = |k: Key, m: Modifiers| matches(&presses, k, m);
        let first_pressed = |k: Key, m: Modifiers| matches(&first_presses, k, m);
        let cmd = Modifiers::COMMAND;

        let pointer_busy = !matches!(self.drag, Drag::None) || input.pointer.any_down();
        // Menu commands and tools with a shortcut of the user's (Options >
        // Customization > Commands) come first: they may take a built-in
        // shortcut over (#1595).
        for (key, mods) in &presses {
            let text = crate::shortcuts::format(*mods, *key);
            let Some(id) = self
                .settings
                .shortcuts
                .iter()
                .find(|(_, v)| crate::shortcuts::same(v, &text))
                .map(|(id, _)| id.clone())
            else {
                continue;
            };
            // The Navigator's key works while held (`ui::view_navigator`).
            if id == crate::ui::NAVIGATOR_KEY {
                return;
            }
            if id.starts_with("menu.") {
                self.pending_command = Some(id);
                return;
            }
            if let Some(t) = Tool::ALL.iter().find(|t| t.id() == id) {
                // A key held picks its tool once (#1502); W and K during a
                // move say what a drop on a clip frame does and pick none.
                let drop_key = matches!(self.drag, Drag::Move { .. })
                    && mods.is_none()
                    && matches!(key, Key::W | Key::K);
                if first_pressed(*key, *mods) && !drop_key {
                    self.pick_tool_by_key(*t, pointer_busy);
                }
                return;
            }
        }
        // Built-in shortcuts the user removed (Options > Customization >
        // Commands, Delete): their keys do nothing.
        if self.settings.shortcuts.iter().any(|(_, v)| v.is_empty()) {
            if self.shortcut_editor.entries.is_empty() {
                self.shortcut_editor.entries = crate::ui::menus::commands(self, ctx);
            }
            let removed = presses.iter().any(|(key, mods)| {
                let text = crate::shortcuts::format(*mods, *key);
                self.shortcut_editor.entries.iter().any(|e| {
                    crate::shortcuts::same(&e.shortcut, &text)
                        && self
                            .settings
                            .shortcuts
                            .iter()
                            .any(|(id, v)| id == &e.key && v.is_empty())
                })
            });
            if removed {
                return;
            }
        }

        // Tools > Macros > Run Macro (Alt+Shift+F8: Alt+F8 is the
        // Transformations panel's Rotate) and Macro Editor.
        if pressed(Key::F8, Modifiers::ALT | Modifiers::SHIFT) {
            self.open_run_macro();
            return;
        }
        if pressed(Key::F11, Modifiers::ALT) {
            self.open_macro_editor();
            return;
        }
        // Help > Search Commands, with or without a drawing open.
        if first_pressed(Key::P, cmd | Modifiers::SHIFT) {
            self.command_search.open();
            return;
        }
        // The Welcome Screen alone (no drawing open): only the commands
        // that need no drawing.
        if !self.has_document() {
            if pressed(Key::N, cmd) {
                self.request_new_document();
            }
            if pressed(Key::O, cmd) {
                self.open_dialog();
            }
            if pressed(Key::J, cmd) {
                self.dialog = crate::ui::dialogs::Dialog::Options;
            }
            if pressed(Key::F4, Modifiers::ALT) {
                self.request_exit();
            }
            return;
        }

        // Alt+Backspace undoes too, the older key for it (#2201).
        if pressed(Key::Z, cmd) || pressed(Key::Backspace, Modifiers::ALT) {
            self.undo();
        }
        if pressed(Key::Z, cmd | Modifiers::SHIFT) {
            self.redo();
        }
        if pressed(Key::S, cmd) {
            self.save(false);
        }
        if pressed(Key::S, cmd | Modifiers::SHIFT) {
            self.save(true);
        }
        if pressed(Key::S, Modifiers::ALT) && !self.selection.is_empty() {
            self.create_symmetry();
        }
        if pressed(Key::E, Modifiers::ALT) {
            self.edit_symmetry();
        }
        if pressed(Key::E, Modifiers::ALT | Modifiers::SHIFT) {
            self.finish_symmetry_edit();
        }
        if pressed(Key::S, Modifiers::ALT | Modifiers::SHIFT) {
            self.remove_symmetry();
        }
        if pressed(Key::X, Modifiers::ALT) && self.selected_symmetry().is_some() {
            self.break_symmetry_link();
        }
        // Docker shortcuts (Ctrl+F9 Contour, Alt+F3 Lens, Alt+Enter
        // Properties, ...): toggle.
        for tab in crate::app::DockerTab::ALL {
            let label = tab.shortcut();
            // Transformations opens on a page, below.
            if label.is_empty() || tab == crate::app::DockerTab::Transformations {
                continue;
            }
            let Some((k, shift)) = Tool::parse_shortcut(label) else {
                continue;
            };
            let mut m = Modifiers::NONE;
            if label.contains("Ctrl+") {
                m |= cmd;
            }
            if label.contains("Alt+") {
                m |= Modifiers::ALT;
            }
            if shift {
                m |= Modifiers::SHIFT;
            }
            if pressed(k, m) {
                self.toggle_docker(tab);
            }
        }
        // Alt+F7 to Alt+F10: the Transformations docker on Position,
        // Rotate, Scale and Mirror or Size.
        for (k, tab) in [
            (Key::F7, crate::app::TransformTab::Position),
            (Key::F8, crate::app::TransformTab::Rotate),
            (Key::F9, crate::app::TransformTab::Scale),
            (Key::F10, crate::app::TransformTab::Size),
        ] {
            if pressed(k, Modifiers::ALT) {
                crate::ui::transform_docker::open(self, tab, true);
            }
        }
        if pressed(Key::O, cmd) {
            self.open_dialog();
        }
        if pressed(Key::N, cmd) {
            self.request_new_document();
        }
        if pressed(Key::E, cmd) {
            self.export();
        }
        // File > Export with Options: the options whatever the setting.
        if pressed(Key::E, cmd | Modifiers::SHIFT) {
            self.export_with_options();
        }
        if pressed(Key::I, cmd) {
            self.import();
        }
        if pressed(Key::G, cmd) {
            self.group_selection();
        }
        // Ctrl+Shift+G: unselected nodes filled or hollow.
        if pressed(Key::G, cmd | Modifiers::SHIFT) {
            self.settings.nodes.unselected_filled = !self.settings.nodes.unselected_filled;
        }
        if pressed(Key::U, cmd) {
            self.ungroup_selection();
        }
        // Bitmap adjustments of the Effects menu.
        if self
            .selected_rc()
            .iter()
            .any(|s| matches!(s.kind, ShapeKind::Bitmap { .. }))
        {
            for (key, mods, id) in [
                (Key::B, cmd, "brightness_contrast_intensity"),
                (Key::B, cmd | Modifiers::SHIFT, "color_balance"),
                (Key::U, cmd | Modifiers::SHIFT, "hue_saturation_lightness"),
            ] {
                if pressed(key, mods) {
                    crate::ui::effect_dialog::choose_effect(self, id);
                }
            }
        }
        if pressed(Key::Q, cmd | Modifiers::SHIFT) {
            self.convert_outline_to_object();
        } else if pressed(Key::Q, cmd) {
            self.convert_to_curves();
        }
        if pressed(Key::A, cmd) {
            // With a guideline selected: every guideline of the page, as
            // Edit > Select All > Guidelines (#1753).
            if !self.selected_guides.is_empty() && self.selection.is_empty() {
                self.select_all_guides();
            } else if self.tool == Tool::Shape && !self.selection.is_empty() {
                self.select_all_nodes();
            } else {
                self.select_all();
            }
        }
        if pressed(Key::L, cmd) {
            self.combine();
        }
        // Copy Properties From: its dialog, then a click on the object
        // to copy from (#1345).
        if pressed(Key::A, cmd | Modifiers::SHIFT) {
            self.open_copy_properties();
        }
        if pressed(Key::J, cmd) {
            self.dialog = crate::ui::dialogs::Dialog::Options;
        }
        // Ctrl+Tab and Ctrl+Shift+Tab: the next and the previous document
        // tab, round the end (#2068, #2058).
        if self.docs.len() > 1 {
            let n = self.docs.len();
            if pressed(Key::Tab, Modifiers::CTRL) {
                self.switch_document((self.active_doc + 1) % n);
            } else if pressed(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT) {
                self.switch_document((self.active_doc + n - 1) % n);
            }
        }
        // With the Shape tool, Tab steps through the nodes of the curve.
        let shape_tool = self.tool == Tool::Shape;
        if pressed(Key::Tab, Modifiers::NONE) && !(shape_tool && self.cycle_node(true)) {
            self.cycle_selection(true);
        }
        if pressed(Key::Tab, Modifiers::SHIFT) && !(shape_tool && self.cycle_node(false)) {
            self.cycle_selection(false);
        }
        if pressed(Key::K, cmd) {
            self.break_apart();
        }
        // Selection groups 0 to 9: Ctrl+number stores the selection under
        // the number, Alt+number adds the selection to it, the number
        // selects the group again (a second time also zooms to it). With
        // text selected Ctrl+8 and Ctrl+2 keep changing the text size.
        let text_selected = self
            .selected_rc()
            .iter()
            .any(|s| matches!(s.kind, ShapeKind::Text { .. }));
        for (n, k) in [
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ]
        .into_iter()
        .enumerate()
        {
            let n = n as u8;
            if pressed(k, cmd)
                && !self.selection.is_empty()
                && !(text_selected && (n == 2 || n == 8))
            {
                self.set_selection_group(n, self.selection.clone());
            }
            if pressed(k, Modifiers::ALT) && !self.selection.is_empty() {
                let mut ids = self.selection_group(n);
                for id in self.selection.clone() {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                self.set_selection_group(n, ids);
            }
            if pressed(k, Modifiers::NONE) {
                self.recall_selection_group(n);
            }
        }
        // With the Shape tool and nodes selected, the segments between
        // them are copied, cut or duplicated.
        let segments = shape_tool && !self.node_selection.is_empty();
        // The windowing layer turns Ctrl+C, Ctrl+X and Ctrl+V (and
        // Ctrl+Insert, Shift+Delete and Shift+Insert) into copy, cut and
        // paste events instead of key presses; a paste event comes only
        // while the system clipboard holds text, which a copy here puts
        // there. Either form counts, once per frame.
        let (copy_event, cut_event, paste_text) = clipboard_events(&input.events);
        let paste_event = paste_text.is_some();
        let pasted_without_text = paste_without_text(&input.events, &mut self.paste_seen);
        let shift_held = input.modifiers.shift;
        if (pressed(Key::C, cmd) || copy_event) && !(segments && self.copy_segments()) {
            self.copy_with_system();
        }
        if (pressed(Key::X, cmd) || cut_event) && !(segments && self.cut_segments()) {
            self.cut();
        }
        if pressed(Key::V, cmd) || (paste_event && !shift_held) || pasted_without_text {
            self.paste_with_event_text(paste_text);
        } else if paste_event && shift_held {
            self.paste_in_view();
        }
        if pressed(Key::D, cmd) && !(segments && self.duplicate_segments()) {
            self.duplicate();
        }
        // Ctrl+Space: the Pick tool, whatever tool is active.
        if pressed(Key::Space, cmd) && self.tool != Tool::Pick {
            self.set_tool(Tool::Pick);
        }
        // Order: Ctrl+Home/End to the front/back of the page, Shift+PgUp/PgDn
        // within the layer, Ctrl+PgUp/PgDn one step.
        if pressed(Key::Home, cmd) {
            self.order_page(true);
        } else if pressed(Key::PageUp, Modifiers::SHIFT) {
            self.order(0);
        }
        if pressed(Key::End, cmd) {
            self.order_page(false);
        } else if pressed(Key::PageDown, Modifiers::SHIFT) {
            self.order(3);
        }
        if pressed(Key::PageUp, cmd) {
            self.order(1);
        }
        if pressed(Key::PageDown, cmd) {
            self.order(2);
        }
        // The bracket keys order too (#2201): Ctrl+] and Ctrl+[ one step,
        // with Shift to the front and the back of the layer.
        if pressed(Key::CloseBracket, cmd) {
            self.order(1);
        }
        if pressed(Key::OpenBracket, cmd) {
            self.order(2);
        }
        if pressed(Key::CloseBracket, cmd | Modifiers::SHIFT) {
            self.order(0);
        }
        if pressed(Key::OpenBracket, cmd | Modifiers::SHIFT) {
            self.order(3);
        }
        // The rest of the menu shortcuts.
        if pressed(Key::P, cmd) {
            self.print();
        }
        if pressed(Key::R, cmd) {
            self.repeat_last();
        }
        if pressed(Key::V, cmd | Modifiers::SHIFT) && !paste_event {
            self.paste_in_view();
        }
        if pressed(Key::M, cmd) {
            self.table_op(crate::table::TableOp::Merge);
        }
        if pressed(Key::W, cmd) {
            self.raster.borrow_mut().invalidate();
        }
        if pressed(Key::F4, cmd) {
            self.close_document();
        }
        if pressed(Key::F4, Modifiers::ALT) {
            self.request_exit();
        }
        if pressed(Key::F12, Modifiers::ALT) {
            self.align_chars_to_baseline();
        }
        // F9: View > Full-Screen Preview (#2195). The press is taken so
        // that the preview, drawn later this frame, does not see it as
        // its own F9 and close at once; while it shows, its F9 closes it.
        if !self.fullscreen_preview && first_pressed(Key::F9, Modifiers::NONE) {
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F9));
            self.fullscreen_preview = true;
        }
        // F11: the Edit Fill dialog. F3: zoom out.
        if pressed(Key::F11, Modifiers::NONE) {
            self.open_edit_fill();
        }
        if pressed(Key::F3, Modifiers::NONE) {
            self.zoom_step(false);
        }
        // Shift+F3: Text > Change Case for the selected text (#801).
        if pressed(Key::F3, Modifiers::SHIFT)
            && self
                .selected_shapes()
                .iter()
                .any(|s| matches!(s.kind, ShapeKind::Text { .. }))
        {
            self.dialog = crate::ui::dialogs::Dialog::ChangeCase(crate::ops2::CaseMode::Sentence);
        }
        // Into the selected clip frame's contents, and out again.
        if pressed(Key::F3, cmd | Modifiers::SHIFT) {
            self.toggle_clip_frame_edit();
        }
        if pressed(Key::T, cmd | Modifiers::SHIFT) {
            self.open_edit_text_dialog();
        }
        if pressed(Key::Q, Modifiers::ALT) {
            self.snap.off = !self.snap.off;
        }
        if pressed(Key::Z, Modifiers::ALT) {
            self.snap.objects = !self.snap.objects;
        }
        if pressed(Key::Y, cmd) {
            self.snap.grid = !self.snap.grid;
        }
        if pressed(Key::A, Modifiers::ALT | Modifiers::SHIFT) {
            self.snap.alignment_guides = !self.snap.alignment_guides;
        }
        if pressed(Key::D, Modifiers::ALT | Modifiers::SHIFT) {
            self.snap.dynamic_guides = !self.snap.dynamic_guides;
        }
        if pressed(Key::R, Modifiers::ALT | Modifiers::SHIFT) {
            self.show_rulers = !self.show_rulers;
        }
        // View > Guidelines and View > Grid > Document Grid (#2201).
        if pressed(Key::Semicolon, cmd) {
            self.show_guides = !self.show_guides;
        }
        if pressed(Key::Quote, cmd) {
            self.show_grid = !self.show_grid;
        }
        // Edit > Invert Selection (#2201).
        if pressed(Key::I, cmd | Modifiers::SHIFT) {
            self.invert_selection();
        }
        // Ctrl+8 and Ctrl+2: selected text one keyboard text increment
        // larger or smaller (Options > Text).
        if self
            .selected_rc()
            .iter()
            .any(|s| matches!(s.kind, ShapeKind::Text { .. }))
        {
            if pressed(Key::Num8, cmd) {
                self.step_text_size(true);
            }
            if pressed(Key::Num2, cmd) {
                self.step_text_size(false);
            }
            // Ctrl+Shift+> and Ctrl+Shift+<: the letters further apart
            // or closer (#193).
            if pressed(Key::Period, cmd | Modifiers::SHIFT) {
                self.step_char_spacing(true);
            }
            if pressed(Key::Comma, cmd | Modifiers::SHIFT) {
                self.step_char_spacing(false);
            }
        }
        if pressed(Key::Plus, cmd) || pressed(Key::Equals, cmd) {
            self.zoom_step(true);
        }
        if pressed(Key::Minus, cmd) {
            self.zoom_step(false);
        }
        if pressed(Key::Delete, Modifiers::NONE) || pressed(Key::Backspace, Modifiers::NONE) {
            if self.curve.is_some() {
                // While a curve is drawn the keys take its last node back,
                // never the selection (#1527).
                self.curve_remove_last_node();
            } else if matches!(self.tool, Tool::MeshFill | Tool::Shape)
                && self.selected_mesh_node().is_some()
            {
                self.delete_mesh_lines();
            } else if matches!(self.tool, Tool::Envelope | Tool::Shape)
                && self.selected_envelope_node().is_some()
            {
                // The node goes; a corner stays (the status bar says so).
                self.delete_selected_envelope_node();
            } else if self.tool == Tool::Shape && self.delete_chosen_segment() {
                // A segment clicked on went; its nodes stay.
            } else if self.tool == Tool::Shape && !self.node_selection.is_empty() {
                self.delete_selected_nodes();
            } else if self.tool == Tool::VariableOutline && self.delete_variable_outline_node() {
                // The width node went, the object stays.
            } else if self.tool == Tool::AnchorEditing && self.delete_selected_anchor() {
                // The anchor went, the object stays.
            } else if self.delete_picked_shadow() {
                // The chosen drop shadow went, its object stays (#2545).
            } else if !self.selected_guides.is_empty() {
                if !self.delete_selected_guides() {
                    self.status = crate::i18n::tr("status.guideline_locked");
                }
            } else if first_pressed(Key::Delete, Modifiers::NONE)
                || first_pressed(Key::Backspace, Modifiers::NONE)
            {
                // Whole objects go only on a press of their own, never on
                // the repeats of a key held down: the press that took the
                // chosen nodes (or the last node of a curve being drawn)
                // must not run on into the drawing itself (#1982).
                self.delete_selection();
            }
        }
        if pressed(Key::F4, Modifiers::SHIFT) {
            self.zoom_to_page();
        }
        if pressed(Key::F4, Modifiers::NONE) {
            self.zoom_to_fit();
        }
        if pressed(Key::F2, Modifiers::SHIFT) {
            self.zoom_to_selection();
        }
        if pressed(Key::F2, Modifiers::NONE) && self.tool != Tool::Zoom {
            // One zoom, then back to the tool before (#1039); Z keeps
            // the Zoom tool.
            self.set_tool(Tool::Zoom);
            self.zoom_one_shot = true;
        }
        // + and -: with the Shape tool and nodes chosen, a node in the
        // middle of each segment ending at them, and deleting them; with a
        // Pick tool and a selection, + duplicates it in place; otherwise
        // a zoom step about the window's centre.
        let node_keys = self.tool == Tool::Shape && !self.node_selection.is_empty();
        let pick_selection =
            matches!(self.tool, Tool::Pick | Tool::FreeformPick) && !self.selection.is_empty();
        if pressed(Key::Plus, Modifiers::NONE) || pressed(Key::Equals, Modifiers::NONE) {
            if node_keys {
                self.add_node_midpoints();
            } else if pick_selection && pressed(Key::Plus, Modifiers::NONE) {
                self.duplicate_in_place();
                self.status = crate::i18n::tr("status.duplicated_in_place");
            } else {
                let c = self.canvas_rect.center();
                self.view.zoom_at(c, 1.25);
            }
        }
        if pressed(Key::Minus, Modifiers::NONE) {
            if node_keys {
                self.delete_selected_nodes();
            } else {
                let c = self.canvas_rect.center();
                self.view.zoom_at(c, 0.8);
            }
        }
        if pressed(Key::Enter, Modifiers::NONE) {
            if self.tool == Tool::Crop && self.crop_area.is_some() {
                self.apply_crop();
            }
            self.finish_curve();
        }
        // Space finishes a Bezier curve, or a Freehand line of clicks
        // (#2339), too (then, as the Pick tool's key, it switches to Pick).
        if pressed(Key::Space, Modifiers::NONE)
            && matches!(self.tool, Tool::Bezier | Tool::Freehand)
        {
            self.finish_curve();
        }
        // Esc with a menu or a list open only closes it.
        // Esc with a crop area waiting only clears it.
        if pressed(Key::Escape, Modifiers::NONE)
            && !egui::Popup::is_any_open(ctx)
            && self.crop_area.take().is_none()
        {
            // A curve being drawn is finished as it stands, the segment
            // to the pointer dropped; the tool stays (#314). A lone first
            // node is dropped.
            if self.curve.as_ref().is_some_and(|c| c.nodes.len() >= 2) {
                self.finish_curve();
                return;
            }
            // Editing a symbol with nothing selected: Finish (#1980).
            if self.selection.is_empty() && self.symbol_edit_index().is_some() {
                self.finish_clip_frame_edit();
                return;
            }
            // In focus mode with nothing selected: one level out.
            if self.selection.is_empty() && self.focus_target().is_some() {
                self.focus_up();
            }
            self.curve = None;
            self.curve_steps.clear();
            self.dimension_points.clear();
            self.eyedropper_reset();
            self.pending_palette_sample = false;
            self.palette_shades = None;
            self.palette_expanded = None;
            self.select(Vec::new());
            self.deselect_guides();
            if self.tool != Tool::Pick {
                self.set_tool(Tool::Pick);
            }
        }
        // Page Down and Page Up go to the next and the previous page; past
        // the last or the first page they offer a new one there: the
        // Insert Page dialog, after or before it (#1752).
        if pressed(Key::PageDown, Modifiers::NONE) {
            let i = self.page_index();
            if i + 1 < self.doc().pages.len() {
                self.goto_page(i + 1);
            } else {
                self.dialog = crate::ui::dialogs::Dialog::InsertPage {
                    count: 1,
                    after: true,
                };
            }
        }
        if pressed(Key::PageUp, Modifiers::NONE) {
            let i = self.page_index();
            if i > 0 {
                self.goto_page(i - 1);
            } else {
                self.dialog = crate::ui::dialogs::Dialog::InsertPage {
                    count: 1,
                    after: false,
                };
            }
        }
        // The Shape tool with nodes chosen: C makes them cusp, S smooth
        // and Y symmetrical (before the alignment and tool letters).
        // Each key names one type: C used to turn a cusp node smooth
        // and S to make nodes symmetrical, so neither key gave the type
        // it stands for on the node it was pressed on (#2210).
        let nodes_chosen = self.tool == Tool::Shape && !self.node_selection.is_empty();
        if nodes_chosen {
            // The node commands' own keys (Object > Nodes, Shift and a
            // letter: To Line Shift+L, To Curve Shift+U, Cusp Shift+C,
            // Smooth Shift+M, Symmetrical Shift+Y, Break Shift+B, Join
            // Shift+J; #2001).
            if let Some(id) = presses
                .iter()
                .find_map(|(k, m)| App::node_command_key(*k, *m))
            {
                self.run_node_command(id);
                return;
            }
            use tracedraw_core::nodes::NodeType;
            // A B-spline's control point has two states, so C still
            // switches a fixed one back to floating (#1758).
            let cusp = if self.chosen_spline_points() == Some(true) {
                NodeType::Smooth
            } else {
                NodeType::Cusp
            };
            for (k, ty) in [
                (Key::C, cusp),
                (Key::S, NodeType::Smooth),
                (Key::Y, NodeType::Symmetrical),
            ] {
                if pressed(k, Modifiers::NONE) {
                    self.set_selected_node_type(ty);
                    return;
                }
            }
            // Home and End: the curve's first and last node.
            if pressed(Key::Home, Modifiers::NONE) && self.select_end_node(true) {
                return;
            }
            if pressed(Key::End, Modifiers::NONE) && self.select_end_node(false) {
                return;
            }
        }
        // Align shortcuts: plain letters with a selection; with the Shape
        // tool and two or more nodes chosen they align the nodes (C stays
        // the node's cusp key there, P has no node version).
        // (Not while a curve is drawn: C there makes a cusp.)
        if nodes_chosen && self.node_selection.len() >= 2 && self.curve.is_none() {
            use crate::ops::Align;
            for (k, a) in [
                (Key::L, Align::Left),
                (Key::R, Align::Right),
                (Key::T, Align::Top),
                (Key::B, Align::Bottom),
                (Key::E, Align::CenterH),
            ] {
                if pressed(k, Modifiers::NONE) && self.align_nodes_to(a) {
                    return;
                }
            }
        }
        if !self.selection.is_empty() && !nodes_chosen && self.curve.is_none() {
            use crate::ops::Align;
            for (k, a) in [
                (Key::L, Align::Left),
                (Key::R, Align::Right),
                (Key::T, Align::Top),
                (Key::B, Align::Bottom),
                (Key::E, Align::CenterH),
                (Key::C, Align::CenterV),
                (Key::P, Align::CenterPage),
            ] {
                if pressed(k, Modifiers::NONE) {
                    self.align(a);
                    return;
                }
            }
            // Distribute: Shift and the letter that aligns the same way,
            // Shift+P and Shift+A for equal gaps across and down (#2147).
            // The Distribute commands had no keys at all, so Shift+E and
            // Shift+C did nothing.
            for (k, d) in App::DISTRIBUTE_KEYS {
                if pressed(k, Modifiers::SHIFT) {
                    self.distribute(d);
                    return;
                }
            }
        }
        // Tool shortcuts: user overrides from Options, then the defaults.
        for group in crate::tools::GROUPS {
            for t in group.tools {
                // Every key the tool has (#822).
                let labels =
                    crate::shortcuts::keys_for(&self.settings.shortcuts, t.id(), t.shortcut());
                for label in labels {
                    // Any of Ctrl, Alt and Shift with a key, as menu commands.
                    let keys = crate::shortcuts::parse(&label).or_else(|| {
                        Tool::parse_shortcut(&label).map(|(k, shift)| {
                            let mods = if shift {
                                Modifiers::SHIFT
                            } else {
                                Modifiers::NONE
                            };
                            (mods, k)
                        })
                    });
                    if let Some((mods, k)) = keys {
                        // W and K during a move say what a drop on a clip
                        // frame does; they pick no tool then.
                        let drop_key = matches!(self.drag, Drag::Move { .. })
                            && mods.is_none()
                            && matches!(k, Key::W | Key::K);
                        // C and V while a B-spline is drawn make the next
                        // point fixed (#2060); they pick no tool then.
                        let drop_key = drop_key
                            || (self.curve.as_ref().is_some_and(|c| c.bspline)
                                && mods.is_none()
                                && matches!(k, Key::C | Key::V));
                        // A key held picks its tool once: the presses it
                        // repeats would send Space back and forth between
                        // the Pick tool and the tool before it (#1502).
                        if first_pressed(k, mods) && !drop_key {
                            if *t == Tool::Pick && pointer_busy {
                                continue;
                            }
                            if self.pick_tool_by_key(*t, pointer_busy) {
                                return;
                            }
                        }
                    }
                }
            }
        }
        // Arrow nudge: plain, Shift super nudge, Ctrl micro nudge
        // (Document Setup > Rulers).
        let mut d = Vec2::ZERO;
        for (key, mods) in &presses {
            let step = if mods.matches_exact(Modifiers::NONE) {
                self.nudge_mm
            } else if mods.matches_exact(Modifiers::SHIFT) {
                self.settings.super_nudge_mm.max(1e-6)
            } else if mods.matches_exact(Modifiers::COMMAND) {
                self.settings.micro_nudge_mm.max(1e-6)
            } else {
                continue;
            };
            match key {
                Key::ArrowLeft => d.x -= step,
                Key::ArrowRight => d.x += step,
                Key::ArrowUp => d.y += step,
                Key::ArrowDown => d.y -= step,
                _ => {}
            }
        }
        if d.hypot() > 0.0 {
            // With the Shape tool and nodes chosen, the nodes move.
            if self.tool == Tool::Shape && !self.node_selection.is_empty() {
                self.nudge_nodes(d);
            } else {
                self.nudge(d);
            }
        }
    }
}
