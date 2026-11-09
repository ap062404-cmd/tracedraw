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
        (item.h, item.w, true)
    } else {
        (item.w, item.h, false)
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

/// Pack designs with a MaxRects best-short-side-fit heuristic.
///
/// Each selected design is treated as one production unit, even when it
/// consists of multiple vector objects. Copies are placed across as few
/// sheets as the heuristic can find; source artwork is not modified.
pub fn plan(designs: &[DesignSpec], settings: SheetSettings) -> Result<ImpositionPlan, PlanError> {
    let values = [
        settings.width_mm, settings.height_mm,
        settings.margin_left_mm, settings.margin_right_mm,
        settings.margin_top_mm, settings.margin_bottom_mm,
        settings.gutter_x_mm, settings.gutter_y_mm, settings.bleed_mm,
    ];
    if values.iter().any(|v| !v.is_finite()) || settings.width_mm <= 0.0 || settings.height_mm <= 0.0 {
        return Err(PlanError::InvalidSheet);
    }
    if values[2..].iter().any(|v| *v < 0.0) {
        return Err(PlanError::InvalidMargins);
    }
    let usable = Rect {
        x: settings.margin_left_mm,
        y: settings.margin_top_mm,
        w: settings.width_mm - settings.margin_left_mm - settings.margin_right_mm,
        h: settings.height_mm - settings.margin_top_mm - settings.margin_bottom_mm,
    };
    if usable.w <= 0.0 || usable.h <= 0.0 { return Err(PlanError::InvalidMargins); }

    let bleed = settings.bleed_mm;
    let mut items = Vec::new();
    for d in designs {
        if !d.width_mm.is_finite() || !d.height_mm.is_finite()
            || d.width_mm <= 0.0 || d.height_mm <= 0.0 {
            return Err(PlanError::InvalidDesign { id: d.id });
        }
        for copy in 0..d.copies {
            items.push(Instance {
                id: d.id, copy, w: d.width_mm + 2.0 * bleed,
                h: d.height_mm + 2.0 * bleed, rotate: d.allow_rotation,
            });
        }
    }
    let total = items.len();
    items.sort_by(|a, b| {
        (b.w * b.h).total_cmp(&(a.w * a.h))
            .then_with(|| b.w.max(b.h).total_cmp(&a.w.max(a.h)))
    });

    let mut sheets: Vec<Vec<Rect>> = Vec::new();
    let mut placements = Vec::with_capacity(total);
    let mut used_area = 0.0;
    for item in items {
        let mut best: Option<(usize, Candidate)> = None;
        for (sheet_index, free) in sheets.iter().enumerate() {
            for (free_index, rect) in free.iter().copied().enumerate() {
                for rotate in [false, true] {
                    let Some(c) = candidate(rect, item, rotate, free_index) else { continue; };
                    let score = (c.short_side, c.long_side);
                    let replace = best.as_ref().is_none_or(|(_, old)| score < (old.short_side, old.long_side));
                    if replace { best = Some((sheet_index, c)); }
                }
            }
        }
        let (sheet_index, c) = if let Some(best) = best {
            best
        } else {
            let Some(c) = candidate(usable, item, false, 0)
                .or_else(|| candidate(usable, item, true, 0)) else {
                    return Err(PlanError::DoesNotFit {
                        id: item.id,
                        width_mm: item.w - 2.0 * bleed,
                        height_mm: item.h - 2.0 * bleed,
                    });
                };
            sheets.push(vec![usable]);
            (sheets.len() - 1, c)
        };
        let free = sheets[sheet_index][c.free_index];
        let outer = Rect { x: free.x, y: free.y, w: c.w, h: c.h };
        sheets[sheet_index] = split_free_rects(&sheets[sheet_index], outer);
        placements.push(ObjectPlacement {
            design_id: item.id,
            copy_index: item.copy,
            sheet_index,
            x_mm: outer.x + bleed,
            y_mm: outer.y + bleed,
            width_mm: c.w - 2.0 * bleed,
            height_mm: c.h - 2.0 * bleed,
            rotated: c.rotated,
        });
        used_area += c.w * c.h;
    }
    let sheet_area = usable.w * usable.h * sheets.len() as f64;
    Ok(ImpositionPlan {
        placements,
        sheet_count: sheets.len(),
        total_copies: total,
        utilization_percent: if sheet_area > 0.0 { used_area / sheet_area * 100.0 } else { 0.0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
