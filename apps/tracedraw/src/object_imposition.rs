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

/// Length available for a crop mark on one side of a gutter.
///
/// The bleed allowance is already accounted for by the mark's offset from
/// the trim edge, so it must not be subtracted from the gutter a second time.
fn crop_mark_length(gutter_mm: f64) -> f64 {
    (gutter_mm * 0.5 - 0.25).clamp(0.0, 3.0)
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
    use tracedraw_core::{Affine, Document, Layer, Page, Point, Shape, Size};

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
        let angle = if placement.rotated { std::f64::consts::FRAC_PI_2 } else { 0.0 };
        let rotate_about_center = Affine::translate((source_center.x, source_center.y))
            * Affine::rotate(angle)
            * Affine::translate((-source_center.x, -source_center.y));
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
            // Crop marks sit outside the reserved bleed allowance, not
            // inside the bleed artwork itself.
            // The trim-to-trim gutter includes bleed on both sides.
            // Keep 0.25 mm clear beyond each bleed edge; the remaining
            // half-gutter is available for the mark. A 2 mm gutter therefore
            // gives 0.75 mm marks, regardless of bleed width.
            let gap = sheet.bleed_mm + 0.25_f64;
            let length_x = crop_mark_length(sheet.gutter_x_mm);
            let length_y = crop_mark_length(sheet.gutter_y_mm);
            let mut add_mark = |points: [Point; 2]| {
                // Keep marks on the sheet and out of neighboring trim areas.
                let min_x = points[0].x.min(points[1].x);
                let max_x = points[0].x.max(points[1].x);
                let min_y = points[0].y.min(points[1].y);
                let max_y = points[0].y.max(points[1].y);
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
                mark.stroke = Some(tracedraw_core::Stroke::default());
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
                x_mm: pos.0, y_mm: pos.1, width_mm: pw, height_mm: ph, rotated: pos.2,
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
    let mut sheets: Vec<Vec<Rect>> = Vec::new();
    let mut placements = Vec::with_capacity(total);
    let mut artwork_area = 0.0;
    for item in items {
        let mut best: Option<(usize,Candidate)> = None;
        for (si, free) in sheets.iter().enumerate() {
            for (fi, rect) in free.iter().copied().enumerate() {
                for rotate in [false,true] {
                    if let Some(c) = candidate(rect,item,rotate,fi) {
                        let score=(c.short_side,c.long_side);
                        if best.as_ref().is_none_or(|(_,old)| score < (old.short_side,old.long_side)) { best=Some((si,c)); }
                    }
                }
            }
        }
        let (si,c)=if let Some(v)=best { v } else {
            let Some(c)=candidate(usable,item,false,0).or_else(|| candidate(usable,item,true,0)) else {
                return Err(PlanError::DoesNotFit { id:item.id,width_mm:item.w-2.0*bleed,height_mm:item.h-2.0*bleed });
            };
            sheets.push(vec![usable]); (sheets.len()-1,c)
        };
        let free=sheets[si][c.free_index];
        let outer=Rect{x:free.x,y:free.y,w:c.w,h:c.h};
        sheets[si]=split_free_rects(&sheets[si],outer);
        placements.push(ObjectPlacement { design_id:item.id,copy_index:item.copy,sheet_index:si,
            x_mm:outer.x+bleed,y_mm:outer.y+bleed,
            width_mm:c.w-settings.gutter_x_mm-2.0*bleed,
            height_mm:c.h-settings.gutter_y_mm-2.0*bleed,rotated:c.rotated });
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
    fn crop_mark_length_uses_half_gutter_after_clearance() {
        assert!((crop_mark_length(2.0) - 0.75).abs() < 1e-9);
        assert!((crop_mark_length(4.0) - 1.75).abs() < 1e-9);
        assert_eq!(crop_mark_length(0.0), 0.0);
        assert_eq!(crop_mark_length(0.4), 0.0);
        assert_eq!(crop_mark_length(10.0), 3.0);
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
            assert!(outer_a.x >= settings.margin_left_mm - 1e-7);
            assert!(outer_a.y >= settings.margin_top_mm - 1e-7);
            assert!(outer_a.right() <= settings.width_mm - settings.margin_right_mm + 1e-7);
            assert!(outer_a.bottom() <= settings.height_mm - settings.margin_bottom_mm + 1e-7);
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
    fn rejects_non_finite_sheet_settings() {
        let result = plan(
            &[DesignSpec { id: 4, width_mm: 20.0, height_mm: 20.0, copies: 1, allow_rotation: false }],
            SheetSettings { width_mm: f64::NAN, ..SheetSettings::default() },
        );
        assert_eq!(result, Err(PlanError::InvalidSheet));
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
}
