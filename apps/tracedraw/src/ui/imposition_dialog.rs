//! Object-based imposition dialog for print production.
use crate::app::App;
use crate::object_imposition::{self, CropMarkSettings, DesignArtwork, DesignSpec, SheetSettings};
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
    pub margin_x_mm: f64,
    pub margin_y_mm: f64,
    pub gutter_x_mm: f64,
    pub gutter_y_mm: f64,
    pub bleed_mm: f64,
    pub crop_marks: bool,
    pub crop_mark_length_mm: f64,
    pub crop_mark_offset_mm: f64,
    pub crop_mark_width_mm: f64,
    pub shape_nesting: bool,
    /// Name entered while saving the current custom sheet size.
    pub saving_sheet_preset: Option<String>,
    pub preview_sheet_index: usize,
    cached_shape_nesting: bool,
    cached_specs: Vec<DesignSpec>,
    cached_settings: Option<SheetSettings>,
    cached_plan: Option<Result<object_imposition::ImpositionPlan, object_imposition::PlanError>>,
}

impl State {
    pub fn from_selection(shapes: Vec<Shape>) -> Option<Self> {
        // Each top-level selected shape is a separate design. A Group remains
        // one Shape, so its children stay together as one production unit.
        let designs: Vec<DesignEntry> = shapes.into_iter().enumerate()
            .filter_map(|(index, shape)| DesignEntry::from_shapes(index as u64 + 1, vec![shape], 1))
            .collect();
        if designs.is_empty() { return None; }
        let next_design_id = designs.iter().map(|design| design.id).max().unwrap_or(0).saturating_add(1);
        Some(Self {
            designs,
            next_design_id,
            sheet_width_mm: 320.0,
            sheet_height_mm: 480.0,
            margin_x_mm: 5.0,
            margin_y_mm: 5.0,
            gutter_x_mm: 2.0,
            gutter_y_mm: 2.0,
            bleed_mm: 0.0,
            crop_marks: true,
            crop_mark_length_mm: 3.0,
            crop_mark_offset_mm: 0.5,
            crop_mark_width_mm: 0.2,
            shape_nesting: false,
            saving_sheet_preset: None,
            preview_sheet_index: 0,
            cached_shape_nesting: false,
            cached_specs: Vec::new(),
            cached_settings: None,
            cached_plan: None,
        })
    }

    fn add_selection(&mut self, shapes: Vec<Shape>) -> bool {
        if shapes.is_empty() { return false; }
        let mut added = false;
        for shape in shapes {
            let id = self.next_design_id;
            self.next_design_id = self.next_design_id.saturating_add(1);
            if let Some(design) = DesignEntry::from_shapes(id, vec![shape], 1) {
                self.designs.push(design);
                added = true;
            }
        }
        added
    }
}


/// Draw a compact first-sheet preview using the exact placement coordinates
/// produced by the planner. This is a visual aid, not an editable canvas.
fn draw_preview(ui: &mut egui::Ui, plan: &object_imposition::ImpositionPlan, designs: &[DesignEntry], sheet: SheetSettings, sheet_index: usize) {
    use egui::{Align2, Color32, FontId, Stroke};
    let available = egui::vec2(380.0, 230.0);
    let (outer, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let page_w = sheet.width_mm as f32;
    let page_h = sheet.height_mm as f32;
    if page_w <= 0.0 || page_h <= 0.0 { return; }
    let pad = 12.0;
    let scale = ((outer.rect.width() - pad * 2.0) / page_w)
        .min((outer.rect.height() - pad * 2.0) / page_h);
    let w = page_w * scale;
    let h = page_h * scale;
    let page = egui::Rect::from_center_size(outer.rect.center(), egui::vec2(w, h));
    painter.rect_filled(page, 2.0, Color32::from_rgb(250, 250, 246));
    painter.rect_stroke(page, 2.0, Stroke::new(1.0, Color32::DARK_GRAY), egui::StrokeKind::Inside);
    let colors = [
        Color32::from_rgb(76, 132, 190),
        Color32::from_rgb(74, 155, 116),
        Color32::from_rgb(194, 126, 65),
        Color32::from_rgb(151, 111, 184),
        Color32::from_rgb(190, 92, 112),
        Color32::from_rgb(90, 150, 155),
    ];
    for placement in plan.placements.iter().filter(|p| p.sheet_index == sheet_index) {
        let x = page.left() + placement.x_mm as f32 * scale;
        let y = page.top() + placement.y_mm as f32 * scale;
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(placement.width_mm as f32 * scale, placement.height_mm as f32 * scale),
        );
        let color = colors[(placement.design_id as usize) % colors.len()];
        let is_plain_ellipse = designs.iter()
            .find(|d| d.id == placement.design_id)
            .is_some_and(|d| d.shapes.len() == 1
                && matches!(&d.shapes[0].kind, tracedraw_core::ShapeKind::Ellipse { arc: None, .. }));
        if placement.shape_transform.is_some() && is_plain_ellipse {
            let center = rect.center();
            let radius = rect.size() * 0.5;
            let points: Vec<_> = (0..48).map(|i| {
                let angle = i as f32 * std::f32::consts::TAU / 48.0;
                center + egui::vec2(radius.x * angle.cos(), radius.y * angle.sin())
            }).collect();
            painter.add(egui::Shape::convex_polygon(
                points, color, Stroke::new(0.7, Color32::WHITE),
            ));
        } else {
            painter.rect_filled(rect, 1.0, color);
            painter.rect_stroke(rect, 1.0, Stroke::new(0.7, Color32::WHITE), egui::StrokeKind::Inside);
        }
        if rect.width() > 24.0 && rect.height() > 14.0 {
            painter.text(rect.center(), Align2::CENTER_CENTER,
                format!("D{}", placement.design_id), FontId::proportional(10.0), Color32::WHITE);
        }
    }
    painter.text(
        egui::pos2(outer.rect.left() + 4.0, outer.rect.bottom() - 2.0),
        Align2::LEFT_BOTTOM,
        format!("Sheet {} preview", sheet_index + 1),
        FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );
}

pub fn dialog(app: &mut App, ctx: &Context, state: &mut State, close: &mut bool) {
    let mut add_selected = false;
    let mut remove_design = None;
    let mut create_document = false;
    let mut selected_sheet_preset: Option<(f64, f64)> = None;
    let mut delete_sheet_preset: Option<String> = None;
    egui::Window::new("Object Imposition")
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(true)
        .default_width(440.0)
        .default_height(620.0)
        .max_height(ctx.content_rect().height() * 0.92)
        .show(ctx, |ui| {
            let footer_height = 42.0;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height((ui.available_height() - footer_height).max(120.0))
                .show(ui, |ui| {
                    ui.label("Each selected top-level object is one production design. Groups stay together.");
                    ui.label("Select other objects on the canvas, then add them as separate designs.");
                    ui.checkbox(&mut state.shape_nesting, "Nest by true object outlines (experimental)");
                    ui.small("Uses vector contours for tighter packing; rectangular mode remains the default fallback.");
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
                    if ui.button("+ Add selected design(s)").clicked() {
                        add_selected = true;
                    }

                    ui.separator();
                    ui.heading("Sheet settings");
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Paper preset:");
                        if ui.small_button("A4 · 210 × 297").clicked() {
                            state.sheet_width_mm = 210.0;
                            state.sheet_height_mm = 297.0;
                        }
                        if ui.small_button("A3 · 297 × 420").clicked() {
                            state.sheet_width_mm = 297.0;
                            state.sheet_height_mm = 420.0;
                        }
                        if ui.small_button("A3+ · 320 × 480").clicked() {
                            state.sheet_width_mm = 320.0;
                            state.sheet_height_mm = 480.0;
                        }
                        if ui.small_button("SRA3 · 320 × 450").clicked() {
                            state.sheet_width_mm = 320.0;
                            state.sheet_height_mm = 450.0;
                        }
                        ui.menu_button("Saved sizes", |ui| {
                            if app.settings.page_sizes.is_empty() {
                                ui.label("No saved page sizes yet.");
                            }
                            for preset in app.settings.page_sizes.clone() {
                                ui.horizontal(|ui| {
                                    if ui.selectable_label(false, format!(
                                        "{} · {:.1} × {:.1} mm", preset.name,
                                        preset.width_mm, preset.height_mm
                                    )).clicked() {
                                        selected_sheet_preset = Some((preset.width_mm, preset.height_mm));
                                        ui.close();
                                    }
                                    if ui.small_button("×").on_hover_text("Delete saved size").clicked() {
                                        delete_sheet_preset = Some(preset.name.clone());
                                        ui.close();
                                    }
                                });
                            }
                        });
                        if ui.small_button("Save current size…").clicked() {
                            state.saving_sheet_preset = Some(format!(
                                "{:.0} × {:.0} mm", state.sheet_width_mm, state.sheet_height_mm
                            ));
                        }
                    });
                    if let Some((width, height)) = selected_sheet_preset {
                        state.sheet_width_mm = width;
                        state.sheet_height_mm = height;
                    }
                    if let Some(name) = delete_sheet_preset {
                        app.settings.page_sizes.retain(|preset| preset.name != name);
                        app.settings.save();
                    }
                    if let Some(name) = &mut state.saving_sheet_preset {
                        ui.horizontal(|ui| {
                            ui.label("Template name:");
                            ui.text_edit_singleline(name);
                            if ui.button("Save").clicked() && !name.trim().is_empty() {
                                let name = name.trim().to_string();
                                app.settings.page_sizes.retain(|preset| preset.name != name);
                                app.settings.page_sizes.push(crate::settings::PageSizePreset {
                                    name,
                                    width_mm: state.sheet_width_mm,
                                    height_mm: state.sheet_height_mm,
                                });
                                app.settings.save();
                                state.saving_sheet_preset = None;
                            }
                            if ui.button("Cancel").clicked() {
                                state.saving_sheet_preset = None;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Width");
                        ui.add(crate::ui::field::NumField::new(&mut state.sheet_width_mm).range(1.0..=10000.0));
                        ui.label("Height");
                        ui.add(crate::ui::field::NumField::new(&mut state.sheet_height_mm).range(1.0..=10000.0));
                        ui.label("mm");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Margin X");
                        ui.add(crate::ui::field::NumField::new(&mut state.margin_x_mm).range(0.0..=1000.0));
                        ui.label("Margin Y");
                        ui.add(crate::ui::field::NumField::new(&mut state.margin_y_mm).range(0.0..=1000.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Gutter X");
                        ui.add(crate::ui::field::NumField::new(&mut state.gutter_x_mm).range(0.0..=1000.0));
                        ui.label("Gutter Y");
                        ui.add(crate::ui::field::NumField::new(&mut state.gutter_y_mm).range(0.0..=1000.0));
                        ui.label("Bleed");
                        ui.add(crate::ui::field::NumField::new(&mut state.bleed_mm).range(0.0..=1000.0));
                        ui.checkbox(&mut state.crop_marks, "Crop marks");
                    });
                    if state.crop_marks {
                        ui.horizontal(|ui| {
                            ui.label("Mark length");
                            ui.add(crate::ui::field::NumField::new(&mut state.crop_mark_length_mm).range(0.0..=20.0));
                            ui.label("Offset");
                            ui.add(crate::ui::field::NumField::new(&mut state.crop_mark_offset_mm).range(0.0..=20.0));
                            ui.label("mm");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Line width");
                            ui.add(crate::ui::field::NumField::new(&mut state.crop_mark_width_mm).range(0.05..=2.0));
                            ui.label("mm");
                        });
                        ui.small("Crop-mark length is independent of gutter size. Marks that would overlap another design or leave the sheet are omitted.");
                    }
                    ui.small("Bleed reserves space around each design; it does not extend artwork beyond the trim edge.");

                    let settings = SheetSettings {
                        width_mm: state.sheet_width_mm,
                        height_mm: state.sheet_height_mm,
                        margin_left_mm: state.margin_x_mm,
                        margin_right_mm: state.margin_x_mm,
                        margin_top_mm: state.margin_y_mm,
                        margin_bottom_mm: state.margin_y_mm,
                        gutter_x_mm: state.gutter_x_mm,
                        gutter_y_mm: state.gutter_y_mm,
                        bleed_mm: state.bleed_mm,
                    };
                    let specs: Vec<DesignSpec> = state.designs.iter().map(|design| DesignSpec {
                        id: design.id,
                        width_mm: design.width_mm,
                        height_mm: design.height_mm,
                        copies: design.copies,
                        allow_rotation: design.allow_rotation,
                    }).collect();
                    if state.cached_specs != specs
                        || state.cached_settings != Some(settings)
                        || state.cached_shape_nesting != state.shape_nesting
                        || state.cached_plan.is_none()
                    {
                        let artwork: Vec<DesignArtwork> = state.designs.iter().map(|design| DesignArtwork {
                            id: design.id,
                            shapes: design.shapes.clone(),
                        }).collect();
                        state.cached_plan = Some(if state.shape_nesting {
                            object_imposition::plan_shape_aware(&artwork, &specs, settings)
                        } else {
                            object_imposition::plan(&specs, settings)
                        });
                        state.cached_specs = specs;
                        state.cached_settings = Some(settings);
                        state.cached_shape_nesting = state.shape_nesting;
                        state.preview_sheet_index = 0;
                    }
                    match state.cached_plan.as_ref().expect("plan cache initialized") {
                        Ok(plan) => {
                            ui.separator();
                            ui.heading("Layout preview");
                            if plan.sheet_count > 1 {
                                ui.horizontal(|ui| {
                                    if ui.small_button("Previous").clicked() {
                                        state.preview_sheet_index = state.preview_sheet_index.saturating_sub(1);
                                    }
                                    state.preview_sheet_index = state.preview_sheet_index.min(plan.sheet_count - 1);
                                    ui.label(format!("Sheet {} of {}", state.preview_sheet_index + 1, plan.sheet_count));
                                    if ui.small_button("Next").clicked() {
                                        state.preview_sheet_index = (state.preview_sheet_index + 1).min(plan.sheet_count - 1);
                                    }
                                });
                            } else {
                                state.preview_sheet_index = 0;
                            }
                            draw_preview(ui, plan, &state.designs, settings, state.preview_sheet_index);
                            ui.label(format!(
                                "{} copies across {} design(s) · {} sheet(s) · {:.1}% artwork utilization",
                                plan.total_copies, state.designs.len(), plan.sheet_count, plan.utilization_percent
                            ));
                        }
                        Err(error) => {
                            ui.colored_label(egui::Color32::from_rgb(190, 70, 55), format!("Cannot fit artwork: {error:?}"));
                        }
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                let can_create = matches!(state.cached_plan, Some(Ok(_)));
                if ui.add_enabled(can_create, egui::Button::new("Create production document")).clicked() {
                    create_document = true;
                }
                if ui.button("Close").clicked() {
                    *close = true;
                }
            });
        });

    if add_selected {
        let selected = app.selected_shapes();
        if !state.add_selection(selected) {
            app.status = "Select valid objects or groups before adding designs.".into();
        } else {
            state.cached_plan = None;
        }
    }

    if create_document {
        if let Some(Ok(plan)) = state.cached_plan.as_ref() {
            let settings = SheetSettings {
                width_mm: state.sheet_width_mm,
                height_mm: state.sheet_height_mm,
                margin_left_mm: state.margin_x_mm,
                margin_right_mm: state.margin_x_mm,
                margin_top_mm: state.margin_y_mm,
                margin_bottom_mm: state.margin_y_mm,
                gutter_x_mm: state.gutter_x_mm,
                gutter_y_mm: state.gutter_y_mm,
                bleed_mm: state.bleed_mm,
            };
            let artwork: Vec<DesignArtwork> = state.designs.iter().map(|design| DesignArtwork {
                id: design.id,
                shapes: design.shapes.clone(),
            }).collect();
            let crop_settings = CropMarkSettings {
                length_mm: state.crop_mark_length_mm,
                offset_mm: state.crop_mark_offset_mm,
                stroke_width_mm: state.crop_mark_width_mm,
            };
            match object_imposition::build_document_with_crop_settings(
                "Imposition", settings, plan, &artwork, state.crop_marks, crop_settings,
            ) {
                Ok(doc) => {
                    app.open_document(doc, None);
                    *close = true;
                }
                Err(error) => app.status = format!("Imposition: {error:?}"),
            }
        }
    }
}
