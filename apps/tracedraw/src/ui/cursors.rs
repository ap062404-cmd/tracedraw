//! The pointer over the drawing window shows the active tool and what a
//! press would do. The arrow and the resize and move arrows are the
//! system's own; the cross hair and the text I-beam are drawn, black on a
//! white halo, because the system's invert what is under them and can
//! show white on a white page (#655). The others are drawn too: a
//! magnifier with a plus or a minus
//! for Zoom, an open or closed hand for Pan and middle-button panning, a
//! pipette for the eyedroppers and palette sampling, a paint bucket for
//! Area Fill, turning and slanting arrows for the rotate and skew
//! handles, and the Shape tool's solid arrow with a mark for a node, a
//! segment or a control point under it. Drawn cursors hide the system
//! pointer and are painted above everything at the pointer position.
//!
//! A pointer painted in the window shows where the mouse was when the
//! frame was made, and the frame reaches the screen a refresh or more
//! later: next to the system's arrow, which the screen moves on its own,
//! every drawn pointer trailed the hand (#2136, and #2557 on Linux). On
//! Windows and Linux the drawn pointers are therefore handed to the
//! system as bitmaps of the same drawing ([`bitmap`]), and the system
//! moves them as it moves its own; only a nib too large for a pointer
//! bitmap is still painted.
//!
//! The Pick tools: over a corner handle the diagonal resize arrows, over
//! a side handle the horizontal or vertical ones, over the selection (or
//! the centre of rotation) the move arrows; in rotate mode the corners
//! turn and the sides slant. While dragging, the pointer keeps the shape
//! of what is being done.

use crate::app::{App, Drag, Handle};
use crate::tools::Tool;
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// What the pointer looks like over the drawing window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolCursor {
    /// A system cursor.
    System(egui::CursorIcon),
    ZoomIn,
    ZoomOut,
    Hand {
        closed: bool,
    },
    Pipette,
    Bucket,
    /// Turning the selection (rotate mode, corner handles).
    Rotate,
    /// Slanting the selection (rotate mode, side handles): sideways for
    /// the top and bottom handles, up and down for the others.
    Skew {
        horizontal: bool,
    },
    /// The Shape tool: a solid arrow with what is under it.
    ShapeArrow(ShapeMark),
    /// Moving objects with the Pick tool: the arrow with a dotted box at
    /// its lower right; with `copy` (after a right click, the move leaves
    /// the original) a plus in a square beside the box.
    DragMove {
        copy: bool,
    },
    /// The cross hair with the tool's icon at its lower right (the
    /// drawing, effect and fill tools).
    Badge(Tool),
    /// The cross hair alone (the Pick tool with the cross hair option, the
    /// ruler origin dragged out).
    Cross,
    /// The text I-beam, its middle the hot spot (where a click edits
    /// text).
    IBeam,
    /// A curve tool's badge with what a press there would do: continue
    /// the selected curve from its end node, or close the curve being
    /// drawn on its first node.
    CurveBadge(Tool, CurveMark),
    /// Picking the object a command acts on (Place Inside Frame, Order >
    /// In Front Of and Behind): a thick arrow pointing right, its tip the
    /// hot spot (#379).
    TargetArrow,
    /// The Pen tool: a pen nib whose tip, marked with a dot, is the hot
    /// spot (the node goes there, #362), with what a press would do.
    PenNib(Option<CurveMark>),
    /// The cross hair with the nib's outline around it: a circle of
    /// `radius` screen points, or a square of that half side (the Eraser,
    /// the Smooth, Smear, Twirl, Attract and Repel, Smudge and Roughen
    /// brushes).
    Nib {
        radius: f32,
        square: bool,
    },
    /// The Table tool over a row (pointing right) or column (pointing
    /// down) pointer at the table's edge: a click selects it.
    TableArrow {
        down: bool,
    },
    /// File > Import waiting for its place: a corner bracket whose
    /// corner is the hot spot, where the import's top left corner goes
    /// (#1936).
    PlaceCorner,
    /// Over a paragraph frame's flow tab, with any tool: the arrow with a
    /// small frame of text lines and the tab's down arrow beside it; a
    /// press starts flowing the text into another frame (#1986).
    FlowText,
}

/// What a curve tool's press would do besides drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveMark {
    /// Continue the selected curve from its end node.
    Join,
    /// Close the curve being drawn on its first node.
    Close,
    /// A click adds a node on a segment: of the selected curve (Pen,
    /// Bezier) or of the curve being drawn (Pen).
    AddNode,
    /// Pen: a click deletes the node (of the selected curve or of the
    /// curve being drawn).
    DeleteNode,
    /// A node a press edits: a node of the selected curve to drag, or the
    /// last node of the curve being drawn, which a click makes a cusp.
    EditNode,
}

/// What the Shape tool's arrow points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeMark {
    Nothing,
    /// A node (of a curve, or of a rectangle, ellipse or polygon).
    Node,
    /// A segment of a selected curve.
    Segment,
    /// A control point of a selected node.
    Control,
}

/// The system resize arrows for a scale handle.
pub fn resize_icon(h: Handle) -> egui::CursorIcon {
    use egui::CursorIcon as C;
    match h {
        Handle::N | Handle::S => C::ResizeVertical,
        Handle::E | Handle::W => C::ResizeHorizontal,
        Handle::NE | Handle::SW => C::ResizeNeSw,
        Handle::NW | Handle::SE => C::ResizeNwSe,
    }
}

/// The Pick tools' pointer for what is being dragged or what is under
/// `pos` (screen); `None` for the tool's plain pointer.
fn pick_cursor(app: &App, pos: Option<Pos2>) -> Option<ToolCursor> {
    let moving = ToolCursor::System(egui::CursorIcon::Move);
    match &app.drag {
        // Over the document palette letting go adds the colours; after a
        // right click the move leaves the original (a copy moves).
        // A picture over any palette strip: letting go offers its
        // colours (#2137).
        Drag::Move { .. } if app.over_document_palette() || app.picture_drop_target().is_some() => {
            return Some(ToolCursor::System(egui::CursorIcon::Copy))
        }
        Drag::Move { .. } => {
            return Some(ToolCursor::DragMove {
                copy: app.drag_copy,
            })
        }
        Drag::Pivot | Drag::GuidePivot => return Some(moving),
        Drag::Scale { handle, .. } => return Some(ToolCursor::System(resize_icon(*handle))),
        Drag::Rotate { .. } | Drag::RotateGuide { .. } => return Some(ToolCursor::Rotate),
        Drag::Skew { handle, .. } => {
            return Some(ToolCursor::Skew {
                horizontal: matches!(handle, Handle::N | Handle::S),
            })
        }
        Drag::MoveGuide { guide, .. } => return Some(across_guide(&guide.line)),
        Drag::NodeMove { .. } => return Some(ToolCursor::ShapeArrow(ShapeMark::Node)),
        Drag::None => {}
        _ => return None,
    }
    let pos = pos?;
    // Node tracking: over a node of a selected curve, the Shape tool's
    // arrow with the node mark.
    if app.settings.node_tracking
        && !app.rotate_mode
        && app.node_at(app.view.to_page(pos)).is_some()
    {
        return Some(ToolCursor::ShapeArrow(ShapeMark::Node));
    }
    if app.pivot_at(pos) || app.center_mark_at(pos) {
        return Some(moving);
    }
    if let Some(h) = app.handle_at(pos) {
        return Some(if !app.rotate_mode {
            ToolCursor::System(resize_icon(h))
        } else if h.is_corner() {
            ToolCursor::Rotate
        } else {
            ToolCursor::Skew {
                horizontal: matches!(h, Handle::N | Handle::S),
            }
        });
    }
    let p = app.view.to_page(pos);
    // A guideline's rotation centre (dragged, #1850) and handles, then
    // the guideline itself where no object is (objects win a press there).
    if app.guide_pivot_at(p) {
        return Some(moving);
    }
    if app.guide_rotate_handle_at(p) {
        return Some(ToolCursor::Rotate);
    }
    // Rotate mode: a node of the selection turns it (#933).
    if app.grab_turn_point(p).is_some() {
        return Some(ToolCursor::Rotate);
    }
    // A guideline over a picture that is not selected takes the press,
    // so its arrows show there (#2640).
    let guides = app.page_guides();
    let guide = app
        .guide_at(p)
        .and_then(|i| guides.get(i))
        .filter(|g| !g.locked);
    if let Some(g) = guide.filter(|_| app.guide_wins_press(p)) {
        return Some(across_guide(&g.line));
    }
    // Over an object: the move arrows only where a press would move what
    // is selected already (the object or one of its parts); over another
    // object the plain arrow, as a press there picks it first (#885).
    // Objects win a press over guidelines, so no guideline arrows there.
    if let Some(selected) = app.over_selection(p) {
        return selected.then_some(moving);
    }
    let g = guides.get(app.guide_at(p)?)?;
    (!g.locked).then(|| across_guide(&g.line))
}

/// The arrows for moving a guideline: up and down for a horizontal one,
/// sideways for a vertical one, the move arrows for a slanted one.
fn across_guide(line: &tracedraw_core::document::GuideLine) -> ToolCursor {
    use tracedraw_core::document::GuideLine;
    ToolCursor::System(match line {
        GuideLine::Horizontal { .. } => egui::CursorIcon::ResizeVertical,
        GuideLine::Vertical { .. } => egui::CursorIcon::ResizeHorizontal,
        GuideLine::Angled { .. } => egui::CursorIcon::Move,
    })
}

/// The pointer while a guideline or the ruler origin is dragged out of
/// the rulers (with any tool): the arrows across the new guideline, the
/// cross hair for the origin.
pub fn ruler_drag_cursor(app: &App) -> Option<ToolCursor> {
    match &app.drag {
        Drag::NewGuide { horizontal, .. } => Some(ToolCursor::System(if *horizontal {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::ResizeHorizontal
        })),
        Drag::RulerOrigin { .. } => Some(ToolCursor::Cross),
        _ => None,
    }
}

/// The Shape tool's pointer: over the handles framing the selected nodes
/// (Stretch and Scale or Rotate and Skew Nodes) the arrows of the Pick
/// tool's handles, else its arrow with a mark.
fn shape_cursor(app: &App, pos: Option<Pos2>) -> ToolCursor {
    use crate::node_edit::NodeTransformMode as M;
    let handle = match &app.drag {
        Drag::NodeTransform { handle, .. } => Some(*handle),
        Drag::None => pos.and_then(|s| app.node_transform_handle_at(app.view.to_page(s))),
        _ => None,
    };
    match (handle, app.node_transform) {
        (Some(h), M::StretchScale) => ToolCursor::System(resize_icon(h)),
        (Some(h), M::RotateSkew) if h.is_corner() => ToolCursor::Rotate,
        (Some(h), M::RotateSkew) => ToolCursor::Skew {
            horizontal: matches!(h, Handle::N | Handle::S),
        },
        _ => ToolCursor::ShapeArrow(shape_mark(app, pos)),
    }
}

/// What the Shape tool's arrow points at: what is being dragged, else
/// what is under `pos` (screen).
fn shape_mark(app: &App, pos: Option<Pos2>) -> ShapeMark {
    if let Some(d) = &app.envelope_drag {
        return match d.part {
            crate::envelope_edit::Part::Node(_) => ShapeMark::Node,
            crate::envelope_edit::Part::Handle { .. } => ShapeMark::Control,
            crate::envelope_edit::Part::Segment { .. } => ShapeMark::Segment,
        };
    }
    match &app.drag {
        Drag::Node { .. } | Drag::NodeMove { .. } | Drag::KindNode { .. } => {
            return ShapeMark::Node
        }
        Drag::Handle { .. } => return ShapeMark::Control,
        Drag::SegmentBend { .. } => return ShapeMark::Segment,
        Drag::None => {}
        _ => return ShapeMark::Nothing,
    }
    let Some(pos) = pos else {
        return ShapeMark::Nothing;
    };
    let p = app.view.to_page(pos);
    match app.envelope_part_at(p) {
        Some((_, crate::envelope_edit::Part::Node(_))) => return ShapeMark::Node,
        Some((_, crate::envelope_edit::Part::Handle { .. })) => return ShapeMark::Control,
        Some((_, crate::envelope_edit::Part::Segment { .. })) => return ShapeMark::Segment,
        None => {}
    }
    // A segment of an envelope's frame bends when dragged (#1339).
    if app.envelope_segment_at(p).is_some() {
        return ShapeMark::Segment;
    }
    if app.ctrl_handle_at(p).is_some() {
        ShapeMark::Control
    } else if app.node_at(p).is_some()
        || app.kind_node_at(p).is_some()
        || app.effect_node_at(p).is_some()
    {
        ShapeMark::Node
    } else if app.segment_at(p).is_some() {
        ShapeMark::Segment
    } else {
        ShapeMark::Nothing
    }
}

/// The cursor for the current tool and input state, the pointer at `pos`
/// (screen), Ctrl not held (the tests' shorthand).
#[cfg(test)]
pub fn for_tool(
    app: &App,
    shift: bool,
    primary_down: bool,
    middle_down: bool,
    pos: Option<Pos2>,
) -> ToolCursor {
    for_tool_with_ctrl(app, shift, false, primary_down, middle_down, pos)
}

/// [`for_tool`] with Ctrl held or not: Ctrl in an eyedropper's Apply
/// mode samples again, so the pointer is the pipette (#1302).
pub fn for_tool_with_ctrl(
    app: &App,
    shift: bool,
    ctrl: bool,
    primary_down: bool,
    middle_down: bool,
    pos: Option<Pos2>,
) -> ToolCursor {
    if middle_down {
        return ToolCursor::Hand { closed: true };
    }
    if app.pending_palette_sample {
        return ToolCursor::Pipette;
    }
    if app.import_placing.is_some() {
        return ToolCursor::PlaceCorner;
    }
    if app.pending_clip_frame
        || app.pending_order.is_some()
        || app.pending_text_link.is_some()
        || app.pending_shaping.is_some()
        || app.pending_merge_frame
    {
        return ToolCursor::TargetArrow;
    }
    // A frame's flow tab takes a press with any tool (#1986); the Pick
    // tool's handles come first, as they do for the press.
    if let Some(at) = pos.filter(|_| !primary_down && matches!(app.drag, Drag::None)) {
        let handle = (matches!(app.tool, Tool::Pick | Tool::FreeformPick)
            || app.tool.resizes_by_handles())
            && app.handle_at(at).is_some();
        if !handle && app.any_flow_tab_at(at).is_some() {
            return ToolCursor::FlowText;
        }
    }
    if matches!(app.tool, Tool::Pick | Tool::FreeformPick) {
        if let Some(c) = pick_cursor(app, pos) {
            return c;
        }
    }
    // A drawing tool over a handle of the selection resizes it.
    if app.tool.resizes_by_handles() && !app.rotate_mode {
        if let Drag::Scale { handle, .. } = &app.drag {
            return ToolCursor::System(resize_icon(*handle));
        }
        if let Some(h) = pos
            .filter(|_| matches!(app.drag, Drag::None) && app.three_point_base.is_none())
            .and_then(|at| app.handle_at(at))
        {
            return ToolCursor::System(resize_icon(h));
        }
    }
    match app.tool {
        Tool::Zoom if shift => ToolCursor::ZoomOut,
        Tool::Zoom => ToolCursor::ZoomIn,
        Tool::Pan => ToolCursor::Hand {
            closed: primary_down,
        },
        // Apply mode paints what was sampled: the bucket (Ctrl, and Shift
        // with the attributes eyedropper, samples again: the pipette).
        Tool::ColorEyedropper
            if app.eyedropper_apply && app.eyedropper_color.is_some() && !ctrl =>
        {
            ToolCursor::Bucket
        }
        Tool::AttributesEyedropper
            if app.eyedropper_apply && app.eyedropper_attrs.is_some() && !shift && !ctrl =>
        {
            ToolCursor::Bucket
        }
        Tool::ColorEyedropper | Tool::AttributesEyedropper => ToolCursor::Pipette,
        // The Mesh Fill bar's eyedropper waits for its click (#2387).
        Tool::MeshFill if app.mesh_eyedropper => ToolCursor::Pipette,
        Tool::AreaFill => ToolCursor::Bucket,
        // The I-beam where a click edits text, else the cross hair with the
        // tool's sign (a click starts a new text, a drag a frame).
        Tool::Text if over_text(app, pos) => ToolCursor::IBeam,
        Tool::Text => ToolCursor::Badge(Tool::Text),
        Tool::Pick | Tool::FreeformPick if app.settings.crosshair_cursor => ToolCursor::Cross,
        Tool::Pick | Tool::FreeformPick => ToolCursor::System(egui::CursorIcon::Default),
        Tool::Shape => shape_cursor(app, pos),
        Tool::Table => app.table_cursor(pos),
        // Over an envelope's node, handle or segment: the Shape tool's
        // arrow.
        Tool::Envelope
            if app.envelope_drag.is_some()
                || pos.is_some_and(|s| {
                    let p = app.view.to_page(s);
                    app.envelope_part_at(p).is_some() || app.envelope_segment_at(p).is_some()
                }) =>
        {
            ToolCursor::ShapeArrow(shape_mark(app, pos))
        }
        // The fill's (or the mask's) handles and its line move.
        Tool::InteractiveFill | Tool::Transparency
            if matches!(
                app.drag,
                Drag::FountainHandle { .. } | Drag::FountainLine { .. }
            ) || pos.is_some_and(|s| {
                let p = app.view.to_page(s);
                app.fountain_handle_at(p).is_some() || app.fountain_on_line(p)
            }) =>
        {
            ToolCursor::System(egui::CursorIcon::Move)
        }
        // An effect's control line moves the whole effect.
        Tool::DropShadow
            if matches!(app.drag, Drag::ShadowLine { .. })
                || pos.is_some_and(|s| app.shadow_on_line(app.view.to_page(s))) =>
        {
            ToolCursor::System(egui::CursorIcon::Move)
        }
        Tool::Extrude
            if app.extrude_drag.is_some_and(|d| d.line)
                || pos.is_some_and(|s| app.extrusion_line_at(app.view.to_page(s)).is_some()) =>
        {
            ToolCursor::System(egui::CursorIcon::Move)
        }
        Tool::Blend
            if app.blend_handle_drag == Some(crate::blend_tool::BlendHandle::Line)
                || (app.blend_handle_drag.is_none()
                    && pos.is_some_and(|s| {
                        app.blend_handle_at(app.view.to_page(s))
                            == Some(crate::blend_tool::BlendHandle::Line)
                    })) =>
        {
            ToolCursor::System(egui::CursorIcon::Move)
        }
        // A contour handle slides along its line.
        Tool::Contour
            if app.contour_handle_drag.is_some()
                || pos.is_some_and(|s| app.contour_handle_at(app.view.to_page(s)).is_some()) =>
        {
            ToolCursor::System(egui::CursorIcon::ResizeHorizontal)
        }
        Tool::Distort
            if app
                .distort_drag
                .as_ref()
                .is_some_and(|d| d.grab == crate::distort_tool::Grab::Line)
                || pos.is_some_and(|s| app.distortion_line_at(app.view.to_page(s))) =>
        {
            ToolCursor::System(egui::CursorIcon::Move)
        }
        // The nib being sized by a Shift+drag: its circle stays at the
        // press (see `brushes::draw_resize`), the pointer sizes it across.
        t if t.is_shaping_brush() && app.brush_resize.is_some() => {
            ToolCursor::System(egui::CursorIcon::ResizeHorizontal)
        }
        Tool::Eraser => ToolCursor::Nib {
            radius: nib_px(app.eraser_width / 2.0, app.view.zoom),
            square: app.eraser_square,
        },
        Tool::Smooth
        | Tool::Smear
        | Tool::Twirl
        | Tool::AttractRepel
        | Tool::Smudge
        | Tool::Roughen => ToolCursor::Nib {
            radius: nib_px(app.brush_radius, app.view.zoom),
            square: false,
        },
        Tool::Pen => ToolCursor::PenNib(curve_mark(app, pos)),
        t @ (Tool::Freehand
        | Tool::Bezier
        | Tool::BSpline
        | Tool::Polyline
        | Tool::TwoPointLine) => match curve_mark(app, pos) {
            Some(m) => ToolCursor::CurveBadge(t, m),
            None => ToolCursor::Badge(t),
        },
        t => ToolCursor::Badge(t),
    }
}

/// A curve tool over the first node of the curve being drawn (three
/// nodes or more: a press closes it) or over an end node of the selected
/// open curve (the new line continues it).
fn curve_mark(app: &App, pos: Option<Pos2>) -> Option<CurveMark> {
    let pos = pos?;
    let p = app.view.to_page(pos);
    let pen = matches!(app.tool, Tool::Bezier | Tool::Pen);
    if app.pen_editing {
        return Some(CurveMark::EditNode);
    }
    if let Some(c) = &app.curve {
        let near = |q: &tracedraw_core::geometry::Point| {
            app.view.to_screen(*q).distance(pos) as f64 <= app.curve_end_reach_px()
        };
        if c.closes_at_first() && c.nodes.first().is_some_and(|(f, _)| near(f)) {
            return Some(CurveMark::Close);
        }
        if pen
            && c.nodes
                .last()
                .is_some_and(|(l, h)| near(l) && (c.nodes.len() >= 2 || h.is_some()))
        {
            return Some(CurveMark::EditNode);
        }
        // No adding or deleting while a curve is drawn (#1143).
        return None;
    }
    if app.on_join_end(p) {
        return Some(CurveMark::Join);
    }
    match app.pen_edit_at(p) {
        Some(crate::interaction::PenEdit::Node(..)) if app.pen_deletes_nodes() => {
            Some(CurveMark::DeleteNode)
        }
        Some(crate::interaction::PenEdit::Node(..)) => Some(CurveMark::EditNode),
        Some(crate::interaction::PenEdit::Segment(..)) => Some(CurveMark::AddNode),
        None => None,
    }
}

/// The pointer at `pos` is over a text object (or the text being
/// edited), where a Text tool click places the caret.
fn over_text(app: &App, pos: Option<Pos2>) -> bool {
    if matches!(app.drag, Drag::TextSelect) {
        return true;
    }
    let Some(pos) = pos else {
        return false;
    };
    let p = app.view.to_page(pos);
    app.hit_test(p)
        .and_then(|id| app.doc().find_shape(id))
        .is_some_and(|s| matches!(s.kind, tracedraw_core::ShapeKind::Text { .. }))
}

/// A nib of `mm` on screen at `zoom` (points per mm), kept visible and
/// within the window.
fn nib_px(mm: f64, zoom: f32) -> f32 {
    let px = mm as f32 * zoom;
    if px.is_finite() {
        px.clamp(2.0, 400.0)
    } else {
        2.0
    }
}

/// Show `cursor` at `pos` (the pointer): set the system cursor, or hide it
/// and paint the drawn one on top of everything, or (where the system
/// takes pointer bitmaps, see [`bitmap_pointers`]) hand the drawing to
/// the system as the pointer.
pub fn show(ctx: &egui::Context, cursor: ToolCursor, pos: Option<Pos2>) {
    let icon = match cursor {
        ToolCursor::System(icon) => icon,
        // On a software adapter a drawn pointer trails the real one by a
        // slow frame: the system's nearest instead (#616).
        _ if crate::ui::software_renderer() => system_icon(cursor),
        _ => egui::CursorIcon::None,
    };
    ctx.set_cursor_icon(icon);
    if icon != egui::CursorIcon::None {
        return;
    }
    if bitmap_pointers() {
        if let Some(image) = bitmap(cursor, ctx.pixels_per_point()) {
            // Given to the system at the end of the frame, unless
            // something asks for another pointer meanwhile
            // ([`finish_frame`]); nothing is painted.
            ctx.data_mut(|d| {
                d.insert_temp(
                    pending_id(),
                    Pending {
                        image,
                        fallback: system_icon(cursor),
                    },
                )
            });
            return;
        }
    }
    if let Some(p) = pos {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("tool_cursor"),
        ));
        paint(&painter, cursor, p);
    }
}

/// At the end of a frame: on Windows a pointer left hidden (a drawn
/// pointer, or anything else that asked for `CursorIcon::None`) is the
/// blank image instead (see [`hidden_pointer`]). Decided from the frame's
/// last word on the pointer, so a system pointer asked for after a drawn
/// one in the same frame shows, never the blank image in its place.
///
/// A drawn pointer handed over as a bitmap ([`show`]) is given to the
/// system here, by the same rule: only when the pointer is still the
/// hidden one the tool asked for. The system pointer nearest to it goes
/// along, for a window that cannot take the bitmap.
pub fn finish_frame(ctx: &egui::Context) {
    let pending: Option<Pending> = ctx.data_mut(|d| {
        let p = d.get_temp(pending_id());
        d.remove::<Pending>(pending_id());
        p
    });
    ctx.output_mut(|o| match pending {
        Some(p) if o.cursor_icon == egui::CursorIcon::None && o.cursor_image.is_none() => {
            o.cursor_icon = p.fallback;
            o.cursor_image = Some(p.image);
        }
        _ => blank_when_hidden(o, cfg!(target_os = "windows")),
    });
}

/// A drawn pointer waiting for the end of its frame.
#[derive(Clone)]
struct Pending {
    image: egui::CustomCursorImage,
    fallback: egui::CursorIcon,
}

fn pending_id() -> egui::Id {
    egui::Id::new("tool_cursor_bitmap")
}

/// Whether drawn pointers are handed to the system as bitmaps: on
/// Windows and on Linux (X11 and Wayland), where a pointer bitmap is in
/// the screen's pixels as [`bitmap`] draws it. On Linux a painted
/// pointer trailed the mouse as it did on Windows, so with the drawing
/// tools the pointer felt slow next to the Pick tool's arrow (#2557).
/// Elsewhere they are painted as before: the browser takes no bitmap
/// from the window, and macOS shows a pointer bitmap in points, not
/// pixels. `TRACEDRAW_BITMAP_POINTERS=1` turns it on anywhere and `0`
/// off, for trying it out.
fn bitmap_pointers() -> bool {
    #[cfg(test)]
    {
        TEST_BITMAP_POINTERS.with(|b| b.get())
    }
    #[cfg(not(test))]
    {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| {
            if cfg!(target_arch = "wasm32") {
                return false;
            }
            match std::env::var("TRACEDRAW_BITMAP_POINTERS").as_deref() {
                Ok("1") => true,
                Ok("0") => false,
                _ => cfg!(any(target_os = "windows", target_os = "linux")),
            }
        })
    }
}

#[cfg(test)]
thread_local! {
    /// Bitmap pointers on, for the tests of them (off in every other
    /// test, whatever the system, so those see the painted pointer).
    static TEST_BITMAP_POINTERS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Bitmap pointers on or off on this thread, for the frame benchmarks.
#[cfg(test)]
pub(crate) fn set_test_bitmap_pointers(on: bool) {
    TEST_BITMAP_POINTERS.with(|b| b.set(on));
}

/// How far from the hot spot a pointer's drawing may reach on its
/// bitmap, points: the cross hair with a tool's sign reaches 20.
const BITMAP_REACH: f32 = 48.0;

/// Largest side of a pointer bitmap, pixels. The systems take larger
/// ones, but draw them themselves instead of the graphics card, and a
/// pointer that large hides what it points at.
const BITMAP_MAX: usize = 192;

/// Pointer bitmaps kept: a tool has a handful, a nib one for each size.
const BITMAPS_KEPT: usize = 48;

thread_local! {
    /// The bitmaps made so far, by pointer and pixels per point (the
    /// newest last), and the context that draws them.
    static BITMAPS: std::cell::RefCell<Bitmaps> = std::cell::RefCell::new(Bitmaps::default());
}

#[derive(Default)]
struct Bitmaps {
    ctx: Option<egui::Context>,
    made: Vec<(BitmapKey, Option<egui::CustomCursorImage>)>,
}

/// What a pointer bitmap is of: the pointer, the pixels per point, and
/// the theme (a tool's sign is drawn in the theme's accent).
type BitmapKey = (ToolCursor, u32, crate::theme::Theme);

/// The drawn pointer `cursor` as a bitmap for the system to show as the
/// pointer, at `ppp` pixels per point: what [`paint`] draws, in pixels,
/// cut to what is drawn, with the hot spot where the pointer points.
/// `None` for a system pointer and for a drawing that does not fit a
/// pointer bitmap (a wide nib), which is painted instead.
///
/// The same bytes come back for the same pointer every frame (the window
/// makes one system pointer of them and only swaps it when they change).
pub fn bitmap(cursor: ToolCursor, ppp: f32) -> Option<egui::CustomCursorImage> {
    if matches!(cursor, ToolCursor::System(_)) || !(ppp.is_finite() && ppp > 0.0) {
        return None;
    }
    // A nib's size follows the zoom: half-point steps, so that a zoom
    // does not make a bitmap for every frame.
    let cursor = match cursor {
        ToolCursor::Nib { radius, square } => ToolCursor::Nib {
            radius: (radius * 2.0).round() / 2.0,
            square,
        },
        other => other,
    };
    BITMAPS.with(|b| {
        let mut b = b.borrow_mut();
        let key: BitmapKey = (cursor, ppp.to_bits(), crate::theme::theme());
        if let Some(i) = b.made.iter().rposition(|(k, _)| *k == key) {
            return b.made[i].1.clone();
        }
        // The interface's own font, for a sign with a letter in it; the
        // system's fonts (slow to load) have no part in a pointer.
        let ctx = b
            .ctx
            .get_or_insert_with(|| {
                let ctx = egui::Context::default();
                ctx.set_fonts(crate::theme::bundled_font_definitions());
                ctx
            })
            .clone();
        let image = draw_bitmap(&ctx, cursor, ppp);
        if b.made.len() >= BITMAPS_KEPT {
            b.made.remove(0);
        }
        b.made.push((key, image.clone()));
        image
    })
}

/// [`bitmap`], drawn anew.
fn draw_bitmap(
    ctx: &egui::Context,
    cursor: ToolCursor,
    ppp: f32,
) -> Option<egui::CustomCursorImage> {
    if let ToolCursor::Nib { radius, .. } = cursor {
        // (The square nib's corner, and the halo round it.)
        if !radius.is_finite() || radius * std::f32::consts::SQRT_2 + 3.0 > BITMAP_REACH {
            return None;
        }
    }
    let side = (2.0 * BITMAP_REACH) as usize;
    // The hot spot on the middle of a pixel, as the pointer is.
    let hot = (BITMAP_REACH * ppp).floor() as usize;
    let at = Pos2::new((hot as f32 + 0.5) / ppp, (hot as f32 + 0.5) / ppp);
    let canvas =
        crate::soft_canvas::rasterise(ctx, side, side, ppp, |painter| paint(painter, cursor, at));
    // What is drawn, and the hot spot.
    let (mut x0, mut y0, mut x1, mut y1) = (hot, hot, hot + 1, hot + 1);
    let mut any = false;
    for y in 0..canvas.h {
        for x in 0..canvas.w {
            if canvas.px[y * canvas.w + x][3] > 1.0 / 255.0 {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    let (w, h) = (x1.min(canvas.w) - x0, y1.min(canvas.h) - y0);
    // (Drawn up to the canvas's edge: it may not all be there.)
    let cut = x0 == 0 || y0 == 0 || x1 >= canvas.w || y1 >= canvas.h;
    if !any || cut || w > BITMAP_MAX || h > BITMAP_MAX {
        return None;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let p = canvas.px[y * canvas.w + x];
            let a = p[3].clamp(0.0, 1.0);
            // The canvas holds colour times alpha; the system wants the
            // colour itself.
            for c in &p[..3] {
                let v = if a > 0.0 { c / a } else { 0.0 };
                rgba.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            rgba.push((a * 255.0).round() as u8);
        }
    }
    Some(egui::CustomCursorImage {
        rgba: rgba.into(),
        size: [u16::try_from(w).ok()?, u16::try_from(h).ok()?],
        hotspot: [u16::try_from(hot - x0).ok()?, u16::try_from(hot - y0).ok()?],
    })
}

/// The blank image for a pointer the frame left hidden, where `windows`.
fn blank_when_hidden(out: &mut egui::PlatformOutput, windows: bool) {
    if out.cursor_icon == egui::CursorIcon::None && out.cursor_image.is_none() {
        out.cursor_image = hidden_pointer(windows);
    }
}

/// Side of the blank pointer image, in pixels.
const BLANK_SIDE: u16 = 32;

/// How a drawn pointer hides the system one. `CursorIcon::None` hides
/// the pointer through the windowing layer's visibility switch, which on
/// Windows moves the system's cursor display count (`ShowCursor`); Windows
/// shows the change only at the next pointer update, a mouse move. A tool
/// picked by its shortcut over a still mouse therefore kept the old
/// pointer (Pick to Pen: the hide waited), or none at all (Pen to another
/// tool: the show waited, so the arrow set at once stayed invisible),
/// until the mouse moved (#1026, #1124); putting the pointer back where it
/// was (#1112) did not count as a move. A new pointer image is applied at
/// once (`SetCursor`), so on Windows the system pointer is swapped for a
/// fully transparent image instead of hidden, and the display count never
/// changes. The image's bytes are shared, so the integration makes one
/// cursor of it. Elsewhere hiding takes effect at once and stays as it was.
fn hidden_pointer(windows: bool) -> Option<egui::CustomCursorImage> {
    use std::sync::{Arc, OnceLock};
    static BLANK: OnceLock<Arc<[u8]>> = OnceLock::new();
    if !windows {
        return None;
    }
    let side = usize::from(BLANK_SIDE);
    let rgba = BLANK.get_or_init(|| Arc::from(vec![0u8; side * side * 4]));
    Some(egui::CustomCursorImage {
        rgba: Arc::clone(rgba),
        size: [BLANK_SIDE, BLANK_SIDE],
        hotspot: [0, 0],
    })
}

/// The system pointer nearest to a drawn one.
pub fn system_icon(cursor: ToolCursor) -> egui::CursorIcon {
    use egui::CursorIcon as C;
    match cursor {
        ToolCursor::System(icon) => icon,
        ToolCursor::ZoomIn => C::ZoomIn,
        ToolCursor::ZoomOut => C::ZoomOut,
        ToolCursor::Hand { closed: true } => C::Grabbing,
        ToolCursor::Hand { closed: false } => C::Grab,
        ToolCursor::Rotate => C::AllScroll,
        ToolCursor::Skew { horizontal: true } => C::ResizeHorizontal,
        ToolCursor::Skew { horizontal: false } => C::ResizeVertical,
        ToolCursor::DragMove { copy: true } => C::Copy,
        ToolCursor::DragMove { copy: false } => C::Move,
        ToolCursor::IBeam => C::Text,
        ToolCursor::ShapeArrow(_) | ToolCursor::TargetArrow | ToolCursor::TableArrow { .. } => {
            C::Default
        }
        ToolCursor::FlowText => C::Alias,
        ToolCursor::Pipette
        | ToolCursor::Bucket
        | ToolCursor::Badge(_)
        | ToolCursor::CurveBadge(..)
        | ToolCursor::Cross
        | ToolCursor::PenNib(_)
        | ToolCursor::PlaceCorner
        | ToolCursor::Nib { .. } => C::Crosshair,
    }
}

const INK: Color32 = Color32::BLACK;
const HALO: Color32 = Color32::WHITE;

/// Paint a drawn cursor with its hot spot at `p`.
pub fn paint(painter: &Painter, cursor: ToolCursor, p: Pos2) {
    match cursor {
        ToolCursor::System(_) => {}
        ToolCursor::ZoomIn | ToolCursor::ZoomOut => {
            magnifier(painter, p, cursor == ToolCursor::ZoomIn)
        }
        ToolCursor::Hand { closed } => hand(painter, p, closed),
        ToolCursor::Pipette => pipette(painter, p),
        ToolCursor::Bucket => bucket(painter, p),
        ToolCursor::Rotate => rotate_arrows(painter, p),
        ToolCursor::Skew { horizontal } => skew_arrows(painter, p, horizontal),
        ToolCursor::ShapeArrow(mark) => shape_arrow(painter, p, mark),
        ToolCursor::DragMove { copy } => drag_move(painter, p, copy),
        ToolCursor::Badge(tool) => {
            cross_hair(painter, p);
            badge(painter, p, tool);
        }
        ToolCursor::CurveBadge(tool, mark) => {
            cross_hair(painter, p);
            badge(painter, p, tool);
            curve_mark_sign(painter, p, mark);
        }
        ToolCursor::Cross => cross_hair(painter, p),
        ToolCursor::IBeam => i_beam(painter, p),
        ToolCursor::TargetArrow => target_arrow(painter, p),
        ToolCursor::PenNib(mark) => {
            pen_nib(painter, p);
            // The sign above and to the right, clear of the nib's body.
            if let Some(m) = mark {
                curve_mark_at(painter, p + PEN_MARK_AT, m);
            }
        }
        ToolCursor::Nib { radius, square } => {
            nib(painter, p, radius, square);
            cross_hair(painter, p);
        }
        ToolCursor::TableArrow { down } => table_arrow(painter, p, down),
        ToolCursor::PlaceCorner => place_corner(painter, p),
        ToolCursor::FlowText => flow_text(painter, p),
    }
}

/// The flow tab's pointer: a white arrow outlined in black, its tip at
/// `tip`, with a small text frame (three lines of text) at its lower
/// right and, under the frame, the tab's down arrow.
fn flow_text(painter: &Painter, tip: Pos2) {
    let at = |x: f32, y: f32| tip + Vec2::new(x, y);
    let arrow = vec![
        at(0.0, 0.0),
        at(0.0, 15.0),
        at(3.6, 11.6),
        at(6.2, 17.4),
        at(8.6, 16.4),
        at(6.0, 10.8),
        at(10.8, 10.8),
    ];
    painter.add(Shape::closed_line(arrow.clone(), Stroke::new(3.0, HALO)));
    for part in [
        vec![
            at(0.0, 0.0),
            at(0.0, 15.0),
            at(3.6, 11.6),
            at(6.0, 10.8),
            at(10.8, 10.8),
        ],
        vec![at(3.6, 11.6), at(6.2, 17.4), at(8.6, 16.4), at(6.0, 10.8)],
    ] {
        painter.add(Shape::convex_polygon(part, HALO, Stroke::NONE));
    }
    painter.add(Shape::closed_line(arrow, Stroke::new(1.0, INK)));
    // The frame with its lines of text.
    let b = Rect::from_min_size(at(10.0, 12.0), Vec2::new(10.0, 9.0));
    painter.rect(
        b.expand(1.0),
        CornerRadius::ZERO,
        HALO,
        Stroke::NONE,
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        b,
        CornerRadius::ZERO,
        Stroke::new(1.0, INK),
        egui::StrokeKind::Inside,
    );
    for (i, w) in [6.0, 6.0, 4.0].into_iter().enumerate() {
        let y = b.top() + 2.5 + i as f32 * 2.0;
        painter.line_segment(
            [
                Pos2::new(b.left() + 2.0, y),
                Pos2::new(b.left() + 2.0 + w, y),
            ],
            Stroke::new(1.0, INK),
        );
    }
    // The tab's down arrow below the frame's middle.
    let c = Pos2::new(b.center().x, b.bottom() + 3.5);
    painter.add(Shape::convex_polygon(
        vec![
            c + Vec2::new(-3.5, -2.0),
            c + Vec2::new(3.5, -2.0),
            c + Vec2::new(0.0, 2.5),
        ],
        INK,
        Stroke::new(1.5, HALO),
    ));
}

/// The import's placement cursor: a right-angle bracket opening to the
/// lower right, its corner at `p`, black on a white halo.
fn place_corner(painter: &Painter, p: Pos2) {
    use egui::emath::GuiRounding as _;
    let c = p.round_to_pixel_center(painter.pixels_per_point());
    let arm = 14.0;
    let lines = [[c, c + Vec2::new(arm, 0.0)], [c, c + Vec2::new(0.0, arm)]];
    for (w, color) in [(4.0, HALO), (2.0, INK)] {
        for l in lines {
            painter.line_segment(l, Stroke::new(w, color));
        }
    }
}

/// How far each arm of the cross hair reaches from its middle, points.
const CROSS_ARM: f32 = 10.0;

/// The cross hair's two lines, crossing on the middle of the pixel under
/// the hot spot `p` (`ppp` pixels a point), so they stay sharp.
pub fn cross_hair_lines(p: Pos2, ppp: f32) -> [[Pos2; 2]; 2] {
    use egui::emath::GuiRounding as _;
    let c = p.round_to_pixel_center(ppp);
    [
        [c - Vec2::new(CROSS_ARM, 0.0), c + Vec2::new(CROSS_ARM, 0.0)],
        [c - Vec2::new(0.0, CROSS_ARM), c + Vec2::new(0.0, CROSS_ARM)],
    ]
}

/// The cross hair, black on a white halo: seen on any page colour (the
/// system's inverts what is under it and can show white, #655).
fn cross_hair(painter: &Painter, p: Pos2) {
    let lines = cross_hair_lines(p, painter.pixels_per_point());
    for (w, color) in [(3.0, HALO), (1.0, INK)] {
        for l in lines {
            painter.line_segment(l, Stroke::new(w, color));
        }
    }
}

/// The text I-beam, black on a white halo, its middle at `p`.
fn i_beam(painter: &Painter, p: Pos2) {
    use egui::emath::GuiRounding as _;
    let c = p.round_to_pixel_center(painter.pixels_per_point());
    let (h, s) = (8.0, 3.0);
    let lines = [
        [c - Vec2::new(0.0, h), c + Vec2::new(0.0, h)],
        [c + Vec2::new(-s, -h), c + Vec2::new(s, -h)],
        [c + Vec2::new(-s, h), c + Vec2::new(s, h)],
    ];
    for (w, color) in [(3.0, HALO), (1.0, INK)] {
        for l in lines {
            painter.line_segment(l, Stroke::new(w, color));
        }
    }
}

/// A short thick arrow with its tip at the hot spot, pointing right (a
/// row) or down (a column), black on a white halo.
fn table_arrow(painter: &Painter, tip: Pos2, down: bool) {
    let pts = |l: f32, w: f32| -> Vec<Pos2> {
        let raw = [
            (0.0, 0.0),
            (-6.0, -5.0),
            (-6.0, -2.0),
            (-l, -2.0),
            (-l, 2.0),
            (-6.0, 2.0),
            (-6.0, 5.0),
        ];
        raw.iter()
            .map(|(x, y)| {
                let (x, y) = (*x * w, *y * w);
                if down {
                    tip + Vec2::new(-y, x)
                } else {
                    tip + Vec2::new(x, y)
                }
            })
            .collect()
    };
    let shape = pts(13.0, 1.0);
    painter.add(Shape::closed_line(shape.clone(), Stroke::new(3.0, HALO)));
    let head: Vec<Pos2> = shape
        .iter()
        .take(3)
        .copied()
        .chain([shape[5], shape[6]])
        .collect();
    painter.add(Shape::convex_polygon(head, INK, Stroke::NONE));
    let tail = vec![shape[2], shape[3], shape[4], shape[5]];
    painter.add(Shape::convex_polygon(tail, INK, Stroke::NONE));
}

/// The colour of the tool's icon on the cursor's badge: the cursor's own
/// ink, not the theme's icon colour, which is light on the dark themes
/// and vanished on the badge's light patch (the badge then looked like a
/// plain grey box, #244).
fn badge_ink() -> Color32 {
    INK
}

/// Where the tool's icon goes for the hot spot `p`: in the quarter below
/// and to the right of the cross hair, close to its middle, clear of the
/// arms and their halo. It sat 9 points off each way, which read as a
/// shape trailing the pointer (#493).
pub fn badge_rect(p: Pos2) -> Rect {
    Rect::from_min_size(p + Vec2::splat(BADGE_GAP), Vec2::splat(15.0))
}

/// The gap between the cross hair's arms and the badge's icon, points.
const BADGE_GAP: f32 = 4.0;

/// The tool's icon below and to the right of the cross hair, on a light
/// patch so it reads on any drawing.
fn badge(painter: &Painter, p: Pos2, tool: Tool) {
    let r = badge_rect(p);
    painter.rect_filled(
        r.expand(1.0),
        CornerRadius::same(3),
        Color32::from_white_alpha(200),
    );
    crate::ui::icons::draw_on(painter, r, tool, badge_ink(), Color32::WHITE);
}

/// The outline of the arrow that picks a command's target object, its
/// tip at `tip`, pointing right.
pub fn target_arrow_outline(tip: Pos2) -> Vec<Pos2> {
    [
        (0.0, 0.0),
        (-11.0, -9.0),
        (-11.0, -4.0),
        (-24.0, -4.0),
        (-24.0, 4.0),
        (-11.0, 4.0),
        (-11.0, 9.0),
    ]
    .iter()
    .map(|(x, y)| tip + Vec2::new(*x, *y))
    .collect()
}

/// The target arrow: black on a white halo.
fn target_arrow(painter: &Painter, tip: Pos2) {
    let o = target_arrow_outline(tip);
    painter.add(Shape::closed_line(o.clone(), Stroke::new(3.0, HALO)));
    let head = vec![o[0], o[1], o[2], o[5], o[6]];
    painter.add(Shape::convex_polygon(head, INK, Stroke::NONE));
    let shaft = vec![o[2], o[3], o[4], o[5]];
    painter.add(Shape::convex_polygon(shaft, INK, Stroke::NONE));
}

/// The way the Pen tool's nib runs from its tip: down and to the right,
/// as an arrow pointer's body does, so the tip is its upper left point
/// (#362).
fn nib_along() -> Vec2 {
    Vec2::new(1.0, 1.0).normalized()
}

/// Where the Pen tool's sign (close, join, add, delete) sits from the
/// tip: above and to the right, where the nib's body does not reach.
const PEN_MARK_AT: Vec2 = Vec2::new(15.0, -6.0);

/// The outline of the Pen tool's nib with its tip at `tip`: the nib
/// runs down and to the right, so the tip is its highest, leftmost
/// point.
pub fn nib_outline(tip: Pos2) -> Vec<Pos2> {
    // Along the nib (u, from the tip) and across it (v).
    let u = nib_along();
    let v = Vec2::new(u.y, -u.x);
    [
        (0.0, 0.0),
        (9.0, -4.5),
        (15.0, -4.5),
        (17.0, -3.0),
        (17.0, 3.0),
        (15.0, 4.5),
        (9.0, 4.5),
    ]
    .iter()
    .map(|(a, b)| tip + u * *a + v * *b)
    .collect()
}

/// The Pen tool's cursor: a black nib on a white halo, its slit and
/// breather hole white, and a dot at the tip where the node goes.
fn pen_nib(painter: &Painter, tip: Pos2) {
    let outline = nib_outline(tip);
    painter.add(Shape::closed_line(outline.clone(), Stroke::new(2.5, HALO)));
    painter.add(Shape::convex_polygon(outline, INK, Stroke::NONE));
    let u = nib_along();
    painter.line_segment([tip + u * 2.5, tip + u * 8.0], Stroke::new(1.0, HALO));
    painter.circle_filled(tip + u * 9.5, 1.6, HALO);
    // The tip: a dot ringed in white, exactly on the hot spot.
    painter.circle_filled(tip, 2.4, HALO);
    painter.circle_filled(tip, 1.3, INK);
}

/// Above and to the right of the cross hair: a node with the line going
/// on from it (join), or a ring (close).
fn curve_mark_sign(painter: &Painter, p: Pos2, mark: CurveMark) {
    curve_mark_at(painter, p + Vec2::new(12.0, -12.0), mark);
}

/// A curve tool's sign (see [`curve_mark_sign`]) centred at `c`.
fn curve_mark_at(painter: &Painter, c: Pos2, mark: CurveMark) {
    match mark {
        CurveMark::Join => {
            haloed(painter, &[vec![c, c + Vec2::new(7.0, -5.0)]], 1.4);
            let r = Rect::from_center_size(c, Vec2::splat(5.0));
            painter.rect(
                r.expand(1.0),
                CornerRadius::ZERO,
                HALO,
                Stroke::NONE,
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(r, CornerRadius::ZERO, INK);
        }
        CurveMark::Close => {
            painter.circle_stroke(c, 4.0, Stroke::new(3.4, HALO));
            painter.circle_stroke(c, 4.0, Stroke::new(1.4, INK));
        }
        CurveMark::AddNode => {
            haloed(
                painter,
                &[
                    vec![c + Vec2::new(-4.0, 0.0), c + Vec2::new(4.0, 0.0)],
                    vec![c + Vec2::new(0.0, -4.0), c + Vec2::new(0.0, 4.0)],
                ],
                1.4,
            );
        }
        CurveMark::DeleteNode => {
            haloed(
                painter,
                &[vec![c + Vec2::new(-4.0, 0.0), c + Vec2::new(4.0, 0.0)]],
                1.4,
            );
        }
        CurveMark::EditNode => {
            // A hollow node with a small arrow head beside it.
            let r = Rect::from_center_size(c, Vec2::splat(6.0));
            painter.rect(
                r,
                CornerRadius::ZERO,
                HALO,
                Stroke::new(1.2, INK),
                egui::StrokeKind::Inside,
            );
            haloed(
                painter,
                &[vec![c + Vec2::new(-7.0, 3.0), c + Vec2::new(-4.5, 0.5)]],
                1.2,
            );
        }
    }
}

/// The nib's outline around the hot spot: black with a white halo.
fn nib(painter: &Painter, c: Pos2, radius: f32, square: bool) {
    for (w, color) in [(3.0, HALO), (1.0, INK)] {
        let s = Stroke::new(w, color);
        if square {
            painter.rect_stroke(
                Rect::from_center_size(c, Vec2::splat(radius * 2.0)),
                CornerRadius::ZERO,
                s,
                egui::StrokeKind::Middle,
            );
        } else {
            painter.circle_stroke(c, radius, s);
        }
    }
}

/// Draw `segments` twice: a white halo, then the ink.
fn haloed(painter: &Painter, lines: &[Vec<Pos2>], width: f32) {
    for l in lines {
        painter.add(Shape::line(l.clone(), Stroke::new(width + 2.5, HALO)));
    }
    for l in lines {
        painter.add(Shape::line(l.clone(), Stroke::new(width, INK)));
    }
}

/// An arrow that turns around the hot spot: most of a circle with a head
/// at its end.
fn rotate_arrows(painter: &Painter, c: Pos2) {
    let r = 7.0;
    let arc: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = (-200.0 + i as f32 * 11.0f32).to_radians();
            c + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    let end = *arc.last().unwrap_or(&c);
    let a = (-200.0f32 + 24.0 * 11.0).to_radians();
    // The tangent at the end, along the turn.
    let t = Vec2::new(-a.sin(), a.cos());
    let n = Vec2::new(a.cos(), a.sin());
    let head = vec![
        end + n * 3.5 - t * 3.5,
        end + t * 1.5,
        end - n * 3.5 - t * 3.5,
    ];
    haloed(painter, &[arc, head], 1.6);
    painter.circle_filled(c, 1.3, INK);
}

/// Two half arrows going opposite ways along the slant: sideways for a
/// horizontal skew, up and down for a vertical one.
fn skew_arrows(painter: &Painter, c: Pos2, horizontal: bool) {
    // Built sideways, then turned for a vertical skew.
    let at = |x: f32, y: f32| {
        if horizontal {
            c + Vec2::new(x, y)
        } else {
            c + Vec2::new(y, -x)
        }
    };
    let top = vec![at(-8.0, -2.5), at(8.0, -2.5), at(4.5, -6.0)];
    let bottom = vec![at(8.0, 2.5), at(-8.0, 2.5), at(-4.5, 6.0)];
    haloed(painter, &[top, bottom], 1.6);
}

/// The Shape tool's pointer: a solid arrow with its tip on the hot spot
/// and, at its lower right, a hollow square over a node, a wave over a
/// segment or a dot over a control point.
fn shape_arrow(painter: &Painter, tip: Pos2, mark: ShapeMark) {
    let at = |x: f32, y: f32| tip + Vec2::new(x, y);
    let head = vec![at(0.0, 0.0), at(0.0, 14.0), at(10.0, 10.0)];
    let tail = vec![at(3.2, 10.6), at(5.6, 9.7), at(8.6, 15.7), at(6.2, 16.6)];
    for poly in [&head, &tail] {
        painter.add(Shape::convex_polygon(
            poly.clone(),
            HALO,
            Stroke::new(2.4, HALO),
        ));
    }
    for poly in [head, tail] {
        painter.add(Shape::convex_polygon(poly, INK, Stroke::new(0.6, INK)));
    }
    let m = at(14.0, 15.0);
    match mark {
        ShapeMark::Nothing => {}
        ShapeMark::Node => {
            let r = Rect::from_center_size(m, Vec2::splat(6.0));
            painter.rect(
                r,
                CornerRadius::ZERO,
                HALO,
                Stroke::new(1.2, INK),
                egui::StrokeKind::Inside,
            );
        }
        ShapeMark::Segment => {
            let wave: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let x = -4.5 + i as f32 * 0.75;
                    m + Vec2::new(x, -2.2 * (x * 0.7).sin())
                })
                .collect();
            haloed(painter, &[wave], 1.4);
        }
        ShapeMark::Control => {
            painter.circle(m, 2.6, INK, Stroke::new(1.2, HALO));
        }
    }
}

/// The pointer while objects are moved: a white arrow outlined in black
/// with its tip on the hot spot, a dotted box under its tail, and with
/// `copy` a plus in a square at the box's lower right.
fn drag_move(painter: &Painter, tip: Pos2, copy: bool) {
    let at = |x: f32, y: f32| tip + Vec2::new(x, y);
    let arrow = vec![
        at(0.0, 0.0),
        at(0.0, 15.0),
        at(3.6, 11.6),
        at(6.2, 17.4),
        at(8.6, 16.4),
        at(6.0, 10.8),
        at(10.8, 10.8),
    ];
    painter.add(Shape::closed_line(arrow.clone(), Stroke::new(3.0, HALO)));
    // Filled as two convex parts (the head and the tail), outlined whole.
    for part in [
        vec![
            at(0.0, 0.0),
            at(0.0, 15.0),
            at(3.6, 11.6),
            at(6.0, 10.8),
            at(10.8, 10.8),
        ],
        vec![at(3.6, 11.6), at(6.2, 17.4), at(8.6, 16.4), at(6.0, 10.8)],
    ] {
        painter.add(Shape::convex_polygon(part, HALO, Stroke::NONE));
    }
    painter.add(Shape::closed_line(arrow, Stroke::new(1.0, INK)));
    // The dotted box.
    let b = Rect::from_min_size(at(7.0, 13.0), Vec2::new(11.0, 8.0));
    painter.rect_filled(b.expand(1.0), CornerRadius::ZERO, HALO);
    let corners = [
        b.left_top(),
        b.right_top(),
        b.right_bottom(),
        b.left_bottom(),
    ];
    for i in 0..4 {
        painter.extend(Shape::dashed_line(
            &[corners[i], corners[(i + 1) % 4]],
            Stroke::new(1.0, INK),
            1.5,
            1.5,
        ));
    }
    if copy {
        copy_plus(painter, at(15.0, 17.0));
    }
}

/// The plus in a square that says a drag leaves the original, at the
/// lower right of the pointer `p` (a scale, turn or slant after a right
/// click; a move's own pointer draws it beside its box).
pub fn paint_copy_plus(ctx: &egui::Context, p: Pos2) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("tool_cursor_copy"),
    ));
    copy_plus(&painter, p + Vec2::new(14.0, 16.0));
}

/// The colour eyedroppers' readout beside the pointer at `p` (#990): a
/// swatch, the colour's name when it has one, and its values. It sits
/// below and right of the pointer, and on the other side where the
/// window ends. Plain rectangles and a few short texts, whose layout
/// egui keeps from frame to frame.
pub fn paint_color_readout(ctx: &egui::Context, p: Pos2, r: &crate::eyedropper::Reading) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("color_readout"),
    ));
    let font = egui::FontId::proportional(11.0);
    let bold = crate::theme::bold(11.0);
    let text = crate::theme::Tokens::text();
    let mut galleys = Vec::with_capacity(r.lines.len() + 1);
    if !r.name.is_empty() {
        galleys.push(painter.layout_no_wrap(r.name.clone(), bold, text));
    }
    for l in &r.lines {
        galleys.push(painter.layout_no_wrap(l.clone(), font.clone(), text));
    }
    const PAD: f32 = 5.0;
    const SWATCH: f32 = 14.0;
    let text_w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let text_h: f32 = galleys.iter().map(|g| g.size().y).sum();
    let size = Vec2::new(PAD * 3.0 + SWATCH + text_w, PAD * 2.0 + text_h.max(SWATCH));
    let screen = ctx.content_rect();
    let mut at = p + Vec2::new(18.0, 20.0);
    if at.x + size.x > screen.right() {
        at.x = p.x - 8.0 - size.x;
    }
    if at.y + size.y > screen.bottom() {
        at.y = p.y - 8.0 - size.y;
    }
    let frame = Rect::from_min_size(at, size);
    painter.rect(
        frame,
        CornerRadius::same(3),
        crate::theme::Tokens::panel(),
        Stroke::new(1.0, crate::theme::Tokens::control_border()),
        egui::StrokeKind::Inside,
    );
    let [cr, cg, cb] = r.color.to_rgb8();
    let swatch = Rect::from_min_size(at + Vec2::splat(PAD), Vec2::splat(SWATCH));
    painter.rect(
        swatch,
        CornerRadius::ZERO,
        Color32::from_rgb(cr, cg, cb),
        Stroke::new(1.0, crate::theme::Tokens::control_border()),
        egui::StrokeKind::Inside,
    );
    let mut y = at.y + PAD;
    let x = swatch.right() + PAD;
    for g in galleys {
        let h = g.size().y;
        painter.galley(Pos2::new(x, y), g, text);
        y += h;
    }
}

/// A short note beside the pointer at `p`, as the colour readout sits
/// (below and right of it, on the other side where the window ends),
/// wrapped to a narrow box: the Variable Outline tool's word on why the
/// outline comes to a point.
pub fn paint_pointer_note(ctx: &egui::Context, p: Pos2, note: &str) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("pointer_note"),
    ));
    let text = crate::theme::Tokens::text();
    let galley = painter.layout(
        note.to_owned(),
        egui::FontId::proportional(11.0),
        text,
        260.0,
    );
    const PAD: f32 = 6.0;
    let size = galley.size() + Vec2::splat(PAD * 2.0);
    let screen = ctx.content_rect();
    let mut at = p + Vec2::new(18.0, 20.0);
    if at.x + size.x > screen.right() {
        at.x = p.x - 8.0 - size.x;
    }
    if at.y + size.y > screen.bottom() {
        at.y = p.y - 8.0 - size.y;
    }
    painter.rect(
        Rect::from_min_size(at, size),
        CornerRadius::same(3),
        crate::theme::Tokens::panel(),
        Stroke::new(1.0, crate::theme::Tokens::control_border()),
        egui::StrokeKind::Inside,
    );
    painter.galley(at + Vec2::splat(PAD), galley, text);
}

/// A 9-point square with a plus, its top left at `at`.
fn copy_plus(painter: &Painter, at: Pos2) {
    let s = Rect::from_min_size(at, Vec2::splat(9.0));
    painter.rect(
        s,
        CornerRadius::ZERO,
        HALO,
        Stroke::new(1.0, INK),
        egui::StrokeKind::Inside,
    );
    let c = s.center();
    painter.line_segment(
        [c - Vec2::new(2.5, 0.0), c + Vec2::new(2.5, 0.0)],
        Stroke::new(1.2, INK),
    );
    painter.line_segment(
        [c - Vec2::new(0.0, 2.5), c + Vec2::new(0.0, 2.5)],
        Stroke::new(1.2, INK),
    );
}

/// A lens centred on the hot spot with a handle to the lower right and a
/// plus or a minus inside.
fn magnifier(painter: &Painter, c: Pos2, plus: bool) {
    let r = 6.5;
    let a = c + Vec2::splat(r * 0.72);
    let b = c + Vec2::splat(r * 0.72 + 7.0);
    painter.line_segment([a, b], Stroke::new(5.0, HALO));
    painter.circle(c, r, Color32::from_white_alpha(230), Stroke::new(3.0, HALO));
    painter.line_segment([a, b], Stroke::new(3.0, INK));
    painter.circle_stroke(c, r, Stroke::new(1.5, INK));
    let s = Stroke::new(1.4, INK);
    let d = 3.2;
    painter.line_segment([c - Vec2::new(d, 0.0), c + Vec2::new(d, 0.0)], s);
    if plus {
        painter.line_segment([c - Vec2::new(0.0, d), c + Vec2::new(0.0, d)], s);
    }
}

/// An open hand (fingers up) or a closed one (fingers folded): outlined
/// fingers and thumb drawn as capsules, then the palm over their roots.
/// The hot spot is the middle of the palm.
fn hand(painter: &Painter, c: Pos2, closed: bool) {
    let capsule = |a: Pos2, b: Pos2| {
        painter.line_segment([a, b], Stroke::new(4.2, INK));
        painter.line_segment([a, b], Stroke::new(2.2, HALO));
    };
    let tops: [f32; 4] = if closed {
        [-2.5, -3.5, -3.5, -2.0]
    } else {
        [-8.0, -9.5, -9.0, -6.5]
    };
    for (x, top) in [-4.5f32, -1.5, 1.5, 4.5].iter().zip(tops) {
        capsule(c + Vec2::new(*x, top), c + Vec2::new(*x, 2.0));
    }
    if closed {
        capsule(c + Vec2::new(-7.5, 2.0), c + Vec2::new(-4.5, 4.0));
    } else {
        capsule(c + Vec2::new(-9.5, -0.5), c + Vec2::new(-5.5, 4.0));
    }
    let palm = Rect::from_min_max(c + Vec2::new(-6.2, -1.0), c + Vec2::new(6.2, 8.0));
    painter.rect(
        palm,
        CornerRadius::same(3),
        HALO,
        Stroke::new(1.0, INK),
        egui::StrokeKind::Inside,
    );
    // Hide the palm's top edge between the fingers so they join the palm.
    for x in [-4.5f32, -1.5, 1.5, 4.5] {
        painter.line_segment(
            [c + Vec2::new(x, -0.5), c + Vec2::new(x, 1.5)],
            Stroke::new(2.2, HALO),
        );
    }
}

/// A pipette: the tip at the hot spot, the glass tube rising to the upper
/// right and the rubber bulb at its end.
fn pipette(painter: &Painter, tip: Pos2) {
    let dir = Vec2::new(1.0, -1.0).normalized();
    let tube_start = tip + dir * 1.5;
    let tube_end = tip + dir * 11.0;
    let bulb = tip + dir * 14.5;
    painter.line_segment([tube_start, tube_end], Stroke::new(5.0, HALO));
    painter.circle_filled(bulb, 4.4, HALO);
    painter.line_segment([tube_start, tube_end], Stroke::new(2.6, INK));
    painter.circle_filled(bulb, 3.0, INK);
    painter.circle_filled(tip, 1.4, INK);
}

/// A tipped paint bucket pouring onto the hot spot.
fn bucket(painter: &Painter, spot: Pos2) {
    // The bucket body, a slanted box above and to the right of the spot.
    let o = spot + Vec2::new(4.0, -12.0);
    let pts = [
        o,
        o + Vec2::new(9.0, 4.0),
        o + Vec2::new(5.5, 12.0),
        o + Vec2::new(-3.5, 8.0),
    ];
    painter.add(Shape::convex_polygon(
        pts.to_vec(),
        HALO,
        Stroke::new(3.0, HALO),
    ));
    painter.add(Shape::convex_polygon(
        pts.to_vec(),
        HALO,
        Stroke::new(1.3, INK),
    ));
    // The handle.
    painter.line_segment(
        [o + Vec2::new(1.0, -1.0), o + Vec2::new(7.0, 2.0)],
        Stroke::new(1.2, INK),
    );
    // Paint flowing from the lip to the spot.
    painter.line_segment([o + Vec2::new(-3.0, 7.0), spot], Stroke::new(3.0, HALO));
    painter.line_segment([o + Vec2::new(-3.0, 7.0), spot], Stroke::new(1.6, INK));
    painter.circle_filled(spot, 2.0, INK);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a software adapter the drawn pointers give way to the system's
    /// nearest (#616): the cross hair for the drawing tools, the I-beam
    /// for text, the hands and magnifiers as the system has them.
    #[test]
    fn a_software_adapter_shows_the_system_pointers() {
        use egui::CursorIcon as C;
        assert_eq!(system_icon(ToolCursor::Cross), C::Crosshair);
        assert_eq!(
            system_icon(ToolCursor::Badge(Tool::Rectangle)),
            C::Crosshair
        );
        assert_eq!(system_icon(ToolCursor::IBeam), C::Text);
        assert_eq!(system_icon(ToolCursor::Hand { closed: true }), C::Grabbing);
        assert_eq!(system_icon(ToolCursor::ZoomOut), C::ZoomOut);
        assert_eq!(system_icon(ToolCursor::DragMove { copy: true }), C::Copy);
        assert_eq!(system_icon(ToolCursor::System(C::Move)), C::Move);
        // Nothing maps to a hidden pointer.
        for c in [
            ToolCursor::Pipette,
            ToolCursor::Bucket,
            ToolCursor::Rotate,
            ToolCursor::Skew { horizontal: false },
            ToolCursor::TargetArrow,
            ToolCursor::TableArrow { down: true },
        ] {
            assert_ne!(system_icon(c), C::None);
        }
    }

    /// A drawn pointer hides the system one with a blank pointer image
    /// on Windows, where hiding the pointer took effect only when the
    /// mouse next moved (#1026, #1124): one fully transparent image, the
    /// same bytes every frame so the integration makes one cursor of it.
    /// Elsewhere the pointer is hidden, and a system pointer never comes
    /// with an image, also when it was asked for after a drawn one in the
    /// same frame.
    #[test]
    fn a_drawn_pointer_hides_the_system_one_with_a_blank_image_on_windows() {
        use egui::CursorIcon as C;
        let blank = hidden_pointer(true).unwrap();
        let side = usize::from(BLANK_SIDE);
        assert_eq!(blank.size, [BLANK_SIDE, BLANK_SIDE]);
        assert_eq!(blank.hotspot, [0, 0]);
        assert_eq!(blank.rgba.len(), side * side * 4);
        assert!(blank.rgba.iter().all(|b| *b == 0));
        let again = hidden_pointer(true).unwrap();
        assert!(std::sync::Arc::ptr_eq(&blank.rgba, &again.rgba));
        assert!(hidden_pointer(false).is_none());

        // The end of a frame, on Windows and elsewhere.
        for windows in [true, false] {
            let mut out = egui::PlatformOutput {
                cursor_icon: C::None,
                ..Default::default()
            };
            blank_when_hidden(&mut out, windows);
            assert_eq!(out.cursor_image.is_some(), windows);
            assert_eq!(out.cursor_icon, C::None);
            for icon in [C::Default, C::Text, C::Crosshair, C::Move] {
                let mut out = egui::PlatformOutput {
                    cursor_icon: icon,
                    ..Default::default()
                };
                blank_when_hidden(&mut out, windows);
                assert!(out.cursor_image.is_none(), "{icon:?}");
            }
        }

        let ctx = crate::theme::ui_context();
        let at = Pos2::new(120.0, 80.0);
        // `later`: a system pointer something else in the frame asks for
        // after the tool's pointer.
        let frame = |cursor: ToolCursor, later: Option<C>| {
            let input = egui::RawInput {
                events: vec![egui::Event::PointerMoved(at)],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui.ctx(), cursor, Some(at));
                if let Some(icon) = later {
                    ui.ctx().set_cursor_icon(icon);
                }
                finish_frame(ui.ctx());
            });
            out.textures_delta.clear();
            (
                out.platform_output.cursor_icon,
                out.platform_output.cursor_image.is_some(),
            )
        };
        let frame_of = |cursor: ToolCursor| frame(cursor, None);
        assert_eq!(
            frame_of(ToolCursor::System(C::Default)),
            (C::Default, false)
        );
        // The Pen tool picked by its shortcut: the drawn nib takes the
        // place of the system arrow.
        assert_eq!(
            frame_of(ToolCursor::PenNib(None)),
            (C::None, cfg!(target_os = "windows"))
        );
        assert_eq!(
            frame_of(ToolCursor::PenNib(None)),
            (C::None, cfg!(target_os = "windows"))
        );
        // Back to the Pick tool: the arrow, and no image.
        assert_eq!(
            frame_of(ToolCursor::System(C::Default)),
            (C::Default, false)
        );
        // The nib, then a text box's I-beam in the same frame: the I-beam,
        // never the blank image in its place.
        assert_eq!(
            frame(ToolCursor::PenNib(None), Some(C::Text)),
            (C::Text, false)
        );
        // Every drawn pointer takes the blank image on Windows.
        for cursor in [
            ToolCursor::Cross,
            ToolCursor::IBeam,
            ToolCursor::Badge(Tool::Rectangle),
            ToolCursor::Hand { closed: false },
        ] {
            assert_eq!(
                frame_of(cursor),
                (C::None, cfg!(target_os = "windows")),
                "{cursor:?}"
            );
        }
    }

    /// Turns bitmap pointers on for a test, and off again after it.
    struct BitmapPointers;
    impl BitmapPointers {
        fn on() -> Self {
            TEST_BITMAP_POINTERS.with(|b| b.set(true));
            BitmapPointers
        }
    }
    impl Drop for BitmapPointers {
        fn drop(&mut self) {
            TEST_BITMAP_POINTERS.with(|b| b.set(false));
        }
    }

    /// The pixel of a pointer bitmap at `dx`, `dy` from its hot spot.
    fn pixel(image: &egui::CustomCursorImage, dx: i32, dy: i32) -> [u8; 4] {
        let x = (i32::from(image.hotspot[0]) + dx) as usize;
        let y = (i32::from(image.hotspot[1]) + dy) as usize;
        let i = (y * usize::from(image.size[0]) + x) * 4;
        [
            image.rgba[i],
            image.rgba[i + 1],
            image.rgba[i + 2],
            image.rgba[i + 3],
        ]
    }

    /// A drawn pointer is the system's pointer where the system takes a
    /// bitmap for it (#2136): painted in the window it showed where the
    /// mouse was a frame or more ago, so every tool but the Pick tools
    /// (the system's arrow) had a pointer that trailed the hand. The
    /// frame hands over the same drawing as a bitmap with the hot spot
    /// where the cross hair crosses, the same bytes every frame, paints
    /// no pointer of its own, and names the nearest system pointer for a
    /// window that cannot take the bitmap. Something else asking for a
    /// system pointer later in the frame still wins.
    #[test]
    fn a_drawn_pointer_is_handed_to_the_system_as_a_bitmap() {
        use egui::CursorIcon as C;
        let _on = BitmapPointers::on();
        let ctx = crate::theme::ui_context();
        let at = Pos2::new(120.0, 80.0);
        let frame = |cursor: ToolCursor, later: Option<C>| {
            let input = egui::RawInput {
                events: vec![egui::Event::PointerMoved(at)],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui.ctx(), cursor, Some(at));
                if let Some(icon) = later {
                    ui.ctx().set_cursor_icon(icon);
                }
                finish_frame(ui.ctx());
            });
            out.textures_delta.clear();
            // What the frame painted of a pointer: the cross hair's ink.
            let painted = out
                .shapes
                .iter()
                .filter(|c| {
                    matches!(&c.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == INK)
                })
                .count();
            (
                out.platform_output.cursor_icon,
                out.platform_output.cursor_image.clone(),
                painted,
            )
        };
        let rect = ToolCursor::Badge(Tool::Rectangle);
        let (icon, image, painted) = frame(rect, None);
        let image = image.expect("the pointer as a bitmap");
        assert_eq!(icon, C::Crosshair, "the nearest system pointer goes along");
        assert_eq!(painted, 0, "no pointer painted in the window");
        let [w, h] = image.size.map(usize::from);
        assert_eq!(image.rgba.len(), w * h * 4);
        assert!(w <= BITMAP_MAX && h <= BITMAP_MAX);
        // The cross hair's arms either side of the hot spot, the sign
        // below and to the right: about 21 by 21 with it.
        assert!((24..=40).contains(&w) && (24..=40).contains(&h), "{w}x{h}");
        let [hx, hy] = image.hotspot.map(usize::from);
        assert!(
            (9..=14).contains(&hx) && (9..=14).contains(&hy),
            "{hx},{hy}"
        );
        // Black where the arms cross and along them, the white halo
        // beside them, nothing in the empty corner above and to the
        // right, the sign's patch below and to the right.
        let dark = |p: [u8; 4]| p[..3].iter().all(|c| *c < 100) && p[3] > 250;
        let light = |p: [u8; 4]| p[..3].iter().all(|c| *c > 200) && p[3] > 100;
        for (dx, dy) in [(0, 0), (6, 0), (-6, 0), (0, -6), (0, 6)] {
            assert!(
                dark(pixel(&image, dx, dy)),
                "{dx},{dy}: {:?}",
                pixel(&image, dx, dy)
            );
        }
        for (dx, dy) in [(6, 1), (6, -1), (1, -6), (-1, -6)] {
            assert!(
                light(pixel(&image, dx, dy)),
                "{dx},{dy}: {:?}",
                pixel(&image, dx, dy)
            );
        }
        assert_eq!(pixel(&image, 7, -7)[3], 0);
        assert!(pixel(&image, 8, 8)[3] > 100);
        // The next frames: the very same bytes, so the window keeps the
        // one system pointer it made of them.
        for _ in 0..3 {
            let (_, again, _) = frame(rect, None);
            assert!(std::sync::Arc::ptr_eq(
                &again.expect("bitmap").rgba,
                &image.rgba
            ));
        }
        // Another tool: another bitmap.
        let (_, ellipse, _) = frame(ToolCursor::Badge(Tool::Ellipse), None);
        assert_ne!(ellipse.expect("bitmap").rgba, image.rgba);
        // A text box under the pointer asks for the I-beam after the
        // tool's pointer: the I-beam, no bitmap.
        let (icon, image, _) = frame(rect, Some(C::Text));
        assert_eq!(icon, C::Text);
        assert!(image.is_none());
        // The system's own pointers stay the system's.
        let (icon, image, _) = frame(ToolCursor::System(C::Default), None);
        assert_eq!(icon, C::Default);
        assert!(image.is_none());
        // A nib wider than a pointer bitmap is painted as before, the
        // system pointer hidden.
        let wide = ToolCursor::Nib {
            radius: 120.0,
            square: false,
        };
        let (icon, image, painted) = frame(wide, None);
        assert_eq!(icon, C::None);
        assert_eq!(
            image.is_some(),
            cfg!(target_os = "windows"),
            "the blank one"
        );
        assert!(painted > 0);
        // Off (every system but Windows): painted, as it was.
        drop(_on);
        let (icon, _, painted) = frame(rect, None);
        assert_eq!(icon, C::None);
        assert!(painted > 0);
    }

    /// Every drawn pointer has a bitmap with its hot spot inside it, in
    /// the screen's pixels: twice as many on a screen of two pixels a
    /// point, with the hot spot twice as far in. A small nib has one of
    /// its size; the bitmap of a pointer is kept, not drawn again.
    #[test]
    fn every_drawn_pointer_has_a_bitmap_in_the_screens_pixels() {
        let all = [
            ToolCursor::ZoomIn,
            ToolCursor::ZoomOut,
            ToolCursor::Hand { closed: false },
            ToolCursor::Hand { closed: true },
            ToolCursor::Pipette,
            ToolCursor::Bucket,
            ToolCursor::Rotate,
            ToolCursor::Skew { horizontal: true },
            ToolCursor::Skew { horizontal: false },
            ToolCursor::ShapeArrow(ShapeMark::Nothing),
            ToolCursor::ShapeArrow(ShapeMark::Node),
            ToolCursor::ShapeArrow(ShapeMark::Segment),
            ToolCursor::ShapeArrow(ShapeMark::Control),
            ToolCursor::DragMove { copy: false },
            ToolCursor::DragMove { copy: true },
            ToolCursor::Badge(Tool::Rectangle),
            ToolCursor::Badge(Tool::Text),
            ToolCursor::Cross,
            ToolCursor::IBeam,
            ToolCursor::CurveBadge(Tool::Bezier, CurveMark::Join),
            ToolCursor::CurveBadge(Tool::Freehand, CurveMark::Close),
            ToolCursor::TargetArrow,
            ToolCursor::PenNib(None),
            ToolCursor::PenNib(Some(CurveMark::AddNode)),
            ToolCursor::Nib {
                radius: 8.0,
                square: false,
            },
            ToolCursor::Nib {
                radius: 20.0,
                square: true,
            },
            ToolCursor::TableArrow { down: true },
            ToolCursor::TableArrow { down: false },
            ToolCursor::PlaceCorner,
            ToolCursor::FlowText,
        ];
        for cursor in all {
            let one = bitmap(cursor, 1.0).unwrap_or_else(|| panic!("{cursor:?}"));
            let two = bitmap(cursor, 2.0).unwrap_or_else(|| panic!("{cursor:?} at 2"));
            for image in [&one, &two] {
                let [w, h] = image.size.map(usize::from);
                assert_eq!(image.rgba.len(), w * h * 4, "{cursor:?}");
                assert!(
                    w > 4 && h > 4 && w <= BITMAP_MAX && h <= BITMAP_MAX,
                    "{cursor:?}"
                );
                assert!(
                    usize::from(image.hotspot[0]) < w && usize::from(image.hotspot[1]) < h,
                    "{cursor:?}"
                );
                assert!(
                    image.rgba.chunks_exact(4).any(|p| p[3] == 255),
                    "{cursor:?} draws something solid"
                );
            }
            // (The soft edge is a pixel wide at any scale.)
            let grown = |a: u16, b: u16| (i32::from(b) - 2 * i32::from(a)).abs() <= 6;
            assert!(
                grown(one.size[0], two.size[0]) && grown(one.size[1], two.size[1]),
                "{cursor:?}: {:?} and {:?}",
                one.size,
                two.size
            );
            assert!(
                grown(one.hotspot[0], two.hotspot[0]) && grown(one.hotspot[1], two.hotspot[1]),
                "{cursor:?}: {:?} and {:?}",
                one.hotspot,
                two.hotspot
            );
            // Kept.
            let again = bitmap(cursor, 1.0).expect("bitmap");
            assert!(std::sync::Arc::ptr_eq(&again.rgba, &one.rgba), "{cursor:?}");
        }
        // A nib's bitmap is as wide as the nib: 8 points either side of
        // the hot spot and its outline.
        let nib = bitmap(
            ToolCursor::Nib {
                radius: 8.0,
                square: false,
            },
            1.0,
        )
        .expect("nib");
        // (The cross hair's arms reach 10 points, past this nib.)
        assert!((21..=27).contains(&nib.size[0]), "{:?}", nib.size);
        let wide = bitmap(
            ToolCursor::Nib {
                radius: 30.0,
                square: false,
            },
            1.0,
        )
        .expect("nib");
        assert!((61..=69).contains(&wide.size[0]), "{:?}", wide.size);
        // Too wide for a pointer: none, the nib is painted.
        for radius in [47.0, 120.0, 400.0, f32::NAN] {
            let nib = ToolCursor::Nib {
                radius,
                square: false,
            };
            assert!(bitmap(nib, 1.0).is_none(), "{radius}");
        }
        assert!(bitmap(ToolCursor::Cross, 0.0).is_none());
        assert!(bitmap(ToolCursor::System(egui::CursorIcon::Default), 1.0).is_none());
        // Saved for a look when asked (`POINTER_BITMAPS=<folder>`).
        if let Ok(dir) = std::env::var("POINTER_BITMAPS") {
            for (i, cursor) in all.into_iter().enumerate() {
                for ppp in [1.0f32, 2.0] {
                    let image = bitmap(cursor, ppp).expect("bitmap");
                    let [w, h] = image.size.map(u32::from);
                    let img = image::RgbaImage::from_raw(w, h, image.rgba.to_vec()).expect("image");
                    let path = format!("{dir}/pointer-{i:02}-{ppp}.png");
                    img.save(path).expect("saved");
                }
            }
        }
    }

    /// The cross hair (alone, with a tool's badge, with a nib) and the
    /// I-beam are drawn black on a white halo with the system pointer
    /// hidden, so they show on a page of any colour: the system's invert
    /// what is under them and could show white on a white page (#655). The
    /// cross hair's lines cross on the hot spot; the system's arrows stay.
    #[test]
    fn the_cross_hair_and_the_i_beam_are_drawn_black_on_white() {
        let ctx = crate::theme::ui_context();
        let at = Pos2::new(200.0, 150.0);
        for c in [
            ToolCursor::Badge(Tool::Rectangle),
            ToolCursor::CurveBadge(Tool::Bezier, CurveMark::Join),
            ToolCursor::Nib {
                radius: 8.0,
                square: false,
            },
            ToolCursor::Cross,
            ToolCursor::IBeam,
        ] {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(ui.ctx(), c, Some(at)));
            out.textures_delta.clear();
            assert_eq!(
                out.platform_output.cursor_icon,
                egui::CursorIcon::None,
                "{c:?}"
            );
            let mut ink = Vec::new();
            let mut halo = 0;
            for clipped in &out.shapes {
                if let egui::Shape::LineSegment { points, stroke } = &clipped.shape {
                    if stroke.color == INK && (stroke.width - 1.0).abs() < 1e-3 {
                        ink.push(*points);
                    } else if stroke.color == HALO && stroke.width > 2.0 {
                        halo += 1;
                    }
                }
            }
            let through = |p: &[Pos2; 2]| {
                let (a, b) = (p[0], p[1]);
                let d = b - a;
                let t = ((at - a).dot(d) / d.length_sq()).clamp(0.0, 1.0);
                (a + d * t - at).length() < 0.75
            };
            if c != ToolCursor::IBeam {
                let level = ink
                    .iter()
                    .filter(|p| through(p) && (p[0].y - p[1].y).abs() < 1e-3)
                    .count();
                let upright = ink
                    .iter()
                    .filter(|p| through(p) && (p[0].x - p[1].x).abs() < 1e-3)
                    .count();
                assert!(level >= 1 && upright >= 1, "{c:?}: {ink:?}");
            } else {
                assert!(ink.iter().any(through), "{c:?}");
            }
            assert!(halo >= ink.len().min(2), "a halo under the ink: {c:?}");
        }
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(
                ui.ctx(),
                ToolCursor::System(egui::CursorIcon::Move),
                Some(at),
            )
        });
        assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::Move);
        // The lines cross on the middle of the pixel under the hot spot.
        let [h, v] = cross_hair_lines(Pos2::new(10.2, 20.7), 1.0);
        assert_eq!((h[0].y, v[0].x), (20.5, 10.5));
        assert_eq!(h[1].x - 10.5, 10.5 - h[0].x);
    }

    /// The tool's icon sits close to the cross hair's middle, in its lower
    /// right quarter, its patch clear of the arms and their halo (#493).
    #[test]
    fn the_badge_sits_close_to_the_cross_hair() {
        let p = Pos2::new(200.5, 150.5);
        let r = badge_rect(p);
        assert!(r.min.x - p.x <= 5.0 && r.min.y - p.y <= 5.0, "{r:?}");
        let patch = r.expand(1.0);
        // The halo is 3 points wide, centred on the arms.
        assert!(
            patch.min.x > p.x + 1.5 && patch.min.y > p.y + 1.5,
            "{patch:?}"
        );
    }

    /// The badge's icon reads on its light patch under every theme: the
    /// theme's icon colour is light on the dark ones.
    #[test]
    fn the_badge_icon_is_dark_under_every_theme() {
        for theme in crate::theme::Theme::ALL {
            crate::theme::set_theme(theme);
            let ink = badge_ink();
            let luma = 0.299 * ink.r() as f32 + 0.587 * ink.g() as f32 + 0.114 * ink.b() as f32;
            assert!(luma < 80.0, "{theme:?}: {ink:?}");
            assert_eq!(ink.a(), 255);
        }
    }

    #[test]
    fn tools_get_their_cursors() {
        let mut app = App::headless();
        app.tool = Tool::Zoom;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::ZoomIn
        );
        assert_eq!(
            for_tool(&app, true, false, false, None),
            ToolCursor::ZoomOut
        );
        app.tool = Tool::Pan;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Hand { closed: false }
        );
        assert_eq!(
            for_tool(&app, false, true, false, None),
            ToolCursor::Hand { closed: true }
        );
        app.tool = Tool::ColorEyedropper;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Pipette
        );
        // A colour sampled: Apply mode paints with the bucket (#403).
        app.eyedropper_color = Some(tracedraw_core::Color::rgb8(255, 0, 0));
        app.eyedropper_apply = true;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Bucket
        );
        // Ctrl samples again in Apply mode: the pipette (#1302).
        assert_eq!(
            for_tool_with_ctrl(&app, false, true, false, false, None),
            ToolCursor::Pipette
        );
        app.tool = Tool::AttributesEyedropper;
        app.eyedropper_attrs = Some(crate::eyedropper::SampledAttrs {
            fill: tracedraw_core::Fill::None,
            stroke: None,
            text: None,
            width: 1.0,
            height: 1.0,
            rotation: 0.0,
            center: tracedraw_core::geometry::Point::ZERO,
            opacity: 1.0,
            shadow: None,
            effects: Vec::new(),
            fill_source: None,
        });
        app.eyedropper_apply = true;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Bucket
        );
        for (shift, ctrl) in [(true, false), (false, true)] {
            assert_eq!(
                for_tool_with_ctrl(&app, shift, ctrl, false, false, None),
                ToolCursor::Pipette
            );
        }
        app.tool = Tool::ColorEyedropper;
        app.eyedropper_reset();
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Pipette
        );
        app.tool = Tool::AreaFill;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Bucket
        );
        // The Text tool: the cross hair with its sign away from text, the
        // I-beam over text.
        app.tool = Tool::Text;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Badge(Tool::Text)
        );
        app.canvas_rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let page = app.page_rect();
        app.view.fit(page, app.canvas_rect);
        app.start_text(tracedraw_core::geometry::Point::new(50.0, 50.0), None);
        for c in "Hello".chars() {
            app.text_insert(&c.to_string());
        }
        app.finish_text();
        let on = app
            .view
            .to_screen(tracedraw_core::geometry::Point::new(52.0, 51.0));
        assert_eq!(
            for_tool(&app, false, false, false, Some(on)),
            ToolCursor::IBeam
        );
        app.tool = Tool::Rectangle;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Badge(Tool::Rectangle)
        );
        // Curve tools: a join sign over an end node of the selected open
        // curve, a close sign over the first node of the curve being drawn.
        app.tool = Tool::Bezier;
        app.select(Vec::new());
        app.add_drawn_line({
            let mut l = tracedraw_core::geometry::BezPath::new();
            l.move_to((10.0, 10.0));
            l.line_to((60.0, 10.0));
            l
        });
        let end = app
            .view
            .to_screen(tracedraw_core::geometry::Point::new(60.0, 10.0));
        assert_eq!(
            for_tool(&app, false, false, false, Some(end)),
            ToolCursor::CurveBadge(Tool::Bezier, CurveMark::Join)
        );
        app.curve = Some(crate::app::CurveInProgress {
            nodes: vec![
                (tracedraw_core::geometry::Point::new(100.0, 100.0), None),
                (tracedraw_core::geometry::Point::new(150.0, 100.0), None),
                (tracedraw_core::geometry::Point::new(150.0, 150.0), None),
            ],
            smooth: true,
            dragging_handle: false,
            bspline: false,
            ..Default::default()
        });
        let first = app
            .view
            .to_screen(tracedraw_core::geometry::Point::new(100.0, 100.0));
        assert_eq!(
            for_tool(&app, false, false, false, Some(first)),
            ToolCursor::CurveBadge(Tool::Bezier, CurveMark::Close)
        );
        // The Pen: the nib, with the same signs (#362).
        app.tool = Tool::Pen;
        assert_eq!(
            for_tool(&app, false, false, false, Some(first)),
            ToolCursor::PenNib(Some(CurveMark::Close))
        );
        app.curve = None;
        let away = app
            .view
            .to_screen(tracedraw_core::geometry::Point::new(300.0, 300.0));
        assert_eq!(
            for_tool(&app, false, false, false, Some(away)),
            ToolCursor::PenNib(None)
        );
        // Its tip is the hot spot: the outline starts there and runs down
        // and to the right of it, as an arrow pointer's body does, so the
        // tip is the upper left point (#362).
        let tip = egui::pos2(50.0, 50.0);
        let o = nib_outline(tip);
        assert_eq!(o[0], tip);
        assert!(
            o.iter().all(|q| q.x >= tip.x - 4.0 && q.y >= tip.y - 4.0),
            "{o:?}"
        );
        assert!(o.iter().any(|q| q.x > tip.x + 8.0 && q.y > tip.y + 8.0));
        // The sign sits clear of the nib: no outline point is near it.
        let sign = tip + PEN_MARK_AT;
        assert!(o.iter().all(|q| q.distance(sign) > 9.0), "{o:?}");
        // Picking the frame for Place Inside Frame: the target arrow.
        app.pending_clip_frame = true;
        assert_eq!(
            for_tool(&app, false, false, false, Some(away)),
            ToolCursor::TargetArrow
        );
        app.pending_clip_frame = false;
        let o = target_arrow_outline(tip);
        assert_eq!(o[0], tip);
        assert!(o.iter().all(|q| q.x <= tip.x));
        // The Eraser and the brushes show their nib at its size on screen.
        app.tool = Tool::Eraser;
        app.eraser_width = 10.0;
        app.view.zoom = 2.0;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Nib {
                radius: 10.0,
                square: app.eraser_square
            }
        );
        app.tool = Tool::Smudge;
        app.brush_radius = f64::NAN;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::Nib {
                radius: 2.0,
                square: false
            }
        );
        app.tool = Tool::Pick;
        assert_eq!(
            for_tool(&app, false, false, false, None),
            ToolCursor::System(egui::CursorIcon::Default)
        );
        // Middle-button panning shows the closed hand with any tool.
        assert_eq!(
            for_tool(&app, false, false, true, None),
            ToolCursor::Hand { closed: true }
        );
    }
}
