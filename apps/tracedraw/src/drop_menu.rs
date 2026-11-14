//! Dragging objects with the right mouse button (Pick tools): a blue
//! outline of the selection follows the pointer while the objects stay
//! put; on release a menu asks what to do there: Move Here, Copy Here,
//! Copy Fill Here, Copy Outline Here, Copy All Properties (onto the object
//! under the pointer), Place Inside (that object becomes the selection's
//! clip frame), or Cancel. Ctrl keeps the drag horizontal or vertical;
//! Esc cancels it. Let go over another object with Shift held, that
//! object takes the outline of the selection without the menu, with Alt
//! the fill and with Ctrl fill and outline, as in CorelDRAW. A move with
//! the left button let go over another object with Alt held copies the
//! fill there without a menu, with Alt+Shift the outline
//! (`App::copy_style_onto`, discussion #2066).

use crate::app::App;
use egui::{Painter, Response, Ui};
use tracedraw_core::{
    geometry::{Affine, Point, Vec2},
    Command, ShapeId,
};

/// A right-button drag in progress (page space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RightDrag {
    pub start: Point,
    pub current: Point,
}

impl RightDrag {
    /// How far the selection would go; `constrain` keeps the larger of
    /// the two directions only.
    pub fn offset(&self, constrain: bool) -> Vec2 {
        let d = self.current - self.start;
        if constrain {
            if d.x.abs() >= d.y.abs() {
                Vec2::new(d.x, 0.0)
            } else {
                Vec2::new(0.0, d.y)
            }
        } else {
            d
        }
    }
}

/// The menu shown where a right-button drag ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropMenu {
    /// Where the menu opens (screen).
    pub screen: egui::Pos2,
    pub offset: Vec2,
    /// The object under the pointer that is not being dragged.
    pub target: Option<ShapeId>,
}

/// What the drop menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    Move,
    Copy,
    CopyFill,
    CopyOutline,
    CopyProperties,
    PlaceInside,
    Cancel,
}

impl DropAction {
    pub const ALL: [DropAction; 7] = [
        DropAction::Move,
        DropAction::Copy,
        DropAction::CopyFill,
        DropAction::CopyOutline,
        DropAction::CopyProperties,
        DropAction::PlaceInside,
        DropAction::Cancel,
    ];

    pub fn key(self) -> &'static str {
        match self {
            DropAction::Move => "drop.move_here",
            DropAction::Copy => "drop.copy_here",
            DropAction::CopyFill => "drop.copy_fill_here",
            DropAction::CopyOutline => "drop.copy_outline_here",
            DropAction::CopyProperties => "drop.copy_all_properties",
            DropAction::PlaceInside => "drop.place_inside",
            DropAction::Cancel => "drop.cancel",
        }
    }

    /// What a right-button drag let go over another object does without
    /// the menu, by the keys held (discussion #2066): Shift copies the
    /// outline, Alt the fill, Ctrl (Cmd on a Mac) fill and outline.
    pub fn for_right_drop(mods: egui::Modifiers) -> Option<DropAction> {
        if mods.alt {
            Some(DropAction::CopyFill)
        } else if mods.shift {
            Some(DropAction::CopyOutline)
        } else if mods.command {
            Some(DropAction::CopyProperties)
        } else {
            None
        }
    }

    /// Whether the action needs an object under the pointer.
    pub fn needs_target(self) -> bool {
        matches!(
            self,
            DropAction::CopyFill
                | DropAction::CopyOutline
                | DropAction::CopyProperties
                | DropAction::PlaceInside
        )
    }
}

impl App {
    /// Right-button drags with the Pick tools; true when the input was
    /// used here.
    pub fn right_drag_input(&mut self, response: &Response, p: Point) -> bool {
        if response.drag_started_by(egui::PointerButton::Secondary) {
            // A right drag from a selected object, also one covered by
            // another object there, moves the selection as it is (#1684).
            if !self.selection_under(p) {
                let Some(id) = self.hit_test(p) else {
                    return false;
                };
                if !self.selection.contains(&id) {
                    self.select(vec![id]);
                }
            }
            self.right_drag = Some(RightDrag {
                start: p,
                current: p,
            });
            self.drop_menu = None;
            return true;
        }
        let Some(mut drag) = self.right_drag else {
            return false;
        };
        if response.ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.right_drag = None;
            return true;
        }
        if response.dragged_by(egui::PointerButton::Secondary) {
            drag.current = p;
            self.right_drag = Some(drag);
            return true;
        }
        if response.drag_stopped() || !response.ctx.input(|i| i.pointer.secondary_down()) {
            drag.current = p;
            let mods = response.ctx.input(|i| i.modifiers);
            self.right_drag_released(drag, mods);
            return true;
        }
        true
    }

    /// The object under `p` that is not being dragged.
    fn right_drop_target(&self, p: Point) -> Option<ShapeId> {
        let selection = &self.selection;
        self.hit_test_where(p, |id| selection.contains(&id))
    }

    /// What a right-button drag that ended with the keys `mods` held does
    /// there, and the object it does it to (shown while dragging).
    pub fn right_drop_style(
        &self,
        p: Point,
        mods: egui::Modifiers,
    ) -> Option<(DropAction, ShapeId)> {
        let action = DropAction::for_right_drop(mods)?;
        Some((action, self.right_drop_target(p)?))
    }

    /// A right-button drag let go at `drag.current` with the keys `mods`
    /// held: over another object with Shift, Alt or Ctrl that object takes
    /// the outline, the fill or both at once (discussion #2066); otherwise
    /// the drop menu opens there.
    pub fn right_drag_released(&mut self, drag: RightDrag, mods: egui::Modifiers) {
        self.right_drag = None;
        let p = drag.current;
        if let Some((action, target)) = self.right_drop_style(p, mods) {
            self.copy_style_onto(target, action);
            return;
        }
        let offset = drag.offset(mods.command);
        if offset.hypot() * self.view.zoom as f64 >= 2.0 {
            self.drop_menu = Some(DropMenu {
                screen: self.view.to_screen(p),
                offset,
                target: self.right_drop_target(p),
            });
        }
    }

    /// The selection's outline where a right-button drag would put it;
    /// with Shift, Alt or Ctrl held over another object, that object
    /// marked and what the drop will copy onto it instead (#2066).
    pub fn draw_right_drag(&self, painter: &Painter) {
        let Some(drag) = self.right_drag else {
            return;
        };
        let mods = painter.ctx().input(|i| i.modifiers);
        if let Some((action, target)) = self.right_drop_style(drag.current, mods) {
            crate::canvas::draw_drop_style(self, painter, action, target);
            return;
        }
        let t = Affine::translate(drag.offset(mods.command));
        crate::canvas::draw_transform_outline(self, painter, t);
    }

    /// Carry out a drop menu choice.
    pub fn drop_action(&mut self, menu: DropMenu, action: DropAction) {
        if self.selection.is_empty() {
            return;
        }
        match action {
            DropAction::Move => self.transform_selection(Affine::translate(menu.offset)),
            DropAction::Copy => self.duplicate_by(menu.offset),
            DropAction::CopyFill | DropAction::CopyOutline | DropAction::CopyProperties => {
                if let Some(target) = menu.target {
                    self.copy_style_onto(target, action);
                }
            }
            DropAction::PlaceInside => {
                let Some(target) = menu.target else {
                    return;
                };
                let contents: Vec<ShapeId> = self
                    .selection
                    .iter()
                    .copied()
                    .filter(|id| *id != target)
                    .collect();
                if !contents.is_empty() {
                    self.place_inside(contents, target);
                    self.select(vec![target]);
                }
            }
            DropAction::Cancel => {}
        }
    }

    /// Copy Fill Here, Copy Outline Here or Copy All Properties: object
    /// `target` takes the fill, the outline or both and the transparency
    /// of the first selected object, one undo step. The right-button drop
    /// menu and a left-button move let go with Alt (Alt+Shift) held do
    /// this (discussion #2066). Other actions do nothing here.
    pub fn copy_style_onto(&mut self, target: ShapeId, action: DropAction) {
        if !matches!(
            action,
            DropAction::CopyFill | DropAction::CopyOutline | DropAction::CopyProperties
        ) {
            return;
        }
        let Some(first) = self.selected_shapes().first().cloned() else {
            return;
        };
        if first.id == target {
            return;
        }
        let mut cmds = Vec::new();
        if action != DropAction::CopyOutline {
            cmds.push(Command::SetFill {
                shapes: vec![target],
                fill: first.fill.clone(),
            });
        }
        if action != DropAction::CopyFill {
            cmds.push(Command::SetStroke {
                shapes: vec![target],
                stroke: first.stroke.clone(),
            });
        }
        if action == DropAction::CopyProperties {
            cmds.push(Command::SetOpacity {
                shapes: vec![target],
                opacity: first.opacity,
            });
        }
        let label = match action {
            DropAction::CopyFill => "Copy Fill",
            DropAction::CopyOutline => "Copy Outline",
            _ => "Copy Properties",
        };
        if let Err(e) = self.engine.run_batch(label, &cmds) {
            self.status = e.to_string();
        }
    }

    /// Copies of the selection moved by `offset`, selected.
    pub fn duplicate_by(&mut self, offset: Vec2) {
        // The copy goes where the drag ended, also when the dragged
        // objects are the copies of a Duplicate still selected: the step
        // a repeated Duplicate takes put it far from the drop (#2640).
        self.last_duplicate.clear();
        let saved = self.duplicate_offset;
        self.duplicate_offset = offset;
        self.duplicate();
        self.duplicate_offset = saved;
    }
}

/// The drop menu, drawn like the context menu where the drag ended.
pub fn drop_menu_ui(app: &mut App, ui: &mut Ui) {
    let Some(menu) = app.drop_menu else {
        return;
    };
    let id = egui::Id::new("canvas_drop_menu");
    let ctx = ui.ctx().clone();
    let pos_id = id.with("pos");
    if ctx.data(|d| d.get_temp::<egui::Pos2>(pos_id)) != Some(menu.screen) {
        ctx.data_mut(|d| d.insert_temp(pos_id, menu.screen));
        egui::Popup::open_id(&ctx, id);
    }
    let mut chosen: Option<DropAction> = None;
    let shown = egui::Popup::new(
        id,
        ctx.clone(),
        egui::PopupAnchor::Position(menu.screen),
        ui.layer_id(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .style(crate::ui::menus::menu_popup_style)
    .open_memory(None)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .show(|ui| {
        ui.set_min_width(190.0);
        crate::ui::menus::body(ui, |ui| {
            for action in DropAction::ALL {
                if matches!(action, DropAction::CopyFill | DropAction::Cancel) {
                    crate::ui::menus::sep(ui);
                }
                let enabled = !action.needs_target() || menu.target.is_some();
                if crate::ui::menus::item(ui, action.key(), "", enabled) {
                    chosen = Some(action);
                }
            }
        })
    });
    if let Some(action) = chosen {
        app.drop_action(menu, action);
    }
    if chosen.is_some() || shown.is_none() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        egui::Popup::close_id(&ctx, id);
        app.drop_menu = None;
        ctx.data_mut(|d| d.remove::<egui::Pos2>(pos_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::{Color, Fill, Rect, ShapeKind, Stroke};

    fn two_boxes(app: &mut App) -> (ShapeId, ShapeId) {
        let a = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("a");
        let b = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(50.0, 0.0, 70.0, 20.0),
                radius: 0.0,
                corners: None,
            })
            .expect("b");
        app.run(Command::SetFill {
            shapes: vec![a],
            fill: Fill::Solid(Color::rgb8(255, 0, 0)),
        });
        app.run(Command::SetStroke {
            shapes: vec![a],
            stroke: Some(Stroke::new(Color::rgb8(0, 0, 255), 1.5)),
        });
        (a, b)
    }

    fn bounds(app: &App, id: ShapeId) -> tracedraw_core::Rect {
        app.doc()
            .shape(id)
            .map(|(_, s)| s.bounds())
            .unwrap_or_default()
    }

    #[test]
    fn a_ctrl_drag_keeps_to_one_direction() {
        let d = RightDrag {
            start: Point::new(0.0, 0.0),
            current: Point::new(10.0, 3.0),
        };
        assert_eq!(d.offset(false), Vec2::new(10.0, 3.0));
        assert_eq!(d.offset(true), Vec2::new(10.0, 0.0));
    }

    #[test]
    fn move_here_and_copy_here() {
        let mut app = App::headless();
        let (a, _) = two_boxes(&mut app);
        app.select(vec![a]);
        let menu = DropMenu {
            screen: egui::Pos2::ZERO,
            offset: Vec2::new(20.0, 5.0),
            target: None,
        };
        app.drop_action(menu, DropAction::Copy);
        // A copy, selected, where the drop was; the original stays.
        assert_eq!(app.selection.len(), 1);
        let copy = app.selection[0];
        assert_ne!(copy, a);
        assert!((bounds(&app, copy).x0 - 20.0).abs() < 1e-6);
        assert!((bounds(&app, a).x0).abs() < 1e-6);
        app.select(vec![a]);
        app.drop_action(menu, DropAction::Move);
        assert!((bounds(&app, a).x0 - 20.0).abs() < 1e-6);
        // Cancel changes nothing.
        let before = app.doc().clone();
        app.drop_action(menu, DropAction::Cancel);
        assert_eq!(*app.doc(), before);
    }

    #[test]
    fn copy_here_from_copies_still_selected_lands_where_the_drag_ended() {
        // #2640: a right drag copied from a copy still selected (after
        // Duplicate or an earlier Copy Here) landed by the earlier step,
        // not where the drag ended.
        let mut app = App::headless();
        let (a, _) = two_boxes(&mut app);
        app.select(vec![a]);
        let menu = |dx: f64, dy: f64| DropMenu {
            screen: egui::Pos2::ZERO,
            offset: Vec2::new(dx, dy),
            target: None,
        };
        app.drop_action(menu(80.0, 0.0), DropAction::Copy);
        let first = app.selection[0];
        assert!((bounds(&app, first).x0 - 80.0).abs() < 1e-6);
        app.drop_action(menu(5.0, 30.0), DropAction::Copy);
        let second = app.selection[0];
        assert_ne!(second, first);
        let r = bounds(&app, second);
        assert!((r.x0 - 85.0).abs() < 1e-6, "copy at {r:?}");
        assert!((r.y0 - (bounds(&app, first).y0 + 30.0)).abs() < 1e-6);
        // The same after Edit > Duplicate.
        app.select(vec![a]);
        app.duplicate();
        let dup = app.selection[0];
        let at = bounds(&app, dup);
        app.drop_action(menu(0.0, 40.0), DropAction::Copy);
        let r = bounds(&app, app.selection[0]);
        assert!((r.x0 - at.x0).abs() < 1e-6 && (r.y0 - (at.y0 + 40.0)).abs() < 1e-6);
    }

    #[test]
    fn fill_outline_and_properties_go_onto_the_target() {
        let mut app = App::headless();
        let (a, b) = two_boxes(&mut app);
        app.select(vec![a]);
        let menu = DropMenu {
            screen: egui::Pos2::ZERO,
            offset: Vec2::new(55.0, 5.0),
            target: Some(b),
        };
        app.drop_action(menu, DropAction::CopyFill);
        let shape = |app: &App, id| app.doc().shape(id).map(|(_, s)| s.clone()).ok();
        let (sa, sb) = (shape(&app, a).expect("a"), shape(&app, b).expect("b"));
        assert_eq!(sb.fill, sa.fill);
        assert_ne!(sb.stroke, sa.stroke);
        app.drop_action(menu, DropAction::CopyOutline);
        let sb = shape(&app, b).expect("b");
        assert_eq!(sb.stroke, sa.stroke);
        // The source did not move.
        assert!((bounds(&app, a).x0).abs() < 1e-6);
        // Without a target these do nothing.
        let none = DropMenu {
            target: None,
            ..menu
        };
        let before = app.doc().clone();
        app.drop_action(none, DropAction::CopyProperties);
        assert_eq!(*app.doc(), before);
    }

    #[test]
    fn place_inside_makes_the_target_a_clip_frame() {
        let mut app = App::headless();
        let (a, b) = two_boxes(&mut app);
        app.select(vec![a]);
        let menu = DropMenu {
            screen: egui::Pos2::ZERO,
            offset: Vec2::new(55.0, 5.0),
            target: Some(b),
        };
        app.drop_action(menu, DropAction::PlaceInside);
        let frame = app.doc().shape(b).map(|(_, s)| s.clone()).expect("frame");
        assert!(matches!(frame.kind, ShapeKind::ClipFrame { .. }));
        assert_eq!(app.selection, vec![b]);
    }

    /// Discussion #2066: a right-button drag let go over another object
    /// with Shift held copies the outline there without the menu, with
    /// Alt the fill and with Ctrl both, as in CorelDRAW; without keys the
    /// menu opens.
    #[test]
    fn right_drop_with_shift_alt_or_ctrl_copies_without_the_menu() {
        let shape = |app: &App, id| app.doc().shape(id).map(|(_, s)| s.clone()).ok();
        let mods = |shift, alt, command| egui::Modifiers {
            shift,
            alt,
            command,
            ctrl: command,
            ..Default::default()
        };
        for (keys, fill, outline) in [
            (mods(true, false, false), false, true),
            (mods(false, true, false), true, false),
            (mods(false, false, true), true, true),
        ] {
            let mut app = App::headless();
            let (a, b) = two_boxes(&mut app);
            app.select(vec![a]);
            let over_b = bounds(&app, b).center();
            let drag = RightDrag {
                start: bounds(&app, a).center(),
                current: over_b,
            };
            app.right_drag_released(drag, keys);
            assert!(app.drop_menu.is_none(), "no menu with {keys:?}");
            let (sa, sb) = (shape(&app, a).expect("a"), shape(&app, b).expect("b"));
            assert_eq!(sb.fill == sa.fill, fill, "fill with {keys:?}");
            assert_eq!(sb.stroke == sa.stroke, outline, "outline with {keys:?}");
            assert!((bounds(&app, a).x0).abs() < 1e-6, "the source stays");
        }
        // No keys: the menu, nothing changed yet.
        let mut app = App::headless();
        let (a, b) = two_boxes(&mut app);
        app.select(vec![a]);
        let before = app.doc().clone();
        let drag = RightDrag {
            start: bounds(&app, a).center(),
            current: bounds(&app, b).center(),
        };
        app.right_drag_released(drag, egui::Modifiers::default());
        assert_eq!(app.drop_menu.map(|m| m.target), Some(Some(b)));
        assert_eq!(*app.doc(), before);
    }
}
