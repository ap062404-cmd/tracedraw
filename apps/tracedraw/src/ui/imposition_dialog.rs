//! Object-based imposition dialog for print production.
use crate::app::App;
use crate::object_imposition::{self, DesignArtwork, DesignSpec, SheetSettings};
use egui::Context;
use tracedraw_core::Shape;

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub shapes: Vec<Shape>,
    pub design_width_mm: f64,
    pub design_height_mm: f64,
    pub copies: u32,
    pub allow_rotation: bool,
    pub sheet_width_mm: f64,
    pub sheet_height_mm: f64,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    pub bleed_mm: f64,
}
impl State {
    pub fn from_selection(shapes: Vec<Shape>) -> Option<Self> {
        let bounds = shapes.iter().map(Shape::bounds).reduce(|a, b| a.union(b))?;
        let width = bounds.x1 - bounds.x0;
        let height = bounds.y1 - bounds.y0;
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 { return None; }
        Some(Self { shapes, design_width_mm: width, design_height_mm: height, copies: 10,
            allow_rotation: true, sheet_width_mm: 297.0, sheet_height_mm: 420.0,
            margin_mm: 5.0, gutter_mm: 2.0, bleed_mm: 0.0 })
    }
}
pub fn dialog(app: &mut App, ctx: &Context, state: &mut State, close: &mut bool) {
    egui::Window::new("Object Imposition").collapsible(false).resizable(true).default_width(380.0).show(ctx, |ui| {
        ui.label("Selected artwork is treated as one production unit.");
        ui.separator();
        ui.label("Artwork size (mm)");
        ui.horizontal(|ui| {
            ui.label("W"); ui.add(crate::ui::field::NumField::new(&mut state.design_width_mm).range(0.1..=5000.0));
            ui.label("H"); ui.add(crate::ui::field::NumField::new(&mut state.design_height_mm).range(0.1..=5000.0));
        });
        ui.horizontal(|ui| {
            ui.label("Copies"); ui.add(crate::ui::field::NumField::new(&mut state.copies).range(1..=10000));
            ui.checkbox(&mut state.allow_rotation, "Allow 90° rotation");
        });
        ui.separator();
        ui.label("Sheet size (mm)");
        ui.horizontal(|ui| {
            ui.label("W"); ui.add(crate::ui::field::NumField::new(&mut state.sheet_width_mm).range(1.0..=10000.0));
            ui.label("H"); ui.add(crate::ui::field::NumField::new(&mut state.sheet_height_mm).range(1.0..=10000.0));
        });
        ui.horizontal(|ui| {
            ui.label("Margin"); ui.add(crate::ui::field::NumField::new(&mut state.margin_mm).range(0.0..=1000.0));
            ui.label("Gutter"); ui.add(crate::ui::field::NumField::new(&mut state.gutter_mm).range(0.0..=1000.0));
            ui.label("Bleed"); ui.add(crate::ui::field::NumField::new(&mut state.bleed_mm).range(0.0..=1000.0));
        });
        let settings = SheetSettings { width_mm: state.sheet_width_mm, height_mm: state.sheet_height_mm,
            margin_left_mm: state.margin_mm, margin_right_mm: state.margin_mm,
            margin_top_mm: state.margin_mm, margin_bottom_mm: state.margin_mm,
            gutter_x_mm: state.gutter_mm, gutter_y_mm: state.gutter_mm, bleed_mm: state.bleed_mm };
        let spec = DesignSpec { id: 1, width_mm: state.design_width_mm, height_mm: state.design_height_mm,
            copies: state.copies, allow_rotation: state.allow_rotation };
        match object_imposition::plan(&[spec], settings) {
            Ok(plan) => {
                ui.separator();
                ui.label(format!("{} copies · {} sheet(s) · {:.1}% utilization", plan.total_copies, plan.sheet_count, plan.utilization_percent));
                if ui.button("Create production document").clicked() {
                    let artwork = DesignArtwork { id: 1, shapes: state.shapes.clone() };
                    match object_imposition::build_document("Imposition", settings, &plan, &[artwork]) {
                        Ok(doc) => { app.open_document(doc, None); *close = true; }
                        Err(error) => app.status = format!("Imposition: {error:?}"),
                    }
                }
            }
            Err(error) => { ui.colored_label(egui::Color32::from_rgb(190, 70, 55), format!("Cannot fit artwork: {error:?}")); }
        }
        if ui.button("Close").clicked() { *close = true; }
    });
}
