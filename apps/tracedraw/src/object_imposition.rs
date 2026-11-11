//! Object-based prepress imposition planning.
//!
//! This module deliberately contains no document/UI code: it turns design
//! dimensions and copy counts into deterministic sheet placements. The UI
//! and document builder can therefore be tested independently.

#[derive(Debug, Clone, PartialEq)]
pub struct DesignSpec {
    pub id: u64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub copies: u32,
    pub allow_rotation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SheetSettings {
    pub width_mm: f64,
    pub height_mm: f64,
    pub margin_left_mm: f64,
    pub margin_right_mm: f64,
    pub margin_top_mm: f64,
    pub margin_bottom_mm: f64,
    pub gutter_x_mm: f64,
    pub gutter_y_mm: f64,
    pub bleed_mm: f64,
}

impl Default for SheetSettings {
    fn default() -> Self {
        Self {
            width_mm: 210.0,
            height_mm: 297.0,
            margin_left_mm: 5.0,
            margin_right_mm: 5.0,
            margin_top_mm: 5.0,
            margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0,
            gutter_y_mm: 2.0,
            bleed_mm: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectPlacement {
    pub design_id: u64,
    /// Zero-based copy number within this design.
    pub copy_index: u32,
    /// Zero-based output sheet.
    pub sheet_index: usize,
    /// Top-left of the trimmed artwork, in millimetres from sheet origin.
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub rotated: bool,
    /// Exact page-space transform for true-outline nesting; `None` uses the rectangular planner.
    pub shape_transform: Option<[f64; 6]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    InvalidSheet,
    InvalidMargins,
    InvalidDesign { id: u64 },
    DoesNotFit { id: u64, width_mm: f64, height_mm: f64 },
    InvalidPlacementSheet { index: usize, sheet_count: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImpositionPlan {
    pub placements: Vec<ObjectPlacement>,
    pub sheet_count: usize,
    pub total_copies: usize,
    pub utilization_percent: f64,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn right(self) -> f64 { self.x + self.w }
    fn bottom(self) -> f64 { self.y + self.h }
    fn intersects(self, other: Self) -> bool {
        self.x < other.right() && self.right() > other.x
            && self.y < other.bottom() && self.bottom() > other.y
    }
    fn contains(self, other: Self) -> bool {
        self.x <= other.x && self.y <= other.y
            && self.right() >= other.right() && self.bottom() >= other.bottom()
    }
}

#[derive(Debug, Clone, Copy)]
struct Instance {
    id: u64,
    copy: u32,
    w: f64,
    h: f64,
    rotate: bool,
    gutter_x: f64,
    gutter_y: f64,
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    free_index: usize,
    w: f64,
    h: f64,
    rotated: bool,
    short_side: f64,
    long_side: f64,
}

fn candidate(free: Rect, item: Instance, rotate: bool, index: usize) -> Option<Candidate> {
    let (w, h, rotated) = if rotate && item.rotate {
        (item.h + item.gutter_x, item.w + item.gutter_y, true)
    } else {
        (item.w + item.gutter_x, item.h + item.gutter_y, false)
    };
    if w > free.w + 1e-7 || h > free.h + 1e-7 { return None; }
    let dw = (free.w - w).max(0.0);
    let dh = (free.h - h).max(0.0);
    Some(Candidate {
        free_index: index,
        w,
        h,
        rotated,
        short_side: dw.min(dh),
        long_side: dw.max(dh),
    })
}

fn split_free_rects(free: &[Rect], used: Rect) -> Vec<Rect> {
    let mut out = Vec::new();
    for r in free.iter().copied() {
        if !r.intersects(used) {
            out.push(r);
            continue;
        }
        if used.x > r.x && used.x < r.right() {
            out.push(Rect { w: used.x - r.x, ..r });
        }
        if used.right() < r.right() && used.right() > r.x {
            out.push(Rect { x: used.right(), w: r.right() - used.right(), ..r });
        }
        if used.y > r.y && used.y < r.bottom() {
            out.push(Rect { h: used.y - r.y, ..r });
        }
        if used.bottom() < r.bottom() && used.bottom() > r.y {
            out.push(Rect { y: used.bottom(), h: r.bottom() - used.bottom(), ..r });
        }
    }
    out.retain(|r| r.w > 1e-7 && r.h > 1e-7);
    let mut keep = vec![true; out.len()];
    for i in 0..out.len() {
        if !keep[i] { continue; }
        for j in 0..out.len() {
            if i != j && keep[j] && out[j].contains(out[i]) {
                keep[i] = false;
                break;
            }
        }
    }
    out.into_iter().zip(keep).filter_map(|(r, keep)| keep.then_some(r)).collect()
}

/// A design unit is one selected object or group of objects.
#[derive(Debug, Clone)]
pub struct DesignArtwork {
    pub id: u64,
    pub shapes: Vec<tracedraw_core::Shape>,
}

/// User-adjustable crop-mark geometry. Values are in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropMarkSettings {
    /// Length of each crop-mark segment, measured outwards from the trim.
    pub length_mm: f64,
    /// Clear space between the trim/bleed edge and the crop mark.
    pub offset_mm: f64,
    /// Printed line thickness.
    pub stroke_width_mm: f64,
}

impl Default for CropMarkSettings {
    fn default() -> Self {
        Self { length_mm: 3.0, offset_mm: 0.5, stroke_width_mm: 0.2 }
    }
}

impl CropMarkSettings {
    fn safe(self) -> Self {
        Self {
            length_mm: if self.length_mm.is_finite() { self.length_mm.clamp(0.0, 20.0) } else { 3.0 },
            offset_mm: if self.offset_mm.is_finite() { self.offset_mm.clamp(0.0, 20.0) } else { 0.5 },
            stroke_width_mm: if self.stroke_width_mm.is_finite() { self.stroke_width_mm.clamp(0.05, 2.0) } else { 0.2 },
        }
    }
}

/// Build a separate production document from a completed plan.
///
/// The source shapes are expected in page coordinates (as returned by
/// App::selected_shapes). Each placement gets fresh object IDs. The source
/// document is never mutated.
pub fn build_document(
    title: &str,
    sheet: SheetSettings,
    plan: &ImpositionPlan,
    designs: &[DesignArtwork],
    crop_marks: bool,
) -> Result<tracedraw_core::Document, PlanError> {
    build_document_with_crop_settings(title, sheet, plan, designs, crop_marks, CropMarkSettings::default())
}

/// Build a production document with explicit crop-mark geometry.
pub fn build_document_with_crop_settings(
    title: &str,
    sheet: SheetSettings,
    plan: &ImpositionPlan,
    designs: &[DesignArtwork],
    crop_marks: bool,
    crop_settings: CropMarkSettings,
) -> Result<tracedraw_core::Document, PlanError> {
    use tracedraw_core::{Affine, Document, Layer, Page, Point, Shape, Size};
    let crop_settings = crop_settings.safe();

    if sheet.width_mm <= 0.0 || sheet.height_mm <= 0.0
        || !sheet.width_mm.is_finite() || !sheet.height_mm.is_finite()
    {
        return Err(PlanError::InvalidSheet);
    }
    let mut doc = Document::new(
        title,
        Size::new(sheet.width_mm, sheet.height_mm),
    );
    if plan.sheet_count == 0 {
        return Ok(doc);
    }
    for sheet_index in 0..plan.sheet_count {
        if sheet_index == 0 {
            let page = &mut doc.pages[0];
            page.name = format!("Sheet {}", sheet_index + 1);
            page.size = Size::new(sheet.width_mm, sheet.height_mm);
        } else {
            let page_id = doc.ids_mut().page();
            let layer_id = doc.ids_mut().layer();
            doc.pages.push(Page {
                id: page_id,
                name: format!("Sheet {}", sheet_index + 1),
                size: Size::new(sheet.width_mm, sheet.height_mm),
                layers: vec![Layer::new(layer_id, "Artwork")],
                guides: Vec::new(),
                background: None,
            });
        }
    }
    for placement in &plan.placements {
        if placement.sheet_index >= plan.sheet_count {
            return Err(PlanError::InvalidPlacementSheet {
                index: placement.sheet_index,
                sheet_count: plan.sheet_count,
            });
        }
        let placement_values = [
            placement.x_mm,
            placement.y_mm,
            placement.width_mm,
            placement.height_mm,
        ];
        if placement_values.iter().any(|v| !v.is_finite())
            || placement.width_mm <= 0.0
            || placement.height_mm <= 0.0
        {
            return Err(PlanError::InvalidDesign { id: placement.design_id });
        }
        let design = designs.iter().find(|d| d.id == placement.design_id)
            .ok_or(PlanError::InvalidDesign { id: placement.design_id })?;
        if design.shapes.is_empty() {
            return Err(PlanError::InvalidDesign { id: placement.design_id });
        }
        let source_bounds = design.shapes.iter().map(Shape::bounds)
            .reduce(|a, b| a.union(b))
            .ok_or(PlanError::InvalidDesign { id: placement.design_id })?;
        let target_y = sheet.height_mm - placement.y_mm - placement.height_mm;
        let source_center = source_bounds.center();
        let rotate_about_center = if let Some(coeffs) = placement.shape_transform {
            Affine::new(coeffs)
        } else {
            let angle = if placement.rotated { std::f64::consts::FRAC_PI_2 } else { 0.0 };
            Affine::translate((source_center.x, source_center.y))
                * Affine::rotate(angle)
                * Affine::translate((-source_center.x, -source_center.y))
        };
        let rotated_corners = [
            Point::new(source_bounds.x0, source_bounds.y0),
            Point::new(source_bounds.x1, source_bounds.y0),
            Point::new(source_bounds.x1, source_bounds.y1),
            Point::new(source_bounds.x0, source_bounds.y1),
        ].map(|p| rotate_about_center * p);
        let min_x = rotated_corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let min_y = rotated_corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let max_x = rotated_corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let max_y = rotated_corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        let rotated_bounds = tracedraw_core::Rect::new(min_x, min_y, max_x, max_y);
        let move_to_sheet = Affine::translate((
            placement.x_mm - rotated_bounds.x0,
            target_y - rotated_bounds.y0,
        ));
        let world = move_to_sheet * rotate_about_center;
        fn refresh_identity(
            shape: &mut Shape,
            allocate: &mut impl FnMut() -> tracedraw_core::ShapeId,
        ) {
            shape.id = allocate();
            shape.uid = None;
            match &mut shape.kind {
                tracedraw_core::ShapeKind::Group { children } => {
                    for child in children {
                        refresh_identity(child, allocate);
                    }
                }
                tracedraw_core::ShapeKind::ClipFrame { frame, contents } => {
                    refresh_identity(frame, allocate);
                    for child in contents {
                        refresh_identity(child, allocate);
                    }
                }
                _ => {}
            }
        }

        let mut copies = Vec::with_capacity(design.shapes.len());
        for original in &design.shapes {
            let mut shape = original.clone();
            refresh_identity(&mut shape, &mut || doc.ids_mut().shape());
            shape.transform = world * shape.transform;
            copies.push(shape);
        }
        let mut crop_shapes = Vec::new();
        if crop_marks {
            // Editable vector crop marks in page coordinates (mm).
            // Planner coordinates are top-left / Y-down; document geometry
            // is bottom-left / Y-up.
            let left = placement.x_mm;
            let right = placement.x_mm + placement.width_mm;
            let bottom = target_y;
            let top = target_y + placement.height_mm;
            // Marks use the requested length and offset independently of
            // the gutter. A mark is omitted only when it would leave the
            // sheet or collide with another design's reserved bleed area.
            let gap = sheet.bleed_mm + crop_settings.offset_mm;
            let length_x = crop_settings.length_mm;
            let length_y = crop_settings.length_mm;
            let mut add_mark = |points: [Point; 2]| {
                // Keep marks on the sheet and out of neighboring trim areas.
                let min_x = points[0].x.min(points[1].x);
                let max_x = points[0].x.max(points[1].x);
                let min_y = points[0].y.min(points[1].y);
                let max_y = points[0].y.max(points[1].y);
                // A gutter that is too narrow has no room for a crop mark.
                // Do not add a zero-length path to the production document.
                if (max_x - min_x).abs() <= 1e-7
                    && (max_y - min_y).abs() <= 1e-7
                {
                    return;
                }
                if min_x < 0.0 || min_y < 0.0
                    || max_x > sheet.width_mm || max_y > sheet.height_mm
                {
                    return;
                }
                let mark_bounds = tracedraw_core::Rect::new(
                    min_x - 1e-6, min_y - 1e-6, max_x + 1e-6, max_y + 1e-6,
                );
                let touches_neighbor = plan.placements.iter().any(|other| {
                    if other.sheet_index != placement.sheet_index
                        || (other.design_id == placement.design_id
                            && other.copy_index == placement.copy_index)
                    {
                        return false;
                    }
                    let other_bottom = sheet.height_mm - other.y_mm - other.height_mm;
                    // Keep marks clear of the neighboring design's
                    // reserved bleed and gutter area, not just its trim box.
                    let other_bounds = tracedraw_core::Rect::new(
                        other.x_mm - sheet.bleed_mm,
                        other_bottom - sheet.bleed_mm,
                        other.x_mm + other.width_mm + sheet.bleed_mm,
                        other_bottom + other.height_mm + sheet.bleed_mm,
                    );
                    mark_bounds.x0 < other_bounds.x1
                        && mark_bounds.x1 > other_bounds.x0
                        && mark_bounds.y0 < other_bounds.y1
                        && mark_bounds.y1 > other_bounds.y0
                });
                if touches_neighbor {
                    return;
                }
                let mut path = tracedraw_core::BezPath::new();
                path.move_to(points[0]);
                path.line_to(points[1]);
                let mut mark = Shape::new(
                    doc.ids_mut().shape(),
                    tracedraw_core::ShapeKind::Path { path, closed: false },
                );
                mark.name = Some("Crop mark".into());
                mark.fill = tracedraw_core::Fill::None;
                let mut stroke = tracedraw_core::Stroke::default();
                stroke.width = crop_settings.stroke_width_mm;
                mark.stroke = Some(stroke);
                crop_shapes.push(mark);
            };
            add_mark([Point::new(left - gap - length_x, bottom), Point::new(left - gap, bottom)]);
            add_mark([Point::new(left, bottom - gap - length_y), Point::new(left, bottom - gap)]);
            add_mark([Point::new(right + gap, bottom), Point::new(right + gap + length_x, bottom)]);
            add_mark([Point::new(right, bottom - gap - length_y), Point::new(right, bottom - gap)]);
            add_mark([Point::new(left - gap - length_x, top), Point::new(left - gap, top)]);
            add_mark([Point::new(left, top + gap), Point::new(left, top + gap + length_y)]);
            add_mark([Point::new(right + gap, top), Point::new(right + gap + length_x, top)]);
            add_mark([Point::new(right, top + gap), Point::new(right, top + gap + length_y)]);
        }
        let page = &mut doc.pages[placement.sheet_index];
        let layer = page.layers.first_mut().expect("each output page has a layer");
        layer.shapes.extend(copies);
        layer.shapes.extend(crop_shapes);
    }
    Ok(doc)
}

/// Plan copies by their actual vector outlines using TraceCut's contour nesting engine.
/// The returned transform is in page coordinates (mm, Y up), ready to apply to each source shape.
pub fn plan_shape_aware(
    designs: &[DesignArtwork],
    specs: &[DesignSpec],
    settings: SheetSettings,
) -> Result<ImpositionPlan, PlanError> {
    use tracedraw_tracecut::nest::{
        shapes::{nest_shapes, Rotations, ShapeSettings},
        SheetSpec,
    };

    if specs.len() != designs.len() {
        return Err(PlanError::InvalidSheet);
    }
    let values = [
        settings.width_mm, settings.height_mm, settings.margin_left_mm,
        settings.margin_right_mm, settings.margin_top_mm, settings.margin_bottom_mm,
        settings.gutter_x_mm, settings.gutter_y_mm, settings.bleed_mm,
    ];
    if values.iter().any(|v| !v.is_finite()) || settings.width_mm <= 0.0 || settings.height_mm <= 0.0 {
        return Err(PlanError::InvalidSheet);
    }
    if values[2..].iter().any(|v| *v < 0.0) {
        return Err(PlanError::InvalidMargins);
    }

    let mut parts = Vec::new();
    let mut owners = Vec::new();
    for (design, spec) in designs.iter().zip(specs) {
        if design.id != spec.id || spec.copies == 0 {
            if design.id != spec.id {
                return Err(PlanError::InvalidDesign { id: spec.id });
            }
            continue;
        }
        if !spec.width_mm.is_finite() || !spec.height_mm.is_finite()
            || spec.width_mm <= 0.0 || spec.height_mm <= 0.0
        {
            return Err(PlanError::InvalidDesign { id: spec.id });
        }
        let paths: Vec<_> = design.shapes.iter()
            .flat_map(tracedraw_tracecut::outlines)
            .collect();
        if paths.is_empty() {
            return Err(PlanError::InvalidDesign { id: spec.id });
        }
        for copy_index in 0..spec.copies {
            parts.push(paths.clone());
            owners.push((spec.id, copy_index));
        }
    }
    if parts.is_empty() {
        return Ok(ImpositionPlan {
            placements: Vec::new(), sheet_count: 0, total_copies: 0,
            utilization_percent: 0.0,
        });
    }

    // The shape engine uses a uniform inset and spacing. Use the larger
    // horizontal/vertical margins and gutter conservatively, and reserve
    // bleed on every side so no artwork can enter a neighboring bleed.
    let margin = settings.margin_left_mm
        .max(settings.margin_right_mm)
        .max(settings.margin_top_mm)
        .max(settings.margin_bottom_mm)
        + settings.bleed_mm;
    let spacing = settings.gutter_x_mm.max(settings.gutter_y_mm)
        + 2.0 * settings.bleed_mm;
    let nest_settings = ShapeSettings {
        sheet: SheetSpec {
            width: settings.width_mm,
            height: settings.height_mm,
            margin,
            spacing,
            ..Default::default()
        },
        rotations: Rotations::Quarter,
        holes: true,
        tolerance: 0.15,
        rounds: 40,
        seed: 1,
    };
    let mut never_stop = || false;
    let layout = nest_shapes(&parts, &nest_settings, &mut never_stop);
    let mut placements = Vec::with_capacity(parts.len());
    for (index, placement) in layout.placements.iter().enumerate() {
        let Some(placement) = placement else {
            let (id, _) = owners[index];
            let spec = specs.iter().find(|s| s.id == id).expect("owner has spec");
            return Err(PlanError::DoesNotFit {
                id,
                width_mm: spec.width_mm,
                height_mm: spec.height_mm,
            });
        };
        let bounds = tracedraw_tracecut::nest::shapes::placed_bounds(
            &parts[index], placement.transform,
        ).ok_or_else(|| {
            let (id, _) = owners[index];
            PlanError::InvalidDesign { id }
        })?;
        let (design_id, copy_index) = owners[index];
        placements.push(ObjectPlacement {
            design_id,
            copy_index,
            sheet_index: placement.sheet,
            x_mm: bounds.x0,
            y_mm: settings.height_mm - bounds.y1,
            width_mm: bounds.width(),
            height_mm: bounds.height(),
            rotated: placement.angle.abs() > 1e-7,
            shape_transform: Some(placement.transform.as_coeffs()),
        });
    }
    let sheet_count = layout.sheets;
    let artwork_area = layout.parts_area;
    let sheet_area = settings.width_mm * settings.height_mm * sheet_count as f64;
    Ok(ImpositionPlan {
        total_copies: placements.len(),
        placements,
        sheet_count,
        utilization_percent: if sheet_area > 0.0 {
            artwork_area / sheet_area * 100.0
        } else {
            0.0
        },
    })
}

/// Plan regular grid and cross-rotated candidates, matching the calculator's
/// deterministic "maximum copies, then simplest layout" strategy for a
/// single design. Multiple designs retain the general MaxRects fallback.
pub fn plan(designs: &[DesignSpec], settings: SheetSettings) -> Result<ImpositionPlan, PlanError> {
    let values = [
        settings.width_mm, settings.height_mm, settings.margin_left_mm,
        settings.margin_right_mm, settings.margin_top_mm, settings.margin_bottom_mm,
        settings.gutter_x_mm, settings.gutter_y_mm, settings.bleed_mm,
    ];
    if values.iter().any(|v| !v.is_finite()) || settings.width_mm <= 0.0 || settings.height_mm <= 0.0 {
        return Err(PlanError::InvalidSheet);
    }
    if values[2..].iter().any(|v| *v < 0.0) {
        return Err(PlanError::InvalidMargins);
    }
    let usable = Rect {
        x: settings.margin_left_mm, y: settings.margin_top_mm,
        w: settings.width_mm - settings.margin_left_mm - settings.margin_right_mm,
        h: settings.height_mm - settings.margin_top_mm - settings.margin_bottom_mm,
    };
    if usable.w <= 0.0 || usable.h <= 0.0 { return Err(PlanError::InvalidMargins); }
    for d in designs {
        if !d.width_mm.is_finite() || !d.height_mm.is_finite() || d.width_mm <= 0.0 || d.height_mm <= 0.0 {
            return Err(PlanError::InvalidDesign { id: d.id });
        }
    }
    if designs.len() == 1 {
        let d = &designs[0];
        if d.copies == 0 {
            return Ok(ImpositionPlan { placements: Vec::new(), sheet_count: 0, total_copies: 0, utilization_percent: 0.0 });
        }
        let bleed = settings.bleed_mm;
        let w = d.width_mm + 2.0 * bleed;
        let h = d.height_mm + 2.0 * bleed;
        // Every candidate is a list of trim origins in sheet coordinates.
        let grid = |x: f64, y: f64, aw: f64, ah: f64, iw: f64, ih: f64, rotated: bool| -> Vec<(f64,f64,bool)> {
            let mut out = Vec::new();
            if aw <= 0.0 || ah <= 0.0 { return out; }
            let sx = iw + settings.gutter_x_mm;
            let sy = ih + settings.gutter_y_mm;
            let nx = ((aw + settings.gutter_x_mm) / sx).floor().max(0.0) as usize;
            let ny = ((ah + settings.gutter_y_mm) / sy).floor().max(0.0) as usize;
            for row in 0..ny {
                for col in 0..nx {
                    let px = x + col as f64 * sx;
                    let py = y + row as f64 * sy;
                    if px + iw <= x + aw + 1e-7 && py + ih <= y + ah + 1e-7 {
                        out.push((px + bleed, py + bleed, rotated));
                    }
                }
            }
            out
        };
        let mut candidates: Vec<(String, Vec<(f64,f64,bool)>)> = Vec::new();
        candidates.push(("Grid normal".into(), grid(usable.x, usable.y, usable.w, usable.h, w, h, false)));
        if d.allow_rotation {
            candidates.push(("Grid rotasi".into(), grid(usable.x, usable.y, usable.w, usable.h, h, w, true)));
            // Normal grid with a rotated right strip and/or bottom strip.
            let nx = ((usable.w + settings.gutter_x_mm) / (w + settings.gutter_x_mm)).floor() as usize;
            let ny = ((usable.h + settings.gutter_y_mm) / (h + settings.gutter_y_mm)).floor() as usize;
            let main_w = (nx as f64 * (w + settings.gutter_x_mm) - settings.gutter_x_mm).max(0.0);
            let main_h = (ny as f64 * (h + settings.gutter_y_mm) - settings.gutter_y_mm).max(0.0);
            let gx = settings.gutter_x_mm;
            let gy = settings.gutter_y_mm;
            let mut right = grid(usable.x + main_w + gx, usable.y, usable.w - main_w - gx, usable.h, h, w, true);
            let mut base = grid(usable.x, usable.y, main_w, usable.h, w, h, false);
            let mut mixed = base.clone(); mixed.append(&mut right);
            candidates.push(("Grid normal + rotasi kanan".into(), mixed));
            let mut bottom = grid(usable.x, usable.y + main_h + gy, usable.w, usable.h - main_h - gy, h, w, true);
            base = grid(usable.x, usable.y, usable.w, main_h, w, h, false);
            base.append(&mut bottom);
            candidates.push(("Grid normal + rotasi bawah".into(), base));
            // Rotated base grid with normal fill in the remaining strips.
            let rnx = ((usable.w + gx) / (h + gx)).floor() as usize;
            let rny = ((usable.h + gy) / (w + gy)).floor() as usize;
            let rot_w = (rnx as f64 * (h + gx) - gx).max(0.0);
            let rot_h = (rny as f64 * (w + gy) - gy).max(0.0);
            let mut right2 = grid(usable.x + rot_w + gx, usable.y, usable.w - rot_w - gx, usable.h, w, h, false);
            let mut base2 = grid(usable.x, usable.y, rot_w, usable.h, h, w, true);
            base2.append(&mut right2);
            candidates.push(("Grid rotasi + normal kanan".into(), base2));
            let mut bottom2 = grid(usable.x, usable.y + rot_h + gy, usable.w, usable.h - rot_h - gy, w, h, false);
            let mut base3 = grid(usable.x, usable.y, usable.w, rot_h, h, w, true);
            base3.append(&mut bottom2);
            candidates.push(("Grid rotasi + normal bawah".into(), base3));
        }
        // Match the calculator's tie-break: maximum copies first,
        // then prefer normal, rotated, and finally mixed layouts.
        let layout_weight = |name: &str| {
            if name == "Grid normal" { 0 }
            else if name == "Grid rotasi" { 1 }
            else { 3 }
        };
        candidates.sort_by(|a, b| {
            b.1.len().cmp(&a.1.len())
                .then_with(|| layout_weight(&a.0).cmp(&layout_weight(&b.0)))
                .then_with(|| a.0.cmp(&b.0))
        });
        let best = candidates.into_iter().next().map(|x| x.1).unwrap_or_default();
        if best.is_empty() {
            return Err(PlanError::DoesNotFit { id: d.id, width_mm: d.width_mm, height_mm: d.height_mm });
        }
        let per_sheet = best.len();
        let mut placements = Vec::with_capacity(d.copies as usize);
        for copy in 0..d.copies {
            let sheet_index = copy as usize / per_sheet;
            let pos = best[copy as usize % per_sheet];
            let (pw, ph) = if pos.2 { (d.height_mm, d.width_mm) } else { (d.width_mm, d.height_mm) };
            placements.push(ObjectPlacement {
                design_id: d.id, copy_index: copy, sheet_index,
                x_mm: pos.0, y_mm: pos.1, width_mm: pw, height_mm: ph, rotated: pos.2, shape_transform: None,
            });
        }
        let sheet_count = (d.copies as usize + per_sheet - 1) / per_sheet;
        let artwork_area = placements.iter().map(|p| p.width_mm * p.height_mm).sum::<f64>();
        let sheet_area = usable.w * usable.h * sheet_count as f64;
        return Ok(ImpositionPlan {
            placements, sheet_count, total_copies: d.copies as usize,
            utilization_percent: if sheet_area > 0.0 { artwork_area / sheet_area * 100.0 } else { 0.0 },
        });
    }

    // If all selected designs share the same physical dimensions and
    // rotation policy, treat them as one production grid. Fill its slots in
    // design order so separate files never get scattered by MaxRects.
    let same_size_group = designs.len() > 1
        && designs.iter().all(|d| {
            (d.width_mm - designs[0].width_mm).abs() < 1e-6
                && (d.height_mm - designs[0].height_mm).abs() < 1e-6
                && d.allow_rotation == designs[0].allow_rotation
        });
    if same_size_group {
        let total_copies: u32 = designs.iter().map(|d| d.copies).sum();
        if total_copies == 0 {
            return Ok(ImpositionPlan {
                placements: Vec::new(), sheet_count: 0, total_copies: 0,
                utilization_percent: 0.0,
            });
        }
        let combined = DesignSpec {
            id: designs[0].id,
            width_mm: designs[0].width_mm,
            height_mm: designs[0].height_mm,
            copies: total_copies,
            allow_rotation: designs[0].allow_rotation,
        };
        let mut grouped = plan(&[combined], settings)?;
        let mut design_index = 0usize;
        let mut assigned = 0u32;
        for placement in &mut grouped.placements {
            while design_index < designs.len() && assigned >= designs[design_index].copies {
                design_index += 1;
                assigned = 0;
            }
            if design_index >= designs.len() {
                return Err(PlanError::InvalidDesign { id: placement.design_id });
            }
            placement.design_id = designs[design_index].id;
            placement.copy_index = assigned;
            assigned += 1;
        }
        return Ok(grouped);
    }

    // Mixed-size jobs retain the collision-safe MaxRects planner.
    let bleed = settings.bleed_mm;
    let mut items = Vec::new();
    for d in designs {
        for copy in 0..d.copies {
            items.push(Instance { id: d.id, copy, w: d.width_mm + 2.0 * bleed,
                h: d.height_mm + 2.0 * bleed, rotate: d.allow_rotation,
                gutter_x: settings.gutter_x_mm, gutter_y: settings.gutter_y_mm });
        }
    }
    let total = items.len();
    items.sort_by(|a,b| (b.w*b.h).total_cmp(&(a.w*a.h)).then_with(|| b.w.max(b.h).total_cmp(&a.w.max(a.h))));
    // Gutter is required only between neighboring designs, never between
    // the final design and the sheet edge. Extend the packing workspace by
    // one gutter to the right and bottom so padded instances can use that
    // edge space without pushing their bleed outside the usable sheet.
    let packing_area = Rect {
        x: usable.x,
        y: usable.y,
        w: usable.w + settings.gutter_x_mm,
        h: usable.h + settings.gutter_y_mm,
    };
    let mut sheets: Vec<Vec<Rect>> = Vec::new();
    let mut placements = Vec::with_capacity(total);
    // Keep the padded rectangles alongside placements so the mixed-size
    // heuristic can prefer extending the current design's block.
    let mut placed_outer: Vec<(u64, usize, Rect)> = Vec::with_capacity(total);
    let mut design_rotations: std::collections::HashMap<u64, bool> = std::collections::HashMap::new();
    let mut artwork_area = 0.0;
    for item in items {
        let rotation_options: &[bool] = match design_rotations.get(&item.id) {
            Some(true) => &[true],
            Some(false) => &[false],
            None => &[false, true],
        };
        let mut best: Option<(usize,Candidate,(u8,f64,f64))> = None;
        for (si, free) in sheets.iter().enumerate() {
            for (fi, rect) in free.iter().copied().enumerate() {
                for &rotate in rotation_options {
                    if let Some(c) = candidate(rect,item,rotate,fi) {
                        let proposed = Rect { x: rect.x, y: rect.y, w: c.w, h: c.h };
                        // A same-design neighbor is preferred before local
                        // free-rectangle fit. This keeps copies together
                        // instead of scattering them into unrelated gaps.
                        let extends_group = placed_outer.iter().any(|(id, sheet, placed)| {
                            if *id != item.id || *sheet != si { return false; }
                            let horizontal_touch =
                                (proposed.x - placed.right()).abs() <= 1e-6
                                || (placed.x - proposed.right()).abs() <= 1e-6;
                            let vertical_overlap =
                                proposed.y < placed.bottom() - 1e-6
                                && proposed.bottom() > placed.y + 1e-6;
                            let vertical_touch =
                                (proposed.y - placed.bottom()).abs() <= 1e-6
                                || (placed.y - proposed.bottom()).abs() <= 1e-6;
                            let horizontal_overlap =
                                proposed.x < placed.right() - 1e-6
                                && proposed.right() > placed.x + 1e-6;
                            (horizontal_touch && vertical_overlap)
                                || (vertical_touch && horizontal_overlap)
                        });
                        let score = (
                            if extends_group { 0 } else { 1 },
                            c.short_side,
                            c.long_side,
                        );
                        if best.as_ref().is_none_or(|(_,_,old)| score < *old) {
                            best = Some((si,c,score));
                        }
                    }
                }
            }
        }
        let (si,c)=if let Some((si,c,_))=best { (si,c) } else {
            let new_sheet_candidate = match design_rotations.get(&item.id) {
                Some(&rotate) => candidate(packing_area,item,rotate,0),
                None => candidate(packing_area,item,false,0)
                    .or_else(|| candidate(packing_area,item,true,0)),
            };
            let Some(c)=new_sheet_candidate else {
                return Err(PlanError::DoesNotFit { id:item.id,width_mm:item.w-2.0*bleed,height_mm:item.h-2.0*bleed });
            };
            sheets.push(vec![packing_area]); (sheets.len()-1,c)
        };
        // Rotation is selected once per design, not independently for each
        // copy. This keeps every copy in a design block consistently oriented.
        design_rotations.entry(item.id).or_insert(c.rotated);
        let free=sheets[si][c.free_index];
        let outer=Rect{x:free.x,y:free.y,w:c.w,h:c.h};
        sheets[si]=split_free_rects(&sheets[si],outer);
        placed_outer.push((item.id, si, outer));
        placements.push(ObjectPlacement { design_id:item.id,copy_index:item.copy,sheet_index:si,
            x_mm:outer.x+bleed,y_mm:outer.y+bleed,
            width_mm:c.w-settings.gutter_x_mm-2.0*bleed,
            height_mm:c.h-settings.gutter_y_mm-2.0*bleed,rotated:c.rotated,shape_transform:None });
        let (tw,th)=if c.rotated {(item.h-2.0*bleed,item.w-2.0*bleed)} else {(item.w-2.0*bleed,item.h-2.0*bleed)};
        artwork_area+=tw*th;
    }
    let sheet_area=usable.w*usable.h*sheets.len() as f64;
    Ok(ImpositionPlan { placements,sheet_count:sheets.len(),total_copies:total,
        utilization_percent:if sheet_area>0.0 {artwork_area/sheet_area*100.0} else {0.0} })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_mark_settings_have_print_friendly_defaults_and_safe_bounds() {
        assert_eq!(CropMarkSettings::default(), CropMarkSettings {
            length_mm: 3.0, offset_mm: 0.5, stroke_width_mm: 0.2,
        });
        let safe = CropMarkSettings {
            length_mm: f64::NAN, offset_mm: -4.0, stroke_width_mm: 9.0,
        }.safe();
        assert_eq!(safe.length_mm, 3.0);
        assert_eq!(safe.offset_mm, 0.0);
        assert_eq!(safe.stroke_width_mm, 2.0);
    }

    #[test]
    fn business_cards_fit_twenty_five_up_on_a3_plus() {
        let settings = SheetSettings {
            width_mm: 320.0, height_mm: 480.0,
            margin_left_mm: 5.0, margin_right_mm: 5.0,
            margin_top_mm: 5.0, margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0, gutter_y_mm: 2.0, bleed_mm: 0.0,
        };
        let result = plan(
            &[DesignSpec { id: 1, width_mm: 90.0, height_mm: 55.0, copies: 25, allow_rotation: true }],
            settings,
        ).unwrap();
        assert_eq!(result.total_copies, 25);
        assert_eq!(result.sheet_count, 1);
        assert_eq!(result.placements.len(), 25);
        assert!(result.placements.iter().all(|p| p.rotated));
        assert!(result.placements.iter().all(|p| (p.width_mm, p.height_mm) == (55.0, 90.0)));
    }

    #[test]
    fn equal_size_designs_share_one_regular_grid_in_design_order() {
        let settings = SheetSettings {
            width_mm: 320.0, height_mm: 480.0,
            margin_left_mm: 5.0, margin_right_mm: 5.0,
            margin_top_mm: 5.0, margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0, gutter_y_mm: 2.0, bleed_mm: 0.0,
        };
        let result = plan(
            &[
                DesignSpec { id: 10, width_mm: 90.0, height_mm: 55.0, copies: 10, allow_rotation: true },
                DesignSpec { id: 20, width_mm: 90.0, height_mm: 55.0, copies: 10, allow_rotation: true },
            ],
            settings,
        ).unwrap();
        assert_eq!(result.total_copies, 20);
        assert_eq!(result.sheet_count, 1);
        assert_eq!(result.placements.len(), 20);
        assert!(result.placements[..10].iter().all(|p| p.design_id == 10));
        assert!(result.placements[10..].iter().all(|p| p.design_id == 20));
        assert!(result.placements.iter().all(|p| p.rotated));
        let mut xs: Vec<_> = result.placements.iter().map(|p| p.x_mm).collect();
        let mut ys: Vec<_> = result.placements.iter().map(|p| p.y_mm).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        xs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        ys.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        assert_eq!(xs.len(), 5);
        assert_eq!(ys.len(), 4);
    }

    #[test]
    fn unequal_copy_counts_keep_equal_size_designs_on_the_same_grid() {
        let settings = SheetSettings {
            width_mm: 320.0, height_mm: 480.0,
            margin_left_mm: 5.0, margin_right_mm: 5.0,
            margin_top_mm: 5.0, margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0, gutter_y_mm: 2.0, bleed_mm: 1.0,
        };
        let grouped = plan(
            &[
                DesignSpec { id: 31, width_mm: 90.0, height_mm: 55.0, copies: 3, allow_rotation: true },
                DesignSpec { id: 32, width_mm: 90.0, height_mm: 55.0, copies: 7, allow_rotation: true },
            ],
            settings,
        ).unwrap();
        let reference = plan(
            &[DesignSpec { id: 99, width_mm: 90.0, height_mm: 55.0, copies: 10, allow_rotation: true }],
            settings,
        ).unwrap();

        assert_eq!(grouped.sheet_count, reference.sheet_count);
        assert_eq!(grouped.placements.len(), 10);
        assert_eq!(
            grouped.placements.iter().map(|p| (p.x_mm, p.y_mm, p.width_mm, p.height_mm, p.rotated)).collect::<Vec<_>>(),
            reference.placements.iter().map(|p| (p.x_mm, p.y_mm, p.width_mm, p.height_mm, p.rotated)).collect::<Vec<_>>(),
        );
        assert!(grouped.placements[..3].iter().all(|p| p.design_id == 31));
        assert!(grouped.placements[3..].iter().all(|p| p.design_id == 32));

        for (i, a) in grouped.placements.iter().enumerate() {
            let a_rect = Rect {
                x: a.x_mm - settings.bleed_mm,
                y: a.y_mm - settings.bleed_mm,
                w: a.width_mm + 2.0 * settings.bleed_mm,
                h: a.height_mm + 2.0 * settings.bleed_mm,
            };
            for b in grouped.placements.iter().skip(i + 1).filter(|b| b.sheet_index == a.sheet_index) {
                let b_rect = Rect {
                    x: b.x_mm - settings.bleed_mm,
                    y: b.y_mm - settings.bleed_mm,
                    w: b.width_mm + 2.0 * settings.bleed_mm,
                    h: b.height_mm + 2.0 * settings.bleed_mm,
                };
                assert!(!a_rect.intersects(b_rect), "grouped designs overlap: {a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn repeats_master_design_on_one_sheet() {
        let result = plan(
            &[DesignSpec { id: 7, width_mm: 90.0, height_mm: 50.0, copies: 4, allow_rotation: false }],
            SheetSettings { width_mm: 200.0, height_mm: 120.0, margin_left_mm: 5.0, margin_right_mm: 5.0, margin_top_mm: 5.0, margin_bottom_mm: 5.0, gutter_x_mm: 0.0, gutter_y_mm: 0.0, bleed_mm: 0.0 },
        ).unwrap();
        assert_eq!(result.total_copies, 4);
        assert_eq!(result.sheet_count, 1);
        assert!(result.placements.iter().all(|p| p.design_id == 7));
    }

    #[test]
    fn combines_multiple_designs_and_rotates_when_needed() {
        let result = plan(
            &[
                DesignSpec { id: 1, width_mm: 80.0, height_mm: 40.0, copies: 2, allow_rotation: true },
                DesignSpec { id: 2, width_mm: 40.0, height_mm: 80.0, copies: 1, allow_rotation: true },
            ],
            SheetSettings { width_mm: 130.0, height_mm: 90.0, margin_left_mm: 0.0, margin_right_mm: 0.0, margin_top_mm: 0.0, margin_bottom_mm: 0.0, gutter_x_mm: 0.0, gutter_y_mm: 0.0, bleed_mm: 0.0 },
        ).unwrap();
        assert_eq!(result.total_copies, 3);
        assert_eq!(result.sheet_count, 1);
    }

    #[test]
    fn mixed_size_copies_prefer_contiguous_design_blocks() {
        let settings = SheetSettings {
            width_mm: 160.0, height_mm: 100.0,
            margin_left_mm: 5.0, margin_right_mm: 5.0,
            margin_top_mm: 5.0, margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0, gutter_y_mm: 2.0, bleed_mm: 0.0,
        };
        let result = plan(
            &[
                DesignSpec { id: 61, width_mm: 60.0, height_mm: 30.0, copies: 2, allow_rotation: false },
                DesignSpec { id: 62, width_mm: 20.0, height_mm: 20.0, copies: 4, allow_rotation: false },
            ],
            settings,
        ).unwrap();

        assert_eq!(result.sheet_count, 1);
        for design_id in [61, 62] {
            let group: Vec<_> = result.placements.iter()
                .filter(|p| p.design_id == design_id)
                .collect();
            assert!(!group.is_empty());
            // Each copy should touch at least one other copy of the same
            // design when measured using its padded (gutter-inclusive) box.
            for placement in &group {
                let a = Rect {
                    x: placement.x_mm,
                    y: placement.y_mm,
                    w: placement.width_mm + settings.gutter_x_mm,
                    h: placement.height_mm + settings.gutter_y_mm,
                };
                let has_neighbor = group.iter().any(|other| {
                    if other.copy_index == placement.copy_index { return false; }
                    let b = Rect {
                        x: other.x_mm,
                        y: other.y_mm,
                        w: other.width_mm + settings.gutter_x_mm,
                        h: other.height_mm + settings.gutter_y_mm,
                    };
                    let horizontal = (a.right() - b.x).abs() <= 1e-6
                        || (b.right() - a.x).abs() <= 1e-6;
                    let vertical_overlap = a.y < b.bottom() - 1e-6
                        && a.bottom() > b.y + 1e-6;
                    let vertical = (a.bottom() - b.y).abs() <= 1e-6
                        || (b.bottom() - a.y).abs() <= 1e-6;
                    let horizontal_overlap = a.x < b.right() - 1e-6
                        && a.right() > b.x + 1e-6;
                    (horizontal && vertical_overlap) || (vertical && horizontal_overlap)
                });
                assert!(has_neighbor, "design {design_id} copy {} is isolated", placement.copy_index);
            }
        }
    }

    #[test]
    fn mixed_size_design_copies_keep_one_rotation_orientation() {
        let settings = SheetSettings {
            width_mm: 180.0, height_mm: 110.0,
            margin_left_mm: 5.0, margin_right_mm: 5.0,
            margin_top_mm: 5.0, margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0, gutter_y_mm: 2.0, bleed_mm: 0.0,
        };
        let result = plan(
            &[
                DesignSpec { id: 71, width_mm: 75.0, height_mm: 35.0, copies: 3, allow_rotation: true },
                DesignSpec { id: 72, width_mm: 25.0, height_mm: 18.0, copies: 5, allow_rotation: true },
            ],
            settings,
        ).unwrap();

        for design_id in [71, 72] {
            let orientations: Vec<_> = result.placements.iter()
                .filter(|p| p.design_id == design_id)
                .map(|p| p.rotated)
                .collect();
            assert!(!orientations.is_empty());
            assert!(orientations.iter().all(|&rotated| rotated == orientations[0]),
                "design {design_id} copies use inconsistent rotation: {orientations:?}");
        }
    }

    #[test]
    fn rejects_a_design_that_cannot_fit() {
        let result = plan(
            &[DesignSpec { id: 9, width_mm: 300.0, height_mm: 20.0, copies: 1, allow_rotation: false }],
            SheetSettings::default(),
        );
        assert!(matches!(result, Err(PlanError::DoesNotFit { id: 9, .. })));
    }

    #[test]
    fn rotated_placement_reports_the_rotated_trim_size() {
        let result = plan(
            &[DesignSpec { id: 11, width_mm: 40.0, height_mm: 80.0, copies: 1, allow_rotation: true }],
            SheetSettings { width_mm: 90.0, height_mm: 50.0, margin_left_mm: 0.0, margin_right_mm: 0.0, margin_top_mm: 0.0, margin_bottom_mm: 0.0, gutter_x_mm: 0.0, gutter_y_mm: 0.0, bleed_mm: 0.0 },
        ).unwrap();
        let p = result.placements[0];
        assert!(p.rotated);
        assert_eq!((p.width_mm, p.height_mm), (80.0, 40.0));
    }

    #[test]
    fn zero_copies_produce_no_empty_sheet() {
        let result = plan(
            &[DesignSpec { id: 12, width_mm: 40.0, height_mm: 30.0, copies: 0, allow_rotation: false }],
            SheetSettings::default(),
        ).unwrap();
        assert_eq!(result.total_copies, 0);
        assert_eq!(result.sheet_count, 0);
        assert!(result.placements.is_empty());
    }

    #[test]
    fn placements_stay_inside_sheet_and_do_not_overlap() {
        let settings = SheetSettings {
            width_mm: 210.0,
            height_mm: 148.0,
            margin_left_mm: 5.0,
            margin_right_mm: 5.0,
            margin_top_mm: 5.0,
            margin_bottom_mm: 5.0,
            gutter_x_mm: 3.0,
            gutter_y_mm: 4.0,
            bleed_mm: 2.0,
        };
        let result = plan(
            &[
                DesignSpec { id: 1, width_mm: 70.0, height_mm: 40.0, copies: 4, allow_rotation: true },
                DesignSpec { id: 2, width_mm: 35.0, height_mm: 25.0, copies: 3, allow_rotation: false },
            ],
            settings,
        ).unwrap();

        assert_eq!(result.total_copies, 7);
        for (i, a) in result.placements.iter().enumerate() {
            let outer_a = Rect {
                x: a.x_mm - settings.bleed_mm,
                y: a.y_mm - settings.bleed_mm,
                w: a.width_mm + 2.0 * settings.bleed_mm + settings.gutter_x_mm,
                h: a.height_mm + 2.0 * settings.bleed_mm + settings.gutter_y_mm,
            };
            // Bleed must stay inside the usable sheet. The trailing
            // gutter may extend beyond its edge because it is not printable
            // content and is not required after the final item.
            let bleed_a = Rect {
                x: a.x_mm - settings.bleed_mm,
                y: a.y_mm - settings.bleed_mm,
                w: a.width_mm + 2.0 * settings.bleed_mm,
                h: a.height_mm + 2.0 * settings.bleed_mm,
            };
            assert!(bleed_a.x >= settings.margin_left_mm - 1e-7);
            assert!(bleed_a.y >= settings.margin_top_mm - 1e-7);
            assert!(bleed_a.right() <= settings.width_mm - settings.margin_right_mm + 1e-7);
            assert!(bleed_a.bottom() <= settings.height_mm - settings.margin_bottom_mm + 1e-7);
            for b in result.placements.iter().skip(i + 1).filter(|b| b.sheet_index == a.sheet_index) {
                let outer_b = Rect {
                    x: b.x_mm - settings.bleed_mm,
                    y: b.y_mm - settings.bleed_mm,
                    w: b.width_mm + 2.0 * settings.bleed_mm + settings.gutter_x_mm,
                    h: b.height_mm + 2.0 * settings.bleed_mm + settings.gutter_y_mm,
                };
                assert!(!outer_a.intersects(outer_b), "placements overlap: {a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn mixed_design_job_does_not_require_a_trailing_sheet_edge_gutter() {
        let settings = SheetSettings {
            width_mm: 100.0,
            height_mm: 80.0,
            margin_left_mm: 5.0,
            margin_right_mm: 5.0,
            margin_top_mm: 5.0,
            margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0,
            gutter_y_mm: 2.0,
            bleed_mm: 0.0,
        };
        // A 90 mm design exactly fills the usable width. The second,
        // different-size design has no copies, forcing the mixed-size path.
        let result = plan(
            &[
                DesignSpec { id: 1, width_mm: 90.0, height_mm: 40.0, copies: 1, allow_rotation: false },
                DesignSpec { id: 2, width_mm: 30.0, height_mm: 20.0, copies: 0, allow_rotation: false },
            ],
            settings,
        ).unwrap();
        assert_eq!(result.sheet_count, 1);
        assert_eq!(result.placements.len(), 1);
        let p = result.placements[0];
        assert!((p.x_mm - 5.0).abs() < 1e-7);
        assert!((p.x_mm + p.width_mm - 95.0).abs() < 1e-7);
    }

    #[test]
    fn rejects_non_finite_sheet_settings() {
        let result = plan(
            &[DesignSpec { id: 4, width_mm: 20.0, height_mm: 20.0, copies: 1, allow_rotation: false }],
            SheetSettings { width_mm: f64::NAN, ..SheetSettings::default() },
        );
        assert_eq!(result, Err(PlanError::InvalidSheet));
    }


    #[test]
    fn mixed_rotated_item_keeps_sheet_axis_gutters_with_asymmetric_spacing() {
        let settings = SheetSettings {
            width_mm: 50.0,
            height_mm: 90.0,
            margin_left_mm: 0.0,
            margin_right_mm: 0.0,
            margin_top_mm: 0.0,
            margin_bottom_mm: 0.0,
            gutter_x_mm: 3.0,
            gutter_y_mm: 7.0,
            bleed_mm: 1.0,
        };
        // The 70 x 40 artwork only fits when rotated. The gutter dimensions
        // remain tied to the sheet axes: 3 mm horizontally and 7 mm vertically.
        let result = plan(
            &[
                DesignSpec { id: 41, width_mm: 70.0, height_mm: 40.0, copies: 1, allow_rotation: true },
                DesignSpec { id: 42, width_mm: 10.0, height_mm: 10.0, copies: 0, allow_rotation: false },
            ],
            settings,
        ).unwrap();

        assert_eq!(result.placements.len(), 1);
        let p = result.placements[0];
        assert!(p.rotated);
        assert!((p.width_mm - 40.0).abs() < 1e-7);
        assert!((p.height_mm - 70.0).abs() < 1e-7);
        let bleed_bounds = Rect {
            x: p.x_mm - settings.bleed_mm,
            y: p.y_mm - settings.bleed_mm,
            w: p.width_mm + 2.0 * settings.bleed_mm,
            h: p.height_mm + 2.0 * settings.bleed_mm,
        };
        assert!(bleed_bounds.x >= -1e-7);
        assert!(bleed_bounds.y >= -1e-7);
        assert!(bleed_bounds.right() <= settings.width_mm + 1e-7);
        assert!(bleed_bounds.bottom() <= settings.height_mm + 1e-7);
    }


    #[test]
    fn rotated_mixed_designs_keep_horizontal_gutter_in_sheet_coordinates() {
        let settings = SheetSettings {
            width_mm: 55.0,
            height_mm: 90.0,
            margin_left_mm: 0.0,
            margin_right_mm: 0.0,
            margin_top_mm: 0.0,
            margin_bottom_mm: 0.0,
            gutter_x_mm: 3.0,
            gutter_y_mm: 7.0,
            bleed_mm: 1.0,
        };
        let result = plan(
            &[
                DesignSpec { id: 51, width_mm: 70.0, height_mm: 40.0, copies: 1, allow_rotation: true },
                DesignSpec { id: 52, width_mm: 5.0, height_mm: 5.0, copies: 1, allow_rotation: false },
            ],
            settings,
        ).unwrap();

        assert_eq!(result.placements.len(), 2);
        let rotated = result.placements.iter().find(|p| p.design_id == 51).unwrap();
        let neighbor = result.placements.iter().find(|p| p.design_id == 52).unwrap();
        assert!(rotated.rotated);
        assert_eq!(rotated.sheet_index, neighbor.sheet_index);
        let rotated_bleed_right = rotated.x_mm + rotated.width_mm + settings.bleed_mm;
        let neighbor_bleed_left = neighbor.x_mm - settings.bleed_mm;
        let horizontal_gap = if neighbor_bleed_left >= rotated_bleed_right {
            neighbor_bleed_left - rotated_bleed_right
        } else {
            rotated.x_mm - settings.bleed_mm
                - (neighbor.x_mm + neighbor.width_mm + settings.bleed_mm)
        };
        assert!((horizontal_gap - settings.gutter_x_mm).abs() < 1e-7);
    }

    #[test]
    fn bleed_is_kept_outside_trim_dimensions() {
        let result = plan(
            &[DesignSpec { id: 3, width_mm: 40.0, height_mm: 30.0, copies: 1, allow_rotation: false }],
            SheetSettings { bleed_mm: 2.0, ..SheetSettings::default() },
        ).unwrap();
        let p = result.placements[0];
        assert_eq!((p.width_mm, p.height_mm), (40.0, 30.0));
        assert_eq!((p.x_mm, p.y_mm), (7.0, 7.0));
    }

    fn crop_mark_test_design() -> DesignArtwork {
        let mut source = tracedraw_core::Document::new(
            "Source",
            tracedraw_core::Size::new(30.0, 20.0),
        );
        let shape = tracedraw_core::Shape::new(
            source.ids_mut().shape(),
            tracedraw_core::ShapeKind::Rect {
                rect: tracedraw_core::Rect::new(0.0, 0.0, 30.0, 20.0),
                radius: 0.0,
                corners: None,
            },
        );
        DesignArtwork { id: 1, shapes: vec![shape] }
    }

    #[test]
    fn document_builder_emits_eight_editable_crop_marks_for_isolated_object() {
        let sheet = SheetSettings {
            width_mm: 100.0, height_mm: 80.0,
            margin_left_mm: 5.0, margin_right_mm: 5.0,
            margin_top_mm: 5.0, margin_bottom_mm: 5.0,
            gutter_x_mm: 2.0, gutter_y_mm: 2.0, bleed_mm: 0.0,
        };
        let plan = ImpositionPlan {
            placements: vec![ObjectPlacement {
                design_id: 1, copy_index: 0, sheet_index: 0,
                x_mm: 20.0, y_mm: 20.0, width_mm: 30.0, height_mm: 20.0,
                rotated: false, shape_transform: None,
            }],
            sheet_count: 1, total_copies: 1, utilization_percent: 7.5,
        };
        let design = crop_mark_test_design();

        let with_marks = build_document("With marks", sheet, &plan, &[design.clone()], true).unwrap();
        let shapes = &with_marks.pages[0].layers[0].shapes;
        let marks: Vec<_> = shapes.iter().filter(|shape| shape.name.as_deref() == Some("Crop mark")).collect();
        assert_eq!(marks.len(), 8, "an isolated object should receive four corner pairs");
        assert!(marks.iter().all(|shape| {
            matches!(&shape.kind, tracedraw_core::ShapeKind::Path { closed: false, .. })
                && shape.fill == tracedraw_core::Fill::None
                && shape.stroke.as_ref().is_some_and(|stroke| (stroke.width - 0.2).abs() < 1e-7)
        }));

        let without_marks = build_document("Without marks", sheet, &plan, &[design], false).unwrap();
        assert!(without_marks.pages[0].layers[0].shapes.iter()
            .all(|shape| shape.name.as_deref() != Some("Crop mark")));
    }

    #[test]
    fn crop_marks_are_suppressed_when_they_enter_neighbor_bleed() {
        let sheet = SheetSettings {
            width_mm: 100.0, height_mm: 80.0,
            margin_left_mm: 5.0, margin_right_mm: 5.0,
            margin_top_mm: 5.0, margin_bottom_mm: 5.0,
            gutter_x_mm: 1.0, gutter_y_mm: 2.0, bleed_mm: 0.5,
        };
        let plan = ImpositionPlan {
            placements: vec![
                ObjectPlacement { design_id: 1, copy_index: 0, sheet_index: 0,
                    x_mm: 20.0, y_mm: 20.0, width_mm: 30.0, height_mm: 20.0, rotated: false, shape_transform: None },
                ObjectPlacement { design_id: 2, copy_index: 0, sheet_index: 0,
                    x_mm: 51.0, y_mm: 20.0, width_mm: 20.0, height_mm: 20.0, rotated: false, shape_transform: None },
            ],
            sheet_count: 1, total_copies: 2, utilization_percent: 15.0,
        };
        let designs = [crop_mark_test_design(), DesignArtwork {
            id: 2,
            shapes: crop_mark_test_design().shapes,
        }];
        let document = build_document("Neighbor marks", sheet, &plan, &designs, true).unwrap();
        let marks: Vec<_> = document.pages[0].layers[0].shapes.iter()
            .filter(|shape| shape.name.as_deref() == Some("Crop mark"))
            .collect();
        assert!(marks.len() < 16, "marks entering a neighboring bleed must be omitted");
        assert!(marks.len() > 0, "safe crop marks on the outer corners should remain");
    }
}
