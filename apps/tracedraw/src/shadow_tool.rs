//! Drop Shadow tool: the drag that makes a flat or perspective shadow,
//! the handles on the canvas (start, end with the shadow colour, and the
//! opacity and feathering sliders on either side of the line between
//! them), presets and Break Drop Shadow Apart.
//! See docs/behavior/drop-shadow.md.

use crate::app::App;
use crate::i18n::tr;
use tracedraw_core::{
    geometry::{Point, Rect, Vec2},
    live::{Effect, MergeMode},
    shadow::{edge_near, ShadowEdge},
    Color, Command, Fill, Shadow, Shape, ShapeId, ShapeKind,
};

/// Which shadow handle a drag holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowHandle {
    /// Where the shadow starts: the object's centre or the edge it stands on.
    Start,
    /// Where it goes to; colours dropped on it become the shadow colour.
    End,
    /// The opacity slider on one side of the line between them.
    Opacity,
    /// The feathering slider on the other side of the line.
    Feather,
}

/// How near (screen pixels) a press takes a shadow handle: more than the
/// drawn square, so it is easy to grab at any zoom (#1323).
pub const HANDLE_PX: f64 = 11.0;
/// The side of a drawn shadow handle, in screen pixels.
pub const HANDLE_SIDE_PX: f32 = 12.0;

/// The resolution Break Drop Shadow Apart makes the bitmap at.
pub const BREAK_APART_DPI: f64 = 150.0;

/// How far the opacity and feathering sliders stand off the line, to
/// the middle of each, screen pixels: the opacity slider on the left of
/// the line looking from the start to the end, the feathering one on
/// the right.
pub const SLIDER_OFFSET_PX: f32 = 12.0;
/// The feathering the slider gives at the end of the line, millimetres
/// (the top of the property bar's range).
pub const FEATHER_MAX_MM: f64 = 50.0;

/// Where along the line, from the start (0) to the end (1), the
/// feathering slider stands for feathering `mm`: the square root of its
/// share of [`FEATHER_MAX_MM`], so the small values most shadows use
/// get most of the line (1.5 mm is at about a sixth, 12.5 mm half way).
pub fn feather_fraction(mm: f64) -> f64 {
    let mm = if mm.is_finite() { mm } else { 0.0 };
    (mm / FEATHER_MAX_MM).clamp(0.0, 1.0).sqrt()
}

/// The feathering a slider at `t` along the line gives, to a tenth of a
/// millimetre (the inverse of [`feather_fraction`]).
pub fn feather_at_fraction(t: f64) -> f64 {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (t * t * FEATHER_MAX_MM * 10.0).round() / 10.0
}

/// The handles of `sh` on an object with page path bounds `b`, end first
/// (it wins when they overlap). The sliders are given at their points on
/// the line; they are drawn and taken beside it
/// ([`App::shown_shadow_handles`]).
pub fn shadow_handles(sh: &Shadow, b: Rect) -> [(ShadowHandle, Point); 4] {
    let a = sh.anchor(b);
    let e = sh.end_point(b);
    on_line(a, e, sh)
}

/// The handles for a line from `a` to `e`, the sliders on the line.
fn on_line(a: Point, e: Point, sh: &Shadow) -> [(ShadowHandle, Point); 4] {
    let t = if sh.opacity.is_finite() {
        sh.opacity.clamp(0.0, 1.0)
    } else {
        0.0
    };
    [
        (ShadowHandle::End, e),
        (ShadowHandle::Opacity, a + (e - a) * t),
        (
            ShadowHandle::Feather,
            a + (e - a) * feather_fraction(sh.blur),
        ),
        (ShadowHandle::Start, a),
    ]
}

/// How near (screen pixels) a press must be to a slider's tab to take it.
pub const SLIDER_REACH_PX: f32 = 6.0;

/// The distance from `p` to the segment from `a` to `b`, screen space.
fn segment_distance(a: egui::Pos2, b: egui::Pos2, p: egui::Pos2) -> f32 {
    let d = b - a;
    let len2 = d.length_sq();
    let t = if len2 > 1e-6 {
        ((p - a).dot(d) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + d * t - p).length()
}

/// The screen-space step from the line to the middle of a slider: to
/// the left of the line (looking from `a` to `e`, on screen) for the
/// opacity slider, to the right for the feathering one, nothing for the
/// squares.
pub fn slider_offset(h: ShadowHandle, a: egui::Pos2, e: egui::Pos2) -> egui::Vec2 {
    let d = e - a;
    let len = d.length();
    // Straight up (opacity) and down when the line has no length.
    let left = if len < 1e-3 {
        egui::vec2(0.0, -1.0)
    } else {
        egui::vec2(d.y, -d.x) / len
    };
    match h {
        ShadowHandle::Opacity => left * SLIDER_OFFSET_PX,
        ShadowHandle::Feather => -left * SLIDER_OFFSET_PX,
        ShadowHandle::Start | ShadowHandle::End => egui::Vec2::ZERO,
    }
}

/// The bounds the shadow geometry is measured on: the object's outline
/// as it is drawn, an envelope or another warp taken in, which is what
/// the shadow is drawn from (#2217).
pub fn shadow_box(s: &Shape) -> Rect {
    s.shadow_box()
}

/// `p` (page space) lies on the drop shadow of `s`: inside its
/// silhouette moved or sheared as the shadow is drawn, the feathered
/// fringe left out. Text, groups, tables, bitmaps, clip frames and
/// symbols cast it from anywhere in their box, as they are picked
/// themselves. A shadow with no opacity is not there to pick.
pub fn shadow_covers(s: &Shape, p: Point) -> bool {
    use tracedraw_core::geometry::Shape as _;
    let Some(sh) = s.shadow.filter(|sh| sh.opacity > 0.0) else {
        return false;
    };
    let path = s.shadow_outline();
    let b = path.bounding_box();
    if !(Shadow { blur: 0.0, ..sh }).bounds(b).contains(p) {
        return false;
    }
    let t = sh.silhouette_transform(b);
    let boxed = matches!(
        s.kind,
        ShapeKind::Text { .. }
            | ShapeKind::Group { .. }
            | ShapeKind::Table(_)
            | ShapeKind::Bitmap { .. }
            | ShapeKind::ClipFrame { .. }
            | ShapeKind::SymbolInstance { .. }
    );
    if boxed {
        (t * b.to_path(0.1)).contains(p)
    } else {
        (t * path).contains(p)
    }
}

/// The outline of `s` as it is drawn goes round `p`: one of its parts
/// alone holds it, so the hole of a ring and the inside of an unfilled
/// shape count.
fn outline_goes_round(s: &Shape, p: Point) -> bool {
    use tracedraw_core::geometry::Shape as _;
    let drawn = if s.effects.is_empty() {
        s.page_path()
    } else {
        tracedraw_core::live::evaluate(s).main.page_path()
    };
    tracedraw_core::effects::subpaths(&drawn)
        .iter()
        .any(|(part, _)| part.contains(p))
}

impl App {
    /// The object whose drop shadow is chosen on its own (#2545): its
    /// shadow was clicked with the Pick tool, it alone is selected and it
    /// still casts a shadow.
    pub fn picked_shadow(&self) -> Option<ShapeId> {
        let id = self.shadow_picked?;
        if self.selection.as_slice() != [id] {
            return None;
        }
        let s = self.doc().find_shape(id)?;
        s.shadow.is_some_and(|sh| sh.opacity > 0.0).then_some(id)
    }

    /// The box the chosen drop shadow covers on the page, its feathered
    /// edge included (#2545).
    pub fn picked_shadow_bounds(&self) -> Option<Rect> {
        let id = self.picked_shadow()?;
        let s = self.doc().shape_on_page(id)?;
        let sh = s.shadow?;
        let b = shadow_box(&s);
        let r = sh.bounds(b);
        (r.width().is_finite() && r.height().is_finite()).then_some(r)
    }

    /// A click with the Pick tool on the drop shadow of `id` (not on the
    /// object itself) chooses the shadow alone: it shows its own box, and
    /// Delete takes the shadow away and leaves the object (#2545).
    pub fn pick_shadow(&mut self, id: ShapeId) {
        self.select(vec![id]);
        if self.selection.as_slice() == [id] {
            self.shadow_picked = Some(id);
        }
    }

    /// Delete with a shadow chosen on its own: the shadow goes, its
    /// object stays selected. False when no shadow is chosen.
    pub fn delete_picked_shadow(&mut self) -> bool {
        let Some(id) = self.picked_shadow() else {
            return false;
        };
        self.run(Command::SetShadow {
            shapes: vec![id],
            shadow: None,
        });
        self.shadow_picked = None;
        true
    }

    /// The object a drag with the Drop Shadow or the Block Shadow tool
    /// starts on at `p`: the one picked there, else the topmost one whose
    /// outline goes round `p`. A press in the hole of a ring, or in the
    /// middle of a shape without a fill, started nothing, so such an
    /// object took a shadow only from a press on its outline or its
    /// narrow filled part (#2219).
    pub fn shadow_drag_target(&self, p: Point) -> Option<ShapeId> {
        if let Some(id) = self.hit_test(p) {
            return Some(id);
        }
        if self.focus_shape_on_page().is_some() {
            return None;
        }
        self.page_layers()
            .into_iter()
            .rev()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.shapes.iter().rev())
            .filter(|s| !s.locked && s.visible && !self.outside_clip_edit(s.id))
            .find(|s| self.visual_bounds(s).contains(p) && outline_goes_round(s, p))
            .map(|s| s.id)
    }

    /// The object a Drop Shadow drag starts on at `p`, with the edge it
    /// stands the shadow on (`None` for a flat shadow from the middle):
    /// the object under `p` ([`Self::shadow_drag_target`]), else the
    /// topmost object `p` lies just outside of, within a handle's reach
    /// of its box, which takes a perspective shadow standing on the edge
    /// `p` is next to. A press on an edge itself, or a hair beyond it,
    /// started nothing before, so a shadow from an edge could only be
    /// begun from inside the object's outer fifth.
    pub fn shadow_drag_start(&self, p: Point) -> Option<(ShapeId, Option<ShadowEdge>)> {
        let edge_of = |id: ShapeId| {
            let s = self.doc().shape_on_page(id)?;
            Some((id, edge_near(shadow_box(&s), p)))
        };
        if let Some(id) = self.shadow_drag_target(p) {
            return edge_of(id);
        }
        if self.focus_shape_on_page().is_some() {
            return None;
        }
        let tol = HANDLE_PX / f64::from(self.view.zoom.max(1e-6));
        self.page_layers()
            .into_iter()
            .rev()
            .filter(|l| l.visible && !l.locked)
            .flat_map(|l| l.shapes.iter().rev())
            .filter(|s| !s.locked && s.visible && !self.outside_clip_edit(s.id))
            .filter(|s| self.visual_bounds(s).inflate(tol, tol).contains(p))
            .find(|s| {
                let b = shadow_box(s);
                b.width() > 1e-9
                    && b.height() > 1e-9
                    && !b.contains(p)
                    && b.inflate(tol, tol).contains(p)
            })
            .and_then(|s| edge_of(s.id))
    }

    /// Where a Drop Shadow drag from the hovered point `p` would stand
    /// the shadow, for the hint drawn on the canvas: the object under `p`
    /// or just outside it, and its edge. Nothing over a shadow handle, the
    /// shadow's line or the shadow itself (a drag there edits the shadow),
    /// nor in the middle of an object (a flat shadow).
    pub fn shadow_edge_hint(&self, p: Point) -> Option<(Rect, ShadowEdge)> {
        if self.tool != crate::tools::Tool::DropShadow
            || self.shadow_handle_at(p).is_some()
            || self.shadow_on_line(p)
        {
            return None;
        }
        let picked = self.hit_test(p);
        if picked.is_none() && self.pick_at(p).is_some() {
            return None;
        }
        let (id, edge) = match picked {
            Some(id) => {
                let s = self.doc().shape_on_page(id)?;
                (id, edge_near(shadow_box(&s), p)?)
            }
            None => {
                let (id, edge) = self.shadow_drag_start(p)?;
                (id, edge?)
            }
        };
        let s = self.doc().shape_on_page(id)?;
        Some((shadow_box(&s), edge))
    }

    /// The shadow the Drop Shadow tool edits: the first selected object
    /// that has one, with its id and box. Its handles show and the
    /// property bar shows its settings, also with objects of no shadow
    /// picked before it.
    pub fn selected_shadow(&self) -> Option<(ShapeId, Shadow, Rect)> {
        self.selected_rc()
            .iter()
            .find_map(|s| Some((s.id, s.shadow?, shadow_box(s))))
    }

    /// The settings the Drop Shadow property bar shows: the selected
    /// shadow's (see [`Self::selected_shadow`]), else those new shadows
    /// get.
    pub fn shadow_bar_shadow(&self) -> Shadow {
        self.selected_shadow()
            .map_or(self.shadow_default, |(_, sh, _)| sh)
    }

    /// Edit Shadow can act: a selected object has a drop shadow.
    pub fn can_edit_shadow(&self) -> bool {
        self.selected_shadow().is_some()
    }

    /// Edit Shadow (the object's right-click menu, the Properties
    /// docker, a double click on a shadow): the Drop Shadow tool on the
    /// first selected object with a shadow, that object alone selected,
    /// with its handles on the canvas and its settings on the property
    /// bar.
    pub fn edit_shadow(&mut self) {
        let Some((id, _, _)) = self.selected_shadow() else {
            return;
        };
        self.set_tool(crate::tools::Tool::DropShadow);
        self.select(vec![id]);
    }

    /// The handles of the selected shadow where they are drawn, end
    /// first, with the Drop Shadow tool: the squares at the start and the
    /// end, the opacity slider beside the line on one side and the
    /// feathering slider on the other. While the start handle is dragged
    /// it is drawn under the pointer, in every mode (the shadow itself
    /// stands on the edge it is over, or lies flat in the middle), with
    /// the sliders on the line from there; let go, it is back where the
    /// shadow starts.
    pub fn shown_shadow_handles(&self) -> Option<[(ShadowHandle, Point); 4]> {
        if self.tool != crate::tools::Tool::DropShadow {
            return None;
        }
        let (_, sh, b) = self.selected_shadow()?;
        let e = sh.end_point(b);
        let mut a = sh.anchor(b);
        let mut hs = if let (
            crate::app::Drag::ShadowHandle {
                handle: ShadowHandle::Start,
                grab,
                ..
            },
            Some(at),
        ) = (&self.drag, self.pointer_page)
        {
            a = at + *grab;
            on_line(a, e, &sh)
        } else {
            shadow_handles(&sh, b)
        };
        let view = &self.view;
        let (sa, se) = (view.to_screen(a), view.to_screen(e));
        for (h, p) in &mut hs {
            let off = slider_offset(*h, sa, se);
            if off != egui::Vec2::ZERO {
                *p = view.to_page(view.to_screen(*p) + off);
            }
        }
        Some(hs)
    }

    /// The shadow handle under page point `p`, with the Drop Shadow tool:
    /// the nearest one within reach of `p` (the end first where two lie
    /// on one spot), so each handle is taken by a press on it however
    /// close the others lie, flat or standing on an edge. The sliders are
    /// taken where they are drawn, beside the line.
    pub fn shadow_handle_at(&self, p: Point) -> Option<ShadowHandle> {
        let shown = self.shown_shadow_handles()?;
        let view = &self.view;
        let at = view.to_screen(p);
        let screen = |h: ShadowHandle| {
            shown
                .iter()
                .find(|(k, _)| *k == h)
                .map(|(_, q)| view.to_screen(*q))
        };
        let (a, e) = (screen(ShadowHandle::Start)?, screen(ShadowHandle::End)?);
        shown
            .iter()
            .filter_map(|(h, q)| {
                let c = view.to_screen(*q);
                let off = slider_offset(*h, a, e);
                if off == egui::Vec2::ZERO {
                    let d = (c - at).length();
                    (d <= HANDLE_PX as f32).then_some((*h, d))
                } else {
                    // A slider is taken anywhere on its tab, from its foot
                    // on the line to its outer end.
                    let d = segment_distance(c - off, c + off, at);
                    (d <= SLIDER_REACH_PX).then_some((*h, d))
                }
            })
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(h, _)| h)
    }

    /// A press at `p` on an object's drop shadow away from the object
    /// itself (Drop Shadow tool): the object becomes the selection, and
    /// what is returned is the way from `p` to the shadow's end, so that
    /// dragging the end by it moves the whole shadow with the pointer
    /// (#935). `None` anywhere else.
    pub fn grab_shadow(&mut self, p: Point) -> Option<Vec2> {
        let id = self.pick_at(p)?;
        if self.hit_test(p) == Some(id) {
            return None;
        }
        self.select(vec![id]);
        let (_, sh, b) = self.selected_shadow()?;
        Some(sh.end_point(b) - p)
    }

    /// A drag on `shape` started at `start` and is now at `p`. The first
    /// frame (`first`) picks a flat shadow or, next to an edge, a
    /// perspective one; later frames move its end with the pointer.
    pub fn drag_new_shadow(&mut self, shape: ShapeId, start: Point, p: Point, first: bool) {
        let Some(s) = self.doc().shape_on_page(shape) else {
            return;
        };
        let b = shadow_box(&s);
        let base = s.shadow.unwrap_or(self.shadow_default);
        let mut sh = if first {
            Shadow::for_drag_start(&base, b, start)
        } else {
            base
        };
        if sh.perspective.is_some() {
            sh.set_end_point(b, p);
        } else {
            sh.offset = p - start;
        }
        // The other selected objects take the same shadow (#2218).
        if !self.selection.contains(&shape) {
            self.select(vec![shape]);
        }
        self.set_shadow(Some(sh), true);
    }

    /// The selected shadow's line from its start to its end passes within
    /// the pick distance of `p`, away from its handles (Drop Shadow tool):
    /// a drag from there moves the shadow by its line.
    pub fn shadow_on_line(&self, p: Point) -> bool {
        if self.tool != crate::tools::Tool::DropShadow || self.shadow_handle_at(p).is_some() {
            return false;
        }
        let Some((_, sh, b)) = self.selected_shadow() else {
            return false;
        };
        crate::effect_line::near_segment(sh.anchor(b), sh.end_point(b), p, self.line_pick_tol())
    }

    /// A drag on the shadow's line, pressed at `press` when the shadow's
    /// end was at `end`, is at `p`: the end moves by the pointer's way
    /// (kept straight with `ctrl`), so the whole shadow follows; its start
    /// stays where it stands on the object. The first move of a drag is an
    /// undo step "Drop Shadow", the next ones replace it (`begun`).
    pub fn drag_shadow_line(
        &mut self,
        press: Point,
        end: Point,
        p: Point,
        ctrl: bool,
        begun: bool,
    ) {
        let d = crate::effect_line::constrain_move(p - press, ctrl);
        self.drag_shadow_handle(ShadowHandle::End, end + d, begun);
    }

    /// Move a shadow handle of the selected shadow to `p`: a new undo step
    /// "Drop Shadow" at the first move of a drag, replaced by the next
    /// ones (`begun`), so a drag is one step.
    pub fn drag_shadow_handle(&mut self, handle: ShadowHandle, p: Point, begun: bool) {
        let Some((_, mut sh, b)) = self.selected_shadow() else {
            return;
        };
        match handle {
            ShadowHandle::End => sh.set_end_point(b, p),
            ShadowHandle::Opacity | ShadowHandle::Feather => {
                // How far along the line the pointer is, from the start
                // (0) to the end (1); beside the line counts as on it.
                let a = sh.anchor(b);
                let d = sh.end_point(b) - a;
                let len2 = d.hypot2();
                if len2 > 1e-12 {
                    let t = ((p - a).dot(d) / len2).clamp(0.0, 1.0);
                    if handle == ShadowHandle::Opacity {
                        sh.opacity = (t * 100.0).round() / 100.0;
                    } else {
                        sh.blur = feather_at_fraction(t);
                    }
                }
            }
            ShadowHandle::Start => {
                // Moving the start to another edge stands the shadow there;
                // to the middle makes it flat.
                let end = sh.end_point(b);
                match edge_near(b, p) {
                    Some(e) if sh.perspective.map(|q| q.edge) != Some(e) => {
                        let mut per = sh
                            .perspective
                            .unwrap_or_else(|| tracedraw_core::shadow::Perspective::new(e));
                        per.edge = e;
                        sh.perspective = Some(per);
                        sh.set_end_point(b, end);
                    }
                    None if sh.perspective.is_some() => {
                        sh.perspective = None;
                        sh.set_end_point(b, end);
                    }
                    _ => {}
                }
            }
        }
        self.shadow_default = sh;
        self.set_shadow(Some(sh), begun);
    }

    /// A field of the Drop Shadow property bar changed to `sh`: it becomes
    /// the default for new shadows and goes to the selection, also when
    /// the selection has no shadow yet (`had_shadow` false), so Flat, an
    /// edge or an offset chosen there applies at once instead of only
    /// after Apply (#1323). Typing in a field again merges into the same
    /// undo step.
    pub fn shadow_bar_changed(&mut self, sh: Shadow, had_shadow: bool) {
        self.shadow_default = sh;
        if !self.selection.is_empty() {
            self.set_shadow(Some(sh), had_shadow);
        }
    }

    /// A colour dropped on the end handle: the shadow takes it (one step).
    pub fn set_shadow_color(&mut self, shape: ShapeId, color: Color) {
        let Some(mut sh) = self.doc().find_shape(shape).and_then(|s| s.shadow) else {
            return;
        };
        sh.color = color;
        self.run(Command::SetShadow {
            shapes: vec![shape],
            shadow: Some(sh),
        });
    }

    /// Apply built-in preset `id` to the selection's shadows (and make it
    /// the default for new ones), keeping each shadow's colour.
    pub fn apply_shadow_preset(&mut self, id: &str) {
        let Some((_, preset)) = Shadow::presets().into_iter().find(|(k, _)| *k == id) else {
            return;
        };
        let color = self
            .selected_shadow()
            .map(|(_, s, _)| s.color)
            .unwrap_or(self.shadow_default.color);
        let sh = Shadow { color, ..preset };
        self.shadow_default = sh;
        self.set_shadow(Some(sh), false);
    }

    /// True when Break Apart would break a drop shadow off.
    pub fn can_break_shadow_apart(&self) -> bool {
        self.selection.iter().any(|id| {
            self.doc()
                .shape(*id)
                .is_ok_and(|(_, s)| s.shadow.is_some_and(|sh| sh.opacity > 0.0))
        })
    }

    /// Break Drop Shadow Apart: each selected object's shadow becomes a
    /// bitmap of its own just behind the object (merged as the shadow
    /// was), and the object loses its shadow, in one undo step. The new
    /// bitmaps come out selected. Objects inside groups are left alone.
    pub fn break_shadow_apart(&mut self) {
        let mut cmds = Vec::new();
        let mut made = Vec::new();
        for id in self.selection.clone() {
            let Ok((layer, s)) = self.doc().shape(id) else {
                continue;
            };
            let (layer, s) = (layer.id, s.clone());
            if s.shadow.is_none() {
                continue;
            }
            let Some(bitmap) = self.shadow_bitmap(&s) else {
                continue;
            };
            let new_id = bitmap.id;
            cmds.push(Command::AddShape {
                layer,
                shape: bitmap,
            });
            cmds.push(Command::OrderRelative {
                shapes: vec![new_id],
                reference: id,
                in_front: false,
            });
            cmds.push(Command::SetShadow {
                shapes: vec![id],
                shadow: None,
            });
            made.push(new_id);
        }
        if cmds.is_empty() {
            return;
        }
        match self.engine.run_batch("Break Drop Shadow Apart", &cmds) {
            Ok(()) => self.select(made),
            Err(e) => self.status = e.to_string(),
        }
    }

    /// The bitmap object that stands for the shadow of `s`.
    fn shadow_bitmap(&mut self, s: &Shape) -> Option<Shape> {
        let sh = s.shadow?;
        let (rect, pm) = tracedraw_render::render_shadow_image(s, BREAK_APART_DPI)?;
        let (w, h) = (pm.width(), pm.height());
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for p in pm.pixels() {
            let c = p.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        let img = image::RgbaImage::from_raw(w, h, rgba)?;
        let mut png = Vec::new();
        if let Err(e) = image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        {
            self.status =
                crate::i18n::trf("status.could_not_encode_image", &[("e", &e.to_string())]);
            return None;
        }
        let mut shape = Shape::new(
            self.engine.new_shape_id(),
            ShapeKind::Bitmap {
                rect,
                width_px: w,
                height_px: h,
                png: png.into(),
                fx: None,
            },
        );
        shape.fill = Fill::None;
        shape.stroke = None;
        shape.name = Some(tr("shadow.broken_name"));
        if sh.merge != MergeMode::Normal {
            // An opaque transparency that only carries the merge mode.
            shape.effects = vec![Effect::Transparency {
                mask: Fill::Solid(Color::WHITE),
                merge: sh.merge,
                target: 2,
            }];
        }
        Some(shape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_rect() -> (App, ShapeId) {
        let mut app = App::headless();
        app.canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let page = app.page_rect();
        app.view.fit(page, app.canvas_rect);
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(20.0, 20.0, 60.0, 80.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![id]);
        app.apply_fill(Fill::Solid(Color::rgb8(0, 0, 255)));
        app.tool = crate::tools::Tool::DropShadow;
        (app, id)
    }

    fn shadow(app: &App, id: ShapeId) -> Option<Shadow> {
        app.doc().find_shape(id).and_then(|s| s.shadow)
    }

    /// #2217: the shadow of an object with an envelope is measured on the
    /// outline the envelope draws, as the renderer casts it: a drag from
    /// the middle of what shows gives a flat shadow, the handles sit on
    /// the drawn object and the shadow is picked where it lies. Before,
    /// all of it went by the outline without the envelope, so a press on
    /// the bent object stood a perspective shadow on a far edge.
    #[test]
    fn the_shadow_of_an_enveloped_object_goes_by_what_is_drawn() {
        use tracedraw_core::live::Effect;
        let (mut app, id) = app_with_rect();
        // The envelope stretches the 40 x 60 rectangle to three times its
        // height: it shows from 20 to 200.
        let drawn = Rect::new(20.0, 20.0, 60.0, 200.0);
        app.push_effect(Effect::envelope_around(drawn, false), true);
        let s = app.doc().shape_on_page(id).expect("shape");
        let b = shadow_box(&s);
        assert!(
            (b.y0 - 20.0).abs() < 0.1 && (b.y1 - 200.0).abs() < 0.1 && (b.x1 - 60.0).abs() < 0.1,
            "{b:?}"
        );
        // The middle of the drawn object lies above the plain outline.
        let mid = Point::new(40.0, 110.0);
        assert_eq!(app.hit_test(mid), Some(id));
        app.drag_new_shadow(id, mid, mid, true);
        app.drag_new_shadow(id, mid, Point::new(46.0, 104.0), false);
        let sh = shadow(&app, id).expect("shadow");
        assert!(sh.perspective.is_none(), "{:?}", sh.perspective);
        assert!((sh.offset.x - 6.0).abs() < 1e-9 && (sh.offset.y + 6.0).abs() < 1e-9);
        // The handles start at the middle of the drawn object.
        let hs = app.shown_shadow_handles().expect("handles");
        let start = hs
            .iter()
            .find(|(h, _)| *h == ShadowHandle::Start)
            .map(|(_, p)| *p)
            .expect("start");
        assert!((start - Point::new(40.0, 110.0)).hypot() < 0.1, "{start:?}");
        // The shadow is picked where it is drawn: beside the top of the
        // bent object, far above the plain outline.
        let s = app.doc().shape_on_page(id).expect("shape");
        assert!(shadow_covers(&s, Point::new(63.0, 150.0)));
        assert!(!shadow_covers(&s, Point::new(70.0, 150.0)));
        // A drag from the bottom fifth of what shows stands the shadow on
        // the drawn bottom edge and reaches the pointer.
        app.undo();
        let low = Point::new(40.0, 30.0);
        app.drag_new_shadow(id, low, low, true);
        app.drag_new_shadow(id, low, Point::new(120.0, 60.0), false);
        let sh = shadow(&app, id).expect("shadow");
        assert_eq!(sh.perspective.map(|p| p.edge), Some(ShadowEdge::Bottom));
        let s = app.doc().shape_on_page(id).expect("shape");
        let end = sh.end_point(shadow_box(&s));
        assert!((end - Point::new(120.0, 60.0)).hypot() < 1e-6, "{end:?}");
    }

    #[test]
    fn a_drag_from_the_bottom_edge_makes_a_perspective_shadow_that_multiplies() {
        let (mut app, id) = app_with_rect();
        let start = Point::new(40.0, 22.0);
        app.drag_new_shadow(id, start, Point::new(41.0, 23.0), true);
        app.drag_new_shadow(id, start, Point::new(90.0, 40.0), false);
        let sh = shadow(&app, id).expect("shadow");
        assert_eq!(sh.merge, MergeMode::Multiply);
        let per = sh.perspective.expect("perspective");
        assert_eq!(per.edge, ShadowEdge::Bottom);
        let b = Rect::new(20.0, 20.0, 60.0, 80.0);
        assert!((sh.end_point(b) - Point::new(90.0, 40.0)).hypot() < 1e-9);
        // One drag, one undo step.
        app.undo();
        assert!(shadow(&app, id).is_none());
        // From the middle: flat, moved by the drag.
        let mid = Point::new(40.0, 50.0);
        app.drag_new_shadow(id, mid, mid, true);
        app.drag_new_shadow(id, mid, Point::new(43.0, 47.0), false);
        let sh = shadow(&app, id).expect("shadow");
        assert!(sh.perspective.is_none());
        assert!((sh.offset.x - 3.0).abs() < 1e-9 && (sh.offset.y + 3.0).abs() < 1e-9);
    }

    /// A rectangle with an ellipse placed inside it: a drag on it with
    /// the Drop Shadow tool gives the clip frame its shadow, which draws
    /// beside the frame with its contents (#873).
    #[test]
    fn a_clip_frame_takes_a_shadow_that_shows() {
        let (mut app, rect) = app_with_rect();
        let ellipse = app
            .new_shape(ShapeKind::Ellipse {
                rect: Rect::new(30.0, 30.0, 50.0, 70.0),
                arc: None,
            })
            .expect("ellipse");
        app.select(vec![ellipse]);
        app.apply_fill(Fill::Solid(Color::rgb8(255, 0, 0)));
        app.place_inside(vec![ellipse], rect);
        app.tool = crate::tools::Tool::DropShadow;
        let mid = Point::new(40.0, 50.0);
        let hit = app.hit_test(mid).expect("the clip frame");
        assert!(matches!(
            app.doc().find_shape(hit).map(|s| &s.kind),
            Some(ShapeKind::ClipFrame { .. })
        ));
        app.drag_new_shadow(hit, mid, mid, true);
        app.drag_new_shadow(hit, mid, Point::new(70.0, 50.0), false);
        let sh = shadow(&app, hit).expect("shadow");
        assert!((sh.offset.x - 30.0).abs() < 1e-9);
        // Drawn: right of the frame, where only its shadow falls.
        let dpi = 25.4;
        let pm = tracedraw_render::render_page_image(app.doc(), app.page, dpi).expect("page");
        let page = app.page_rect();
        let px = |x: f64, y: f64| {
            pm.pixel((x - page.x0) as u32, (page.y1 - y) as u32)
                .expect("pixel")
                .demultiply()
        };
        let under = px(85.0, 50.0);
        assert!(under.red() < 200, "{under:?}");
        let clear = px(120.0, 50.0);
        assert!(clear.red() > 240, "{clear:?}");
    }

    #[test]
    fn handles_move_the_end_set_the_opacity_and_take_dropped_colours() {
        let (mut app, id) = app_with_rect();
        app.set_shadow(Some(Shadow::default()), false);
        let b = Rect::new(20.0, 20.0, 60.0, 80.0);
        let sh = shadow(&app, id).expect("shadow");
        let [(_, end), (_, slider), _, (_, start)] = shadow_handles(&sh, b);
        assert_eq!(start, b.center());
        assert_eq!(app.shadow_handle_at(end), Some(ShadowHandle::End));
        app.drag_shadow_handle(ShadowHandle::End, Point::new(50.0, 40.0), false);
        let sh = shadow(&app, id).expect("shadow");
        assert!((sh.offset.x - 10.0).abs() < 1e-9 && (sh.offset.y + 10.0).abs() < 1e-9);
        let _ = slider;
        // The slider three quarters of the way along: 75 %.
        let a = sh.anchor(b);
        let e = sh.end_point(b);
        app.drag_shadow_handle(ShadowHandle::Opacity, a + (e - a) * 0.75, false);
        assert!((shadow(&app, id).expect("shadow").opacity - 0.75).abs() < 1e-9);
        // The start moved to the top edge stands the shadow there.
        app.drag_shadow_handle(ShadowHandle::Start, Point::new(40.0, 79.0), false);
        let per = shadow(&app, id)
            .and_then(|s| s.perspective)
            .expect("perspective");
        assert_eq!(per.edge, ShadowEdge::Top);
        // A colour dropped on the end handle colours the shadow.
        let sh = shadow(&app, id).expect("shadow");
        let at = app.view.to_screen(sh.end_point(b));
        let red = Color::rgb8(200, 0, 0);
        assert!(app.drop_color(Some(red), at));
        assert_eq!(shadow(&app, id).map(|s| s.color), Some(red));
        app.undo();
        assert_eq!(shadow(&app, id).map(|s| s.color), Some(Color::BLACK));
    }

    /// The opacity and feathering sliders stand beside the line between
    /// the start and the end, one on each side, a tab each that is taken
    /// where it is drawn; the feathering one sets the feathering by how
    /// far along the line it is dragged (a quarter of the most at half
    /// way), the opacity one the opacity, and neither touches the other.
    #[test]
    fn the_opacity_and_feathering_sliders_lie_on_either_side_of_the_line() {
        let (mut app, id) = app_with_rect();
        let sh = Shadow {
            offset: Vec2::new(20.0, -20.0),
            opacity: 0.5,
            blur: 1.5,
            ..Shadow::default()
        };
        app.set_shadow(Some(sh), false);
        let b = Rect::new(20.0, 20.0, 60.0, 80.0);
        let shown = app.shown_shadow_handles().expect("handles");
        let at = |h: ShadowHandle| shown.iter().find(|(k, _)| *k == h).expect("handle").1;
        let (a, e) = (
            app.view.to_screen(sh.anchor(b)),
            app.view.to_screen(sh.end_point(b)),
        );
        let d = (e - a).normalized();
        // Signed distance from the line on screen, + to the left.
        let side = |p: Point| {
            let v = app.view.to_screen(p) - a;
            d.x * v.y - d.y * v.x
        };
        let (o, f) = (at(ShadowHandle::Opacity), at(ShadowHandle::Feather));
        assert!((side(o) + SLIDER_OFFSET_PX).abs() < 0.01, "{}", side(o));
        assert!((side(f) - SLIDER_OFFSET_PX).abs() < 0.01, "{}", side(f));
        assert_eq!(app.shadow_handle_at(o), Some(ShadowHandle::Opacity));
        assert_eq!(app.shadow_handle_at(f), Some(ShadowHandle::Feather));
        // Feathering 1.5 mm stands at about a sixth of the line.
        assert!((feather_fraction(1.5) - 0.1732).abs() < 1e-3);
        assert_eq!(feather_at_fraction(feather_fraction(1.5)), 1.5);
        assert_eq!(feather_fraction(f64::NAN), 0.0);
        assert_eq!(feather_at_fraction(f64::INFINITY), 0.0);
        // Half way along the line (beside it): 12.5 mm.
        let (pa, pe) = (sh.anchor(b), sh.end_point(b));
        let half = pa + (pe - pa) * 0.5;
        app.drag_shadow_handle(ShadowHandle::Feather, half, false);
        let got = shadow(&app, id).expect("shadow");
        assert_eq!(got.blur, 12.5);
        assert_eq!(got.opacity, 0.5);
        // Past the end: the most; before the start: none.
        app.drag_shadow_handle(ShadowHandle::Feather, pe + (pe - pa), true);
        assert_eq!(shadow(&app, id).expect("shadow").blur, FEATHER_MAX_MM);
        app.drag_shadow_handle(ShadowHandle::Feather, pa - (pe - pa), true);
        assert_eq!(shadow(&app, id).expect("shadow").blur, 0.0);
        // The opacity slider leaves the feathering alone.
        app.drag_shadow_handle(ShadowHandle::Opacity, pa + (pe - pa) * 0.25, false);
        let got = shadow(&app, id).expect("shadow");
        assert_eq!((got.opacity, got.blur), (0.25, 0.0));
    }

    /// A Drop Shadow drag starts from an edge also when it is pressed on
    /// the edge or a hair outside it (within a handle's reach), standing
    /// the shadow on that edge; before, only a press inside the outer
    /// fifth of the object did, and a press just outside started nothing.
    /// The edge a drag would stand on is shown while hovering, nothing in
    /// the middle (flat) or far away.
    #[test]
    fn a_drag_from_just_outside_an_edge_stands_the_shadow_on_it() {
        let (mut app, id) = app_with_rect();
        let tol = HANDLE_PX / f64::from(app.view.zoom);
        let cases = [
            (Point::new(40.0, 20.0 - tol * 0.5), ShadowEdge::Bottom),
            (Point::new(40.0, 80.0 + tol * 0.5), ShadowEdge::Top),
            (Point::new(20.0 - tol * 0.5, 50.0), ShadowEdge::Left),
            (Point::new(60.0 + tol * 0.5, 50.0), ShadowEdge::Right),
        ];
        for (p, edge) in cases {
            assert_eq!(app.hit_test(p), None, "{edge:?}: outside the object");
            assert_eq!(app.shadow_drag_start(p), Some((id, Some(edge))), "{edge:?}");
            assert_eq!(
                app.shadow_edge_hint(p).map(|(_, e)| e),
                Some(edge),
                "{edge:?}"
            );
        }
        // Inside next to an edge, and in the middle.
        assert_eq!(
            app.shadow_drag_start(Point::new(40.0, 22.0)),
            Some((id, Some(ShadowEdge::Bottom)))
        );
        assert_eq!(
            app.shadow_drag_start(Point::new(40.0, 50.0)),
            Some((id, None))
        );
        assert_eq!(app.shadow_edge_hint(Point::new(40.0, 50.0)), None);
        // Far outside: nothing.
        let far = Point::new(40.0, 20.0 - tol * 3.0);
        assert_eq!(app.shadow_drag_start(far), None);
        assert_eq!(app.shadow_edge_hint(far), None);
        // The drag from below the bottom edge makes a shadow standing on it.
        let (p, _) = cases[0];
        app.drag_new_shadow(id, p, p, true);
        app.drag_new_shadow(id, p, Point::new(70.0, 30.0), false);
        let per = shadow(&app, id)
            .and_then(|s| s.perspective)
            .expect("perspective");
        assert_eq!(per.edge, ShadowEdge::Bottom);
    }

    /// A shadow is picked with its object (#935): where it lies over an
    /// object beneath, it wins, as it is drawn over that object; outside
    /// its silhouette (the feathered fringe included) nothing is picked;
    /// a perspective shadow is picked where it falls; and a shadow with
    /// no opacity is not there.
    #[test]
    fn a_shadow_is_picked_with_its_object_above_what_lies_beneath() {
        let (mut app, below) = app_with_rect();
        let top = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(100.0, 20.0, 120.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![top]);
        app.apply_fill(Fill::Solid(Color::rgb8(255, 0, 0)));
        app.set_shadow(
            Some(Shadow {
                offset: Vec2::new(-60.0, 20.0),
                ..Shadow::default()
            }),
            false,
        );
        app.select(Vec::new());
        // The shadow covers 40 to 60 by 40 to 60, over the rectangle below.
        let on_both = Point::new(50.0, 50.0);
        assert_eq!(app.hit_test(on_both), Some(below));
        assert_eq!(app.pick_at(on_both), Some(top));
        // The rectangle below where no shadow is, and the top object itself.
        assert_eq!(app.pick_at(Point::new(30.0, 70.0)), Some(below));
        assert_eq!(app.pick_at(Point::new(110.0, 30.0)), Some(top));
        // Just outside the silhouette, in the feathering: nothing new.
        assert_eq!(app.pick_at(Point::new(50.0, 61.0)), Some(below));
        assert_eq!(app.pick_at(Point::new(80.0, 50.0)), None);
        // A click there with a tool that selects on click.
        app.click_select(on_both, egui::Modifiers::NONE);
        assert_eq!(app.selection, vec![top]);
        // A perspective shadow on the floor, falling to the right.
        let mut sh = Shadow {
            perspective: Some(tracedraw_core::shadow::Perspective::new(ShadowEdge::Bottom)),
            ..Shadow::default()
        };
        let b = Rect::new(100.0, 20.0, 120.0, 40.0);
        sh.set_end_point(b, Point::new(150.0, 30.0));
        app.set_shadow(Some(sh), false);
        app.select(Vec::new());
        let s = app.doc().find_shape(top).expect("top").clone();
        let far = sh.silhouette_transform(b) * Point::new(110.0, 39.0);
        assert!(shadow_covers(&s, far), "{far:?}");
        assert_eq!(app.pick_at(far), Some(top));
        // No opacity: nothing to pick.
        app.select(vec![top]);
        app.set_shadow(Some(Shadow { opacity: 0.0, ..sh }), false);
        let s = app.doc().find_shape(top).expect("top").clone();
        assert!(!shadow_covers(&s, far));
    }

    /// Native ellipses and stars cast and pick their shadow by their
    /// outline, as their curves do: a press in the box's corner beside
    /// the shadowed outline is not on the shadow (#1323).
    #[test]
    fn native_shapes_pick_their_shadow_by_the_outline() {
        let (mut app, _) = app_with_rect();
        let kinds = [
            ShapeKind::Ellipse {
                rect: Rect::new(100.0, 100.0, 140.0, 140.0),
                arc: None,
            },
            ShapeKind::Polygon {
                rect: Rect::new(100.0, 100.0, 140.0, 140.0),
                points: 5,
                sharpness: 0.5,
                complex: None,
                twist: 0.0,
            },
        ];
        for kind in kinds {
            let id = app.new_shape(kind.clone()).expect("shape");
            app.select(vec![id]);
            app.apply_fill(Fill::Solid(Color::rgb8(0, 0, 255)));
            app.set_shadow(
                Some(Shadow {
                    offset: Vec2::new(50.0, 0.0),
                    blur: 0.0,
                    ..Shadow::default()
                }),
                false,
            );
            let s = app.doc().find_shape(id).expect("shape").clone();
            let mut curves = s.clone();
            curves.kind = ShapeKind::Path {
                path: s.local_path(),
                closed: true,
            };
            // The middle of the shadow, and the corner of its box.
            for (p, on) in [
                (Point::new(170.0, 120.0), true),
                (Point::new(151.0, 139.0), false),
            ] {
                assert_eq!(shadow_covers(&s, p), on, "{kind:?} {p:?}");
                assert_eq!(shadow_covers(&curves, p), on, "{kind:?} curves {p:?}");
            }
        }
    }

    /// The property bar applies at once: with an object that has no
    /// shadow selected, choosing Flat (or any field) gives it the shadow
    /// (#1323).
    #[test]
    fn the_property_bar_applies_a_shadow_directly() {
        let (mut app, id) = app_with_rect();
        assert!(shadow(&app, id).is_none());
        let flat = Shadow {
            perspective: None,
            offset: Vec2::new(3.0, -3.0),
            ..Shadow::default()
        };
        app.shadow_bar_changed(flat, false);
        assert_eq!(shadow(&app, id), Some(flat));
        assert_eq!(app.shadow_default, flat);
        // Changed again: one more step merged into the same one.
        let steps = app.engine.history_labels().0.len();
        let flat2 = Shadow {
            opacity: 0.3,
            ..flat
        };
        app.shadow_bar_changed(flat2, true);
        assert_eq!(shadow(&app, id), Some(flat2));
        assert_eq!(app.engine.history_labels().0.len(), steps);
        // Nothing selected: only the default changes.
        app.select(Vec::new());
        let other = Shadow {
            opacity: 0.9,
            ..flat
        };
        app.shadow_bar_changed(other, false);
        assert_eq!(app.shadow_default, other);
        assert_eq!(shadow(&app, id), Some(flat2));
        // The handles are taken from further than they are drawn.
        app.select(vec![id]);
        let (_, sh, b) = app.selected_shadow().expect("shadow");
        let end = shadow_handles(&sh, b)[0].1;
        let off = (HANDLE_SIDE_PX as f64 / 2.0 + 3.0) / f64::from(app.view.zoom);
        assert_eq!(
            app.shadow_handle_at(end + Vec2::new(off, 0.0)),
            Some(ShadowHandle::End)
        );
    }

    #[test]
    fn presets_apply_and_keep_the_colour() {
        let (mut app, id) = app_with_rect();
        let green = Color::rgb8(0, 120, 0);
        app.set_shadow(
            Some(Shadow {
                color: green,
                ..Shadow::default()
            }),
            false,
        );
        for (key, p) in Shadow::presets() {
            app.apply_shadow_preset(key);
            let sh = shadow(&app, id).expect("shadow");
            assert_eq!(sh.color, green, "{key}");
            assert_eq!(sh.perspective, p.perspective, "{key}");
            assert_eq!(sh.merge, p.merge, "{key}");
        }
    }

    #[test]
    fn break_apart_makes_a_bitmap_behind_the_object_in_one_step() {
        let (mut app, id) = app_with_rect();
        app.set_shadow(Some(Shadow::default()), false);
        let count = |app: &App| {
            app.doc().pages[0]
                .layers
                .iter()
                .map(|l| l.shapes.len())
                .sum::<usize>()
        };
        let before = count(&app);
        assert!(app.can_break_shadow_apart());
        app.break_apart();
        assert_eq!(count(&app), before + 1);
        assert!(shadow(&app, id).is_none());
        let bitmap_id = app.selection[0];
        let (layer, bmp) = app.doc().shape(bitmap_id).expect("bitmap");
        assert!(matches!(bmp.kind, ShapeKind::Bitmap { .. }));
        // Merged as the shadow was (multiply).
        assert!(matches!(
            bmp.effects.as_slice(),
            [Effect::Transparency {
                merge: MergeMode::Multiply,
                ..
            }]
        ));
        // Just behind the object.
        let ib = layer.shapes.iter().position(|s| s.id == bitmap_id);
        let io = layer.shapes.iter().position(|s| s.id == id);
        assert_eq!(ib.map(|i| i + 1), io);
        // It covers the shadow: right of and below the object.
        let b = bmp.bounds();
        assert!(b.x1 > 60.0 && b.y0 < 20.0, "{b:?}");
        // One undo brings the shadow back and takes the bitmap away.
        app.undo();
        assert_eq!(count(&app), before);
        assert!(shadow(&app, id).is_some());
    }
}
