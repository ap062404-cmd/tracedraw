//! The Shaping docker: the seven operations as picture buttons at the
//! top; the one chosen shows its name, a large picture and what it does,
//! what it leaves of the originals (Weld, Trim and Intersect: the source
//! objects and the target, each operation keeping its own choice), and
//! the button that applies it. The last object selected is the target;
//! the others act on it.

use crate::app::App;
use crate::i18n::tr;
use crate::ops::Shaping;
use crate::theme::Tokens;
use crate::ui::propbar::{draw_pic, pic_button, Pic};
use crate::ui::tips::RichTip;
use egui::{Rect, Sense, Ui, Vec2};

/// The picture, name and description of an operation.
fn info(op: Shaping) -> (Pic, &'static str, &'static str) {
    match op {
        Shaping::Weld => (Pic::Weld, "menu.object.weld", "docker.weld_hint"),
        Shaping::Trim => (Pic::Trim, "menu.object.trim", "docker.trim_hint"),
        Shaping::Intersect => (
            Pic::Intersect,
            "menu.object.intersect",
            "docker.intersect_hint",
        ),
        Shaping::Simplify => (
            Pic::Simplify,
            "menu.object.simplify",
            "docker.simplify_hint",
        ),
        Shaping::FrontMinusBack => (
            Pic::FrontMinusBack,
            "menu.object.front_minus_back",
            "docker.front_minus_back_hint",
        ),
        Shaping::BackMinusFront => (
            Pic::BackMinusFront,
            "menu.object.back_minus_front",
            "docker.back_minus_front_hint",
        ),
        Shaping::Boundary => (
            Pic::Boundary,
            "menu.object.boundary",
            "docker.boundary_hint",
        ),
    }
}

/// Where Weld, Trim and Intersect keep their choice in
/// `App::shaping_leave`.
fn leave_slot(op: Shaping) -> Option<usize> {
    match op {
        Shaping::Weld => Some(0),
        Shaping::Trim => Some(1),
        Shaping::Intersect => Some(2),
        _ => None,
    }
}

/// Apply the docker's operation to the selection.
pub fn apply(app: &mut App) {
    let op = app.shaping_op;
    let (source, target) = leave_slot(op)
        .map(|i| (app.shaping_leave[i][0], app.shaping_leave[i][1]))
        .unwrap_or_else(|| op.default_leave());
    app.begin_shaping(op, source, target);
}

/// Draw the docker.
pub fn shaping(app: &mut App, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for op in Shaping::ALL {
            let (pic, key, _) = info(op);
            if pic_button(ui, pic, &tr(key), true, app.shaping_op == op).clicked() {
                app.shaping_op = op;
            }
        }
    });
    ui.add_space(6.0);
    let op = app.shaping_op;
    let (pic, key, hint) = info(op);
    ui.label(egui::RichText::new(tr(key)).font(crate::theme::bold(13.0)));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
        ui.painter().rect_filled(rect, 3.0, egui::Color32::WHITE);
        ui.painter().rect_stroke(
            rect,
            3.0,
            egui::Stroke::new(1.0, Tokens::border()),
            egui::StrokeKind::Inside,
        );
        draw_pic(
            ui.painter(),
            Rect::from_center_size(rect.center(), Vec2::splat(56.0)),
            pic,
            Tokens::icon(),
        );
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.label(tr(hint));
            ui.label(
                egui::RichText::new(tr("docker.shaping_target"))
                    .color(Tokens::text_dim())
                    .size(11.0),
            );
            if op.picks_target() {
                ui.label(
                    egui::RichText::new(tr("docker.shaping_pick"))
                        .color(Tokens::text_dim())
                        .size(11.0),
                );
            }
        });
    });
    ui.add_space(6.0);
    if let Some(i) = leave_slot(op) {
        ui.checkbox(
            &mut app.shaping_leave[i][0],
            tr("docker.leave_original_source"),
        )
        .rich_tip(crate::ui::tips::keyed("docker.leave_original_source"));
        ui.checkbox(
            &mut app.shaping_leave[i][1],
            tr("docker.leave_original_target"),
        )
        .rich_tip(crate::ui::tips::keyed("docker.leave_original_target"));
        ui.add_space(6.0);
    }
    let enough = op.ready(app.selection.len());
    let width = ui.available_width().min(240.0);
    let clicked = ui
        .add_enabled_ui(enough, |ui| {
            ui.add_sized([width, 26.0], egui::Button::new(tr(key)))
                .rich_tip_with(|| format!("{}\n{}", tr(key), tr("docker.shaping_apply_desc")))
                .clicked()
        })
        .inner;
    if clicked {
        apply(app);
    }
    if !enough {
        ui.label(
            egui::RichText::new(if op.needs() > 1 && !op.picks_target() {
                tr("docker.shaping_needs_two")
            } else {
                tr("docker.shaping_needs_one")
            })
            .color(Tokens::text_dim())
            .size(11.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::{Point, ShapeId, ShapeKind};

    fn rect(app: &mut App, x0: f64, y0: f64, x1: f64, y1: f64) -> ShapeId {
        app.new_shape(ShapeKind::Rect {
            rect: tracedraw_core::Rect::new(x0, y0, x1, y1),
            radius: 0.0,
            corners: None,
        })
        .expect("shape")
    }

    fn count(app: &App) -> usize {
        app.doc().pages[0].layers[0].shapes.len()
    }

    /// Trim keeps the object that cuts, Intersect keeps both, Weld
    /// replaces them; the docker's own choice is what it leaves.
    #[test]
    fn what_the_operations_leave_of_the_originals() {
        let mut app = App::headless();
        let cutter = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let target = rect(&mut app, 10.0, 10.0, 30.0, 30.0);
        app.select(vec![cutter, target]);
        app.shaping(Shaping::Trim);
        assert!(app.doc().find_shape(cutter).is_some());
        assert!(app.doc().find_shape(target).is_none());
        assert_eq!(count(&app), 2);
        app.undo();
        app.select(vec![cutter, target]);
        app.shaping(Shaping::Intersect);
        assert_eq!(count(&app), 3);
        // The overlap: 10 x 10.
        let result = app.selection[0];
        let b = app.doc().shape_on_page(result).expect("result").bounds();
        assert!((b.width() - 10.0).abs() < 1e-6 && (b.height() - 10.0).abs() < 1e-6);
        app.undo();
        app.select(vec![cutter, target]);
        app.shaping(Shaping::Weld);
        assert_eq!(count(&app), 1);
        app.undo();
        // The docker: Weld leaving the target.
        app.select(vec![cutter, target]);
        app.shaping_op = Shaping::Weld;
        app.shaping_leave[0] = [false, true];
        apply(&mut app);
        assert!(app.doc().find_shape(cutter).is_none());
        assert!(app.doc().find_shape(target).is_some());
        assert_eq!(count(&app), 2);
        // Nothing found inside the welded shape is lost: it spans both.
        let welded = app.selection[0];
        let b = app.doc().shape_on_page(welded).expect("weld").bounds();
        assert!(b.contains(Point::new(29.0, 29.0)) && b.contains(Point::new(1.0, 1.0)));
    }

    #[test]
    fn the_docker_draws_every_operation() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        for (i, op) in Shaping::ALL.into_iter().enumerate() {
            if i == 3 {
                let a = rect(&mut app, 0.0, 0.0, 10.0, 10.0);
                app.select(vec![a]);
            }
            app.shaping_op = op;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 600.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| shaping(&mut app, ui));
            out.textures_delta.clear();
        }
        assert_eq!(Shaping::from_id("trim"), Shaping::Trim);
        assert_eq!(Shaping::from_id("no.such.key"), Shaping::Weld);
    }

    /// A group of `a` (bottom) and `b` (top), selected alone.
    fn grouped(app: &mut App, a: ShapeId, b: ShapeId) -> ShapeId {
        app.select(vec![a, b]);
        app.group_selection();
        let g = app.selection[0];
        app.select(vec![g]);
        g
    }

    /// #2306: Weld, Trim and Intersect on one selected group act on the
    /// group's objects, as in CorelDRAW (the bottom one the target), in
    /// one undo step, instead of waiting for a target.
    #[test]
    fn the_operations_act_on_a_selected_group() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 10.0, 10.0, 30.0, 30.0);
        let g = grouped(&mut app, a, b);
        for op in [Shaping::Weld, Shaping::Trim, Shaping::Intersect] {
            assert!(app.shaping_ready(op), "{op:?} offered for a group");
        }

        app.shaping(Shaping::Weld);
        assert!(app.pending_shaping.is_none(), "no target to pick");
        assert_eq!(count(&app), 1);
        let welded = app
            .doc()
            .shape_on_page(app.selection[0])
            .expect("weld")
            .bounds();
        assert!((welded.width() - 30.0).abs() < 1e-6, "{welded:?}");
        app.undo();
        assert_eq!(count(&app), 1);
        assert!(
            matches!(
                app.doc().find_shape(g).map(|s| &s.kind),
                Some(ShapeKind::Group { .. })
            ),
            "one undo brings the group back"
        );

        // Trim: the bottom object is cut by the top one, which stays.
        app.select(vec![g]);
        app.shaping(Shaping::Trim);
        assert!(app.pending_shaping.is_none());
        assert!(app.doc().find_shape(g).is_none());
        assert!(app.doc().find_shape(a).is_none(), "the target was cut");
        assert!(app.doc().find_shape(b).is_some(), "the cutter stays");
        assert_eq!(count(&app), 2);
        app.undo();
        assert!(app.doc().find_shape(g).is_some());

        // Intersect: the overlap, 10 x 10, beside both objects.
        app.select(vec![g]);
        app.shaping(Shaping::Intersect);
        assert_eq!(count(&app), 3);
        let r = app
            .doc()
            .shape_on_page(app.selection[0])
            .expect("result")
            .bounds();
        assert!((r.width() - 10.0).abs() < 1e-6 && (r.height() - 10.0).abs() < 1e-6);
        app.undo();
        assert!(app.doc().find_shape(g).is_some());
        assert_eq!(count(&app), 1);
    }

    /// Objects of a group that do not overlap: Intersect makes nothing
    /// and the group stays as it was, selected, with no undo step.
    #[test]
    fn a_group_with_nothing_to_intersect_stays() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 10.0, 10.0);
        let b = rect(&mut app, 50.0, 50.0, 60.0, 60.0);
        let g = grouped(&mut app, a, b);
        let label = app.engine.undo_label();
        app.shaping(Shaping::Intersect);
        assert_eq!(app.selection, vec![g]);
        assert!(matches!(
            app.doc().find_shape(g).map(|s| &s.kind),
            Some(ShapeKind::Group { .. })
        ));
        assert_eq!(count(&app), 1);
        assert_eq!(app.engine.undo_label(), label);
    }

    /// The Shaping docker still picks a target for one selected group:
    /// the group acts on the object clicked, as before.
    #[test]
    fn the_docker_still_picks_a_target_for_a_group() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 10.0, 10.0, 30.0, 30.0);
        let _ = rect(&mut app, 5.0, 5.0, 25.0, 25.0);
        grouped(&mut app, a, b);
        app.begin_shaping(Shaping::Weld, false, false);
        assert!(app.pending_shaping.is_some());
    }
}
