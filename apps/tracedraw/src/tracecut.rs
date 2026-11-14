//! TraceCut: the tools for cutting plotters, in a window of their own
//! (Launch > TraceCut, Tools > TraceCut). The geometry lives in the
//! `tracedraw-tracecut` crate; this module lists the tools, keeps their
//! settings and presets, works out a tool's result for the preview and
//! turns it into commands, one undo step for OK. The window itself is
//! `ui/tracecut.rs`. See docs/behavior/tracecut.md.
//!
//! Adapted from the Sign & Cut tools of #170 by LaMka21.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::tracecut_cut::{
    BridgeSettings, Changes, CutContourSettings, Job, MarksSettings, OrderSettings, PlotterSettings,
};
use crate::tracecut_nest::{
    CostSettings, RectNestSettings, Report, ShapeNestSettings, TimeSettings,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracedraw_core::geometry::{BezPath, Point, Rect, Shape as _};
use tracedraw_core::{Command, ShapeId, ShapeKind};
use tracedraw_tracecut::boxes::BoxSpec;
use tracedraw_tracecut::cnc::{CncSpec, Program};
use tracedraw_tracecut::cut::weed::WeedSpec;
use tracedraw_tracecut::eyelets::EyeletSpec;
use tracedraw_tracecut::tray::TraySpec;
use tracedraw_tracecut::{cleanup, crossings};

/// The groups of the tool list, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Cut,
    Nesting,
    Measure,
    /// New objects for sign and sheet work: eyelets, boxes, folded trays.
    Make,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Cut, Group::Nesting, Group::Measure, Group::Make];

    /// The i18n key of the group's name.
    pub fn key(self) -> &'static str {
        match self {
            Group::Cut => "tracecut.group_cut",
            Group::Nesting => "tracecut.group_nesting",
            Group::Measure => "tracecut.group_measure",
            Group::Make => "tracecut.group_make",
        }
    }

    /// The tools of the group, in the order they are listed.
    pub fn tools(self) -> Vec<ToolId> {
        ToolId::ALL
            .iter()
            .copied()
            .filter(|t| t.group() == self)
            .collect()
    }
}

/// A TraceCut tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ToolId {
    #[default]
    PlotterOutput,
    CncToolpath,
    Weeding,
    Bridges,
    CutOrder,
    CutContour,
    RegistrationMarks,
    FindDuplicates,
    FindCrossings,
    ShapeNesting,
    RectNesting,
    Measure,
    CuttingTime,
    Cost,
    Eyelets,
    BoxMaker,
    FoldedTray,
}

impl ToolId {
    pub const ALL: [ToolId; 17] = [
        ToolId::PlotterOutput,
        ToolId::CncToolpath,
        ToolId::Weeding,
        ToolId::Bridges,
        ToolId::CutOrder,
        ToolId::CutContour,
        ToolId::RegistrationMarks,
        ToolId::FindDuplicates,
        ToolId::FindCrossings,
        ToolId::ShapeNesting,
        ToolId::RectNesting,
        ToolId::Measure,
        ToolId::CuttingTime,
        ToolId::Cost,
        ToolId::Eyelets,
        ToolId::BoxMaker,
        ToolId::FoldedTray,
    ];

    pub fn group(self) -> Group {
        match self {
            ToolId::ShapeNesting | ToolId::RectNesting => Group::Nesting,
            ToolId::Measure | ToolId::CuttingTime | ToolId::Cost => Group::Measure,
            ToolId::Eyelets | ToolId::BoxMaker | ToolId::FoldedTray => Group::Make,
            _ => Group::Cut,
        }
    }

    /// The name used for the tool's presets (never translated).
    pub fn id(self) -> &'static str {
        match self {
            ToolId::PlotterOutput => "plotter_output",
            ToolId::CncToolpath => "cnc_toolpath",
            ToolId::Weeding => "weeding",
            ToolId::Bridges => "bridges",
            ToolId::CutOrder => "cut_order",
            ToolId::CutContour => "cut_contour",
            ToolId::RegistrationMarks => "registration_marks",
            ToolId::FindDuplicates => "find_duplicates",
            ToolId::FindCrossings => "find_crossings",
            ToolId::ShapeNesting => "shape_nesting",
            ToolId::RectNesting => "rect_nesting",
            ToolId::Measure => "measure",
            ToolId::CuttingTime => "cutting_time",
            ToolId::Cost => "cost",
            ToolId::Eyelets => "eyelets",
            ToolId::BoxMaker => "box_maker",
            ToolId::FoldedTray => "folded_tray",
        }
    }

    /// Whether the tool works on the selection (Box Maker and Folded
    /// Tray make their objects from their settings alone).
    pub fn needs_selection(self) -> bool {
        !matches!(self, ToolId::BoxMaker | ToolId::FoldedTray)
    }

    /// Whether the window works the result out as the settings change,
    /// without Apply (the Measure tools and Folded Tray, all cheap).
    pub fn live(self) -> bool {
        self.group() == Group::Measure || self == ToolId::FoldedTray
    }

    /// The i18n key of the tool's name; `<key>_desc` describes it.
    pub fn key(self) -> &'static str {
        match self {
            ToolId::PlotterOutput => "tracecut.plotter_output",
            ToolId::CncToolpath => "tracecut.cnc_toolpath",
            ToolId::Weeding => "tracecut.weeding",
            ToolId::Bridges => "tracecut.bridges",
            ToolId::CutOrder => "tracecut.cut_order",
            ToolId::CutContour => "tracecut.cut_contour",
            ToolId::RegistrationMarks => "tracecut.registration_marks",
            ToolId::FindDuplicates => "tracecut.find_duplicates",
            ToolId::FindCrossings => "tracecut.find_crossings",
            ToolId::ShapeNesting => "tracecut.shape_nesting",
            ToolId::RectNesting => "tracecut.rect_nesting",
            ToolId::Measure => "tracecut.measure",
            ToolId::CuttingTime => "tracecut.cutting_time",
            ToolId::Cost => "tracecut.cost",
            ToolId::Eyelets => "tracecut.eyelets",
            ToolId::BoxMaker => "tracecut.box_maker",
            ToolId::FoldedTray => "tracecut.tray",
        }
    }
}

/// What TraceCut keeps in the settings: whether it is shown and the
/// tool picked last.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraceCutPrefs {
    /// Options > General: TraceCut is listed in Launch and in the Tools
    /// menu.
    pub show: bool,
    pub tool: ToolId,
    /// Folded Tray's settings as last used.
    pub tray: TraySpec,
}

impl Default for TraceCutPrefs {
    fn default() -> Self {
        TraceCutPrefs {
            show: true,
            tool: ToolId::default(),
            tray: TraySpec::default(),
        }
    }
}

/// Find Duplicates: objects lying on top of an earlier object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuplicatesSettings {
    /// How far two outlines may lie apart and still be the same (mm).
    pub tolerance: f64,
    /// OK deletes the repeats (else it selects them).
    pub delete: bool,
}

impl Default for DuplicatesSettings {
    fn default() -> Self {
        DuplicatesSettings {
            tolerance: 0.05,
            delete: true,
        }
    }
}

/// Find Crossings: places where outlines cross.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrossingsSettings {
    /// Crossings closer than this are one (mm).
    pub tolerance: f64,
    /// Also crossings between different objects (else each object's own).
    pub between: bool,
    /// OK fixes the crossings (else it selects the objects that have them).
    pub fix: bool,
    /// OK moves objects that cross each other apart (before `fix`).
    pub separate: bool,
    /// How far apart Move them apart leaves the objects (mm).
    pub spacing: f64,
}

impl Default for CrossingsSettings {
    fn default() -> Self {
        CrossingsSettings {
            tolerance: 0.05,
            between: false,
            fix: false,
            separate: false,
            spacing: 2.0,
        }
    }
}

/// Tolerances the clean-up tools accept (mm).
pub const TOLERANCE_RANGE: (f64, f64) = (0.001, 10.0);

/// The settings of every tool, as the window shows them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolSettings {
    pub plotter: PlotterSettings,
    pub cnc: CncSpec,
    pub weeding: WeedSpec,
    pub bridges: BridgeSettings,
    pub order: OrderSettings,
    pub contour: CutContourSettings,
    pub marks: MarksSettings,
    pub duplicates: DuplicatesSettings,
    pub crossings: CrossingsSettings,
    pub shape_nest: ShapeNestSettings,
    pub rect_nest: RectNestSettings,
    pub measure: MeasureSettings,
    pub time: TimeSettings,
    pub cost: CostSettings,
    pub eyelets: EyeletSpec,
    pub boxes: BoxSpec,
    pub tray: TraySpec,
}

/// Perimeter and Area has nothing to set; its presets are empty.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MeasureSettings {}

/// Run `$body` with `$v` bound to `tool`'s settings in `$s`.
macro_rules! with_settings {
    ($s:expr, $tool:expr, $v:ident => $body:expr) => {
        match $tool {
            ToolId::PlotterOutput => {
                let $v = &mut $s.plotter;
                $body
            }
            ToolId::CncToolpath => {
                let $v = &mut $s.cnc;
                $body
            }
            ToolId::Weeding => {
                let $v = &mut $s.weeding;
                $body
            }
            ToolId::Bridges => {
                let $v = &mut $s.bridges;
                $body
            }
            ToolId::CutOrder => {
                let $v = &mut $s.order;
                $body
            }
            ToolId::CutContour => {
                let $v = &mut $s.contour;
                $body
            }
            ToolId::RegistrationMarks => {
                let $v = &mut $s.marks;
                $body
            }
            ToolId::FindDuplicates => {
                let $v = &mut $s.duplicates;
                $body
            }
            ToolId::FindCrossings => {
                let $v = &mut $s.crossings;
                $body
            }
            ToolId::ShapeNesting => {
                let $v = &mut $s.shape_nest;
                $body
            }
            ToolId::RectNesting => {
                let $v = &mut $s.rect_nest;
                $body
            }
            ToolId::Measure => {
                let $v = &mut $s.measure;
                $body
            }
            ToolId::CuttingTime => {
                let $v = &mut $s.time;
                $body
            }
            ToolId::Cost => {
                let $v = &mut $s.cost;
                $body
            }
            ToolId::Eyelets => {
                let $v = &mut $s.eyelets;
                $body
            }
            ToolId::BoxMaker => {
                let $v = &mut $s.boxes;
                $body
            }
            ToolId::FoldedTray => {
                let $v = &mut $s.tray;
                $body
            }
        }
    };
}

impl ToolSettings {
    /// `tool`'s settings as JSON (for its presets and to tell when a
    /// result is out of date).
    pub fn to_json(&self, tool: ToolId) -> serde_json::Value {
        let mut copy = self.clone();
        with_settings!(copy, tool, v => serde_json::to_value(v)).unwrap_or(serde_json::Value::Null)
    }

    /// Load `tool`'s settings from JSON; false when it does not read.
    pub fn load_json(&mut self, tool: ToolId, json: &serde_json::Value) -> bool {
        fn load<T: serde::de::DeserializeOwned>(to: &mut T, json: &serde_json::Value) -> bool {
            match serde_json::from_value(json.clone()) {
                Ok(v) => {
                    *to = v;
                    true
                }
                Err(_) => false,
            }
        }
        with_settings!(self, tool, v => load(v, json))
    }

    /// Put `tool`'s settings back to their defaults.
    pub fn reset(&mut self, tool: ToolId) {
        fn clear<T: Default>(v: &mut T) {
            *v = T::default();
        }
        with_settings!(self, tool, v => clear(v))
    }
}

/// The named presets of one tool, kept as JSON values.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPresets {
    /// The preset chosen last.
    pub current: Option<String>,
    pub items: BTreeMap<String, serde_json::Value>,
}

/// Every tool's presets, kept in `tracecut_presets.json` in the settings
/// folder (in memory only where there is none, as in the browser, and in
/// tests).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PresetStore {
    #[serde(default)]
    pub tools: BTreeMap<String, ToolPresets>,
    #[serde(skip)]
    loaded: bool,
}

/// The file the presets are kept in, in the settings folder.
pub const PRESETS_FILE: &str = "tracecut_presets.json";

impl PresetStore {
    fn path() -> Option<std::path::PathBuf> {
        if cfg!(test) {
            return None;
        }
        crate::settings::config_dir().map(|d| d.join(PRESETS_FILE))
    }

    /// The store read from `text`; anything that does not read gives an
    /// empty store.
    pub fn from_json(text: &str) -> PresetStore {
        match serde_json::from_str::<PresetStore>(text) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("TraceCut presets: {e}");
                PresetStore::default()
            }
        }
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Some(p) = Self::path() else { return };
        let Ok(text) = crate::files::read_to_string(&p) else {
            return;
        };
        self.tools = PresetStore::from_json(&text).tools;
    }

    fn save(&self) {
        let Some(p) = Self::path() else { return };
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = crate::files::write_creating(&p, text) {
                    log::warn!("TraceCut presets: {e}");
                }
            }
            Err(e) => log::warn!("TraceCut presets: {e}"),
        }
    }

    /// The presets of `tool`, read from disk the first time.
    pub fn tool(&mut self, tool: ToolId) -> &mut ToolPresets {
        self.load();
        self.tools.entry(tool.id().to_string()).or_default()
    }

    /// Keep `settings` of `tool` as the preset `name` (replacing one of
    /// that name) and make it the current one.
    pub fn save_preset(&mut self, tool: ToolId, name: &str, settings: &ToolSettings) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let v = settings.to_json(tool);
        let p = self.tool(tool);
        p.items.insert(name.to_string(), v);
        p.current = Some(name.to_string());
        self.save();
        true
    }

    /// Load the preset `name` of `tool` into `settings`.
    pub fn apply_preset(&mut self, tool: ToolId, name: &str, settings: &mut ToolSettings) -> bool {
        let p = self.tool(tool);
        let Some(v) = p.items.get(name).cloned() else {
            return false;
        };
        p.current = Some(name.to_string());
        let ok = settings.load_json(tool, &v);
        self.save();
        ok
    }

    /// Delete the preset `name` of `tool`.
    pub fn delete_preset(&mut self, tool: ToolId, name: &str) -> bool {
        let p = self.tool(tool);
        let gone = p.items.remove(name.trim()).is_some();
        if p.current.as_deref() == Some(name.trim()) {
            p.current = None;
        }
        if gone {
            self.save();
        }
        gone
    }
}

/// What a tool found, ready to show in the preview and to apply.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A plotter job and its line for the status strip.
    Job { job: Box<Job>, summary: String },
    /// A CNC program and its line for the status strip.
    Gcode {
        program: Box<Program>,
        summary: String,
    },
    /// Changes of the drawing a Cut tool worked out.
    Changes(Box<Changes>),
    /// A result to read; OK changes nothing.
    Report(Box<Report>),
    /// Each object repeating an earlier one, with that one.
    Duplicates(Vec<(ShapeId, ShapeId)>),
    /// The places where outlines cross, and the objects that have them.
    Crossings {
        points: Vec<Point>,
        objects: Vec<ShapeId>,
    },
}

impl Outcome {
    /// The line under the preview that says what was found.
    pub fn summary(&self) -> String {
        match self {
            Outcome::Job { summary, .. } | Outcome::Gcode { summary, .. } => summary.clone(),
            Outcome::Changes(c) => c.summary.clone(),
            Outcome::Report(r) => r.lines.first().cloned().unwrap_or_default(),
            Outcome::Duplicates(d) if d.is_empty() => tr("tracecut.no_duplicates"),
            Outcome::Duplicates(d) => {
                trf("tracecut.duplicates_found", &[("n", &d.len().to_string())])
            }
            Outcome::Crossings { points, .. } if points.is_empty() => tr("tracecut.no_crossings"),
            Outcome::Crossings { points, objects } => trf(
                "tracecut.crossings_found",
                &[
                    ("n", &points.len().to_string()),
                    ("objects", &objects.len().to_string()),
                ],
            ),
        }
    }
}

/// A result worked out by Apply, with what it was worked out from, so
/// that a change of the drawing, the selection or the settings shows it
/// is out of date.
#[derive(Debug, Clone, PartialEq)]
pub struct Worked {
    pub tool: ToolId,
    pub settings: serde_json::Value,
    pub revision: u64,
    pub targets: Vec<ShapeId>,
    pub outcome: Outcome,
}

/// The TraceCut window between frames.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TraceCutState {
    pub tool: ToolId,
    pub settings: ToolSettings,
    /// The last result of Apply.
    pub worked: Option<Worked>,
    pub presets: PresetStore,
    /// The preset row's name field and whether it is shown.
    pub preset_name: String,
    pub preset_edit: bool,
    /// The preview's zoom and place for each tool zoomed or panned since
    /// the window opened (a tool missing here shows the whole result).
    pub views: Vec<(ToolId, PreviewView)>,
    /// The targets the views were set on: another selection fits again.
    pub view_targets: Vec<ShapeId>,
}

/// A zoomed or panned preview: interface points per millimetre and the
/// page point at the middle of the box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewView {
    pub zoom: f64,
    pub centre: Point,
}

impl TraceCutState {
    /// The window opened on `tool`.
    pub fn new(tool: ToolId) -> TraceCutState {
        TraceCutState {
            tool,
            ..Default::default()
        }
    }

    /// The preview's view for the current tool; None shows the whole
    /// result.
    pub fn view(&self) -> Option<PreviewView> {
        self.views
            .iter()
            .find(|(t, _)| *t == self.tool)
            .map(|(_, v)| *v)
    }

    /// Set (Some) or fit again (None) the current tool's preview.
    pub fn set_view(&mut self, v: Option<PreviewView>) {
        let tool = self.tool;
        self.views.retain(|(t, _)| *t != tool);
        if let Some(v) = v {
            self.views.push((tool, v));
        }
    }

    /// Forget every tool's zoom when the objects worked on change.
    pub fn track_targets(&mut self, targets: &[ShapeId]) {
        if self.view_targets != targets {
            self.views.clear();
            self.view_targets = targets.to_vec();
        }
    }
}

/// What the preview draws: page-space outlines with their roles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    /// The objects the tool works on.
    pub objects: Vec<BezPath>,
    /// Outlines the result points at (drawn over the objects).
    pub marked: Vec<BezPath>,
    /// Points the result points at (drawn as rings).
    pub points: Vec<Point>,
    /// Moves with the knife up (drawn as thin lines).
    pub moves: Vec<(Point, Point)>,
    /// Lines grooved or scored, not cut through.
    pub scored: Vec<BezPath>,
    pub extent: Option<Rect>,
}

impl App {
    /// Open the TraceCut window on the tool picked last.
    pub fn open_tracecut(&mut self) {
        let mut st = TraceCutState::new(self.settings.tracecut.tool);
        st.settings.tray = self.settings.tracecut.tray.clone();
        self.dialog = crate::ui::dialogs::Dialog::TraceCut(Box::new(st));
    }

    /// The objects the tools work on: the selection, in stacking order
    /// (bottom first); objects picked inside groups come after, in the
    /// order picked.
    pub fn tracecut_targets(&self) -> Vec<ShapeId> {
        let doc = self.doc();
        let mut ordered: Vec<ShapeId> = Vec::new();
        for layer in self.page_layers() {
            for s in &layer.shapes {
                if self.selection.contains(&s.id) {
                    ordered.push(s.id);
                }
            }
        }
        for id in &self.selection {
            if !ordered.contains(id) && doc.find_shape(*id).is_some() {
                ordered.push(*id);
            }
        }
        ordered
    }

    /// The outlines of each object in page space.
    pub fn tracecut_outlines(&self, ids: &[ShapeId]) -> Vec<Vec<BezPath>> {
        let doc = self.doc();
        ids.iter()
            .map(|id| {
                doc.shape_on_page(*id)
                    .map(|s| tracedraw_tracecut::outlines(&s))
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Work out what `tool` finds on the targets with `settings`.
    pub fn tracecut_work_out(&self, tool: ToolId, settings: &ToolSettings) -> Worked {
        let targets = self.tracecut_targets();
        let outlines = self.tracecut_outlines(&targets);
        let clamp = |t: f64| {
            if t.is_finite() {
                t.clamp(TOLERANCE_RANGE.0, TOLERANCE_RANGE.1)
            } else {
                DuplicatesSettings::default().tolerance
            }
        };
        let outcome = match tool {
            ToolId::PlotterOutput => {
                let job = self.tracecut_job(&settings.plotter);
                let summary = self.tracecut_job_summary(&job);
                Outcome::Job {
                    job: Box::new(job),
                    summary,
                }
            }
            ToolId::CncToolpath => {
                let program = self.tracecut_cnc(&settings.cnc);
                let summary = self.tracecut_cnc_summary(&program);
                Outcome::Gcode {
                    program: Box::new(program),
                    summary,
                }
            }
            ToolId::Weeding => Outcome::Changes(Box::new(self.tracecut_weeding(&settings.weeding))),
            ToolId::Bridges => Outcome::Changes(Box::new(self.tracecut_bridges(&settings.bridges))),
            ToolId::CutOrder => {
                Outcome::Changes(Box::new(self.tracecut_cut_order(&settings.order)))
            }
            ToolId::CutContour => {
                Outcome::Changes(Box::new(self.tracecut_cut_contour(&settings.contour)))
            }
            ToolId::RegistrationMarks => {
                Outcome::Changes(Box::new(self.tracecut_marks(&settings.marks)))
            }
            ToolId::ShapeNesting => {
                Outcome::Changes(Box::new(self.tracecut_shape_nest(&settings.shape_nest)))
            }
            ToolId::RectNesting => {
                Outcome::Changes(Box::new(self.tracecut_rect_nest(&settings.rect_nest)))
            }
            ToolId::Eyelets => Outcome::Changes(Box::new(self.tracecut_eyelets(&settings.eyelets))),
            ToolId::BoxMaker => Outcome::Changes(Box::new(self.tracecut_box(&settings.boxes))),
            ToolId::FoldedTray => Outcome::Changes(Box::new(self.tracecut_tray(&settings.tray))),
            ToolId::Measure => Outcome::Report(Box::new(self.tracecut_measure())),
            ToolId::CuttingTime => Outcome::Report(Box::new(self.tracecut_time(&settings.time).0)),
            ToolId::Cost => {
                Outcome::Report(Box::new(self.tracecut_cost(&settings.cost, &settings.time)))
            }
            ToolId::FindDuplicates => {
                let found =
                    cleanup::find_duplicates(&outlines, clamp(settings.duplicates.tolerance));
                Outcome::Duplicates(
                    found
                        .iter()
                        .filter_map(|(i, o)| Some((*targets.get(*i)?, *targets.get(*o)?)))
                        .collect(),
                )
            }
            ToolId::FindCrossings => {
                // Moving apart is for objects crossing each other.
                let found = crossings::find_crossings(
                    &outlines,
                    settings.crossings.between || settings.crossings.separate,
                    clamp(settings.crossings.tolerance),
                );
                let mut objects: Vec<ShapeId> = Vec::new();
                for c in &found {
                    for i in [c.a, c.b] {
                        if let Some(id) = targets.get(i) {
                            if !objects.contains(id) {
                                objects.push(*id);
                            }
                        }
                    }
                }
                Outcome::Crossings {
                    points: found.iter().map(|c| c.point).collect(),
                    objects,
                }
            }
        };
        Worked {
            tool,
            settings: settings.to_json(tool),
            revision: self.engine.revision(),
            targets,
            outcome,
        }
    }

    /// Fix the crossings of the targets in one undo step ("Fix
    /// Crossings"): each outline crossing itself is rebuilt as the
    /// outlines of the area it fills, and with `between` the objects
    /// that cross each other are welded into the lowest of them, the
    /// others deleted. Curves, rectangles, ellipses and polygons are
    /// fixed; groups, text, pictures and locked objects are left as they
    /// are. Returns whether anything changed; the status line says what.
    pub fn tracecut_fix_crossings(&mut self, s: &CrossingsSettings) -> bool {
        let targets = self.tracecut_targets();
        let outlines = self.tracecut_outlines(&targets);
        let tol = if s.tolerance.is_finite() {
            s.tolerance.clamp(TOLERANCE_RANGE.0, TOLERANCE_RANGE.1)
        } else {
            CrossingsSettings::default().tolerance
        };
        let found = crossings::find_crossings(&outlines, s.between, tol);
        let doc = self.doc();
        // The object at index `i` on the page, when its outline can be
        // replaced by a curve: with a page transform that can be undone.
        let fixable = |i: usize| -> Option<tracedraw_core::Shape> {
            let id = *targets.get(i)?;
            let shape = doc.shape_on_page(id)?;
            let plain = matches!(
                shape.kind,
                ShapeKind::Rect { .. }
                    | ShapeKind::Ellipse { .. }
                    | ShapeKind::Polygon { .. }
                    | ShapeKind::Path { .. }
            );
            let det = shape.transform.determinant();
            (plain && !shape.locked && det.is_finite() && det.abs() > 1e-12).then_some(shape)
        };
        // A page-space outline as the new outline of `shape`.
        let kind_for = |shape: &tracedraw_core::Shape, page: BezPath| ShapeKind::Path {
            path: shape.transform.inverse() * page,
            closed: !matches!(shape.kind, ShapeKind::Path { closed: false, .. }),
        };
        let mut cmds: Vec<Command> = Vec::new();
        let mut fixed: Vec<ShapeId> = Vec::new();
        let mut deleted: Vec<ShapeId> = Vec::new();
        let mut handled: Vec<usize> = Vec::new();
        if s.between {
            for group in crossings::crossing_groups(&found, targets.len()) {
                let members: Vec<(usize, tracedraw_core::Shape)> = group
                    .iter()
                    .filter_map(|i| Some((*i, fixable(*i)?)))
                    .collect();
                let Some(((_, first), rest)) = members.split_first() else {
                    continue;
                };
                if rest.is_empty() {
                    continue;
                }
                let paths: Vec<_> = members
                    .iter()
                    .map(|(_, m)| (m.page_path(), m.fill_rule))
                    .collect();
                let welded = crossings::weld_crossing(&paths, tol);
                cmds.push(Command::SetShapeKind {
                    shape: first.id,
                    kind: kind_for(first, welded),
                });
                fixed.push(first.id);
                deleted.extend(rest.iter().map(|(_, m)| m.id));
                handled.extend(members.iter().map(|(i, _)| *i));
            }
        }
        let mut own: Vec<usize> = found.iter().filter(|c| c.a == c.b).map(|c| c.a).collect();
        own.sort_unstable();
        own.dedup();
        for i in own {
            if handled.contains(&i) {
                continue;
            }
            let Some(shape) = fixable(i) else {
                continue;
            };
            let resolved = crossings::resolve_crossings(&shape.page_path(), shape.fill_rule, tol);
            cmds.push(Command::SetShapeKind {
                shape: shape.id,
                kind: kind_for(&shape, resolved),
            });
            fixed.push(shape.id);
        }
        if !deleted.is_empty() {
            cmds.push(Command::DeleteShapes {
                shapes: deleted.clone(),
            });
        }
        if cmds.is_empty() {
            self.status = tr("tracecut.crossings_not_fixed");
            return false;
        }
        if let Err(e) = self.engine.run_batch("Fix Crossings", &cmds) {
            self.status = e.to_string();
            return false;
        }
        let n = fixed.len() + deleted.len();
        self.select(fixed);
        self.status = trf("tracecut.crossings_fixed", &[("n", &n.to_string())]);
        true
    }

    /// Move objects that cross each other apart in one undo step
    /// ("Separate Crossings"): in each set of objects crossing one
    /// another the lowest stays (or the locked ones), and the others each
    /// move the shortest way that leaves the spacing to every object
    /// staying or moved before (`crossings::separate_crossing`). Any kind
    /// of object moves; locked ones stay. Returns whether anything moved;
    /// the status line says what.
    pub fn tracecut_separate_crossings(&mut self, s: &CrossingsSettings) -> bool {
        let targets = self.tracecut_targets();
        let outlines = self.tracecut_outlines(&targets);
        let tol = if s.tolerance.is_finite() {
            s.tolerance.clamp(TOLERANCE_RANGE.0, TOLERANCE_RANGE.1)
        } else {
            CrossingsSettings::default().tolerance
        };
        let spacing = if s.spacing.is_finite() {
            s.spacing.clamp(0.0, 1000.0)
        } else {
            CrossingsSettings::default().spacing
        };
        let found = crossings::find_crossings(&outlines, true, tol);
        let doc = self.doc();
        let movable: Vec<bool> = targets
            .iter()
            .map(|id| doc.find_shape(*id).is_some_and(|sh| !sh.locked))
            .collect();
        let moves = crossings::separate_crossing(&outlines, &movable, &found, spacing, tol);
        let mut cmds = Vec::new();
        let mut moved = Vec::new();
        for (id, d) in targets.iter().zip(&moves) {
            if d.hypot() > 1e-9 && d.x.is_finite() && d.y.is_finite() {
                cmds.push(Command::TransformShapes {
                    shapes: vec![*id],
                    transform: tracedraw_core::geometry::Affine::translate(*d),
                });
                moved.push(*id);
            }
        }
        if cmds.is_empty() {
            self.status = tr("tracecut.crossings_not_separated");
            return false;
        }
        if let Err(e) = self.engine.run_batch("Separate Crossings", &cmds) {
            self.status = e.to_string();
            return false;
        }
        let n = moved.len();
        self.select(moved);
        self.status = trf("tracecut.crossings_separated", &[("n", &n.to_string())]);
        true
    }

    /// Whether `w` still matches the drawing, the selection and the
    /// window's tool and settings.
    pub fn tracecut_current(&self, w: &Worked, tool: ToolId, settings: &ToolSettings) -> bool {
        w.tool == tool
            && w.revision == self.engine.revision()
            && w.settings == settings.to_json(tool)
            && w.targets == self.tracecut_targets()
    }

    /// What the preview shows: the targets and, when `worked` is current,
    /// what it found.
    pub fn tracecut_scene(&self, worked: Option<&Worked>) -> Scene {
        let targets = self.tracecut_targets();
        let outlines = self.tracecut_outlines(&targets);
        let mut scene = Scene::default();
        for o in &outlines {
            scene.objects.extend(o.iter().cloned());
        }
        if let Some(w) = worked {
            match &w.outcome {
                Outcome::Duplicates(d) => {
                    for (dup, _) in d {
                        if let Some(i) = targets.iter().position(|t| t == dup) {
                            if let Some(o) = outlines.get(i) {
                                scene.marked.extend(o.iter().cloned());
                            }
                        }
                    }
                }
                Outcome::Crossings { points, .. } => scene.points = points.clone(),
                Outcome::Job { job, .. } => scene.moves = job.moves.clone(),
                Outcome::Gcode { program, .. } => {
                    scene.marked = program.toolpaths.iter().map(|t| t.path()).collect();
                    scene.moves = program.rapids.clone();
                }
                Outcome::Report(_) => {}
                Outcome::Changes(c) => {
                    if c.hide_objects {
                        scene.objects.clear();
                    }
                    scene.marked = c.preview.clone();
                    scene.scored = c.scored.clone();
                    scene.points = c.points.clone();
                    scene.moves = c.moves.clone();
                }
            }
        }
        scene.extent = scene
            .objects
            .iter()
            .chain(scene.marked.iter())
            .chain(scene.scored.iter())
            .map(|p| p.bounding_box())
            .filter(|b| {
                b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()
            })
            .reduce(|a, b| a.union(b));
        scene
    }

    /// Apply `w` to the drawing (OK): Find Duplicates deletes the repeats
    /// in one undo step (or selects them), Find Crossings selects the
    /// objects that have crossings, fixes them or moves them apart. Returns whether anything was done;
    /// the status line says what.
    pub fn tracecut_commit(
        &mut self,
        w: &Worked,
        settings: &ToolSettings,
        ctx: Option<egui::Context>,
    ) -> bool {
        match &w.outcome {
            Outcome::Job { job, summary } => {
                if job.plot.outlines == 0 {
                    self.status = summary.clone();
                    return false;
                }
                self.tracecut_output(&settings.plotter, &job.plot, ctx);
                true
            }
            Outcome::Gcode { program, summary } => {
                if program.toolpaths.is_empty() {
                    self.status = summary.clone();
                    return false;
                }
                self.tracecut_save_gcode(program);
                true
            }
            Outcome::Changes(c) => self.tracecut_apply_changes(c),
            Outcome::Report(r) => {
                self.status = r.lines.first().cloned().unwrap_or_default();
                false
            }
            Outcome::Duplicates(d) => {
                let doc = self.doc();
                let dups: Vec<ShapeId> = d
                    .iter()
                    .map(|(id, _)| *id)
                    .filter(|id| doc.find_shape(*id).is_some_and(|s| !s.locked))
                    .collect();
                let n = dups.len();
                if n == 0 {
                    self.status = tr("tracecut.no_duplicates");
                    return false;
                }
                if settings.duplicates.delete {
                    if let Err(e) = self.engine.run_batch(
                        "Delete Duplicates",
                        &[Command::DeleteShapes {
                            shapes: dups.clone(),
                        }],
                    ) {
                        self.status = e.to_string();
                        return false;
                    }
                    self.selection.retain(|id| !dups.contains(id));
                    self.status = trf("tracecut.duplicates_deleted", &[("n", &n.to_string())]);
                } else {
                    self.select(dups);
                    self.status = trf("tracecut.duplicates_selected", &[("n", &n.to_string())]);
                }
                true
            }
            Outcome::Crossings { points, objects } => {
                if points.is_empty() {
                    self.status = tr("tracecut.no_crossings");
                    return false;
                }
                if settings.crossings.separate {
                    return self.tracecut_separate_crossings(&settings.crossings);
                }
                if settings.crossings.fix {
                    return self.tracecut_fix_crossings(&settings.crossings);
                }
                self.select(objects.clone());
                self.status = w.outcome.summary();
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::dialogs::Dialog;
    use tracedraw_core::ShapeKind;

    fn rect(app: &mut App, x: f64, y: f64, w: f64, h: f64) -> ShapeId {
        app.new_shape(ShapeKind::Rect {
            rect: Rect::new(x, y, x + w, y + h),
            radius: 0.0,
            corners: None,
        })
        .expect("rect")
    }

    #[test]
    fn every_tool_is_in_one_group_and_has_strings() {
        let mut listed = Vec::new();
        for g in Group::ALL {
            assert_ne!(tr(g.key()), g.key(), "{:?}", g);
            listed.extend(g.tools());
        }
        assert_eq!(listed, ToolId::ALL.to_vec());
        for t in ToolId::ALL {
            assert_ne!(tr(t.key()), t.key(), "{t:?}");
            let desc = format!("{}_desc", t.key());
            assert_ne!(tr(&desc), desc, "{t:?}");
        }
    }

    #[test]
    fn duplicates_are_found_and_ok_deletes_them_in_one_step() {
        let mut app = App::headless();
        let a = rect(&mut app, 10.0, 10.0, 40.0, 20.0);
        let b = rect(&mut app, 10.0, 10.0, 40.0, 20.0);
        let c = rect(&mut app, 80.0, 10.0, 10.0, 10.0);
        app.select(vec![a, b, c]);
        let settings = ToolSettings::default();
        let w = app.tracecut_work_out(ToolId::FindDuplicates, &settings);
        assert_eq!(w.outcome, Outcome::Duplicates(vec![(b, a)]));
        // Working it out changes nothing.
        assert!(app.doc().find_shape(b).is_some());
        let undo_before = app.engine.history_labels().0.len();
        assert!(app.tracecut_commit(&w, &settings, None));
        assert!(app.doc().find_shape(b).is_none());
        assert!(app.doc().find_shape(a).is_some() && app.doc().find_shape(c).is_some());
        let (undo, _) = app.engine.history_labels();
        assert_eq!(undo.len(), undo_before + 1);
        assert_eq!(undo.last().copied(), Some("Delete Duplicates"));
        app.undo();
        assert!(app.doc().find_shape(b).is_some());
    }

    #[test]
    fn duplicates_can_be_selected_instead() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        app.select(vec![a, b]);
        let mut settings = ToolSettings::default();
        settings.duplicates.delete = false;
        let w = app.tracecut_work_out(ToolId::FindDuplicates, &settings);
        let rev = app.engine.revision();
        assert!(app.tracecut_commit(&w, &settings, None));
        assert_eq!(app.selection, vec![b]);
        assert_eq!(app.engine.revision(), rev);
    }

    #[test]
    fn ok_fixes_the_crossings_in_one_undo_step() {
        let mut app = App::headless();
        let mut eight = BezPath::new();
        eight.move_to((0.0, 0.0));
        eight.line_to((10.0, 10.0));
        eight.line_to((10.0, 0.0));
        eight.line_to((0.0, 10.0));
        eight.close_path();
        let e = app
            .new_shape(ShapeKind::Path {
                path: eight,
                closed: true,
            })
            .expect("figure eight");
        let a = rect(&mut app, 50.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 60.0, 10.0, 20.0, 20.0);
        app.select(vec![e, a, b]);
        let mut settings = ToolSettings::default();
        settings.crossings.between = true;
        settings.crossings.fix = true;
        let w = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        assert!(matches!(&w.outcome, Outcome::Crossings { points, .. } if points.len() == 3));
        let undo_before = app.engine.history_labels().0.len();
        assert!(app.tracecut_commit(&w, &settings, None));
        let (undo, _) = app.engine.history_labels();
        assert_eq!(undo.len(), undo_before + 1);
        assert_eq!(undo.last().copied(), Some("Fix Crossings"));
        // The two squares are one outline now, the lower one kept.
        assert!(app.doc().find_shape(a).is_some() && app.doc().find_shape(b).is_none());
        app.select(vec![e, a]);
        let again = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        assert!(
            matches!(&again.outcome, Outcome::Crossings { points, .. } if points.is_empty()),
            "{:?}",
            again.outcome
        );
        app.undo();
        assert!(app.doc().find_shape(b).is_some());
        app.select(vec![e, a, b]);
        let back = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        assert_eq!(back.outcome, w.outcome);
    }

    /// The live test of 0.41: Find Crossings found crossings but offered
    /// no way to resolve them by moving the objects. With Move them
    /// apart, OK moves each object crossing a lower one the shortest way
    /// that leaves the spacing (here 6 mm right: 4 of overlap and 2 of
    /// spacing), the lowest stays, an ellipse crossing the moved square
    /// moves too, and all is one undo step; afterwards nothing crosses.
    /// It finds crossings between objects without Also between objects;
    /// a locked object stays and the other moves instead.
    #[test]
    fn ok_moves_crossing_objects_apart_in_one_undo_step() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 16.0, 3.0, 20.0, 20.0);
        let c = app
            .new_shape(ShapeKind::Ellipse {
                rect: Rect::new(30.0, 5.0, 50.0, 15.0),
                arc: None,
            })
            .expect("ellipse");
        let far = rect(&mut app, 0.0, 60.0, 10.0, 10.0);
        app.select(vec![a, b, c, far]);
        let mut settings = ToolSettings::default();
        settings.crossings.separate = true;
        assert!(!settings.crossings.between);
        let w = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        assert!(matches!(&w.outcome, Outcome::Crossings { points, .. } if points.len() == 4));
        let page_box = |app: &App, id: ShapeId| {
            app.doc()
                .shape_on_page(id)
                .expect("shape")
                .page_path()
                .bounding_box()
        };
        let before: Vec<Rect> = [a, b, c, far]
            .iter()
            .map(|id| page_box(&app, *id))
            .collect();
        let undo_before = app.engine.history_labels().0.len();
        assert!(app.tracecut_commit(&w, &settings, None));
        let (undo, _) = app.engine.history_labels();
        assert_eq!(undo.len(), undo_before + 1);
        assert_eq!(undo.last().copied(), Some("Separate Crossings"));
        assert_eq!(page_box(&app, a), before[0], "the lowest stays");
        assert_eq!(page_box(&app, far), before[3], "crossed nothing: stays");
        let moved_b = page_box(&app, b).x0 - before[1].x0;
        assert!((moved_b - 6.0).abs() < 0.01, "moved {moved_b}");
        assert_ne!(page_box(&app, c), before[2]);
        assert_eq!(app.selection, vec![b, c]);
        settings.crossings.separate = false;
        settings.crossings.between = true;
        let again = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        assert!(
            matches!(&again.outcome, Outcome::Crossings { points, .. } if points.is_empty()),
            "{:?}",
            again.outcome
        );
        app.undo();
        assert_eq!(page_box(&app, b), before[1]);
        // The lower one locked: the upper one stays, the lower moves.
        app.select(vec![b]);
        app.set_locked(true);
        // Set as such: a locked object is never selected (#2347), but a
        // selection that held one still leaves it in place.
        app.selection = vec![a, b];
        settings.crossings.separate = true;
        let w = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        assert!(app.tracecut_commit(&w, &settings, None));
        assert_eq!(page_box(&app, b), before[1]);
        assert!((page_box(&app, a).x0 + 6.0).abs() < 0.01);
        // Both locked: nothing to move, and the status line says so.
        app.undo();
        app.select(vec![a]);
        app.set_locked(true);
        app.selection = vec![a, b];
        let w = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        let rev = app.engine.revision();
        assert!(!app.tracecut_commit(&w, &settings, None));
        assert_eq!(app.engine.revision(), rev);
        assert_eq!(app.status, tr("tracecut.crossings_not_separated"));
    }

    #[test]
    fn crossings_select_the_objects_that_cross() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 10.0, 10.0, 20.0, 20.0);
        let c = rect(&mut app, 100.0, 100.0, 5.0, 5.0);
        app.select(vec![a, b, c]);
        let mut settings = ToolSettings::default();
        let w = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        assert!(matches!(&w.outcome, Outcome::Crossings { points, .. } if points.is_empty()));
        settings.crossings.between = true;
        let w = app.tracecut_work_out(ToolId::FindCrossings, &settings);
        let Outcome::Crossings { points, objects } = &w.outcome else {
            panic!("crossings");
        };
        assert_eq!(points.len(), 2);
        assert_eq!(objects, &vec![a, b]);
        assert!(app.tracecut_commit(&w, &settings, None));
        assert_eq!(app.selection, vec![a, b]);
    }

    #[test]
    fn a_result_goes_out_of_date_with_the_drawing_selection_or_settings() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        app.select(vec![a, b]);
        let mut settings = ToolSettings::default();
        let w = app.tracecut_work_out(ToolId::FindDuplicates, &settings);
        assert!(app.tracecut_current(&w, ToolId::FindDuplicates, &settings));
        assert!(!app.tracecut_current(&w, ToolId::FindCrossings, &settings));
        settings.duplicates.tolerance = 1.0;
        assert!(!app.tracecut_current(&w, ToolId::FindDuplicates, &settings));
        settings.duplicates.tolerance = DuplicatesSettings::default().tolerance;
        app.select(vec![a]);
        assert!(!app.tracecut_current(&w, ToolId::FindDuplicates, &settings));
        app.select(vec![a, b]);
        rect(&mut app, 50.0, 50.0, 5.0, 5.0);
        app.select(vec![a, b]);
        assert!(!app.tracecut_current(&w, ToolId::FindDuplicates, &settings));
    }

    #[test]
    fn the_scene_marks_what_was_found() {
        let mut app = App::headless();
        let a = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        let b = rect(&mut app, 0.0, 0.0, 20.0, 20.0);
        app.select(vec![a, b]);
        let w = app.tracecut_work_out(ToolId::FindDuplicates, &ToolSettings::default());
        let scene = app.tracecut_scene(Some(&w));
        assert_eq!(scene.objects.len(), 2);
        assert_eq!(scene.marked.len(), 1);
        let e = scene.extent.expect("extent");
        assert!((e.width() - 20.0).abs() < 1e-6);
        assert!(app.tracecut_scene(None).marked.is_empty());
    }

    #[test]
    fn presets_save_load_and_delete() {
        let mut store = PresetStore::default();
        let mut s = ToolSettings::default();
        s.duplicates.tolerance = 0.5;
        s.duplicates.delete = false;
        assert!(!store.save_preset(ToolId::FindDuplicates, "  ", &s));
        assert!(store.save_preset(ToolId::FindDuplicates, "Coarse", &s));
        let mut t = ToolSettings::default();
        assert!(store.apply_preset(ToolId::FindDuplicates, "Coarse", &mut t));
        assert_eq!(t.duplicates, s.duplicates);
        // Each tool keeps its own.
        assert!(!store.apply_preset(ToolId::FindCrossings, "Coarse", &mut t));
        // The file round trip.
        let text = serde_json::to_string(&store).expect("json");
        let mut back = PresetStore::from_json(&text);
        assert_eq!(
            back.tool(ToolId::FindDuplicates).current.as_deref(),
            Some("Coarse")
        );
        assert!(back.delete_preset(ToolId::FindDuplicates, "Coarse"));
        assert!(back.tool(ToolId::FindDuplicates).items.is_empty());
        assert!(back.tool(ToolId::FindDuplicates).current.is_none());
        // A damaged file gives no presets.
        assert!(PresetStore::from_json("{not json").tools.is_empty());
    }

    #[test]
    fn a_preset_of_another_shape_keeps_the_defaults_it_lacks() {
        let mut s = ToolSettings::default();
        assert!(s.load_json(
            ToolId::FindCrossings,
            &serde_json::json!({ "between": true, "unknown": 3 })
        ));
        assert!(s.crossings.between);
        assert_eq!(
            s.crossings.tolerance,
            CrossingsSettings::default().tolerance
        );
        s.reset(ToolId::FindCrossings);
        assert!(!s.crossings.between);
    }

    #[test]
    fn open_tracecut_starts_on_the_tool_picked_last() {
        let mut app = App::headless();
        app.settings.tracecut.tool = ToolId::FindCrossings;
        app.open_tracecut();
        match &app.dialog {
            Dialog::TraceCut(st) => assert_eq!(st.tool, ToolId::FindCrossings),
            d => panic!("{d:?}"),
        }
    }
}
