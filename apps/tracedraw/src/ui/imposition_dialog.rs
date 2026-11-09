//! Object-based imposition dialog for print production.
use crate::app::App;
use crate::object_imposition::{self, DesignArtwork, DesignSpec, SheetSettings};
use egui::Context;
use tracedraw_core::Shape;

/// One captured artwork unit and its own production quantity.
#[derive(Debug, Clone, PartialEq)]
pub struct DesignEntry {
    pub id: u64,
    pub shapes: Vec<Shape>,
    pub width_mm: f64,
    pub height_mm: f64,
    pub copies: u32,
    pub allow_rotation: bool,
}

impl DesignEntry {
    fn from_shapes(id: u64, shapes: Vec<Shape>, copies: u32) -> Option<Self> {
        let bounds = shapes.iter().map(Shape::bounds).reduce(|a, b| a.union(b))?;
        let width_mm = bounds.x1 - bounds.x0;
        let height_mm = bounds.y1 - bounds.y0;
        if !width_mm.is_finite() || !height_mm.is_finite() || width_mm <= 0.0 || height_mm <= 0.0 {
            return None;
        }
        Some(Self { id, shapes, width_mm, height_mm, copies, allow_rotation: true })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub designs: Vec<DesignEntry>,
    pub next_design_id: u64,
    pub sheet_width_mm: f64,
    pub sheet_height_mm: f64,
    pub margin_mm: f64,
    pub gutter_mm: f64,
    pub bleed_mm: f64,
}

impl State {
    pub fn from_selection(shapes: Vec<Shape>) -> Option<Self> {
        let first = DesignEntry::from_shapes(1, shapes, 10)?;
        Some(Self {
            designs: vec![first],
            next_design_id: 2,
            sheet_width_mm: 297.0,
            sheet_height_mm: 420.0,
            margin_mm: 5.0,
            gutter_mm: 2.0,
            bleed_mm: 0.0,
        })
    }

    fn add_selection(&mut self, shapes: Vec<Shape>) -> bool {
        let id = self.next_design_id;
        let Some(design) = DesignEntry::from_shapes(id, shapes, 1) else { return false; };
        self.designs.push(design);
        self.next_design_id = self.next_design_id.saturating_add(1);
        true
    }
}

pub fn dialog(app: &mut App, ctx: &Context, state: &mut State, close: &mut bool) {
    let mut add_selected = false;
    let mut remove_design = None;
    egui::Window::new("Object Imposition")
        .collapsible(false)
        .resizable(true)
        .default_width(440.0)
        .show(ctx, |ui| {
            ui.label("Each selected object or group is one production design.");
            ui.label("Select another object on the canvas, then add it below.");
            ui.separator();

            ui.heading("Designs");
            for (index, design) in state.designs.iter_mut().enumerate() {
                ui.push_id(design.id, |ui| {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("Design {}", index + 1));
                            ui.label(format!("{:.1} × {:.1} mm", design.width_mm, design.height_mm));
                            if ui.small_button("Remove").clicked() {
                                remove_design = Some(design.id);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Copies");
                            ui.add(crate::ui::field::NumField::new(&mut design.copies).range(1..=10000));
                            ui.checkbox(&mut design.allow_rotation, "Allow 90° rotation");
                        });
                    });
                });
            }
            if let Some(id) = remove_design {
                if state.designs.len() > 1 {
                    state.designs.retain(|design| design.id != id);
                }
            }
            if ui.button("+ Add selected design").clicked() {
                add_selected = true;
            }

            ui.separator();
            ui.heading("Sheet settings");
            ui.horizontal(|ui| {
                ui.label("Width");
                ui.add(crate::ui::field::NumField::new(&mut state.sheet_width_mm).range(1.0..=10000.0));
                ui.label("Height");
                ui.add(crate::ui::field::NumField::new(&mut state.sheet_height_mm).range(1.0..=10000.0));
            });
            ui.horizontal(|ui| {
                ui.label("Margin");
                ui.add(crate::ui::field::NumField::new(&mut state.margin_mm).range(0.0..=1000.0));
                ui.label("Gutter");
                ui.add(crate::ui::field::NumField::new(&mut state.gutter_mm).range(0.0..=1000.0));
                ui.label("Bleed");
                ui.add(crate::ui::field::NumField::new(&mut state.bleed_mm).range(0.0..=1000.0));
            });

            let settings = SheetSettings {
                width_mm: state.sheet_width_mm,
                height_mm: state.sheet_height_mm,
                margin_left_mm: state.margin_mm,
                margin_right_mm: state.margin_mm,
                margin_top_mm: state.margin_mm,
                margin_bottom_mm: state.margin_mm,
                gutter_x_mm: state.gutter_mm,
                gutter_y_mm: state.gutter_mm,
                bleed_mm: state.bleed_mm,
            };
            let specs: Vec<DesignSpec> = state.designs.iter().map(|design| DesignSpec {
                id: design.id,
                width_mm: design.width_mm,
                height_mm: design.height_mm,
                copies: design.copies,
                allow_rotation: design.allow_rotation,
            }).collect();
            match object_imposition::plan(&specs, settings) {
                Ok(plan) => {
                    ui.separator();
                    ui.label(format!(
                        "{} copies across {} design(s) · {} sheet(s) · {:.1}% artwork utilization",
                        plan.total_copies, state.designs.len(), plan.sheet_count, plan.utilization_percent
                    ));
                    if ui.button("Create production document").clicked() {
                        let artwork: Vec<DesignArtwork> = state.designs.iter().map(|design| DesignArtwork {
                            id: design.id,
                            shapes: design.shapes.clone(),
                        }).collect();
                        match object_imposition::build_document("Imposition", settings, &plan, &artwork) {
                            Ok(doc) => {
                                app.open_document(doc, None);
                                *close = true;
                            }
                            Err(error) => app.status = format!("Imposition: {error:?}"),
                        }
                    }
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::from_rgb(190, 70, 55), format!("Cannot fit artwork: {error:?}"));
                }
            }
            if ui.button("Close").clicked() {
                *close = true;
            }
        });

    if add_selected {
        let selected = app.selected_shapes();
        if !state.add_selection(selected) {
            app.status = "Select a valid object or group before adding a design.".into();
        }
    }
}
