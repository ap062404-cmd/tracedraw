//! Tools > Recolor: every colour of the selected objects turned round
//! the hue circle, made more or less saturated, lighter or darker, or
//! tinted with one colour, each colour keeping its lightness next to the
//! others so the drawing's shading stays (`Command::Recolor`, see
//! `tracedraw_core::recolor`). The change shows on the drawing while the
//! dialog is open; OK keeps it as one undo step, Cancel takes it back.
//! See docs/behavior/recolor.md.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::ui::dialogs::window;
use egui::{Context, RichText};
use tracedraw_core::recolor::{recolor_document, Recolor};
use tracedraw_core::{Color, Command, ShapeId};

/// The dialog's values.
#[derive(Debug, Clone, PartialEq)]
pub struct RecolorState {
    pub recolor: Recolor,
    /// Tint on, with the colour in `tint_color`.
    pub tint_on: bool,
    pub tint_color: Color,
    /// The objects it works on (the selection when it opened).
    pub shapes: Vec<ShapeId>,
    /// What the drawing shows now.
    shown: Option<Recolor>,
}

impl RecolorState {
    pub fn new(shapes: Vec<ShapeId>) -> Self {
        RecolorState {
            recolor: Recolor::default(),
            tint_on: false,
            tint_color: Color::rgb8(30, 110, 220),
            shapes,
            shown: None,
        }
    }

    /// What the values ask for.
    pub fn asked(&self) -> Recolor {
        Recolor {
            tint: self.tint_on.then_some(self.tint_color),
            ..self.recolor
        }
    }
}

impl App {
    /// Tools > Recolor: its dialog, for the selected objects.
    pub fn recolor_dialog(&mut self) {
        if self.selection.is_empty() {
            self.status = tr("recolor.nothing_selected");
            return;
        }
        self.dialog =
            crate::ui::dialogs::Dialog::Recolor(RecolorState::new(self.selection.clone()));
    }

    /// Recolour `shapes` by `r` as one undo step; the status line says how
    /// many colours changed. False when none did.
    pub fn recolor_shapes(&mut self, shapes: Vec<ShapeId>, r: Recolor) -> bool {
        if shapes.is_empty() || r.is_identity() {
            return false;
        }
        // How many colours change, on a copy (the drawing's data is shared).
        let mut copy = self.doc().clone();
        let n = recolor_document(&mut copy, &shapes, &r);
        if n == 0 {
            self.status = tr("recolor.none");
            return false;
        }
        self.run(Command::Recolor { shapes, recolor: r });
        self.status = trf("recolor.done", &[("n", &n.to_string())]);
        true
    }

    /// Show the dialog's values on the drawing: taken back and made anew
    /// only when a value changes.
    pub fn preview_recolor(&mut self, st: &mut RecolorState) {
        let now = st.asked();
        if st.shown == Some(now) && self.recolor_preview != now.is_identity() {
            return;
        }
        self.end_recolor_preview();
        if !now.is_identity() {
            self.engine.begin_preview();
            self.recolor_preview = true;
            let _ = self.engine.run(&Command::Recolor {
                shapes: st.shapes.clone(),
                recolor: now,
            });
        }
        st.shown = Some(now);
    }

    /// Take back what the Recolor dialog shows on the drawing.
    pub fn end_recolor_preview(&mut self) {
        if std::mem::take(&mut self.recolor_preview) {
            self.engine.withdraw_preview();
        }
    }
}

/// A slider row: label, slider, value.
fn slider(ui: &mut egui::Ui, key: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) {
    ui.label(tr(key));
    ui.add(
        egui::Slider::new(v, range)
            .step_by(1.0)
            .suffix(if key == "recolor.hue" { "°" } else { "%" }),
    );
    ui.end_row();
}

pub fn dialog(app: &mut App, ctx: &Context, st: &mut RecolorState, close: &mut bool) {
    let mut help = false;
    let mut ok = false;
    let mut reset = false;
    window(ctx, tr("recolor.title")).show(ctx, |ui| {
        ui.set_width(400.0);
        egui::Grid::new("recolor_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut st.tint_on, tr("recolor.tint"));
                });
                ui.add_enabled_ui(st.tint_on, |ui| {
                    crate::ui::color_picker::color_button(ui, &mut st.tint_color);
                });
                ui.end_row();
                slider(ui, "recolor.hue", &mut st.recolor.hue, -180.0..=180.0);
                slider(
                    ui,
                    "recolor.saturation",
                    &mut st.recolor.saturation,
                    -100.0..=100.0,
                );
                slider(
                    ui,
                    "recolor.lightness",
                    &mut st.recolor.lightness,
                    -100.0..=100.0,
                );
            });
        ui.add_space(4.0);
        ui.checkbox(&mut st.recolor.spots, tr("recolor.spots"));
        ui.add_space(4.0);
        ui.label(RichText::new(tr("recolor.note")).weak());
        ui.add_space(4.0);
        reset = ui.button(tr("recolor.reset")).clicked();
        let (o, h) = crate::ui::dialogs::help_ok_cancel(ui, close);
        ok = o;
        help = h;
    });
    if reset {
        st.recolor = Recolor {
            spots: st.recolor.spots,
            ..Recolor::default()
        };
        st.tint_on = false;
    }
    if help {
        app.open_dialog_help();
    }
    if ok {
        app.end_recolor_preview();
        let shapes = st.shapes.clone();
        app.recolor_shapes(shapes, st.asked());
    } else if *close {
        app.end_recolor_preview();
    } else {
        app.preview_recolor(st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::dialogs::Dialog;
    use tracedraw_core::{Fill, ShapeKind};

    fn rect(app: &mut App, c: Color) -> ShapeId {
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.run(Command::SetFill {
            shapes: vec![id],
            fill: Fill::Solid(c),
        });
        id
    }

    fn fill_of(app: &App, id: ShapeId) -> Option<Fill> {
        app.doc().find_shape(id).map(|s| s.fill.clone())
    }

    /// The dialog shows the change while open, Cancel takes it back with
    /// no step left, OK keeps it as one step that Undo takes back.
    #[test]
    fn preview_cancel_and_ok_as_one_step() {
        let mut app = App::headless();
        app.show_welcome = false;
        let red = Color::rgb8(200, 40, 40);
        let a = rect(&mut app, red);
        let b = rect(&mut app, red);
        app.select(vec![a]);
        app.recolor_dialog();
        let Dialog::Recolor(st) = &app.dialog else {
            panic!("open");
        };
        let mut st = st.clone();
        let steps = app.engine.history_labels().0.len();
        st.recolor.hue = 120.0;
        app.preview_recolor(&mut st);
        assert_ne!(fill_of(&app, a), Some(Fill::Solid(red)), "shown");
        assert_eq!(
            fill_of(&app, b),
            Some(Fill::Solid(red)),
            "only the selection"
        );
        app.end_recolor_preview();
        assert_eq!(fill_of(&app, a), Some(Fill::Solid(red)), "cancel");
        assert_eq!(app.engine.history_labels().0.len(), steps);
        st.shown = None;
        app.preview_recolor(&mut st);
        app.end_recolor_preview();
        assert!(app.recolor_shapes(st.shapes.clone(), st.asked()));
        let turned = fill_of(&app, a);
        assert_ne!(turned, Some(Fill::Solid(red)));
        assert_eq!(app.engine.history_labels().0.len(), steps + 1);
        app.undo();
        assert_eq!(fill_of(&app, a), Some(Fill::Solid(red)));
    }

    #[test]
    fn nothing_selected_opens_nothing() {
        let mut app = App::headless();
        app.show_welcome = false;
        app.recolor_dialog();
        assert!(!matches!(app.dialog, Dialog::Recolor(_)));
    }
}
