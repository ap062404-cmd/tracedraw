//! TraceCut's Nesting and Measure tools: shape nesting and rectangle
//! nesting on sheets or a roll, and the perimeter and area, cutting time
//! and cost of the selection. The geometry is `tracedraw_tracecut::nest`,
//! `measure` and `calc`; this module keeps the tools' settings, works
//! their results out and turns nesting into commands. See
//! docs/behavior/tracecut.md.
//!
//! Adapted from the Sign & Cut nesting and calculation tools of #170 by
//! LaMka21.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::tracecut_cut::{fmt_len, AddStyle, Change, Changes};
use serde::{Deserialize, Serialize};
use tracedraw_core::geometry::{BezPath, Point, Rect, Shape as _};
use tracedraw_core::Command;
use tracedraw_tracecut::calc::cost::{
    self, Material, MaterialUnit, Quantities, Service, ServiceUnit,
};
use tracedraw_tracecut::calc::time::{self, TimeProfile, TravelOrder};
use tracedraw_tracecut::measure;
use tracedraw_tracecut::nest::rects::{self, RectItem, RectSettings};
use tracedraw_tracecut::nest::shapes::{self, Rotations, ShapeSettings};
use tracedraw_tracecut::nest::{Layout, SheetSpec};

/// Shape nesting: the material, the turns, holes, accuracy and how long
/// the search may take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeNestSettings {
    pub sheet: SheetSpec,
    pub rotations: Rotations,
    /// Smaller objects may go into the holes of larger ones.
    pub holes: bool,
    /// Accuracy of the outlines (mm).
    pub tolerance: f64,
    /// Rounds of the improvement loop.
    pub rounds: usize,
    /// The improvement loop stops after this many seconds.
    pub seconds: f64,
    /// Draw the sheets (or the roll's length used) as rectangles.
    pub draw_sheets: bool,
}

impl Default for ShapeNestSettings {
    fn default() -> Self {
        ShapeNestSettings {
            sheet: SheetSpec::default(),
            rotations: Rotations::Quarter,
            holes: true,
            tolerance: 0.5,
            rounds: 30,
            seconds: 2.0,
            draw_sheets: true,
        }
    }
}

/// Rectangle nesting: the material and whether boxes may turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RectNestSettings {
    pub sheet: SheetSpec,
    pub rotate: bool,
    pub draw_sheets: bool,
}

impl Default for RectNestSettings {
    fn default() -> Self {
        RectNestSettings {
            sheet: SheetSpec::default(),
            rotate: true,
            draw_sheets: true,
        }
    }
}

/// Cutting time: the machine's speeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeSettings {
    /// Speed while cutting (mm/s).
    pub cut_speed: f64,
    /// Speed of the moves with the tool up (mm/s).
    pub travel_speed: f64,
    /// Lift and lower per outline (s).
    pub lift_time: f64,
    /// Cutting and travel times are multiplied by this.
    pub accel_factor: f64,
    pub passes: u32,
    /// The nearest outline next (else the stacking order).
    pub nearest: bool,
}

impl Default for TimeSettings {
    fn default() -> Self {
        let p = TimeProfile::default();
        TimeSettings {
            cut_speed: p.cut_speed,
            travel_speed: p.travel_speed,
            lift_time: p.lift_time,
            accel_factor: p.accel_factor,
            passes: p.passes,
            nearest: true,
        }
    }
}

impl TimeSettings {
    pub fn profile(&self) -> TimeProfile {
        TimeProfile {
            name: String::new(),
            cut_speed: self.cut_speed,
            travel_speed: self.travel_speed,
            lift_time: self.lift_time,
            accel_factor: self.accel_factor,
            passes: self.passes,
        }
        .sanitized()
    }
}

/// Cost: the material and the prices billed on the job. Prices carry no
/// currency; `currency` is shown after the amounts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CostSettings {
    /// The material is sold by the running metre of a roll (else by the
    /// square metre).
    pub by_roll: bool,
    /// Price per square metre, or per running metre of roll.
    pub material_price: f64,
    /// Width of the roll (mm).
    pub roll_width: f64,
    /// Per metre of outline cut.
    pub cut_price: f64,
    /// Per square metre weeded.
    pub weed_price: f64,
    /// Per object.
    pub piece_price: f64,
    /// Per hour of cutting time (from the Cutting Time settings).
    pub hour_price: f64,
    /// Added on every side of each object's box (mm).
    pub reserve: f64,
    pub copies: u32,
    pub currency: String,
}

impl Default for CostSettings {
    fn default() -> Self {
        CostSettings {
            by_roll: true,
            material_price: 6.0,
            roll_width: 630.0,
            cut_price: 0.5,
            weed_price: 12.0,
            piece_price: 0.0,
            hour_price: 0.0,
            reserve: 5.0,
            copies: 1,
            currency: String::new(),
        }
    }
}

/// A result to read: lines for the window, the first one for the status
/// line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub lines: Vec<String>,
}

/// An area in the document's units, squared.
fn fmt_area(u: crate::app::Units, mm2: f64) -> String {
    let k = u.world_from_mm(1.0);
    format!("{:.2} {}²", mm2 * k * k, u.short())
}

/// A price with the currency the user set.
fn fmt_money(v: f64, currency: &str) -> String {
    let c = currency.trim();
    if c.is_empty() {
        format!("{v:.2}")
    } else {
        format!("{v:.2} {c}")
    }
}

/// Seconds as h:mm:ss.
fn fmt_time(seconds: f64) -> String {
    let (h, m, s) = time::hms(seconds);
    format!("{h}:{m:02}:{s:02}")
}

impl App {
    /// The outlines of the selection's objects, by object.
    fn tracecut_parts(&self) -> (Vec<tracedraw_core::ShapeId>, Vec<Vec<BezPath>>) {
        let ids: Vec<_> = self
            .tracecut_targets()
            .into_iter()
            .filter(|id| self.doc().find_shape(*id).is_some_and(|s| !s.locked))
            .collect();
        let outlines = self.tracecut_outlines(&ids);
        (ids, outlines)
    }

    /// Turn a nesting layout into changes: each placed object moved, the
    /// sheets drawn when asked.
    fn tracecut_layout_changes(
        &self,
        label: &'static str,
        ids: &[tracedraw_core::ShapeId],
        outlines: &[Vec<BezPath>],
        layout: &Layout,
        sheet: &SheetSpec,
        draw_sheets: bool,
    ) -> Changes {
        let mut out = Changes {
            label,
            hide_objects: true,
            ..Default::default()
        };
        for ((id, paths), p) in ids.iter().zip(outlines).zip(&layout.placements) {
            match p {
                Some(p) => {
                    out.preview
                        .extend(paths.iter().map(|path| p.transform * path.clone()));
                    out.changes
                        .push(Change::Cmd(Box::new(Command::TransformShapes {
                            shapes: vec![*id],
                            transform: p.transform,
                        })));
                }
                None => out.preview.extend(paths.iter().cloned()),
            }
        }
        let sheets: Vec<BezPath> = if sheet.roll {
            (layout.sheets > 0)
                .then(|| sheet.sheet_rect(0, layout.length).to_path(0.01))
                .into_iter()
                .collect()
        } else {
            (0..layout.sheets)
                .map(|i| sheet.sheet_rect(i, 0.0).to_path(0.01))
                .collect()
        };
        out.preview.extend(sheets.iter().cloned());
        if draw_sheets && !sheets.is_empty() {
            out.changes.push(Change::AddGroup {
                name: "tracecut.sheets_group",
                paths: sheets,
                style: AddStyle::Hairline,
            });
        }
        let u = self.units;
        let left = ids.len().saturating_sub(layout.placed());
        let mut summary = if sheet.roll {
            trf(
                "tracecut.nest_roll",
                &[
                    ("n", &layout.placed().to_string()),
                    ("length", &fmt_len(u, layout.length)),
                    ("used", &format!("{:.0}", layout.utilisation() * 100.0)),
                ],
            )
        } else {
            trf(
                "tracecut.nest_sheets",
                &[
                    ("n", &layout.placed().to_string()),
                    ("sheets", &layout.sheets.to_string()),
                    ("used", &format!("{:.0}", layout.utilisation() * 100.0)),
                ],
            )
        };
        if left > 0 {
            summary.push(' ');
            summary.push_str(&trf("tracecut.nest_left_out", &[("n", &left.to_string())]));
        }
        out.summary = summary;
        out
    }

    /// Shape nesting of the selection.
    pub fn tracecut_shape_nest(&self, s: &ShapeNestSettings) -> Changes {
        let (ids, outlines) = self.tracecut_parts();
        let settings = ShapeSettings {
            sheet: s.sheet,
            rotations: s.rotations,
            holes: s.holes,
            tolerance: s.tolerance,
            rounds: s.rounds.min(1000),
            seed: 1,
        };
        let limit = if s.seconds.is_finite() {
            s.seconds.clamp(0.0, 60.0)
        } else {
            2.0
        };
        let start = web_time::Instant::now();
        let mut out_of_time = || start.elapsed().as_secs_f64() > limit;
        let layout = shapes::nest_shapes(&outlines, &settings, &mut out_of_time);
        self.tracecut_layout_changes(
            "Shape Nesting",
            &ids,
            &outlines,
            &layout,
            &s.sheet,
            s.draw_sheets,
        )
    }

    /// Rectangle nesting of the selection, by the objects' boxes.
    pub fn tracecut_rect_nest(&self, s: &RectNestSettings) -> Changes {
        let (ids, outlines) = self.tracecut_parts();
        let items: Vec<RectItem> = outlines
            .iter()
            .map(|paths| {
                let m = measure::measure_object(paths);
                RectItem {
                    bounds: m.bbox.unwrap_or(Rect::ZERO),
                    area: m.area,
                }
            })
            .collect();
        let layout = rects::nest_rects(
            &items,
            &RectSettings {
                sheet: s.sheet,
                rotate: s.rotate,
            },
        );
        self.tracecut_layout_changes(
            "Rectangle Nesting",
            &ids,
            &outlines,
            &layout,
            &s.sheet,
            s.draw_sheets,
        )
    }

    /// Perimeter and area of the selection.
    pub fn tracecut_measure(&self) -> Report {
        let (_, outlines) = self.tracecut_parts();
        let m = measure::measure(&outlines);
        let u = self.units;
        Report {
            lines: vec![
                trf("tracecut.measure_length", &[("v", &fmt_len(u, m.length))]),
                trf("tracecut.measure_area", &[("v", &fmt_area(u, m.area))]),
                trf(
                    "tracecut.measure_holes",
                    &[
                        ("outer", &fmt_area(u, m.outer_area)),
                        ("holes", &fmt_area(u, m.hole_area)),
                    ],
                ),
                trf(
                    "tracecut.measure_box",
                    &[
                        ("w", &fmt_len(u, m.bbox.map_or(0.0, |b| b.width()))),
                        ("h", &fmt_len(u, m.bbox.map_or(0.0, |b| b.height()))),
                        ("a", &fmt_area(u, m.bbox_area)),
                    ],
                ),
                trf(
                    "tracecut.measure_counts",
                    &[
                        ("objects", &m.objects.to_string()),
                        ("closed", &m.closed_paths.to_string()),
                        ("open", &m.open_paths.to_string()),
                        ("nodes", &m.nodes.to_string()),
                    ],
                ),
            ],
        }
    }

    /// The cutting time of the selection, and its parts.
    pub fn tracecut_time(&self, s: &TimeSettings) -> (Report, time::TimeEstimate) {
        let (_, outlines) = self.tracecut_parts();
        let order = if s.nearest {
            TravelOrder::Nearest
        } else {
            TravelOrder::AsDrawn
        };
        let job = tracedraw_tracecut::calc::job(&outlines, Point::ZERO, order);
        let est = time::estimate(job.measurement.length, &job.travel, &s.profile());
        let u = self.units;
        let report = Report {
            lines: vec![
                trf("tracecut.time_total", &[("t", &fmt_time(est.total_s))]),
                trf(
                    "tracecut.time_parts",
                    &[
                        ("cut", &fmt_time(est.cut_s)),
                        ("travel", &fmt_time(est.travel_s)),
                        ("lifts", &fmt_time(est.lift_s)),
                    ],
                ),
                trf(
                    "tracecut.time_lengths",
                    &[
                        ("cut", &fmt_len(u, est.cut_length)),
                        ("travel", &fmt_len(u, est.travel.travel_length)),
                        ("n", &est.travel.outlines.to_string()),
                    ],
                ),
            ],
        };
        (report, est)
    }

    /// The cost of the selection: the material, cutting, weeding, pieces
    /// and machine time from the Cutting Time settings.
    pub fn tracecut_cost(&self, s: &CostSettings, t: &TimeSettings) -> Report {
        let (_, outlines) = self.tracecut_parts();
        let job = tracedraw_tracecut::calc::job(&outlines, Point::ZERO, TravelOrder::Nearest);
        let (_, est) = self.tracecut_time(t);
        let q = Quantities {
            blocks: job.blocks.clone(),
            cut_length: job.measurement.length,
            hours: est.total_s / 3600.0,
        };
        let material = Material {
            name: tr("tracecut.cost_material"),
            unit: if s.by_roll {
                MaterialUnit::Roll
            } else {
                MaterialUnit::Area
            },
            price: s.material_price,
            roll_width: s.roll_width,
        };
        let service = |key: &str, unit, price: f64| Service {
            name: tr(key),
            unit,
            price,
            selected: price > 0.0,
        };
        let services = [
            service("tracecut.cost_cutting", ServiceUnit::Metre, s.cut_price),
            service(
                "tracecut.cost_weeding",
                ServiceUnit::SquareMetre,
                s.weed_price,
            ),
            service("tracecut.cost_pieces", ServiceUnit::Piece, s.piece_price),
            service("tracecut.cost_hours", ServiceUnit::Hour, s.hour_price),
        ];
        let spec = cost::CostSpec {
            reserve: s.reserve,
            copies: s.copies.max(1),
            ..Default::default()
        };
        let r = cost::cost(&q, Some(&material), &services, &spec);
        let unit = |u: cost::QtyUnit| match u {
            cost::QtyUnit::SquareMetre => "m²",
            cost::QtyUnit::Metre => "m",
            cost::QtyUnit::Piece => "×",
            cost::QtyUnit::Hour => "h",
        };
        let mut lines = vec![trf(
            "tracecut.cost_total",
            &[("v", &fmt_money(r.total, &s.currency))],
        )];
        for l in &r.lines {
            lines.push(format!(
                "{}: {:.2} {} × {} = {}",
                l.name,
                l.quantity,
                unit(l.unit),
                fmt_money(l.unit_price, &s.currency),
                fmt_money(l.amount, &s.currency)
            ));
        }
        Report { lines }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::ShapeKind;

    fn rect(app: &mut App, x: f64, y: f64, w: f64, h: f64) -> tracedraw_core::ShapeId {
        app.new_shape(ShapeKind::Rect {
            rect: Rect::new(x, y, x + w, y + h),
            radius: 0.0,
            corners: None,
        })
        .expect("rect")
    }

    fn small_sheet() -> SheetSpec {
        SheetSpec {
            width: 100.0,
            height: 100.0,
            spacing: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn rectangle_nesting_moves_the_objects_onto_sheets_in_one_step() {
        let mut app = App::headless();
        let ids: Vec<_> = (0..4)
            .map(|i| rect(&mut app, 300.0 + 70.0 * i as f64, 300.0, 40.0, 40.0))
            .collect();
        app.select(ids.clone());
        let s = RectNestSettings {
            sheet: small_sheet(),
            ..Default::default()
        };
        let c = app.tracecut_rect_nest(&s);
        let before = app.engine.history_labels().0.len();
        assert!(app.tracecut_apply_changes(&c));
        assert_eq!(app.engine.history_labels().0.len(), before + 1);
        for id in &ids {
            let b = app
                .doc()
                .shape_on_page(*id)
                .expect("shape")
                .page_path()
                .bounding_box();
            assert!(b.x0 >= -1e-6 && b.x1 <= 100.0 + 1e-6, "{b:?}");
            assert!(b.y0 >= -1e-6 && b.y1 <= 100.0 + 1e-6, "{b:?}");
        }
        // The sheet is drawn as a group.
        assert!(app.selection.len() == 1);
    }

    #[test]
    fn shape_nesting_places_the_objects_without_overlap() {
        let mut app = App::headless();
        let a = rect(&mut app, 500.0, 500.0, 30.0, 30.0);
        let b = rect(&mut app, 600.0, 500.0, 30.0, 30.0);
        app.select(vec![a, b]);
        let s = ShapeNestSettings {
            sheet: small_sheet(),
            rounds: 2,
            draw_sheets: false,
            ..Default::default()
        };
        let c = app.tracecut_shape_nest(&s);
        assert!(c.summary.contains('2'), "{}", c.summary);
        assert!(app.tracecut_apply_changes(&c));
        let ba = app
            .doc()
            .shape_on_page(a)
            .expect("a")
            .page_path()
            .bounding_box();
        let bb = app
            .doc()
            .shape_on_page(b)
            .expect("b")
            .page_path()
            .bounding_box();
        assert!(ba.x1 <= 100.0 + 1e-6 && bb.x1 <= 100.0 + 1e-6);
        let overlap = ba.intersect(bb);
        assert!(overlap.width() <= 1e-6 || overlap.height() <= 1e-6);
    }

    /// #2227: an object with an envelope is nested by what it draws. A
    /// 30 mm square whose envelope draws it 60 mm wide and high used to
    /// be placed as the 30 mm square it was before the envelope, so its
    /// neighbour landed on top of it.
    #[test]
    fn an_enveloped_object_is_nested_by_what_it_draws() {
        use tracedraw_core::envelope::Frame;
        use tracedraw_core::geometry::Affine;
        for shape_nest in [true, false] {
            let mut app = App::headless();
            let a = rect(&mut app, 500.0, 500.0, 30.0, 30.0);
            let b = rect(&mut app, 600.0, 500.0, 30.0, 30.0);
            app.select(vec![a]);
            app.apply_envelope_frame(|r| {
                Frame::around(r).transformed(
                    Affine::translate((r.x0, r.y0))
                        * Affine::scale(2.0)
                        * Affine::translate((-r.x0, -r.y0)),
                )
            });
            let drawn = |app: &App, id| app.doc().shape_on_page(id).expect("shape").visual_bounds();
            assert!((drawn(&app, a).width() - 60.0).abs() < 0.01);
            app.select(vec![a, b]);
            let c = if shape_nest {
                app.tracecut_shape_nest(&ShapeNestSettings {
                    sheet: small_sheet(),
                    rounds: 2,
                    draw_sheets: false,
                    ..Default::default()
                })
            } else {
                app.tracecut_rect_nest(&RectNestSettings {
                    sheet: small_sheet(),
                    draw_sheets: false,
                    ..Default::default()
                })
            };
            assert!(app.tracecut_apply_changes(&c));
            let (ba, bb) = (drawn(&app, a), drawn(&app, b));
            for r in [ba, bb] {
                assert!(r.x0 >= -0.01 && r.y0 >= -0.01, "{r:?}");
                assert!(r.x1 <= 100.01 && r.y1 <= 100.01, "{r:?}");
            }
            let overlap = ba.intersect(bb);
            assert!(
                overlap.width() <= 1e-6 || overlap.height() <= 1e-6,
                "{ba:?} and {bb:?} overlap (shape nesting: {shape_nest})"
            );
        }
    }

    /// The live test of 0.41: Shape Nesting at 1 mm accuracy left objects
    /// crossing each other and past the sheet's edge. Ellipses, stars,
    /// round-cornered rectangles, an L-shaped curve, some turned or
    /// scaled, nested from the window's tool and applied: in wireframe
    /// no two outlines cross or come closer than the spacing, every
    /// moved object lies on its sheet, and one too large stays where it
    /// was and is reported.
    #[test]
    fn nested_objects_never_cross_and_stay_on_the_sheet() {
        use tracedraw_core::geometry::{Affine, Shape as _};
        let mut app = App::headless();
        let mut ids = Vec::new();
        for i in 0..14 {
            let x = 400.0 + 60.0 * (i % 5) as f64;
            let y = 400.0 + 60.0 * (i / 5) as f64;
            let k = (i % 3) as f64;
            let r = Rect::new(x, y, x + 30.0 + 8.0 * k, y + 18.0 + 6.0 * k);
            let kind = match i % 4 {
                0 => ShapeKind::Ellipse { rect: r, arc: None },
                1 => ShapeKind::Polygon {
                    rect: r,
                    points: 5 + i as u32 % 3,
                    sharpness: 0.5,
                    complex: None,
                    twist: 0.0,
                },
                2 => ShapeKind::Rect {
                    rect: r,
                    radius: 6.0,
                    corners: None,
                },
                _ => {
                    let mut p = BezPath::new();
                    p.move_to((x, y));
                    p.line_to((x + 40.0, y));
                    p.curve_to(
                        (x + 40.0, y + 10.0),
                        (x + 20.0, y + 6.0),
                        (x + 12.0, y + 12.0),
                    );
                    p.line_to((x + 12.0, y + 30.0));
                    p.line_to((x, y + 30.0));
                    p.close_path();
                    ShapeKind::Path {
                        path: p,
                        closed: true,
                    }
                }
            };
            let id = app.new_shape(kind).expect("shape");
            if i % 3 == 1 {
                let c = r.center().to_vec2();
                app.select(vec![id]);
                app.transform_selection(
                    Affine::translate(c)
                        * Affine::rotate(0.5)
                        * Affine::scale(1.2)
                        * Affine::translate(-c),
                );
            }
            ids.push(id);
        }
        let big = rect(&mut app, 900.0, 900.0, 300.0, 300.0);
        let mut all = ids.clone();
        all.push(big);
        app.select(all);
        let before_big = app.doc().shape_on_page(big).expect("big").page_path();
        let mut settings = crate::tracecut::ToolSettings::default();
        settings.shape_nest = ShapeNestSettings {
            sheet: SheetSpec {
                width: 150.0,
                height: 120.0,
                margin: 3.0,
                spacing: 2.0,
                ..Default::default()
            },
            rotations: Rotations::Eighth,
            tolerance: 1.0,
            rounds: 5,
            draw_sheets: false,
            ..Default::default()
        };
        let w = app.tracecut_work_out(crate::tracecut::ToolId::ShapeNesting, &settings);
        assert!(app.tracecut_commit(&w, &settings, None));
        assert!(app
            .status
            .contains(&trf("tracecut.nest_left_out", &[("n", "1")])));
        assert_eq!(
            app.doc().shape_on_page(big).expect("big").page_path(),
            before_big,
            "too large: left where it was"
        );
        let spec = settings.shape_nest.sheet;
        let placed: Vec<BezPath> = ids
            .iter()
            .map(|id| app.doc().shape_on_page(*id).expect("shape").page_path())
            .collect();
        let sheet_of = |p: &BezPath| {
            let b = p.bounding_box();
            (0..20).find(|i| {
                let s = spec.sheet_rect(*i, 0.0).inset(-3.0);
                b.x0 >= s.x0 - 1e-6
                    && b.y0 >= s.y0 - 1e-6
                    && b.x1 <= s.x1 + 1e-6
                    && b.y1 <= s.y1 + 1e-6
            })
        };
        for p in &placed {
            assert!(
                sheet_of(p).is_some(),
                "{:?} is off every sheet",
                p.bounding_box()
            );
        }
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                let shared =
                    tracedraw_core::shaping::overlay(a, b, tracedraw_core::shaping::Op::Intersect);
                assert!(
                    measure::measure_object(&[shared]).area < 1e-6,
                    "two objects cross"
                );
                // Grown by just under half the spacing, they still keep
                // apart.
                let ga = tracedraw_core::shaping::offset(a, 0.98);
                let gb = tracedraw_core::shaping::offset(b, 0.98);
                let shared = tracedraw_core::shaping::overlay(
                    &ga,
                    &gb,
                    tracedraw_core::shaping::Op::Intersect,
                );
                assert!(
                    measure::measure_object(&[shared]).area < 1e-6,
                    "two objects closer than the spacing"
                );
            }
        }
    }

    #[test]
    fn an_object_too_big_for_the_sheet_is_left_where_it_is() {
        let mut app = App::headless();
        let big = rect(&mut app, 0.0, 0.0, 300.0, 300.0);
        app.select(vec![big]);
        let c = app.tracecut_rect_nest(&RectNestSettings {
            sheet: small_sheet(),
            ..Default::default()
        });
        assert!(c
            .summary
            .contains(&trf("tracecut.nest_left_out", &[("n", "1")])));
        assert!(!c.changes.iter().any(
            |ch| matches!(ch, Change::Cmd(cmd) if matches!(**cmd, Command::TransformShapes { .. }))
        ));
    }

    #[test]
    fn perimeter_area_time_and_cost() {
        let mut app = App::headless();
        app.units = crate::app::Units::Millimeters;
        let a = rect(&mut app, 0.0, 0.0, 100.0, 50.0);
        app.select(vec![a]);
        let m = app.tracecut_measure();
        assert!(m.lines[0].contains("300.0"), "{:?}", m.lines);
        assert!(m.lines[1].contains("5000.00"), "{:?}", m.lines);
        let t = TimeSettings {
            cut_speed: 100.0,
            travel_speed: 100.0,
            lift_time: 0.0,
            accel_factor: 1.0,
            passes: 1,
            nearest: true,
        };
        let (_, est) = app.tracecut_time(&t);
        // 300 mm at 100 mm/s, plus the move to the start.
        assert!((est.cut_s - 3.0).abs() < 1e-9);
        let cost = app.tracecut_cost(
            &CostSettings {
                by_roll: false,
                material_price: 10.0,
                cut_price: 1.0,
                weed_price: 0.0,
                reserve: 0.0,
                currency: "EUR".into(),
                ..Default::default()
            },
            &t,
        );
        // 0.005 m2 x 10 + 0.3 m x 1 = 0.35
        assert!(cost.lines[0].contains("0.35 EUR"), "{:?}", cost.lines);
        assert_eq!(cost.lines.len(), 3);
    }
}
