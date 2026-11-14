//! JavaScript automation (Scripts panel, Tools > Scripts). The object
//! model: `Application.ActiveDocument`,
//! `ActivePage`, `ActiveLayer`, `Shapes`, `Shape` with `Fill`, `Outline`,
//! `Move`, `Rotate`, `Stretch`, `Delete`, `Duplicate`, `ConvertToCurves`,
//! `Curve` with `SubPaths` and `Nodes` (position, type, move, delete,
//! add), `Effects` (count, type, add, remove, clear),
//! plus `CreateRectangle`, `CreateEllipse`, `CreateArtisticText`,
//! `CreateCurve`, `CreatePath` (SVG path data), `CreatePolygon`, and
//! `Shape.PathData` back. Recorded macros are plain scripts; a script that
//! defines `TraceDrawFilter` can be installed as a file format
//! ([`ScriptFilter`], `script_filters.rs`).
//!
//! Implementation: one native dispatcher `__td(method, argsJson)` works on
//! a snapshot of the document and records commands; the JS prelude builds
//! the object model on top. After the script the commands run as one
//! undo step. Scripts an AI service wrote run untrusted
//! ([`run_untrusted`]): without file access, with limits, all or nothing.

pub mod vba_run;
mod vg;

use crate::app::App;
use boa_engine::{
    js_string, native_function::NativeFunction, object::ObjectInitializer, Context, JsData,
    JsResult, JsValue, Source,
};
use serde_json::{json, Value};
use std::cell::RefCell;
use tracedraw_core::{
    document::{Shape, ShapeKind},
    geometry::{Affine, Point, Rect},
    Color, Command, Document, Fill, LayerId, PageId, ShapeId, Stroke, TextSpan,
};

struct HostInner {
    doc: Document,
    page: PageId,
    layer: Option<LayerId>,
    selection: Vec<ShapeId>,
    commands: Vec<Command>,
    output: Vec<String>,
    next_id: u64,
    trust: Trust,
    /// The drawing's file, when it has one (`Document.FullFileName`).
    file: Option<String>,
    /// The object model of VBA macros (see `scripting/vg.rs`) instead of
    /// TraceDraw's own JavaScript one.
    vg: bool,
    /// What the app shows that macros read: see [`vg::AppView`].
    view: vg::AppView,
    /// The status line text a macro left (`Application.Status`).
    status: Option<String>,
    /// View > Simulate Overprints as a macro set it (`ActiveView.Type`).
    simulate_overprints: Option<bool>,
    /// What a macro's `Copy` or `Cut` took, for its `Paste`: the macro's
    /// own, not the system clipboard, which macros do not reach.
    clip: Vec<Shape>,
}

/// How far a script is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trust {
    /// The user's own scripts: every method; the changes made before an
    /// error are kept.
    User,
    /// Scripts an AI service wrote (AI Drawing): no file access, loops,
    /// recursion and changes limited, nothing kept unless all of it runs.
    Untrusted,
}

/// Untrusted scripts: loop iterations per function call, call depth and
/// changes to the drawing.
const UNTRUSTED_LOOP_LIMIT: u64 = 1_000_000;
const UNTRUSTED_RECURSION_LIMIT: usize = 400;
const UNTRUSTED_MAX_CHANGES: usize = 20_000;
/// Methods that reach files, refused to untrusted scripts.
const FILE_METHODS: [&str; 2] = ["export", "import_bitmap"];

struct Host(RefCell<HostInner>);

impl boa_engine::gc::Finalize for Host {}
// The host holds no garbage-collected values.
unsafe impl boa_engine::gc::Trace for Host {
    boa_engine::gc::empty_trace!();
}
impl JsData for Host {}

/// What both object models share: the call into the host and printing.
const COMMON: &str = r#"
function __call(m, a) { return JSON.parse(__td(m, JSON.stringify(a || {}))); }
function print(...a) { __call('log', {text: a.map(String).join(' ')}); }
const console = { log: print, error: print, warn: print, info: print };
function MsgBox(t) { print(t); }
"#;

/// TraceDraw's own JavaScript object model (lengths in millimetres).
const PRELUDE: &str = r#"
class Color {
  constructor(v) { this.v = v || {model:'rgb', r:0, g:0, b:0}; }
  static RGB(r, g, b) { return new Color({model:'rgb', r:r/255, g:g/255, b:b/255}); }
  static CMYK(c, m, y, k) { return new Color({model:'cmyk', c:c/100, m:m/100, y:y/100, k:k/100}); }
  static Gray(v) { return new Color({model:'gray', v:v/100}); }
  get Hex() { return __call('color_hex', {color:this.v}).hex; }
}
class Fill {
  constructor(id) { this.id = id; }
  get Type() { return __call('fill_type', {id:this.id}).type; }
  ApplyNoFill() { __call('set_fill', {id:this.id, fill:{fill:'none'}}); }
  ApplyUniformFill(color) { __call('set_fill', {id:this.id, fill:{fill:'solid', ...color.v}}); }
  ApplyFountainFill(from, to, angle) { __call('set_fountain', {id:this.id, from:from.v, to:to.v, angle:angle||0}); }
  get UniformColor() { return new Color(__call('fill_color', {id:this.id}).color); }
}
class Outline {
  constructor(id) { this.id = id; }
  SetNoOutline() { __call('set_outline', {id:this.id, outline:null}); }
  SetProperties(width, color) { __call('set_outline_props', {id:this.id, width:width, color:color ? color.v : null}); }
  get Width() { return __call('outline_width', {id:this.id}).width; }
  set Width(w) { __call('set_outline_props', {id:this.id, width:w, color:null}); }
  get Color() { return new Color(__call('outline_color', {id:this.id}).color); }
  set Color(c) { __call('set_outline_props', {id:this.id, width:null, color:c.v}); }
}
class Shape {
  constructor(id) { this.id = id; this.Fill = new Fill(id); this.Outline = new Outline(id); }
  get Name() { return __call('shape_info', {id:this.id}).name; }
  set Name(n) { __call('set_name', {id:this.id, name:n}); }
  get Type() { return __call('shape_info', {id:this.id}).type; }
  get PositionX() { return __call('shape_info', {id:this.id}).x; }
  get PositionY() { return __call('shape_info', {id:this.id}).y; }
  get SizeWidth() { return __call('shape_info', {id:this.id}).w; }
  get SizeHeight() { return __call('shape_info', {id:this.id}).h; }
  get CenterX() { const i = __call('shape_info', {id:this.id}); return i.x + i.w/2; }
  get CenterY() { const i = __call('shape_info', {id:this.id}); return i.y + i.h/2; }
  // The bounding box in one reading, for scripts that measure rather than
  // place: one call instead of four, and the edges come named.
  get Bounds() {
    const i = __call('shape_info', {id:this.id});
    return { x: i.x, y: i.y, width: i.w, height: i.h,
             left: i.x, bottom: i.y, right: i.x + i.w, top: i.y + i.h,
             centerX: i.x + i.w/2, centerY: i.y + i.h/2 };
  }
  get Text() { return __call('shape_info', {id:this.id}).text; }
  set Text(t) { __call('set_text', {id:this.id, text:t}); }
  get Font() { return __call('shape_info', {id:this.id}).font; }
  set Font(f) { __call('set_text_style', {id:this.id, font:f}); }
  get FontSize() { return __call('shape_info', {id:this.id}).size_pt; }
  set FontSize(z) { __call('set_text_style', {id:this.id, size:z}); }
  get Bold() { return __call('shape_info', {id:this.id}).bold; }
  set Bold(b) { __call('set_text_style', {id:this.id, bold:!!b}); }
  get Italic() { return __call('shape_info', {id:this.id}).italic; }
  set Italic(i) { __call('set_text_style', {id:this.id, italic:!!i}); }
  SetTextStyle(style) { __call('set_text_style', {id:this.id, ...style}); }
  SetPosition(x, y) { __call('set_position', {id:this.id, x:x, y:y}); }
  SetSize(w, h) { __call('set_size', {id:this.id, w:w, h:h}); }
  Move(dx, dy) { __call('move', {id:this.id, dx:dx, dy:dy}); }
  Rotate(deg) { __call('rotate', {id:this.id, deg:deg}); }
  Stretch(sx, sy) { __call('stretch', {id:this.id, sx:sx, sy:sy===undefined?sx:sy}); }
  Skew(ax, ay) { __call('skew', {id:this.id, ax:ax, ay:ay||0}); }
  Delete() { __call('delete', {id:this.id}); }
  Duplicate(dx, dy) { return new Shape(__call('duplicate', {id:this.id, dx:dx||0, dy:dy||0}).id); }
  ConvertToCurves() { __call('convert_to_curves', {id:this.id}); return this; }
  Ungroup() { __call('ungroup', {id:this.id}); }
  BreakApart() { return __call('break_apart', {id:this.id}).ids.map(i => new Shape(i)); }
  AddToSelection() { __call('select', {ids:[this.id], add:true}); }
  CreateSelection() { __call('select', {ids:[this.id], add:false}); }
  OrderToFront() { __call('order', {id:this.id, op:'front'}); }
  OrderToBack() { __call('order', {id:this.id, op:'back'}); }
  SetOpacity(pct) { __call('set_opacity', {id:this.id, opacity:1 - pct/100}); }
  get Transparency() { return __call('shape_info', {id:this.id}).transparency; }
  AddDropShadow(dx, dy, opacity) { __call('drop_shadow', {id:this.id, dx:dx, dy:dy, opacity:opacity===undefined?50:opacity}); }
  get Curve() { return new Curve(this.id); }
  get Effects() { return new Effects(this.id); }
  get PathData() { return __call('path_data', {id:this.id}).d; }
}
class Effects {
  constructor(id) { this.id = id; }
  get types() { return __call('effects', {id:this.id}).types; }
  get Count() { return this.types.length; }
  Item(i) { const t = this.types[i-1]; return t === undefined ? null : { Type: t, Index: i }; }
  Remove(i) { __call('remove_effects', {id:this.id, index:i-1}); }
  Clear() { __call('remove_effects', {id:this.id}); }
  Add(effect) { __call('add_effect', {id:this.id, effect:effect}); return this.Count; }
  AddExtrude(dx, dy) { return this.Add({effect:'extrude', depth:{x:dx, y:dy}, vanishing:null, amount:0, shade_from:null, shade_to:null, light_angle:135, light_intensity:50, bevel:0, kind:'back_parallel'}); }
  AddBlockShadow(dx, dy, color) { return this.Add({effect:'blockshadow', offset:{x:dx, y:dy}, color:(color || Color.RGB(0,0,0)).v, gap:0}); }
}
class Node {
  constructor(id, index) { this.id = id; this.index = index; }
  get info() { return __call('curve_nodes', {id:this.id}).nodes.find(n => n.index === this.index); }
  get PositionX() { return this.info.x; }
  get PositionY() { return this.info.y; }
  get Type() { return this.info.type; }
  set Type(t) { __call('node_type', {id:this.id, index:this.index, type:t}); }
  get SubPathIndex() { return this.info.subpath + 1; }
  get IsEnding() { return this.info.ending; }
  SetPosition(x, y) { __call('node_move', {id:this.id, index:this.index, x:x, y:y}); }
  Move(dx, dy) { __call('node_move', {id:this.id, index:this.index, dx:dx, dy:dy}); }
  Delete() { __call('node_delete', {id:this.id, index:this.index}); }
  AddNodeBefore(t) { __call('node_add', {id:this.id, index:this.index, t:t===undefined?0.5:t}); }
}
class Nodes {
  constructor(id, subpath) { this.id = id; this.subpath = subpath; }
  get list() { const all = __call('curve_nodes', {id:this.id}).nodes; return this.subpath === null ? all : all.filter(n => n.subpath === this.subpath); }
  get Count() { return this.list.length; }
  Item(i) { const n = this.list[i-1]; return n ? new Node(this.id, n.index) : null; }
  get First() { return this.Item(1); }
  get Last() { return this.Item(this.Count); }
  forEach(f) { this.list.map(n => new Node(this.id, n.index)).forEach(f); }
}
class SubPath {
  constructor(id, i) { this.id = id; this.i = i; this.Nodes = new Nodes(id, i); }
  get Closed() { return __call('curve_info', {id:this.id}).subpaths[this.i].closed; }
  get Index() { return this.i + 1; }
}
class SubPaths {
  constructor(id) { this.id = id; }
  get Count() { return __call('curve_info', {id:this.id}).subpaths.length; }
  Item(i) { return i >= 1 && i <= this.Count ? new SubPath(this.id, i-1) : null; }
  forEach(f) { for (let i = 1; i <= this.Count; i++) f(this.Item(i)); }
}
class Curve {
  constructor(id) { this.id = id; this.Nodes = new Nodes(id, null); this.SubPaths = new SubPaths(id); }
  get Closed() { return __call('curve_info', {id:this.id}).subpaths.every(p => p.closed); }
}
class ShapeRange {
  constructor(ids) { this.ids = ids; }
  get Count() { return this.ids.length; }
  Item(i) { return new Shape(this.ids[i-1]); }
  get Shapes() { return this.ids.map(id => new Shape(id)); }
  forEach(f) { this.Shapes.forEach(f); }
  Delete() { this.ids.forEach(id => __call('delete', {id:id})); }
  Move(dx, dy) { this.ids.forEach(id => __call('move', {id:id, dx:dx, dy:dy})); }
  Group() { return new Shape(__call('group', {ids:this.ids}).id); }
  Ungroup() { this.ids.forEach(id => __call('ungroup', {id:id})); }
  Combine() { __call('combine', {ids:this.ids}); }
  AlignLeft() { __call('align', {ids:this.ids, how:'left'}); }
  AlignRight() { __call('align', {ids:this.ids, how:'right'}); }
  AlignTop() { __call('align', {ids:this.ids, how:'top'}); }
  AlignBottom() { __call('align', {ids:this.ids, how:'bottom'}); }
  AlignCentersHorizontally() { __call('align', {ids:this.ids, how:'center_h'}); }
  AlignCentersVertically() { __call('align', {ids:this.ids, how:'center_v'}); }
  Distribute(how, to) { __call('distribute', {ids:this.ids, how:how, to:to || 'selection'}); }
  DistributeLeft() { this.Distribute('left'); }
  DistributeRight() { this.Distribute('right'); }
  DistributeTop() { this.Distribute('top'); }
  DistributeBottom() { this.Distribute('bottom'); }
  DistributeCentersHorizontally() { this.Distribute('center_h'); }
  DistributeCentersVertically() { this.Distribute('center_v'); }
  DistributeSpacingHorizontally() { this.Distribute('spacing_h'); }
  DistributeSpacingVertically() { this.Distribute('spacing_v'); }
  CreateSelection() { __call('select', {ids:this.ids, add:false}); }
  ReplaceText(needle, replacement, options) {
    const o = options || {};
    return __call('replace_text', {ids:this.ids, needle:needle, replacement:replacement,
      match_case:!!o.matchCase, whole_word:!!o.wholeWord}).replaced;
  }
}
class Shapes {
  constructor(layer) { this.layer = layer; }
  get ids() { return __call('shapes', {layer:this.layer}).ids; }
  get Count() { return this.ids.length; }
  Item(i) { return new Shape(this.ids[i-1]); }
  get All() { return new ShapeRange(this.ids); }
  FindShape(name) { const id = __call('find', {layer:this.layer, name:name}).id; return id ? new Shape(id) : null; }
  forEach(f) { this.ids.forEach(id => f(new Shape(id))); }
}
class Layer {
  constructor(id) { this.id = id; this.Shapes = new Shapes(id); }
  get Name() { return __call('layer_info', {layer:this.id}).name; }
  set Name(n) { __call('set_layer_name', {layer:this.id, name:n}); }
  get Visible() { return __call('layer_info', {layer:this.id}).visible; }
  set Visible(v) { __call('set_layer_visible', {layer:this.id, visible:v}); }
  get Locked() { return __call('layer_info', {layer:this.id}).locked; }
  set Locked(v) { __call('set_layer_locked', {layer:this.id, locked:v}); }
  Delete() { __call('delete_layer', {layer:this.id}); }
  Activate() { __call('activate_layer', {layer:this.id}); }
  CreateRectangle(x, y, w, h, radius) { return new Shape(__call('create_rect', {layer:this.id, x:x, y:y, w:w, h:h, radius:radius||0}).id); }
  CreateEllipse(x, y, w, h) { return new Shape(__call('create_ellipse', {layer:this.id, x:x, y:y, w:w, h:h}).id); }
  CreatePolygon(x, y, w, h, points) { return new Shape(__call('create_polygon', {layer:this.id, x:x, y:y, w:w, h:h, points:points||5, sharpness:0}).id); }
  CreateStar(x, y, w, h, points, sharpness) { return new Shape(__call('create_polygon', {layer:this.id, x:x, y:y, w:w, h:h, points:points||5, sharpness:sharpness===undefined?0.5:sharpness}).id); }
  CreateArtisticText(x, y, text, font, size) { return new Shape(__call('create_text', {layer:this.id, x:x, y:y, text:text, font:font||'', size:size||24, frame:null}).id); }
  CreateParagraphText(x, y, w, h, text, font, size) { return new Shape(__call('create_text', {layer:this.id, x:x, y:y, text:text, font:font||'', size:size||12, frame:[w, h]}).id); }
  CreateLineSegment(x1, y1, x2, y2) { return new Shape(__call('create_curve', {layer:this.id, points:[[x1,y1],[x2,y2]], closed:false}).id); }
  CreateCurve(points, closed) { return new Shape(__call('create_curve', {layer:this.id, points:points, closed:!!closed}).id); }
  CreateCurveSegment(points) { return this.CreateCurve(points, false); }
  CreatePath(d) { return new Shape(__call('create_path', {layer:this.id, d:d}).id); }
  ImportBitmap(path) { return new Shape(__call('import_bitmap', {layer:this.id, path:path}).id); }
}
class Layers {
  constructor(page) { this.page = page; }
  get ids() { return __call('layers', {page:this.page}).ids; }
  get Count() { return this.ids.length; }
  Item(i) { return new Layer(this.ids[i-1]); }
  forEach(f) { this.ids.forEach(id => f(new Layer(id))); }
  Find(name) { const id = __call('find_layer', {page:this.page, name:name}).id; return id ? new Layer(id) : null; }
}
class Page {
  constructor(id) { this.id = id; this.Layers = new Layers(id); }
  get Name() { return __call('page_info', {page:this.id}).name; }
  set Name(n) { __call('rename_page', {page:this.id, name:n}); }
  get SizeWidth() { return __call('page_info', {page:this.id}).w; }
  get SizeHeight() { return __call('page_info', {page:this.id}).h; }
  SetSize(w, h) { __call('set_page_size', {page:this.id, w:w, h:h}); }
  Delete() { __call('delete_page', {page:this.id}); }
  Duplicate(before) { return new Page(__call('duplicate_page', {page:this.id, before:!!before}).id); }
  get ActiveLayer() { return new Layer(__call('active_layer', {page:this.id}).id); }
  get Shapes() { const page = this.id; return { get All() { return new ShapeRange(__call('page_shapes', {page:page}).ids); }, get Count() { return __call('page_shapes', {page:page}).ids.length; } }; }
  ReplaceText(needle, replacement, options) {
    const o = options || {};
    return __call('replace_text', {needle:needle, replacement:replacement,
      match_case:!!o.matchCase, whole_word:!!o.wholeWord}).replaced;
  }
  CreateLayer(name) { return new Layer(__call('create_layer', {page:this.id, name:name}).id); }
  Activate() { __call('activate_page', {page:this.id}); }
  get Index() { return __call('page_info', {page:this.id}).index; }
}
class Pages {
  get ids() { return __call('pages', {}).ids; }
  get Count() { return this.ids.length; }
  Item(i) { return new Page(this.ids[i-1]); }
  forEach(f) { this.ids.forEach(id => f(new Page(id))); }
}
class Document {
  constructor() { this.Pages = new Pages(); }
  get Name() { return __call('doc_info', {}).title; }
  get ActivePage() { return new Page(__call('doc_info', {}).page); }
  get ActiveLayer() { return this.ActivePage.ActiveLayer; }
  get Selection() { return new ShapeRange(__call('selection', {}).ids); }
  get SelectionRange() { return this.Selection; }
  ClearSelection() { __call('select', {ids:[], add:false}); }
  SelectAll() { __call('select_all', {}); }
  ReplaceText(needle, replacement, options) {
    return this.ActivePage.ReplaceText(needle, replacement, options);
  }
  FindByProperties(query) { return __call('find_by_properties', query).ids.map(i => new Shape(i)); }
  ReplaceByProperties(query) { return __call('replace_by_properties', query).replaced; }
  SelectByProperties(query) {
    const s = this.FindByProperties(query);
    if (s.length) new ShapeRange(s.map(x => x.id)).CreateSelection();
    return s.length;
  }
  AddPages(n) { for (let i = 0; i < (n||1); i++) __call('add_page', {}); return this.Pages.Item(this.Pages.Count); }
  CreateRectangle(x, y, w, h, r) { return this.ActiveLayer.CreateRectangle(x, y, w, h, r); }
  CreateEllipse(x, y, w, h) { return this.ActiveLayer.CreateEllipse(x, y, w, h); }
  CreateArtisticText(x, y, t, f, s) { return this.ActiveLayer.CreateArtisticText(x, y, t, f, s); }
  CreatePath(d) { return this.ActiveLayer.CreatePath(d); }
  Export(path, format) { __call('export', {path:path, format:format||''}); }
  BeginCommandGroup(name) { }
  EndCommandGroup() { }
}
const Application = {
  get ActiveDocument() { return new Document(); },
  get ActivePage() { return new Document().ActivePage; },
  get ActiveLayer() { return new Document().ActiveLayer; },
  get ActiveShape() { const s = new Document().Selection; return s.Count ? s.Item(1) : null; },
  get ActiveSelection() { return new Document().Selection; },
  get Version() { return __call('version', {}).version; },
  get Name() { return 'TraceDraw'; },
  CreateColor: Color,
};
// The active document, page and layer are read when a script uses them, not
// bound once: a script that activates another page would otherwise go on
// adding objects to the first one's layer. A getter on the global object is
// the only way to keep them live while leaving the names the scripts use
// unchanged.
for (const name of ['ActiveDocument', 'ActivePage', 'ActiveLayer', 'ActiveSelection']) {
  Object.defineProperty(globalThis, name, {
    get: () => Application[name], configurable: true,
  });
}
"#;

/// Run a script against the app; returns the printed lines.
pub fn run(app: &mut App, src: &str) -> Result<Vec<String>, String> {
    run_with(app, src, Trust::User, "Script").map(|(output, _)| output)
}

/// Run JavaScript written against the object model VBA macros use (what
/// the VBA compiler produces): `Application`, `ActiveDocument`, `Shapes`
/// with 1-based items, lengths in `ActiveDocument.Unit` and the `cdr*`
/// constants (`scripting/vg.rs`). Its changes are one undo step named
/// `label`, kept up to an error like a script's. Returns the printed
/// lines.
// Called by the VBA macro runner (another part of the app).
#[allow(dead_code)]
pub fn run_vgcore(app: &mut App, src: &str, label: &'static str) -> Result<Vec<String>, String> {
    run_host(app, src, Trust::User, label, true).map(|(output, _)| output)
}

/// Run a script an AI service wrote: no file access, loops, call depth
/// and changes limited, and nothing applied unless the whole script runs.
/// Its changes are one undo step named `label`. Returns the printed lines
/// and the number of changes.
pub fn run_untrusted(
    app: &mut App,
    src: &str,
    label: &'static str,
) -> Result<(Vec<String>, usize), String> {
    run_with(app, src, Trust::Untrusted, label)
}

fn host_for(app: &App, trust: Trust) -> HostInner {
    HostInner {
        doc: app.engine.document().clone(),
        page: app.page,
        layer: app.active_layer(),
        selection: app.selection.clone(),
        commands: Vec::new(),
        output: Vec::new(),
        next_id: app.engine.document().ids().peek_next(),
        trust,
        file: app.file.as_ref().map(|f| f.display().to_string()),
        vg: false,
        view: vg::AppView::of(app),
        status: None,
        simulate_overprints: None,
        clip: Vec::new(),
    }
}

/// What a script evaluation gave: the completion value (or the error)
/// and the host with the recorded commands and printed lines.
struct Evaluated {
    inner: HostInner,
    result: Result<JsValue, String>,
    /// The completion value as text, when it is a string.
    text: Option<String>,
    shown: String,
}

/// Evaluate the prelude, `src`, then `call` (when given) with the global
/// `__input` set to `input`.
fn evaluate(
    inner: HostInner,
    src: &str,
    call: Option<&str>,
    input: Option<&str>,
) -> Result<Evaluated, String> {
    evaluate_with(inner, src, call, input, None)
}

fn evaluate_with(
    inner: HostInner,
    src: &str,
    call: Option<&str>,
    input: Option<&str>,
    debug: Option<DebugState>,
) -> Result<Evaluated, String> {
    let trust = inner.trust;
    let trust_vg = inner.vg;
    let mut ctx = Context::default();
    if let Some(debug) = debug {
        ctx.insert_data(DebugHandle(std::rc::Rc::new(debug)));
        ctx.register_global_callable(
            js_string!("__bp"),
            1,
            NativeFunction::from_fn_ptr(debug_check),
        )
        .map_err(|e| e.to_string())?;
        ctx.register_global_callable(
            js_string!("__pause"),
            2,
            NativeFunction::from_fn_ptr(debug_pause),
        )
        .map_err(|e| e.to_string())?;
    }
    if trust == Trust::Untrusted {
        let limits = ctx.runtime_limits_mut();
        limits.set_loop_iteration_limit(UNTRUSTED_LOOP_LIMIT);
        limits.set_recursion_limit(UNTRUSTED_RECURSION_LIMIT);
    }
    ctx.insert_data(Host(RefCell::new(inner)));
    let dispatcher = NativeFunction::from_fn_ptr(dispatch);
    let global = ObjectInitializer::new(&mut ctx).build();
    let _ = global;
    ctx.register_global_callable(js_string!("__td"), 2, dispatcher)
        .map_err(|e| e.to_string())?;
    if let Some(input) = input {
        ctx.register_global_property(
            js_string!("__input"),
            JsValue::from(boa_engine::JsString::from(input)),
            boa_engine::property::Attribute::all(),
        )
        .map_err(|e| e.to_string())?;
    }
    let prelude = if trust_vg { vg::prelude() } else { PRELUDE };
    ctx.eval(Source::from_bytes(COMMON))
        .map_err(|e| format!("prelude: {e}"))?;
    ctx.eval(Source::from_bytes(prelude))
        .map_err(|e| format!("prelude: {e}"))?;
    let mut result = ctx.eval(Source::from_bytes(src)).map_err(|e| e.to_string());
    if let (Ok(_), Some(call)) = (&result, call) {
        result = ctx
            .eval(Source::from_bytes(call))
            .map_err(|e| e.to_string());
    }
    let text = match &result {
        Ok(v) => v.as_string().map(|s| s.to_std_string_escaped()),
        Err(_) => None,
    };
    let shown = match &result {
        Ok(v) if !v.is_undefined() => v.display().to_string(),
        _ => String::new(),
    };
    let host = ctx
        .remove_data::<Host>()
        .ok_or_else(|| "script host lost".to_string())?;
    Ok(Evaluated {
        inner: host.0.into_inner(),
        result,
        text,
        shown,
    })
}

fn run_with(
    app: &mut App,
    src: &str,
    trust: Trust,
    label: &'static str,
) -> Result<(Vec<String>, usize), String> {
    run_host(app, src, trust, label, false)
}

fn run_host(
    app: &mut App,
    src: &str,
    trust: Trust,
    label: &'static str,
    vg: bool,
) -> Result<(Vec<String>, usize), String> {
    let mut host = host_for(app, trust);
    host.vg = vg;
    let Evaluated {
        inner,
        result,
        shown,
        ..
    } = evaluate(host, src, None, None)?;
    let mut output = inner.output;
    match result {
        Ok(_) => {
            // Echo the completion value only when nothing was printed,
            // so one-liners such as `ActiveDocument.Pages.Count` show a result.
            if output.is_empty() && !shown.is_empty() {
                output.push(shown);
            }
        }
        Err(e) => {
            // The user's scripts keep what ran before the error.
            if trust == Trust::User {
                apply(app, inner.commands, inner.selection, inner.page, label);
            }
            return Err(e);
        }
    }
    let changes = inner.commands.len();
    if let Some(s) = inner.status {
        app.status = s;
    }
    if let Some(on) = inner.simulate_overprints {
        app.simulate_overprints = on;
    }
    apply(app, inner.commands, inner.selection, inner.page, label);
    Ok((output, changes))
}

/// A script that reads or writes a file format: it defines the global
/// `TraceDrawFilter` with a `name`, its `extensions` and an `Import(text)`
/// and/or `Export()` function.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScriptFilter {
    pub name: String,
    /// Lower case, without the dot.
    pub extensions: Vec<String>,
    pub import: bool,
    pub export: bool,
    pub source: String,
}

impl ScriptFilter {
    /// For file dialogs: "Name (*.a, *.b)".
    pub fn label(&self) -> String {
        let exts: Vec<String> = self.extensions.iter().map(|e| format!("*.{e}")).collect();
        format!("{} ({})", self.name, exts.join(", "))
    }

    pub fn handles(&self, ext: &str) -> bool {
        self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }
}

/// The names the object model for macros defines (globals, classes,
/// constants): the VBA compiler leaves them to the host. Worked out once.
pub fn vg_global_names() -> Vec<String> {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| {
            let mut inner = blank_host(Document::default(), Trust::User);
            inner.vg = true;
            let mut names: Vec<String> = evaluate_with(
                inner,
                "Object.getOwnPropertyNames(globalThis).join('\\n')",
                None,
                None,
                None,
            )
            .ok()
            .and_then(|e| e.text)
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default();
            for src in [COMMON, vg::PRELUDE] {
                for line in src.lines() {
                    for kw in ["const ", "let ", "class ", "function "] {
                        if let Some(rest) = line.strip_prefix(kw) {
                            let n: String = rest
                                .chars()
                                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                                .collect();
                            if !n.is_empty() {
                                names.push(n);
                            }
                        }
                    }
                }
            }
            names.retain(|n| !n.starts_with("__") && !n.is_empty());
            names.sort();
            names.dedup();
            names
        })
        .clone()
}

fn blank_host(doc: Document, trust: Trust) -> HostInner {
    let page = doc.pages.first().map(|p| p.id).unwrap_or(PageId(0));
    let layer = doc
        .pages
        .first()
        .and_then(|p| p.layers.first())
        .map(|l| l.id);
    let next_id = doc.ids().peek_next();
    HostInner {
        doc,
        page,
        layer,
        selection: Vec::new(),
        commands: Vec::new(),
        output: Vec::new(),
        next_id,
        trust,
        file: None,
        vg: false,
        view: vg::AppView::default(),
        status: None,
        simulate_overprints: None,
        clip: Vec::new(),
    }
}

/// Read a filter script: what `TraceDrawFilter` says about itself. The
/// script runs with the limits of untrusted scripts against an empty
/// drawing, and nothing it does is kept.
pub fn filter_info(src: &str) -> Result<ScriptFilter, String> {
    let doc = Document::new("Filter", tracedraw_core::Size::new(210.0, 297.0));
    let ev = evaluate(
        blank_host(doc, Trust::Untrusted),
        src,
        Some(
            "typeof TraceDrawFilter === 'undefined' ? '' : JSON.stringify({\
             name: String(TraceDrawFilter.name || ''),\
             extensions: [].concat(TraceDrawFilter.extensions || []).map(String),\
             import: typeof TraceDrawFilter.Import === 'function',\
             export: typeof TraceDrawFilter.Export === 'function'})",
        ),
        None,
    )?;
    ev.result?;
    let text = ev.text.unwrap_or_default();
    if text.is_empty() {
        return Err("the script defines no TraceDrawFilter".into());
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let name = v["name"].as_str().unwrap_or("").trim().to_string();
    let extensions: Vec<String> = v["extensions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str())
                .map(|e| {
                    e.trim()
                        .trim_start_matches("*.")
                        .trim_start_matches('.')
                        .to_ascii_lowercase()
                })
                .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
                .collect()
        })
        .unwrap_or_default();
    let import = v["import"].as_bool().unwrap_or(false);
    let export = v["export"].as_bool().unwrap_or(false);
    if name.is_empty() {
        return Err("TraceDrawFilter has no name".into());
    }
    if extensions.is_empty() {
        return Err("TraceDrawFilter has no extensions".into());
    }
    if !import && !export {
        return Err("TraceDrawFilter has neither Import nor Export".into());
    }
    Ok(ScriptFilter {
        name,
        extensions,
        import,
        export,
        source: src.to_string(),
    })
}

/// Import `text` through a filter into the active layer: its objects are
/// added as one undo step and selected. Nothing is kept when the filter
/// fails. Returns the printed lines.
pub fn run_filter_import(
    app: &mut App,
    filter: &ScriptFilter,
    text: &str,
) -> Result<Vec<String>, String> {
    let ev = evaluate(
        host_for(app, Trust::User),
        &filter.source,
        Some("TraceDrawFilter.Import(__input)"),
        Some(text),
    )?;
    ev.result?;
    let inner = ev.inner;
    let added: Vec<ShapeId> = inner
        .commands
        .iter()
        .filter_map(|c| match c {
            Command::AddShape { shape, .. } => Some(shape.id),
            _ => None,
        })
        .collect();
    apply(app, inner.commands, added, inner.page, "Import");
    Ok(inner.output)
}

/// Export `doc` through a filter: the text its `Export()` returns. The
/// filter sees `doc` as the active document, its first page active; what
/// it changes is dropped.
pub fn run_filter_export(doc: Document, filter: &ScriptFilter) -> Result<String, String> {
    let ev = evaluate(
        blank_host(doc, Trust::User),
        &filter.source,
        Some("TraceDrawFilter.Export()"),
        None,
    )?;
    ev.result?;
    ev.text
        .ok_or_else(|| format!("{}: Export() returned no text", filter.name))
}

/// Control sent to a script paused in the debugger.
#[derive(Debug, Clone, PartialEq)]
pub enum DebugCtl {
    Continue,
    /// Run to the next statement that has a check.
    Step,
    /// Throw an error at the pause (and at every check after it).
    Stop,
    /// Evaluate an expression in the paused scope.
    Eval(String),
}

/// What a debugged script reports.
#[derive(Debug, Clone, PartialEq)]
pub enum DebugEvent {
    /// Paused before the statement on this line; the lines printed since
    /// the last pause.
    Paused { line: u32, output: Vec<String> },
    /// The result (or the error) of an [`DebugCtl::Eval`].
    Value(String),
}

/// A script running under the debugger on a worker thread (desktop).
pub struct DebugRun {
    pub ctl: std::sync::mpsc::Sender<DebugCtl>,
    pub events: std::sync::mpsc::Receiver<DebugEvent>,
    /// The lines to stop at; the panel may change them while it runs.
    pub breakpoints: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u32>>>,
    /// The lines that can hold a breakpoint.
    pub lines: Vec<u32>,
    handle: std::thread::JoinHandle<Result<(HostInner, Option<String>), String>>,
}

impl DebugRun {
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

struct DebugState {
    ctl: std::sync::mpsc::Receiver<DebugCtl>,
    events: std::sync::mpsc::Sender<DebugEvent>,
    breakpoints: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<u32>>>,
    stepping: std::cell::Cell<bool>,
    stopped: std::cell::Cell<bool>,
}

struct DebugHandle(std::rc::Rc<DebugState>);
impl boa_engine::gc::Finalize for DebugHandle {}
// The debugger state holds no garbage-collected values.
unsafe impl boa_engine::gc::Trace for DebugHandle {
    boa_engine::gc::empty_trace!();
}
impl JsData for DebugHandle {}

fn debug_state(ctx: &Context) -> Option<std::rc::Rc<DebugState>> {
    ctx.get_data::<DebugHandle>().map(|h| h.0.clone())
}

fn stopped_error() -> boa_engine::JsError {
    boa_engine::JsNativeError::error()
        .with_message("stopped in the debugger")
        .into()
}

/// `__bp(line)`: whether to pause before the statement on `line`.
fn debug_check(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let Some(st) = debug_state(ctx) else {
        return Ok(JsValue::from(false));
    };
    if st.stopped.get() {
        return Err(stopped_error());
    }
    let line = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let hit = st.stepping.get()
        || st
            .breakpoints
            .lock()
            .map(|b| b.contains(&line))
            .unwrap_or(false);
    Ok(JsValue::from(hit))
}

/// `__pause(line, evaluate)`: report the pause and wait for the panel.
fn debug_pause(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let Some(st) = debug_state(ctx) else {
        return Ok(JsValue::undefined());
    };
    let line = args.first().and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let evaluate = args.get(1).and_then(|v| v.as_callable()).cloned();
    let output = ctx
        .get_data::<Host>()
        .map(|h| h.0.borrow_mut().output.drain(..).collect())
        .unwrap_or_default();
    if st.events.send(DebugEvent::Paused { line, output }).is_err() {
        st.stopped.set(true);
        return Err(stopped_error());
    }
    loop {
        match st.ctl.recv() {
            Ok(DebugCtl::Continue) => {
                st.stepping.set(false);
                return Ok(JsValue::undefined());
            }
            Ok(DebugCtl::Step) => {
                st.stepping.set(true);
                return Ok(JsValue::undefined());
            }
            Ok(DebugCtl::Eval(expr)) => {
                let text = match &evaluate {
                    Some(f) => match f.call(
                        &JsValue::undefined(),
                        &[JsValue::from(boa_engine::JsString::from(expr.as_str()))],
                        ctx,
                    ) {
                        Ok(v) => v.display().to_string(),
                        Err(e) => e.to_string(),
                    },
                    None => String::new(),
                };
                if st.events.send(DebugEvent::Value(text)).is_err() {
                    st.stopped.set(true);
                    return Err(stopped_error());
                }
            }
            Ok(DebugCtl::Stop) | Err(_) => {
                st.stopped.set(true);
                return Err(stopped_error());
            }
        }
    }
}

/// Run a script under the debugger on a worker thread: it pauses at the
/// `breakpoints` (lines) and, after Step, at every statement. Desktop only.
pub fn start_debug(app: &App, src: &str, breakpoints: &[u32]) -> Result<DebugRun, String> {
    start_debug_with(app, src, breakpoints, false)
}

/// [`start_debug`] for JavaScript compiled from a VBA macro: it runs
/// against the object model VBA macros use (`scripting/vg.rs`).
#[allow(dead_code)]
pub fn start_debug_vgcore(app: &App, src: &str, breakpoints: &[u32]) -> Result<DebugRun, String> {
    start_debug_with(app, src, breakpoints, true)
}

fn start_debug_with(
    app: &App,
    src: &str,
    breakpoints: &[u32],
    vg: bool,
) -> Result<DebugRun, String> {
    use std::sync::{mpsc::channel, Arc, Mutex};
    let (code, lines) = crate::script_debug::instrument(src);
    let set: std::collections::BTreeSet<u32> = breakpoints
        .iter()
        .filter_map(|l| crate::script_debug::effective_line(&lines, *l))
        .collect();
    let breakpoints = Arc::new(Mutex::new(set));
    let (ctl_tx, ctl_rx) = channel();
    let (ev_tx, ev_rx) = channel();
    let mut inner = host_for(app, Trust::User);
    inner.vg = vg;
    let bps = breakpoints.clone();
    let handle = std::thread::Builder::new()
        .name("script-debug".into())
        .spawn(move || {
            let state = DebugState {
                ctl: ctl_rx,
                events: ev_tx,
                breakpoints: bps,
                stepping: std::cell::Cell::new(false),
                stopped: std::cell::Cell::new(false),
            };
            evaluate_with(inner, &code, None, None, Some(state))
                .map(|ev| (ev.inner, ev.result.err()))
        })
        .map_err(|e| e.to_string())?;
    Ok(DebugRun {
        ctl: ctl_tx,
        events: ev_rx,
        breakpoints,
        lines,
        handle,
    })
}

/// A finished debug run: its changes are applied like a script run's
/// (kept up to an error); the printed lines and the error, if any.
pub fn finish_debug(app: &mut App, run: DebugRun) -> (Vec<String>, Option<String>) {
    let (inner, error) = match run.handle.join() {
        Ok(Ok(done)) => done,
        Ok(Err(e)) => return (Vec::new(), Some(e)),
        Err(_) => return (Vec::new(), Some("script host lost".into())),
    };
    let output = inner.output;
    apply(app, inner.commands, inner.selection, inner.page, "Script");
    (output, error)
}

fn apply(
    app: &mut App,
    commands: Vec<Command>,
    selection: Vec<ShapeId>,
    page: PageId,
    label: &'static str,
) {
    if !commands.is_empty() {
        if let Err(e) = app.engine.run_batch(label, &commands) {
            app.status = e.to_string();
        }
    }
    app.page = page;
    app.selection = selection
        .into_iter()
        .filter(|id| app.doc().find_shape(*id).is_some())
        .collect();
}

fn dispatch(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let method = args
        .first()
        .map(|v| v.to_string(ctx))
        .transpose()?
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_default();
    let arg_json = args
        .get(1)
        .map(|v| v.to_string(ctx))
        .transpose()?
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_else(|| "{}".into());
    let a: Value = serde_json::from_str(&arg_json).unwrap_or(Value::Null);
    let Some(host) = ctx.get_data::<Host>() else {
        return Err(boa_engine::JsNativeError::error()
            .with_message("no host")
            .into());
    };
    let mut h = host.0.borrow_mut();
    let out = h.call(&method, &a).map_err(|e| {
        boa_engine::JsError::from(boa_engine::JsNativeError::error().with_message(e))
    })?;
    Ok(JsValue::from(js_string!(out.to_string())))
}

fn id_arg(a: &Value, key: &str) -> Result<u64, String> {
    a.get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("missing {key}"))
}

fn num(a: &Value, key: &str, default: f64) -> f64 {
    a.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn color_from(v: &Value) -> Option<Color> {
    serde_json::from_value(v.clone()).ok()
}

/// The type named by a script's `type`, in the words the object model uses
/// for `Shape.Type`, so a script writes the same name it reads.
fn object_type_from(name: &str) -> Result<tracedraw_core::find::ObjectType, String> {
    use tracedraw_core::find::ObjectType as T;
    Ok(match name.to_ascii_lowercase().as_str() {
        "rectangle" => T::Rectangle,
        "ellipse" => T::Ellipse,
        "polygon" | "star" | "complexstar" => T::Polygon,
        "curve" => T::Curve,
        "text" | "artistictext" => T::ArtisticText,
        "paragraphtext" => T::ParagraphText,
        "group" => T::Group,
        "bitmap" => T::Bitmap,
        "clipframe" => T::ClipFrame,
        "table" => T::Table,
        "symbol" => T::Symbol,
        other => return Err(format!("no object type {other:?}")),
    })
}

/// A colour criterion from a script: `null` for "no paint", a colour
/// object for a colour, and nothing at all for "do not look at this".
///
/// A script's `Color` keeps its value under `v` (`new Color(...)` in the
/// prelude), while the host takes a colour as `{model, ...}`; both are
/// accepted so `{fill: Color.RGB(255,0,0)}` and a `Color` kept in a
/// variable work the same.
fn color_query_from(v: Option<&Value>) -> Result<Option<tracedraw_core::find::ColorQuery>, String> {
    use tracedraw_core::find::ColorQuery;
    let Some(v) = v else { return Ok(None) };
    if v.is_null() {
        return Ok(Some(ColorQuery::Nothing));
    }
    let inner = v.get("v").unwrap_or(v);
    let c = color_from(inner).ok_or("not a colour")?;
    Ok(Some(ColorQuery::Color(c)))
}

/// The criteria a script gives, in the words `find_by_properties` and
/// `replace_by_properties` take.
fn query_from_json(a: &Value) -> Result<tracedraw_core::find::ObjectQuery, String> {
    let mut q = tracedraw_core::find::ObjectQuery::default();
    if let Some(t) = a.get("type").and_then(|v| v.as_str()) {
        q.object_type = Some(object_type_from(t)?);
    }
    q.fill = color_query_from(a.get("fill"))?;
    q.outline = color_query_from(a.get("outline"))?;
    if let Some(w) = a.get("outlineWidth").and_then(|v| v.as_f64()) {
        q.outline_width = Some(w);
    }
    if let Some(f) = a.get("font").and_then(|v| v.as_str()) {
        if !f.trim().is_empty() {
            q.font = Some(f.trim().to_string());
        }
    }
    Ok(q)
}

/// What to put in place of what was found. The colours take the same two
/// shapes `color_query_from` takes.
fn replacement_from_json(a: &Value) -> Result<tracedraw_core::find::Replacement, String> {
    let mut r = tracedraw_core::find::Replacement::default();
    if let Some(v) = a.get("new_fill") {
        r.fill = color_from(v.get("v").unwrap_or(v));
    }
    if let Some(v) = a.get("new_outline") {
        r.outline = color_from(v.get("v").unwrap_or(v));
    }
    if let Some(w) = a.get("new_outline_width").and_then(|v| v.as_f64()) {
        r.outline_width = Some(w);
    }
    if let Some(f) = a.get("new_font").and_then(|v| v.as_str()) {
        if !f.trim().is_empty() {
            r.font = Some(f.trim().to_string());
        }
    }
    Ok(r)
}

impl HostInner {
    fn run(&mut self, cmd: Command) -> Result<(), String> {
        if self.trust == Trust::Untrusted && self.commands.len() >= UNTRUSTED_MAX_CHANGES {
            return Err(format!(
                "more than {UNTRUSTED_MAX_CHANGES} changes; draw fewer objects"
            ));
        }
        cmd.apply(&mut self.doc).map_err(|e| e.to_string())?;
        self.commands.push(cmd);
        Ok(())
    }

    fn new_id(&mut self) -> ShapeId {
        // Minted from the drawing's own counter: adding the shape when the
        // batch replays keeps the app's counter past it (see
        // `Command::AddShape`).
        let id = self.doc.ids_mut().shape();
        self.next_id = id.raw() + 1;
        id
    }

    fn shape(&self, id: u64) -> Result<Shape, String> {
        self.doc
            .find_shape(ShapeId(id))
            .cloned()
            .ok_or_else(|| format!("no shape {id}"))
    }

    /// Drop from the selection the objects a command took away, so a later
    /// call does not name one of them.
    fn drop_missing_selection(&mut self) {
        let alive: Vec<ShapeId> = self
            .selection
            .iter()
            .copied()
            .filter(|id| self.doc.find_shape(*id).is_some())
            .collect();
        self.selection = alive;
    }

    fn layer_for(&self, a: &Value) -> Result<LayerId, String> {
        if let Some(l) = a.get("layer").and_then(|v| v.as_u64()) {
            return Ok(LayerId(l));
        }
        // The layer the script last chose, but only while it is on the page
        // the script is on: after `Page.Activate()` the old page's layer
        // would take the new objects to a page nobody asked for.
        if let Some(l) = self.layer {
            if self
                .doc
                .page(self.page)
                .is_ok_and(|p| p.layers.iter().any(|x| x.id == l))
            {
                return Ok(l);
            }
        }
        self.doc
            .page(self.page)
            .ok()
            .and_then(|p| p.layers.last().map(|l| l.id))
            .ok_or_else(|| "no active layer".to_string())
    }

    fn add(&mut self, layer: LayerId, mut shape: Shape) -> Result<Value, String> {
        shape.id = self.new_id();
        let id = shape.id;
        self.run(Command::AddShape { layer, shape })?;
        // CorelDRAW selects what a Create* method makes: macros gather
        // the new shapes from `ActiveSelectionRange` (#2165).
        self.selection = vec![id];
        Ok(json!({"id": id.raw()}))
    }

    fn call(&mut self, method: &str, a: &Value) -> Result<Value, String> {
        if self.trust == Trust::Untrusted && FILE_METHODS.contains(&method) {
            return Err(format!("{method} is not available here: no file access"));
        }
        Ok(match method {
            "version" => json!({"version": env!("CARGO_PKG_VERSION")}),
            "log" => {
                self.output.push(
                    a.get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                );
                json!({})
            }
            "doc_info" => json!({"title": self.doc.title, "page": self.page.raw()}),
            "pages" => {
                json!({"ids": self.doc.pages.iter().map(|p| p.id.raw()).collect::<Vec<_>>()})
            }
            "page_info" => {
                let pid = PageId(id_arg(a, "page")?);
                let (i, p) = self
                    .doc
                    .pages
                    .iter()
                    .enumerate()
                    .find(|(_, p)| p.id == pid)
                    .ok_or("no page")?;
                json!({"name": p.name, "w": p.size.width, "h": p.size.height, "index": i + 1})
            }
            "set_page_size" => {
                let page = PageId(id_arg(a, "page")?);
                let size = tracedraw_core::Size::new(num(a, "w", 210.0), num(a, "h", 297.0));
                self.run(Command::ResizePage { page, size })?;
                json!({})
            }
            "activate_page" => {
                self.page = PageId(id_arg(a, "page")?);
                json!({})
            }
            "add_page" => {
                let size = self
                    .doc
                    .page(self.page)
                    .map(|p| p.size)
                    .unwrap_or(tracedraw_core::document::paper::A4);
                self.run(Command::AddPage {
                    name: None,
                    size,
                    layer: None,
                })?;
                json!({})
            }
            "delete_page" => {
                let page = PageId(id_arg(a, "page")?);
                self.run(Command::DeletePage { page })?;
                // The page the script worked on may be the one taken out:
                // the script goes on the first page left rather than on an
                // id that no longer names anything.
                if self.page == page {
                    if let Some(first) = self.doc.pages.first() {
                        self.page = first.id;
                    }
                }
                self.drop_missing_selection();
                json!({})
            }
            "rename_page" => {
                let page = PageId(id_arg(a, "page")?);
                let name = a
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                self.run(Command::RenamePage { page, name })?;
                json!({})
            }
            "duplicate_page" => {
                let page = PageId(id_arg(a, "page")?);
                let before: Vec<PageId> = self.doc.pages.iter().map(|p| p.id).collect();
                self.run(Command::DuplicatePage {
                    page,
                    before: a.get("before").and_then(|v| v.as_bool()).unwrap_or(false),
                    contents: a.get("contents").and_then(|v| v.as_bool()).unwrap_or(true),
                })?;
                let new = self
                    .doc
                    .pages
                    .iter()
                    .map(|p| p.id)
                    .find(|id| !before.contains(id))
                    .ok_or("the page was not copied")?;
                json!({"id": new.raw()})
            }
            "layers" => {
                let p = self
                    .doc
                    .page(PageId(id_arg(a, "page")?))
                    .map_err(|e| e.to_string())?;
                json!({"ids": p.layers.iter().map(|l| l.id.raw()).collect::<Vec<_>>()})
            }
            "layer_info" => {
                let l = self
                    .doc
                    .layer(LayerId(id_arg(a, "layer")?))
                    .map_err(|e| e.to_string())?;
                json!({"name": l.name, "visible": l.visible, "locked": l.locked})
            }
            "set_layer_name" => {
                let layer = LayerId(id_arg(a, "layer")?);
                let name = a
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                self.run(Command::RenameLayer { layer, name })?;
                json!({})
            }
            "set_layer_visible" => {
                let layer = LayerId(id_arg(a, "layer")?);
                let visible = a.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
                self.run(Command::SetLayerVisible { layer, visible })?;
                json!({})
            }
            "set_layer_locked" => {
                let layer = LayerId(id_arg(a, "layer")?);
                let locked = a.get("locked").and_then(|v| v.as_bool()).unwrap_or(false);
                self.run(Command::SetLayerLocked { layer, locked })?;
                json!({})
            }
            "delete_layer" => {
                let layer = LayerId(id_arg(a, "layer")?);
                let page = self.doc.page(self.page).ok().map(|p| p.id);
                self.run(Command::DeleteLayer { layer })?;
                // The layer the script worked in may be the one taken out.
                if self.layer == Some(layer) {
                    self.layer = page
                        .and_then(|p| self.doc.page(p).ok())
                        .and_then(|p| p.layers.last().map(|l| l.id));
                }
                self.drop_missing_selection();
                json!({})
            }
            "activate_layer" => {
                self.layer = Some(LayerId(id_arg(a, "layer")?));
                json!({})
            }
            "active_layer" => {
                let p = self
                    .doc
                    .page(PageId(id_arg(a, "page")?))
                    .map_err(|e| e.to_string())?;
                let id = self
                    .layer
                    .filter(|l| p.layers.iter().any(|x| x.id == *l))
                    .or_else(|| p.layers.last().map(|l| l.id))
                    .ok_or("no layer")?;
                json!({"id": id.raw()})
            }
            "create_layer" => {
                let page = PageId(id_arg(a, "page")?);
                let name = a
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("Layer")
                    .to_string();
                self.run(Command::AddLayer { page, name })?;
                let id = self
                    .doc
                    .page(page)
                    .ok()
                    .and_then(|p| p.layers.last().map(|l| l.id))
                    .ok_or("no layer")?;
                json!({"id": id.raw()})
            }
            "find_layer" => {
                let p = self
                    .doc
                    .page(PageId(id_arg(a, "page")?))
                    .map_err(|e| e.to_string())?;
                let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
                json!({"id": p.layers.iter().find(|l| l.name == name).map(|l| l.id.raw())})
            }
            "shapes" => {
                let l = self
                    .doc
                    .layer(LayerId(id_arg(a, "layer")?))
                    .map_err(|e| e.to_string())?;
                json!({"ids": l.shapes.iter().map(|s| s.id.raw()).collect::<Vec<_>>()})
            }
            "page_shapes" => {
                let p = self
                    .doc
                    .page(PageId(id_arg(a, "page")?))
                    .map_err(|e| e.to_string())?;
                json!({"ids": p.layers.iter().flat_map(|l| l.shapes.iter().map(|s| s.id.raw())).collect::<Vec<_>>()})
            }
            "find" => {
                let l = self
                    .doc
                    .layer(LayerId(id_arg(a, "layer")?))
                    .map_err(|e| e.to_string())?;
                let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
                json!({"id": l.shapes.iter().find(|s| s.name.as_deref() == Some(name)).map(|s| s.id.raw())})
            }
            "selection" => {
                json!({"ids": self.selection.iter().map(|s| s.raw()).collect::<Vec<_>>()})
            }
            "select" => {
                let ids: Vec<ShapeId> = a
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(ShapeId).collect())
                    .unwrap_or_default();
                let add = a.get("add").and_then(|v| v.as_bool()).unwrap_or(false);
                if add {
                    for id in ids {
                        if !self.selection.contains(&id) {
                            self.selection.push(id);
                        }
                    }
                } else {
                    self.selection = ids;
                }
                json!({})
            }
            "select_all" => {
                let p = self.doc.page(self.page).map_err(|e| e.to_string())?;
                self.selection = p
                    .layers
                    .iter()
                    .flat_map(|l| l.shapes.iter().map(|s| s.id))
                    .collect();
                json!({})
            }
            "find_by_properties" => {
                let q = query_from_json(a)?;
                if q.is_empty() {
                    return Err("no criterion: name a type, a fill, an outline, \
                                an outline width or a font"
                        .into());
                }
                let whole = a
                    .get("whole_drawing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let scope = if whole {
                    tracedraw_core::find::Scope::Document
                } else {
                    tracedraw_core::find::Scope::Page
                };
                let hits = tracedraw_core::find::find(&self.doc, &q, scope, {
                    self.doc
                        .pages
                        .iter()
                        .position(|p| p.id == self.page)
                        .unwrap_or(0)
                });
                json!({"ids": hits.iter().map(|h| h.id.raw()).collect::<Vec<_>>()})
            }
            "replace_by_properties" => {
                let q = query_from_json(a)?;
                if q.is_empty() {
                    return Err("no criterion: name what to look for".into());
                }
                let r = replacement_from_json(a)?;
                if r.is_empty() {
                    return Err("no replacement: name a fill, an outline, \
                                an outline width or a font"
                        .into());
                }
                let whole = a
                    .get("whole_drawing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let scope = if whole {
                    tracedraw_core::find::Scope::Document
                } else {
                    tracedraw_core::find::Scope::Page
                };
                let page = self
                    .doc
                    .pages
                    .iter()
                    .position(|p| p.id == self.page)
                    .unwrap_or(0);
                let hits = tracedraw_core::find::find(&self.doc, &q, scope, page);
                let cmds = tracedraw_core::find::replace_commands(&self.doc, &hits, &q, &r);
                let changed = hits.len();
                for c in cmds {
                    self.run(c)?;
                }
                json!({"replaced": changed})
            }
            "shape_info" => {
                let s = self.shape(id_arg(a, "id")?)?;
                let b = s.bounds();
                // The text's own style, from its first span: what the
                // properties bar shows and what a script reads back after
                // setting it.
                let style = match &s.kind {
                    ShapeKind::Text { spans, .. } => spans.first().cloned(),
                    _ => None,
                };
                let text = match &s.kind {
                    ShapeKind::Text { spans, .. } => {
                        spans.iter().map(|x| x.text.as_str()).collect::<String>()
                    }
                    _ => String::new(),
                };
                json!({
                    "name": s.name.clone().unwrap_or_default(),
                    "type": crate::ui::dockers::kind_id(&s.kind),
                    "x": b.x0, "y": b.y0, "w": b.width(), "h": b.height(),
                    "text": text,
                    "transparency": ((1.0 - s.opacity) * 100.0).round(),
                    "font": style.as_ref().map(|sp| sp.font_family.clone()),
                    "size_pt": style.as_ref().map(|sp| sp.size_pt),
                    "bold": style.as_ref().map(|sp| sp.bold),
                    "italic": style.as_ref().map(|sp| sp.italic),
                })
            }
            "set_text_style" => {
                let id = ShapeId(id_arg(a, "id")?);
                let s = self.shape(id.0)?;
                let ShapeKind::Text {
                    spans,
                    origin,
                    frame,
                    align,
                    para,
                    on_path,
                } = s.kind
                else {
                    return Err("the object is not text".into());
                };
                let font = a.get("font").and_then(|v| v.as_str()).map(String::from);
                let size = a.get("size").and_then(|v| v.as_f64());
                let bold = a.get("bold").and_then(|v| v.as_bool());
                let italic = a.get("italic").and_then(|v| v.as_bool());
                if font.is_none() && size.is_none() && bold.is_none() && italic.is_none() {
                    return Err("nothing to change".into());
                }
                let spans: Vec<TextSpan> = spans
                    .into_iter()
                    .map(|mut sp| {
                        if let Some(f) = &font {
                            sp.font_family = f.clone();
                        }
                        // A size of nothing or less is no size at all.
                        if let Some(z) = size.filter(|z| *z > 0.0 && z.is_finite()) {
                            sp.size_pt = z;
                        }
                        if let Some(b) = bold {
                            sp.bold = b;
                        }
                        if let Some(i) = italic {
                            sp.italic = i;
                        }
                        sp
                    })
                    .collect();
                self.run(Command::SetShapeKind {
                    shape: id,
                    kind: ShapeKind::Text {
                        spans,
                        origin,
                        frame,
                        align,
                        para,
                        on_path,
                    },
                })?;
                json!({})
            }
            "path_data" => {
                let s = self.shape(id_arg(a, "id")?)?;
                let path = s.transform * s.local_path_with(&self.doc.symbols);
                json!({"d": path.to_svg()})
            }
            "node_count" => {
                let s = self.shape(id_arg(a, "id")?)?;
                let n = s
                    .local_path()
                    .elements()
                    .iter()
                    .filter(|e| !matches!(e, kurbo::PathEl::ClosePath))
                    .count();
                json!({"count": n})
            }
            "set_name" => {
                let shape = ShapeId(id_arg(a, "id")?);
                let name = a
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string());
                self.run(Command::SetShapeName { shape, name })?;
                json!({})
            }
            "replace_text" => {
                let needle = a.get("needle").and_then(|v| v.as_str()).unwrap_or("");
                if needle.is_empty() {
                    return Err("nothing to find: the text to replace is empty".into());
                }
                let replacement = a
                    .get("replacement")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let match_case = a
                    .get("match_case")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let whole_word = a
                    .get("whole_word")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                // The objects named, or every text object of the page. The
                // ones inside groups and frames are walked into, as the
                // interface's Find and Replace does.
                let only: Option<Vec<ShapeId>> = a
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(ShapeId).collect());
                let walk = |s: &Shape, out: &mut Vec<Shape>| {
                    fn go(
                        s: &Shape,
                        only: &Option<Vec<ShapeId>>,
                        needle: &str,
                        mc: bool,
                        ww: bool,
                        out: &mut Vec<Shape>,
                    ) {
                        let wanted = only.as_ref().is_none_or(|ids| ids.contains(&s.id));
                        if wanted {
                            if let ShapeKind::Text { spans, .. } = &s.kind {
                                let text: String = spans.iter().map(|x| x.text.as_str()).collect();
                                if crate::export::matches_text(&text, needle, mc, ww) {
                                    out.push(s.clone());
                                }
                            }
                        }
                        match &s.kind {
                            ShapeKind::Group { children } => children
                                .iter()
                                .for_each(|c| go(c, only, needle, mc, ww, out)),
                            ShapeKind::ClipFrame { frame, contents } => {
                                go(frame, only, needle, mc, ww, out);
                                contents
                                    .iter()
                                    .for_each(|c| go(c, only, needle, mc, ww, out));
                            }
                            _ => {}
                        }
                    }
                    go(s, &only, needle, match_case, whole_word, out);
                };
                let page = self.doc.page(self.page).map_err(|e| e.to_string())?.clone();
                let mut hits: Vec<Shape> = Vec::new();
                for s in page.layers.iter().flat_map(|l| &l.shapes) {
                    walk(s, &mut hits);
                }
                let mut changed = 0usize;
                for s in hits {
                    let ShapeKind::Text {
                        spans,
                        origin,
                        frame,
                        align,
                        para,
                        on_path,
                    } = s.kind
                    else {
                        continue;
                    };
                    let spans: Vec<TextSpan> = spans
                        .into_iter()
                        .map(|sp| TextSpan {
                            text: crate::export::replace_in(
                                &sp.text,
                                needle,
                                &replacement,
                                match_case,
                                whole_word,
                            ),
                            ..sp
                        })
                        .collect();
                    self.run(Command::SetShapeKind {
                        shape: s.id,
                        kind: ShapeKind::Text {
                            spans,
                            origin,
                            frame,
                            align,
                            para,
                            on_path,
                        },
                    })?;
                    changed += 1;
                }
                json!({"replaced": changed})
            }
            "set_text" => {
                let id = id_arg(a, "id")?;
                let s = self.shape(id)?;
                if let ShapeKind::Text {
                    spans,
                    origin,
                    frame,
                    align,
                    para,
                    on_path,
                } = s.kind
                {
                    let style = spans
                        .first()
                        .cloned()
                        .unwrap_or_else(|| TextSpan::new("", "Arial", 24.0));
                    let text = a
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    self.run(Command::SetShapeKind {
                        shape: ShapeId(id),
                        kind: ShapeKind::Text {
                            spans: vec![TextSpan { text, ..style }],
                            origin,
                            frame,
                            align,
                            para,
                            on_path,
                        },
                    })?;
                }
                json!({})
            }
            "set_position" => {
                let id = id_arg(a, "id")?;
                let s = self.shape(id)?;
                let b = s.bounds();
                let d = Point::new(num(a, "x", b.x0), num(a, "y", b.y0)) - b.origin();
                self.run(Command::TransformShapes {
                    shapes: vec![ShapeId(id)],
                    transform: Affine::translate(d),
                })?;
                json!({})
            }
            "set_size" => {
                let id = id_arg(a, "id")?;
                let s = self.shape(id)?;
                let b = s.bounds();
                let sx = num(a, "w", b.width()) / b.width().max(1e-9);
                let sy = num(a, "h", b.height()) / b.height().max(1e-9);
                let t = Affine::translate(b.origin().to_vec2())
                    * Affine::scale_non_uniform(sx, sy)
                    * Affine::translate(-b.origin().to_vec2());
                self.run(Command::TransformShapes {
                    shapes: vec![ShapeId(id)],
                    transform: t,
                })?;
                json!({})
            }
            "move" => {
                let id = ShapeId(id_arg(a, "id")?);
                self.run(Command::TransformShapes {
                    shapes: vec![id],
                    transform: Affine::translate((num(a, "dx", 0.0), num(a, "dy", 0.0))),
                })?;
                json!({})
            }
            "rotate" => {
                let id = id_arg(a, "id")?;
                let c = self.shape(id)?.bounds().center();
                self.run(Command::TransformShapes {
                    shapes: vec![ShapeId(id)],
                    transform: Affine::rotate_about(num(a, "deg", 0.0).to_radians(), c),
                })?;
                json!({})
            }
            "stretch" => {
                let id = id_arg(a, "id")?;
                let c = self.shape(id)?.bounds().center();
                let t = Affine::translate(c.to_vec2())
                    * Affine::scale_non_uniform(num(a, "sx", 1.0), num(a, "sy", 1.0))
                    * Affine::translate(-c.to_vec2());
                self.run(Command::TransformShapes {
                    shapes: vec![ShapeId(id)],
                    transform: t,
                })?;
                json!({})
            }
            "skew" => {
                let id = id_arg(a, "id")?;
                let c = self.shape(id)?.bounds().center();
                let t = Affine::translate(c.to_vec2())
                    * Affine::skew(
                        num(a, "ax", 0.0).to_radians().tan(),
                        num(a, "ay", 0.0).to_radians().tan(),
                    )
                    * Affine::translate(-c.to_vec2());
                self.run(Command::TransformShapes {
                    shapes: vec![ShapeId(id)],
                    transform: t,
                })?;
                json!({})
            }
            "delete" => {
                let id = ShapeId(id_arg(a, "id")?);
                self.run(Command::DeleteShapes { shapes: vec![id] })?;
                self.selection.retain(|s| *s != id);
                json!({})
            }
            "duplicate" => {
                let id = id_arg(a, "id")?;
                let s = self.shape(id)?;
                let layer = self.layer_of(ShapeId(id))?;
                let mut copy = self.fresh_ids(s);
                copy.transform =
                    Affine::translate((num(a, "dx", 0.0), num(a, "dy", 0.0))) * copy.transform;
                let new = copy.id;
                self.run(Command::AddShape { layer, shape: copy })?;
                json!({"id": new.raw()})
            }
            "curve_info" | "curve_nodes" => {
                let s = self.shape(id_arg(a, "id")?)?;
                // Other objects read as the curve they draw.
                let owned = match &s.kind {
                    ShapeKind::Path { path, .. } => path.clone(),
                    _ => s.local_path(),
                };
                let path = &owned;
                let els = path.elements();
                // Subpath of each element, and whether each subpath closes.
                let mut sub_of = Vec::with_capacity(els.len());
                let mut closed: Vec<bool> = Vec::new();
                for e in els {
                    if matches!(e, kurbo::PathEl::MoveTo(_)) {
                        closed.push(false);
                    }
                    let k = closed.len().saturating_sub(1);
                    if matches!(e, kurbo::PathEl::ClosePath) {
                        if let Some(c) = closed.last_mut() {
                            *c = true;
                        }
                    }
                    sub_of.push(k);
                }
                let nodes: Vec<Value> = tracedraw_core::nodes::nodes(path)
                    .iter()
                    .map(|n| {
                        let p = s.transform * n.pos;
                        let ty = match tracedraw_core::nodes::node_type_of_with(n, &s.node_types) {
                            tracedraw_core::nodes::NodeType::Cusp => "cusp",
                            tracedraw_core::nodes::NodeType::Smooth => "smooth",
                            tracedraw_core::nodes::NodeType::Symmetrical => "symmetrical",
                        };
                        let sub = sub_of.get(n.index).copied().unwrap_or(0);
                        let ending = !closed.get(sub).copied().unwrap_or(false)
                            && (n.ctrl_in.is_none() || n.ctrl_out.is_none())
                            && matches!(
                                (els.get(n.index), els.get(n.index + 1)),
                                (Some(kurbo::PathEl::MoveTo(_)), _)
                                    | (_, None)
                                    | (_, Some(kurbo::PathEl::MoveTo(_)))
                            );
                        json!({"index": n.index, "x": p.x, "y": p.y, "type": ty,
                               "subpath": sub, "ending": ending})
                    })
                    .collect();
                let subpaths: Vec<Value> = closed.iter().map(|c| json!({"closed": c})).collect();
                json!({"nodes": nodes, "subpaths": subpaths})
            }
            "node_move" | "node_type" | "node_delete" | "node_add" => {
                let id = id_arg(a, "id")?;
                let s = self.shape(id)?;
                let ShapeKind::Path { path, closed } = &s.kind else {
                    return Err("not a curve: convert it to curves first".into());
                };
                let index = id_arg(a, "index")? as usize;
                let node = tracedraw_core::nodes::nodes(path)
                    .into_iter()
                    .find(|n| n.index == index)
                    .ok_or_else(|| format!("no node {index}"))?;
                let mut types = s.node_types.clone();
                let new_path = match method {
                    "node_move" => {
                        let at = s.transform * node.pos;
                        let to = Point::new(num(a, "x", at.x), num(a, "y", at.y))
                            + kurbo::Vec2::new(num(a, "dx", 0.0), num(a, "dy", 0.0));
                        let inv = s.transform.inverse();
                        let d = inv * to - inv * at;
                        if !(d.x.is_finite() && d.y.is_finite()) {
                            return Err("the curve cannot be moved there".into());
                        }
                        tracedraw_core::nodes::move_node(path, index, d)
                    }
                    "node_type" => {
                        let ty = match a.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                            "cusp" => tracedraw_core::nodes::NodeType::Cusp,
                            "smooth" => tracedraw_core::nodes::NodeType::Smooth,
                            "symmetrical" => tracedraw_core::nodes::NodeType::Symmetrical,
                            other => return Err(format!("no node type {other:?}")),
                        };
                        let out = tracedraw_core::nodes::set_node_type(path, index, ty);
                        // A closing line made a curve moves the elements
                        // after it up by one, their types with them (#1618).
                        if let Some(at) =
                            tracedraw_core::nodes::curve_put_in_before_close(path, &out)
                        {
                            types = types
                                .into_iter()
                                .map(|(k, t)| (if k >= at { k + 1 } else { k }, t))
                                .collect();
                        }
                        types.insert(index, ty);
                        out
                    }
                    "node_delete" => tracedraw_core::nodes::delete_node(path, index),
                    _ => tracedraw_core::nodes::add_node(
                        path,
                        index,
                        num(a, "t", 0.5).clamp(0.0, 1.0),
                    ),
                };
                self.run(Command::SetShapeKind {
                    shape: ShapeId(id),
                    kind: ShapeKind::Path {
                        path: new_path,
                        closed: *closed,
                    },
                })?;
                if method == "node_type" {
                    self.run(Command::SetNodeTypes {
                        shape: ShapeId(id),
                        types,
                    })?;
                }
                json!({})
            }
            "effects" => {
                let s = self.shape(id_arg(a, "id")?)?;
                let types: Vec<Value> = s
                    .effects
                    .iter()
                    .map(|e| {
                        serde_json::to_value(e)
                            .ok()
                            .and_then(|v| v.get("effect").cloned())
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                json!({"types": types})
            }
            "add_effect" => {
                let id = id_arg(a, "id")?;
                let effect: tracedraw_core::live::Effect =
                    serde_json::from_value(a.get("effect").cloned().unwrap_or(Value::Null))
                        .map_err(|e| format!("not an effect: {e}"))?;
                let mut effects = self.shape(id)?.effects;
                effects.push(effect);
                self.run(Command::SetEffects {
                    shape: ShapeId(id),
                    effects,
                })?;
                json!({})
            }
            "remove_effects" => {
                let id = id_arg(a, "id")?;
                let mut effects = self.shape(id)?.effects;
                match a.get("index").and_then(|v| v.as_u64()) {
                    Some(i) if (i as usize) < effects.len() => {
                        effects.remove(i as usize);
                    }
                    Some(i) => return Err(format!("no effect {}", i + 1)),
                    None => effects.clear(),
                }
                self.run(Command::SetEffects {
                    shape: ShapeId(id),
                    effects,
                })?;
                json!({})
            }
            "convert_to_curves" => {
                let id = id_arg(a, "id")?;
                let s = self.shape(id)?;
                let path = s.local_path();
                let closed = path
                    .elements()
                    .iter()
                    .any(|e| matches!(e, kurbo::PathEl::ClosePath));
                self.run(Command::SetShapeKind {
                    shape: ShapeId(id),
                    kind: ShapeKind::Path { path, closed },
                })?;
                json!({})
            }
            "order" => {
                let id = ShapeId(id_arg(a, "id")?);
                let (layer, _) = self.doc.locate(id).map_err(|e| e.to_string())?;
                let n = self.doc.layer(layer).map(|l| l.shapes.len()).unwrap_or(1);
                let index = if a.get("op").and_then(|o| o.as_str()) == Some("front") {
                    n - 1
                } else {
                    0
                };
                self.run(Command::Reorder {
                    shape: id,
                    layer,
                    index,
                })?;
                json!({})
            }
            "set_opacity" => {
                let id = ShapeId(id_arg(a, "id")?);
                self.run(Command::SetOpacity {
                    shapes: vec![id],
                    opacity: num(a, "opacity", 1.0).clamp(0.0, 1.0),
                })?;
                json!({})
            }
            "drop_shadow" => {
                let id = ShapeId(id_arg(a, "id")?);
                let shadow = tracedraw_core::Shadow {
                    offset: tracedraw_core::Vec2::new(num(a, "dx", 3.0), num(a, "dy", -3.0)),
                    opacity: num(a, "opacity", 50.0) / 100.0,
                    ..Default::default()
                };
                self.run(Command::SetShadow {
                    shapes: vec![id],
                    shadow: Some(shadow),
                })?;
                json!({})
            }
            "group" => {
                let ids: Vec<ShapeId> = a
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(ShapeId).collect())
                    .unwrap_or_default();
                if ids.len() < 2 {
                    return Err("group needs two objects".into());
                }
                let gid = self.group_shapes(&ids)?;
                json!({"id": gid.raw()})
            }
            "ungroup" => {
                let id = ShapeId(id_arg(a, "id")?);
                let kind = self.shape(id_arg(a, "id")?).map(|s| s.kind)?;
                if !matches!(kind, ShapeKind::Group { .. }) {
                    return Err("ungroup needs a group".into());
                }
                self.run(Command::Ungroup { group: id })?;
                // The group is gone and its objects took its place; the
                // selection that named it names nothing now.
                self.drop_missing_selection();
                json!({})
            }
            "combine" => {
                let ids: Vec<ShapeId> = a
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(ShapeId).collect())
                    .unwrap_or_default();
                if ids.len() < 2 {
                    return Err("combine needs two objects".into());
                }
                self.run(Command::Combine {
                    shapes: ids.clone(),
                })?;
                // The objects became one: the result keeps the id of the
                // topmost, and the others name nothing now.
                self.selection
                    .retain(|s| !ids.contains(s) || self.doc.find_shape(*s).is_some());
                json!({})
            }
            "break_apart" => {
                let id = ShapeId(id_arg(a, "id")?);
                let page_shapes = |h: &Self| -> Vec<ShapeId> {
                    h.doc
                        .page(h.page)
                        .map(|p| {
                            p.layers
                                .iter()
                                .flat_map(|l| l.shapes.iter().map(|s| s.id))
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let before = page_shapes(self);
                self.run(Command::BreakApart { shape: id })?;
                let after = page_shapes(self);
                // The first piece keeps the broken object's id, so what is
                // new is only part of the answer. The split happened when
                // new ids appeared: with nothing new, the command left the
                // object as it was (a rectangle has no subpaths) and there
                // are no pieces to hand back.
                let mut pieces: Vec<u64> = if after.iter().any(|s| !before.contains(s)) {
                    after
                        .iter()
                        .filter(|s| !before.contains(s) || **s == id)
                        .map(|s| s.raw())
                        .collect()
                } else {
                    Vec::new()
                };
                // On the page's order, so a script reading them gets them
                // in drawing order.
                pieces.sort_by_key(|p| {
                    after
                        .iter()
                        .position(|s| s.raw() == *p)
                        .unwrap_or(usize::MAX)
                });
                json!({"ids": pieces})
            }
            "align" => {
                let ids: Vec<ShapeId> = a
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(ShapeId).collect())
                    .unwrap_or_default();
                let shapes: Vec<Shape> = ids
                    .iter()
                    .filter_map(|id| self.doc.find_shape(*id).cloned())
                    .collect();
                let Some(target) = shapes.last().map(|s| s.bounds()) else {
                    return Ok(json!({}));
                };
                let how = a.get("how").and_then(|h| h.as_str()).unwrap_or("left");
                for s in &shapes {
                    let b = s.bounds();
                    let d = match how {
                        "left" => (target.x0 - b.x0, 0.0),
                        "right" => (target.x1 - b.x1, 0.0),
                        "top" => (0.0, target.y1 - b.y1),
                        "bottom" => (0.0, target.y0 - b.y0),
                        "center_h" => (target.center().x - b.center().x, 0.0),
                        _ => (0.0, target.center().y - b.center().y),
                    };
                    self.run(Command::TransformShapes {
                        shapes: vec![s.id],
                        transform: Affine::translate(d),
                    })?;
                }
                json!({})
            }
            "distribute" => {
                let ids: Vec<ShapeId> = a
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(ShapeId).collect())
                    .unwrap_or_default();
                let how = a.get("how").and_then(|h| h.as_str()).unwrap_or("left");
                let to = a.get("to").and_then(|t| t.as_str()).unwrap_or("selection");
                // 0: the low edges (left, bottom), 1: the centres, 2: the
                // high edges (right, top), 3: the gaps between the boxes.
                // y grows upwards, so bottom is the low edge.
                let (horizontal, kind) = match how {
                    "left" => (true, 0),
                    "center_h" => (true, 1),
                    "right" => (true, 2),
                    "spacing_h" => (true, 3),
                    "bottom" => (false, 0),
                    "center_v" => (false, 1),
                    "top" => (false, 2),
                    "spacing_v" => (false, 3),
                    other => return Err(format!("no distribution {other:?}")),
                };
                let mut boxes: Vec<(Shape, Rect)> = ids
                    .iter()
                    .filter_map(|id| self.doc.find_shape(*id).cloned())
                    .map(|s| {
                        let b = s.bounds();
                        (s, b)
                    })
                    .collect();
                // Between the outermost objects three are needed to have
                // something to spread; across the page, two.
                let need = if to == "page" { 2 } else { 3 };
                if boxes.len() < need {
                    return Err(format!(
                        "distribute needs {need} objects to {to}, {} given",
                        boxes.len()
                    ));
                }
                let lo = |b: &Rect| if horizontal { b.x0 } else { b.y0 };
                let hi = |b: &Rect| if horizontal { b.x1 } else { b.y1 };
                let key = |b: &Rect| match kind {
                    0 => lo(b),
                    2 => hi(b),
                    _ => (lo(b) + hi(b)) / 2.0,
                };
                boxes.sort_by(|x, y| {
                    key(&x.1)
                        .partial_cmp(&key(&y.1))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let n = boxes.len();
                let (e0, e1) = if to == "page" {
                    let size = self
                        .doc
                        .page(self.page)
                        .map(|p| p.size)
                        .map_err(|e| e.to_string())?;
                    if horizontal {
                        (0.0, size.width)
                    } else {
                        (0.0, size.height)
                    }
                } else {
                    (lo(&boxes[0].1), hi(&boxes[n - 1].1))
                };
                let mut shifts: Vec<(ShapeId, f64)> = Vec::with_capacity(n);
                let len = |b: &Rect| hi(b) - lo(b);
                if kind == 3 {
                    // The gaps between the boxes, equal for all of them.
                    let total: f64 = boxes.iter().map(|(_, b)| len(b)).sum();
                    let gap = (e1 - e0 - total) / (n - 1) as f64;
                    let mut cursor = e0;
                    for (s, b) in &boxes {
                        shifts.push((s.id, cursor - lo(b)));
                        cursor += len(b) + gap;
                    }
                } else {
                    // The keys evenly spaced from the first to the last.
                    // Across the page the first and last boxes go flush
                    // with its edges, each by the length of its own key.
                    let (k0, k1) = if to == "page" {
                        match kind {
                            0 => (e0, e1 - len(&boxes[n - 1].1)),
                            1 => (e0 + len(&boxes[0].1) / 2.0, e1 - len(&boxes[n - 1].1) / 2.0),
                            _ => (e0 + len(&boxes[0].1), e1),
                        }
                    } else {
                        (key(&boxes[0].1), key(&boxes[n - 1].1))
                    };
                    let step = (k1 - k0) / (n - 1) as f64;
                    for (i, (s, b)) in boxes.iter().enumerate() {
                        shifts.push((s.id, k0 + step * i as f64 - key(b)));
                    }
                }
                for (id, d) in shifts {
                    if d.abs() > 1e-9 {
                        self.run(Command::TransformShapes {
                            shapes: vec![id],
                            transform: Affine::translate(if horizontal {
                                (d, 0.0)
                            } else {
                                (0.0, d)
                            }),
                        })?;
                    }
                }
                json!({})
            }
            "fill_type" => {
                let s = self.shape(id_arg(a, "id")?)?;
                json!({"type": match s.fill { Fill::None => "none", Fill::Solid(_) => "uniform", Fill::Fountain(_) => "fountain", Fill::Pattern(_) => "pattern", Fill::Texture(_) => "texture", Fill::Mesh(_) => "mesh", Fill::Procedural(_) => "procedural" }})
            }
            "fill_color" => {
                let s = self.shape(id_arg(a, "id")?)?;
                json!({"color": s.fill.preview_color().unwrap_or(Color::BLACK)})
            }
            "set_fill" => {
                let id = ShapeId(id_arg(a, "id")?);
                let fill: Fill =
                    serde_json::from_value(a.get("fill").cloned().unwrap_or(Value::Null))
                        .map_err(|e| e.to_string())?;
                self.run(Command::SetFill {
                    shapes: vec![id],
                    fill,
                })?;
                json!({})
            }
            "set_fountain" => {
                let id = ShapeId(id_arg(a, "id")?);
                let from = a.get("from").and_then(color_from).unwrap_or(Color::BLACK);
                let to = a.get("to").and_then(color_from).unwrap_or(Color::WHITE);
                self.run(Command::SetFill {
                    shapes: vec![id],
                    fill: Fill::linear(from, to, num(a, "angle", 0.0)),
                })?;
                json!({})
            }
            "outline_width" => {
                let s = self.shape(id_arg(a, "id")?)?;
                json!({"width": s.stroke.as_ref().map(|st| st.width).unwrap_or(0.0)})
            }
            "outline_color" => {
                let s = self.shape(id_arg(a, "id")?)?;
                json!({"color": s.stroke.as_ref().map(|st| st.color).unwrap_or(Color::BLACK)})
            }
            "set_outline" => {
                let id = ShapeId(id_arg(a, "id")?);
                self.run(Command::SetStroke {
                    shapes: vec![id],
                    stroke: None,
                })?;
                json!({})
            }
            "set_outline_props" => {
                let id = id_arg(a, "id")?;
                let s = self.shape(id)?;
                let mut st = s.stroke.clone().unwrap_or_default();
                if let Some(w) = a.get("width").and_then(|w| w.as_f64()) {
                    st.width = w;
                }
                if let Some(c) = a.get("color").and_then(color_from) {
                    st.color = c;
                }
                self.run(Command::SetStroke {
                    shapes: vec![ShapeId(id)],
                    stroke: Some(st),
                })?;
                json!({})
            }
            "color_hex" => {
                json!({"hex": a.get("color").and_then(color_from).unwrap_or(Color::BLACK).to_hex()})
            }
            "create_rect" => {
                let layer = self.layer_for(a)?;
                let (x, y, w, h) = (
                    num(a, "x", 0.0),
                    num(a, "y", 0.0),
                    num(a, "w", 10.0),
                    num(a, "h", 10.0),
                );
                let mut s = Shape::new(
                    ShapeId(0),
                    ShapeKind::Rect {
                        rect: Rect::new(x, y, x + w, y + h),
                        radius: num(a, "radius", 0.0),
                        corners: None,
                    },
                );
                s.stroke = Some(Stroke::default());
                self.add(layer, s)?
            }
            "create_ellipse" => {
                let layer = self.layer_for(a)?;
                let (x, y, w, h) = (
                    num(a, "x", 0.0),
                    num(a, "y", 0.0),
                    num(a, "w", 10.0),
                    num(a, "h", 10.0),
                );
                let s = Shape::new(
                    ShapeId(0),
                    ShapeKind::Ellipse {
                        rect: Rect::new(x, y, x + w, y + h),
                        arc: None,
                    },
                );
                self.add(layer, s)?
            }
            "create_polygon" => {
                let layer = self.layer_for(a)?;
                let (x, y, w, h) = (
                    num(a, "x", 0.0),
                    num(a, "y", 0.0),
                    num(a, "w", 10.0),
                    num(a, "h", 10.0),
                );
                let s = Shape::new(
                    ShapeId(0),
                    ShapeKind::Polygon {
                        rect: Rect::new(x, y, x + w, y + h),
                        points: num(a, "points", 5.0) as u32,
                        sharpness: num(a, "sharpness", 0.0),
                        complex: None,
                        twist: 0.0,
                    },
                );
                self.add(layer, s)?
            }
            "create_text" => {
                let layer = self.layer_for(a)?;
                let text = a
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let font = a
                    .get("font")
                    .and_then(|f| f.as_str())
                    .filter(|f| !f.is_empty())
                    .unwrap_or("Arial")
                    .to_string();
                let frame = a.get("frame").and_then(|f| f.as_array()).and_then(|f| {
                    Some(tracedraw_core::Size::new(
                        f.first()?.as_f64()?,
                        f.get(1)?.as_f64()?,
                    ))
                });
                let mut s = Shape::new(
                    ShapeId(0),
                    ShapeKind::Text {
                        spans: vec![TextSpan::new(text, font, num(a, "size", 24.0))],
                        origin: Point::new(num(a, "x", 0.0), num(a, "y", 0.0)),
                        frame,
                        align: tracedraw_core::TextAlign::Left,
                        para: Default::default(),
                        on_path: None,
                    },
                );
                s.fill = Fill::Solid(Color::BLACK);
                s.stroke = None;
                self.add(layer, s)?
            }
            "create_curve" => {
                let layer = self.layer_for(a)?;
                let pts: Vec<Point> = a
                    .get("points")
                    .and_then(|p| p.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|p| {
                                let xy = p.as_array()?;
                                Some(Point::new(xy.first()?.as_f64()?, xy.get(1)?.as_f64()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if pts.len() < 2 {
                    return Err("a curve needs at least two points".into());
                }
                let closed = a.get("closed").and_then(|c| c.as_bool()).unwrap_or(false);
                let path = tracedraw_core::geometry::polyline_path(&pts, closed);
                let s = Shape::new(ShapeId(0), ShapeKind::Path { path, closed });
                self.add(layer, s)?
            }
            "create_path" => {
                let layer = self.layer_for(a)?;
                let d = a
                    .get("d")
                    .and_then(|d| d.as_str())
                    .ok_or("missing path data")?;
                let path = tracedraw_core::geometry::BezPath::from_svg(d)
                    .map_err(|e| format!("path data: {e}"))?;
                if path.elements().len() < 2 {
                    return Err("path data draws nothing".into());
                }
                let closed = path
                    .elements()
                    .iter()
                    .any(|e| matches!(e, tracedraw_core::geometry::PathEl::ClosePath));
                let s = Shape::new(ShapeId(0), ShapeKind::Path { path, closed });
                self.add(layer, s)?
            }
            "import_bitmap" => {
                let layer = self.layer_for(a)?;
                let path = a
                    .get("path")
                    .and_then(|p| p.as_str())
                    .ok_or("missing path")?;
                let img = crate::files::open_image(path)
                    .map_err(|e| e.to_string())?
                    .to_rgba8();
                let (w, h) = img.dimensions();
                let png = crate::bitmap_fx::encode(&img).ok_or("encode failed")?;
                let mm_w = w as f64 * 25.4 / 96.0;
                let mm_h = h as f64 * 25.4 / 96.0;
                let s = Shape::new(
                    ShapeId(0),
                    ShapeKind::Bitmap {
                        rect: Rect::new(0.0, 0.0, mm_w, mm_h),
                        width_px: w,
                        height_px: h,
                        png: png.into(),
                        fx: None,
                    },
                );
                self.add(layer, s)?
            }
            "export" => {
                let path = a
                    .get("path")
                    .and_then(|p| p.as_str())
                    .ok_or("missing path")?;
                let p = std::path::Path::new(path);
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let idx = self
                    .doc
                    .pages
                    .iter()
                    .position(|pg| pg.id == self.page)
                    .unwrap_or(0);
                let data = match ext.as_str() {
                    "svg" => tracedraw_io::svg::page_to_svg(&self.doc, idx).into_bytes(),
                    "pdf" => tracedraw_io::pdf::document_to_pdf(&self.doc),
                    "tdraw" => tracedraw_text::fonts()
                        .with_carried_fonts(&self.doc)
                        .to_json()
                        .map_err(|e| e.to_string())?
                        .into_bytes(),
                    "png" => {
                        let pm = tracedraw_render::render_page_image(&self.doc, self.page, 300.0)
                            .ok_or("render failed")?;
                        pm.encode_png().map_err(|e| e.to_string())?
                    }
                    other => return Err(format!("unsupported export format: {other}")),
                };
                crate::files::write(p, data).map_err(|e| e.to_string())?;
                json!({})
            }
            other => match self.vg_call(other, a)? {
                Some(v) => v,
                None => return Err(format!("unknown method {other}")),
            },
        })
    }
}

/// Turn recorded commands into a script the user can edit and replay.
pub fn commands_to_script(cmds: &[Command]) -> String {
    let mut out = String::from("// Recorded macro\nconst doc = Application.ActiveDocument;\nconst layer = doc.ActiveLayer;\n");
    for c in cmds {
        match c {
            Command::AddShape { shape, .. } => {
                let b = shape.bounds();
                match &shape.kind {
                    ShapeKind::Rect { radius, .. } => out.push_str(&format!(
                        "layer.CreateRectangle({:.3}, {:.3}, {:.3}, {:.3}, {:.3});\n",
                        b.x0,
                        b.y0,
                        b.width(),
                        b.height(),
                        radius
                    )),
                    ShapeKind::Ellipse { .. } => out.push_str(&format!(
                        "layer.CreateEllipse({:.3}, {:.3}, {:.3}, {:.3});\n",
                        b.x0,
                        b.y0,
                        b.width(),
                        b.height()
                    )),
                    ShapeKind::Polygon {
                        points, sharpness, ..
                    } => out.push_str(&format!(
                        "layer.CreateStar({:.3}, {:.3}, {:.3}, {:.3}, {}, {:.2});\n",
                        b.x0,
                        b.y0,
                        b.width(),
                        b.height(),
                        points,
                        sharpness
                    )),
                    ShapeKind::Text { spans, origin, .. } => {
                        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                        let font = spans
                            .first()
                            .map(|s| s.font_family.clone())
                            .unwrap_or_default();
                        let size = spans.first().map(|s| s.size_pt).unwrap_or(24.0);
                        out.push_str(&format!(
                            "layer.CreateArtisticText({:.3}, {:.3}, {}, {}, {});\n",
                            origin.x,
                            origin.y,
                            serde_json::to_string(&text).unwrap_or_default(),
                            serde_json::to_string(&font).unwrap_or_default(),
                            size
                        ));
                    }
                    _ => out.push_str(&format!("// {} (not scriptable yet)\n", c.label())),
                }
            }
            Command::TransformShapes { shapes, transform } => {
                let [a, b, _, _, e, f] = transform.as_coeffs();
                let deg = b.atan2(a).to_degrees();
                for s in shapes {
                    if deg.abs() > 1e-6 {
                        out.push_str(&format!("doc.Shape({}).Rotate({:.3});\n", s.raw(), deg));
                    } else {
                        out.push_str(&format!(
                            "new Shape({}).Move({:.3}, {:.3});\n",
                            s.raw(),
                            e,
                            f
                        ));
                    }
                }
            }
            Command::SetFill { shapes, fill } => {
                let j = serde_json::to_string(fill).unwrap_or_default();
                for s in shapes {
                    out.push_str(&format!(
                        "__call('set_fill', {{id:{}, fill:{j}}});\n",
                        s.raw()
                    ));
                }
            }
            Command::SetStroke { shapes, stroke } => {
                for s in shapes {
                    match stroke {
                        None => out
                            .push_str(&format!("new Shape({}).Outline.SetNoOutline();\n", s.raw())),
                        Some(st) => out.push_str(&format!(
                            "new Shape({}).Outline.SetProperties({:.3}, new Color({}));\n",
                            s.raw(),
                            st.width,
                            serde_json::to_string(&st.color).unwrap_or_default()
                        )),
                    }
                }
            }
            Command::DeleteShapes { shapes } => {
                for s in shapes {
                    out.push_str(&format!("new Shape({}).Delete();\n", s.raw()));
                }
            }
            other => out.push_str(&format!("// {}\n", other.label())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::headless()
    }

    #[test]
    fn script_creates_and_moves_a_rectangle() {
        let mut a = app();
        let out = run(
            &mut a,
            "const r = ActiveLayer.CreateRectangle(10, 20, 30, 40); r.Move(5, 5); r.Fill.ApplyUniformFill(Color.RGB(255,0,0)); print('w=' + r.SizeWidth); r.Name = 'box';",
        )
        .unwrap();
        assert_eq!(out, vec!["w=30".to_string()]);
        let shapes = a.doc().pages[0].layers[0].shapes.clone();
        assert_eq!(shapes.len(), 1);
        let b = shapes[0].bounds();
        assert!((b.x0 - 15.0).abs() < 1e-6 && (b.y0 - 25.0).abs() < 1e-6);
        assert_eq!(shapes[0].name.as_deref(), Some("box"));
        assert_eq!(a.engine.undo_label(), Some("Script"));
    }

    #[test]
    fn the_readme_example_draws_a_fan_of_bars() {
        // The JavaScript shown in README.md, word for word.
        let mut a = app();
        run(
            &mut a,
            "for (let i = 0; i < 12; i++) {\n  const bar = ActiveLayer.CreateRectangle(80, 120, 60, 12, 3);\n  bar.Fill.ApplyUniformFill(Color.RGB(255, 60 + i * 15, 60));\n  bar.Rotate(i * 15);\n}",
        )
        .expect("script runs");
        let shapes = &a.doc().pages[0].layers[0].shapes;
        assert_eq!(shapes.len(), 12);
        // Each bar is a little more orange than the one before.
        let green = |s: &tracedraw_core::Shape| match &s.fill {
            tracedraw_core::Fill::Solid(c) => c.to_rgb8()[1],
            _ => 0,
        };
        assert!(green(&shapes[11]) > green(&shapes[0]));
    }

    #[test]
    fn script_error_is_reported() {
        let mut a = app();
        let r = run(&mut a, "nosuchthing.foo();");
        assert!(r.is_err());
    }

    #[test]
    fn curves_show_their_subpaths_and_nodes() {
        let mut app = app();
        let out = run(
            &mut app,
            r#"
            const c = ActiveLayer.CreateCurve([[0,0],[10,0],[10,10]], false);
            print(c.Curve.SubPaths.Count, c.Curve.Nodes.Count, c.Curve.Closed);
            const n = c.Curve.Nodes.Item(2);
            print(n.PositionX, n.PositionY, n.Type, c.Curve.Nodes.First.IsEnding);
            n.Move(5, 1);
            print(c.Curve.Nodes.Item(2).PositionX, c.Curve.Nodes.Item(2).PositionY);
            c.Curve.Nodes.Item(3).SetPosition(20, 20);
            c.Curve.Nodes.Item(2).AddNodeBefore(0.5);
            print(c.Curve.Nodes.Count);
            c.Curve.Nodes.Item(2).Delete();
            print(c.Curve.Nodes.Count, c.Curve.Nodes.Last.PositionX);
            const r = ActiveLayer.CreateRectangle(0, 0, 10, 10);
            print(r.Curve.Nodes.Count);
            try { r.Curve.Nodes.Item(1).Move(1, 1); } catch (e) { print('err'); }
            "#,
        )
        .expect("runs");
        assert_eq!(out[0], "1 3 false");
        assert_eq!(out[1], "10 0 cusp true");
        assert_eq!(out[2], "15 1");
        assert_eq!(out[3], "4");
        assert_eq!(out[4], "3 20");
        assert_eq!(out[5], "4");
        assert_eq!(out[6], "err");
    }

    #[test]
    fn effects_are_listed_and_removed() {
        let mut app = app();
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        let effects = vec![
            tracedraw_core::live::Effect::BlockShadow {
                offset: kurbo::Vec2::new(1.0, -1.0),
                color: Color::BLACK,
                gap: 0.0,
            };
            2
        ];
        app.run(Command::SetEffects { shape: id, effects });
        app.select(vec![id]);
        let out = run(
            &mut app,
            r#"
            const r = ActiveSelection.Item(1);
            print(r.Effects.Count, r.Effects.Item(1).Type);
            r.Effects.Remove(1);
            print(r.Effects.Count);
            r.Effects.Clear();
            print(r.Effects.Count);
            try { r.Effects.Remove(1); } catch (e) { print('none'); }
            "#,
        )
        .expect("runs");
        assert_eq!(out, vec!["2 blockshadow", "1", "0", "none"]);
        let out = run(
            &mut app,
            r#"
            const r = ActiveSelection.Item(1);
            r.Effects.AddExtrude(5, 5);
            r.Effects.AddBlockShadow(1, -1, Color.RGB(255, 0, 0));
            print(r.Effects.Count, r.Effects.Item(1).Type, r.Effects.Item(2).Type);
            try { r.Effects.Add({effect: 'nothing'}); } catch (e) { print('refused'); }
            print(r.Effects.Count);
            "#,
        )
        .expect("runs");
        assert_eq!(out, vec!["2 extrude blockshadow", "refused", "2"]);
        assert_eq!(app.doc().find_shape(id).expect("shape").effects.len(), 2);
    }

    #[test]
    fn selection_round_trip() {
        let mut a = app();
        run(
            &mut a,
            "const e = ActiveLayer.CreateEllipse(0,0,10,10); e.CreateSelection();",
        )
        .unwrap();
        assert_eq!(a.selection.len(), 1);
        let out = run(
            &mut a,
            "print(ActiveSelection.Count + ' ' + ActiveSelection.Item(1).Type);",
        )
        .unwrap();
        assert_eq!(out[0], "1 Ellipse");
    }

    #[test]
    fn a_group_is_ungrouped_from_a_script() {
        let mut a = app();
        run(
            &mut a,
            "const a = ActiveLayer.CreateRectangle(0,0,10,10);\n\
             const b = ActiveLayer.CreateRectangle(20,0,10,10);\n\
             new ShapeRange([a.id, b.id]).Group();",
        )
        .unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Count);").unwrap();
        assert_eq!(out[0], "1", "the group is one object");
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Type);").unwrap();
        assert_eq!(out[0], "Group");

        run(&mut a, "ActiveLayer.Shapes.Item(1).Ungroup();").unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Count);").unwrap();
        assert_eq!(out[0], "2", "the objects are back");
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Type);").unwrap();
        assert_eq!(out[0], "Rectangle");
    }

    #[test]
    fn ungrouping_something_that_is_not_a_group_is_refused() {
        let mut a = app();
        run(
            &mut a,
            "const r = ActiveLayer.CreateRectangle(0,0,10,10); r.Ungroup();",
        )
        .unwrap_err();
    }

    #[test]
    fn a_page_is_renamed_copied_and_deleted_from_a_script() {
        let mut a = app();
        run(&mut a, "ActivePage.Name = 'Cover';").unwrap();
        let out = run(&mut a, "print(ActivePage.Name);").unwrap();
        assert_eq!(out[0], "Cover");

        let out = run(&mut a, "print(ActiveDocument.Pages.Count);").unwrap();
        assert_eq!(out[0], "1");
        run(&mut a, "ActiveDocument.AddPages(1);").unwrap();
        let out = run(&mut a, "print(ActiveDocument.Pages.Count);").unwrap();
        assert_eq!(out[0], "2");

        run(&mut a, "ActiveDocument.Pages.Item(1).Duplicate();").unwrap();
        let out = run(&mut a, "print(ActiveDocument.Pages.Count);").unwrap();
        assert_eq!(out[0], "3", "the copy sits beside the page");

        run(&mut a, "ActiveDocument.Pages.Item(3).Delete();").unwrap();
        let out = run(&mut a, "print(ActiveDocument.Pages.Count);").unwrap();
        assert_eq!(out[0], "2");
    }

    #[test]
    fn the_last_page_and_the_last_layer_stay() {
        let mut a = app();
        assert!(
            run(&mut a, "ActivePage.Delete();").is_err(),
            "the last page stays"
        );
        assert!(
            run(&mut a, "ActiveLayer.Delete();").is_err(),
            "the last layer stays"
        );
        let out = run(&mut a, "print(ActiveDocument.Pages.Count);").unwrap();
        assert_eq!(out[0], "1");
        let out = run(&mut a, "print(ActivePage.Layers.Count);").unwrap();
        assert_eq!(out[0], "1");
    }

    #[test]
    fn a_layer_is_locked_and_deleted_from_a_script() {
        let mut a = app();
        run(&mut a, "ActivePage.CreateLayer('Guide');").unwrap();
        let out = run(&mut a, "print(ActivePage.Layers.Count);").unwrap();
        assert_eq!(out[0], "2");

        run(&mut a, "ActivePage.Layers.Item(2).Locked = true;").unwrap();
        let out = run(&mut a, "print(ActivePage.Layers.Item(2).Locked);").unwrap();
        assert_eq!(out[0], "true");

        run(&mut a, "ActivePage.Layers.Item(2).Delete();").unwrap();
        let out = run(&mut a, "print(ActivePage.Layers.Count);").unwrap();
        assert_eq!(out[0], "1");
    }

    /// What a script does to a page or a layer is one undo step, as a
    /// drawing change is.
    #[test]
    fn page_and_layer_changes_are_one_undo_step() {
        let mut a = app();
        // The first page's name follows the interface language, so the
        // test reads it rather than naming it.
        let first = run(&mut a, "print(ActivePage.Name);").unwrap()[0].clone();
        run(
            &mut a,
            "ActivePage.Name = 'One';\n\
             ActiveDocument.AddPages(1);\n\
             ActiveDocument.Pages.Item(2).Delete();",
        )
        .unwrap();
        let out = run(&mut a, "print(ActivePage.Name);").unwrap();
        assert_eq!(out[0], "One");
        a.undo();
        let out = run(&mut a, "print(ActivePage.Name);").unwrap();
        assert_eq!(out[0], first, "the name came back");
    }

    #[test]
    fn a_text_object_reads_and_sets_its_font_and_size() {
        let mut a = app();
        run(
            &mut a,
            "const t = ActiveLayer.CreateArtisticText(10, 10, 'Hello', 'Arial', 12);",
        )
        .unwrap();
        let out = run(
            &mut a,
            "const s = ActiveLayer.Shapes.Item(1); print(s.Font + ' ' + s.FontSize);",
        )
        .unwrap();
        assert_eq!(out[0], "Arial 12");

        run(&mut a, "ActiveLayer.Shapes.Item(1).FontSize = 36;").unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).FontSize);").unwrap();
        assert_eq!(out[0], "36");

        run(&mut a, "ActiveLayer.Shapes.Item(1).Font = 'Georgia';").unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Font);").unwrap();
        assert_eq!(out[0], "Georgia");

        // The text is still there: styling does not replace it.
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Text);").unwrap();
        assert_eq!(out[0], "Hello");
    }

    #[test]
    fn a_text_object_reads_and_sets_bold_and_italic() {
        let mut a = app();
        run(
            &mut a,
            "const t = ActiveLayer.CreateArtisticText(10, 10, 'Hi', 'Arial', 12);",
        )
        .unwrap();
        run(&mut a, "ActiveLayer.Shapes.Item(1).Bold = true;").unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Bold);").unwrap();
        assert_eq!(out[0], "true");
        run(&mut a, "ActiveLayer.Shapes.Item(1).Italic = true;").unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Italic);").unwrap();
        assert_eq!(out[0], "true");
        run(&mut a, "ActiveLayer.Shapes.Item(1).Bold = false;").unwrap();
        let out = run(
            &mut a,
            "const s = ActiveLayer.Shapes.Item(1); print(s.Bold + ' ' + s.Italic);",
        )
        .unwrap();
        assert_eq!(out[0], "false true");
    }

    /// SetTextStyle changes only the fields it is given, in one call.
    #[test]
    fn set_text_style_changes_the_fields_it_names() {
        let mut a = app();
        run(
            &mut a,
            "const t = ActiveLayer.CreateArtisticText(10, 10, 'Hi', 'Arial', 12);\n\
             ActiveLayer.Shapes.Item(1).SetTextStyle({font:'Georgia', size:20});",
        )
        .unwrap();
        let out = run(
            &mut a,
            "const s = ActiveLayer.Shapes.Item(1);\n\
             print(s.Font + ' ' + s.FontSize + ' ' + s.Bold + ' ' + s.Text);",
        )
        .unwrap();
        assert_eq!(out[0], "Georgia 20 false Hi");
    }

    /// Styling an object that is not text is an error, not a silent
    /// no-op, and a size of nothing leaves the size it had.
    #[test]
    fn styling_that_cannot_apply_is_refused() {
        let mut a = app();
        run(&mut a, "const r = ActiveLayer.CreateRectangle(0,0,10,10);").unwrap();
        assert!(
            run(&mut a, "ActiveLayer.Shapes.Item(1).Font = 'Georgia';").is_err(),
            "a rectangle has no font"
        );
        assert!(
            run(&mut a, "ActiveLayer.Shapes.Item(1).FontSize = 20;").is_err(),
            "a rectangle has no size"
        );
        run(
            &mut a,
            "const t = ActiveLayer.CreateArtisticText(10, 10, 'Hi', 'Arial', 12);",
        )
        .unwrap();
        run(&mut a, "ActiveLayer.Shapes.Item(2).FontSize = 0;").unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(2).FontSize);").unwrap();
        assert_eq!(out[0], "12", "the size it had");
    }

    /// Three rectangles whose left edges are at 0, 10 and 100 end with
    /// theirs at 0, 50 and 100: the outermost stay where they were.
    #[test]
    fn distribute_spreads_left_edges_evenly() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(10, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(100, 0, 10, 10);\n\
             ActivePage.Shapes.All.DistributeLeft();",
        )
        .unwrap();
        let out = run(
            &mut a,
            "print(ActiveLayer.Shapes.All.Shapes.map(s => s.PositionX).join(' '));",
        )
        .unwrap();
        assert_eq!(out[0], "0 50 100");
    }

    /// The gaps come out equal: boxes 10 wide at 0, 20 and 100 leave 40
    /// between them, the outermost staying where they were.
    #[test]
    fn distribute_spacing_makes_the_gaps_equal() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(20, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(100, 0, 10, 10);\n\
             ActivePage.Shapes.All.DistributeSpacingHorizontally();",
        )
        .unwrap();
        let out = run(
            &mut a,
            "print(ActiveLayer.Shapes.All.Shapes.map(s => s.PositionX).join(' '));",
        )
        .unwrap();
        assert_eq!(out[0], "0 50 100");
    }

    /// Along y the boxes go by their top edges, y growing upwards.
    #[test]
    fn distribute_along_y_uses_the_top_edges() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(0, 20, 10, 10);\n\
             ActiveLayer.CreateRectangle(0, 90, 10, 10);\n\
             ActivePage.Shapes.All.DistributeTop();",
        )
        .unwrap();
        // Top edges 10, 30 and 100 end at 10, 55 and 100, so the bottom
        // edges are 0, 45 and 90.
        let out = run(
            &mut a,
            "print(ActiveLayer.Shapes.All.Shapes.map(s => s.PositionY).join(' '));",
        )
        .unwrap();
        assert_eq!(out[0], "0 45 90");
    }

    /// Across the page the outermost boxes reach its edges, each flush by
    /// its own size.
    #[test]
    fn distribute_to_the_page_reaches_its_edges() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(10, 100, 10, 10);\n\
             ActiveLayer.CreateRectangle(50, 100, 10, 10);\n\
             ActivePage.Shapes.All.Distribute('left', 'page');",
        )
        .unwrap();
        let out = run(
            &mut a,
            "print(ActiveLayer.Shapes.All.Shapes.map(s => s.PositionX).join(' '));",
        )
        .unwrap();
        // The page is 210 wide: the first left edge goes to 0, the last to
        // 200, flush with the right edge.
        assert_eq!(out[0], "0 200");
    }

    /// The refusals: too few objects, and a distribution that does not
    /// exist.
    #[test]
    fn distribute_refuses_what_it_cannot_spread() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(50, 0, 10, 10);",
        )
        .unwrap();
        assert!(
            run(&mut a, "ActivePage.Shapes.All.DistributeLeft();").is_err(),
            "two objects have nothing to spread between"
        );
        // Two are enough across the page.
        run(&mut a, "ActivePage.Shapes.All.Distribute('left', 'page');").unwrap();
        assert!(
            run(&mut a, "ActivePage.Shapes.All.Distribute('diagonal');").is_err(),
            "no such distribution"
        );
    }

    /// The whole distribution is one undo step, as an align is.
    #[test]
    fn a_distribution_is_one_undo_step() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(10, 0, 10, 10);\n\
             ActiveLayer.CreateRectangle(100, 0, 10, 10);",
        )
        .unwrap();
        let ler = "print(ActiveLayer.Shapes.All.Shapes.map(s => s.PositionX).join(' '));";
        assert_eq!(run(&mut a, ler).unwrap()[0], "0 10 100");
        run(&mut a, "ActivePage.Shapes.All.DistributeLeft();").unwrap();
        assert_eq!(run(&mut a, ler).unwrap()[0], "0 50 100");
        a.undo();
        assert_eq!(run(&mut a, ler).unwrap()[0], "0 10 100", "one undo");
    }

    /// Every text object of the page takes the replacement, and the call
    /// answers with how many it changed.
    #[test]
    fn replace_text_changes_every_text_on_the_page() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateArtisticText(0, 0, 'Hello World', 'Arial', 12);\n\
             ActiveLayer.CreateArtisticText(0, 40, 'Hello', 'Arial', 12);\n\
             print(ActivePage.ReplaceText('Hello', 'Bye'));",
        )
        .map(|out| assert_eq!(out[0], "2", "two objects changed"))
        .unwrap();
        let out = run(
            &mut a,
            "print(ActivePage.Shapes.All.Shapes.map(s => s.Text).join('|'));",
        )
        .unwrap();
        assert_eq!(out[0], "Bye World|Bye");
    }

    /// Case is ignored unless the call asks for it.
    #[test]
    fn replace_text_takes_case_into_account_when_asked() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateArtisticText(0, 0, 'hello Hello', 'Arial', 12);\n\
             print(ActivePage.ReplaceText('Hello', 'Bye'));",
        )
        .map(|out| assert_eq!(out[0], "1", "one object, both spellings"))
        .unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Text);").unwrap();
        assert_eq!(out[0], "Bye Bye", "case ignored by default");

        // The same call asking for the case replaces only the spelling it
        // was given.
        let mut b = app();
        run(
            &mut b,
            "ActiveLayer.CreateArtisticText(0, 0, 'hello Hello', 'Arial', 12);\n\
             ActivePage.ReplaceText('Hello', 'Bye', {matchCase:true});",
        )
        .unwrap();
        let out = run(&mut b, "print(ActiveLayer.Shapes.Item(1).Text);").unwrap();
        assert_eq!(out[0], "hello Bye");
    }

    /// A whole word is a run of letters or digits, so `cat` does not touch
    /// `scatter`.
    #[test]
    fn replace_text_can_keep_to_whole_words() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateArtisticText(0, 0, 'a cat scatters', 'Arial', 12);\n\
             ActivePage.ReplaceText('cat', 'dog', {wholeWord:true});",
        )
        .unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Text);").unwrap();
        assert_eq!(out[0], "a dog scatters");
    }

    /// Text inside a group is reached, as the interface's Find and Replace
    /// reaches it.
    #[test]
    fn replace_text_reaches_into_groups() {
        let mut a = app();
        run(
            &mut a,
            "const a = ActiveLayer.CreateArtisticText(0, 0, 'one', 'Arial', 12);\n\
             const b = ActiveLayer.CreateArtisticText(0, 40, 'two', 'Arial', 12);\n\
             new ShapeRange([a.id, b.id]).Group();\n\
             print(ActivePage.ReplaceText('one', 'um'));",
        )
        .map(|out| assert_eq!(out[0], "1"))
        .unwrap();
        // Read the group's children from the drawing itself: the object
        // model has no way into a group's contents.
        let shapes = &a.doc().pages[0].layers[0].shapes;
        assert_eq!(shapes.len(), 1, "the group is one object");
        let ShapeKind::Group { children } = &shapes[0].kind else {
            panic!("not a group: {:?}", shapes[0].kind);
        };
        let texts: Vec<String> = children
            .iter()
            .filter_map(|c| match &c.kind {
                ShapeKind::Text { spans, .. } => {
                    Some(spans.iter().map(|x| x.text.as_str()).collect::<String>())
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["um".to_string(), "two".to_string()]);
    }

    /// A range replaces only what it holds.
    #[test]
    fn replace_text_stays_inside_the_range_it_is_given() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateArtisticText(0, 0, 'x', 'Arial', 12);\n\
             ActiveLayer.CreateArtisticText(0, 40, 'x', 'Arial', 12);\n\
             const only = new ShapeRange([ActiveLayer.Shapes.Item(2).id]);\n\
             print(only.ReplaceText('x', 'y'));",
        )
        .map(|out| assert_eq!(out[0], "1", "one of the two"))
        .unwrap();
        let out = run(
            &mut a,
            "print(ActivePage.Shapes.All.Shapes.map(s => s.Text).join('|'));",
        )
        .unwrap();
        assert_eq!(out[0], "x|y");
    }

    /// Nothing to find is an error rather than a silent nothing, and a
    /// script's replacement is one undo step.
    #[test]
    fn replace_text_refuses_an_empty_search_and_undoes_in_one_step() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateArtisticText(0, 0, 'aaa', 'Arial', 12);",
        )
        .unwrap();
        assert!(
            run(&mut a, "ActivePage.ReplaceText('', 'b');").is_err(),
            "an empty search changes nothing and says so"
        );
        run(&mut a, "ActivePage.ReplaceText('a', 'b');").unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Text);").unwrap();
        assert_eq!(out[0], "bbb");
        a.undo();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Text);").unwrap();
        assert_eq!(out[0], "aaa", "one undo puts it back");
    }

    /// Combine turns several objects into one curve, as the interface's
    /// Combine does.
    #[test]
    fn combine_merges_the_objects_into_one() {
        let mut a = app();
        run(
            &mut a,
            "const a = ActiveLayer.CreateRectangle(0, 0, 20, 20);\n\
             const b = ActiveLayer.CreateRectangle(30, 0, 20, 20);\n\
             new ShapeRange([a.id, b.id]).Combine();",
        )
        .unwrap();
        let out = run(&mut a, "print(ActiveLayer.Shapes.Count);").unwrap();
        assert_eq!(out[0], "1", "one object left");
        let out = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Type);").unwrap();
        assert_eq!(out[0], "Curve", "combined into a curve");
    }

    /// BreakApart hands back the objects the subpaths became, so a script
    /// can keep working on them without searching the page again.
    #[test]
    fn break_apart_hands_back_what_it_made() {
        let mut a = app();
        run(
            &mut a,
            "const a = ActiveLayer.CreateRectangle(0, 0, 20, 20);\n\
             const b = ActiveLayer.CreateRectangle(30, 0, 20, 20);\n\
             new ShapeRange([a.id, b.id]).Combine();\n\
             const parts = ActiveLayer.Shapes.Item(1).BreakApart();\n\
             print(parts.length);\n\
             parts[0].Name = 'first';",
        )
        .map(|out| assert_eq!(out[0], "2", "two subpaths, two objects"))
        .unwrap();
        // The returned objects name real ones: naming one worked, and the
        // page has them.
        let out = run(&mut a, "print(ActiveLayer.Shapes.Count);").unwrap();
        assert_eq!(out[0], "2");
        let out = run(
            &mut a,
            "print(ActiveLayer.Shapes.FindShape('first') ? 'sim' : 'nao');",
        )
        .unwrap();
        assert_eq!(out[0], "sim", "the handle names a real object");
    }

    /// Combine and BreakApart are each one undo step, as from the menus.
    #[test]
    fn combine_and_break_apart_are_one_undo_step_each() {
        let mut a = app();
        run(
            &mut a,
            "const a = ActiveLayer.CreateRectangle(0, 0, 20, 20);\n\
             const b = ActiveLayer.CreateRectangle(30, 0, 20, 20);",
        )
        .unwrap();
        let ler = "print(ActiveLayer.Shapes.Count);";
        assert_eq!(run(&mut a, ler).unwrap()[0], "2");
        run(&mut a, "ActivePage.Shapes.All.Combine();").unwrap();
        assert_eq!(run(&mut a, ler).unwrap()[0], "1");
        a.undo();
        assert_eq!(
            run(&mut a, ler).unwrap()[0],
            "2",
            "one undo brings both back"
        );
    }

    /// The refusals: combine needs two objects, and break apart needs an
    /// object that carries an id.
    #[test]
    fn combine_and_break_apart_refuse_what_they_cannot_do() {
        let mut a = app();
        run(
            &mut a,
            "const r = ActiveLayer.CreateRectangle(0, 0, 20, 20);",
        )
        .unwrap();
        assert!(
            run(&mut a, "ActivePage.Shapes.All.Combine();").is_err(),
            "one object has nothing to combine with"
        );
        // A plain rectangle is not a curve, so breaking it apart has
        // nothing to split: no pieces come back and the object stays as it
        // is, which is what the command does from the menus too.
        let out = run(
            &mut a,
            "print(ActiveLayer.Shapes.Item(1).BreakApart().length);",
        )
        .unwrap();
        assert_eq!(out[0], "0", "a rectangle has no subpaths to split");
        let out = run(&mut a, "print(ActiveLayer.Shapes.Count);").unwrap();
        assert_eq!(out[0], "1", "and the object is still there");
    }

    /// Bounds gives the box in one reading, with the edges named the way a
    /// script measuring a layout wants them.
    #[test]
    fn bounds_reads_the_box_in_one_call() {
        let mut a = app();
        run(
            &mut a,
            "const r = ActiveLayer.CreateRectangle(10, 20, 30, 40, 0);\n\
             const b = r.Bounds;\n\
             print(b.left + ',' + b.bottom + ',' + b.right + ',' + b.top);",
        )
        .map(|out| assert_eq!(out[0], "10,20,40,60"))
        .unwrap();
        let out = run(
            &mut a,
            "const b = ActiveLayer.Shapes.Item(1).Bounds;\n\
             print(b.x + ',' + b.y + ',' + b.width + ',' + b.height);",
        )
        .unwrap();
        assert_eq!(out[0], "10,20,30,40");
        let out = run(
            &mut a,
            "const b = ActiveLayer.Shapes.Item(1).Bounds;\n\
             print(b.centerX + ',' + b.centerY);",
        )
        .unwrap();
        assert_eq!(out[0], "25,40", "the centre of the box");
    }

    /// Bounds and the four properties it is made of agree, so a script can
    /// use either without getting two answers.
    #[test]
    fn bounds_agrees_with_the_single_properties() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateEllipse(5, 15, 25, 35);\n\
             const s = ActiveLayer.Shapes.Item(1);\n\
             const b = s.Bounds;\n\
             const iguais = b.x === s.PositionX && b.y === s.PositionY &&\n\
               b.width === s.SizeWidth && b.height === s.SizeHeight &&\n\
               b.centerX === s.CenterX && b.centerY === s.CenterY;\n\
             print(iguais ? 'sim' : 'nao');",
        )
        .map(|out| assert_eq!(out[0], "sim"))
        .unwrap();
    }

    /// A text object's box is its drawn box, not its frame: the box is
    /// settled by the layout, so Bounds reads it back the same way the
    /// single properties do.
    #[test]
    fn bounds_of_text_is_the_drawn_box() {
        let mut a = app();
        run(
            &mut a,
            "const t = ActiveLayer.CreateArtisticText(20, 100, 'Hello', 'Arial', 12);\n\
             const b = t.Bounds;\n\
             print(b.width > 0 && b.height > 0 ? 'sim' : 'nao');",
        )
        .map(|out| assert_eq!(out[0], "sim"))
        .unwrap();
        let out = run(
            &mut a,
            "const t = ActiveLayer.Shapes.Item(1);\n\
             print(t.Bounds.width === t.SizeWidth ? 'sim' : 'nao');",
        )
        .unwrap();
        assert_eq!(out[0], "sim");
    }

    /// FindByProperties finds by type, by fill, and by "no fill", the ways
    /// the interface's Find and Replace by properties does.
    #[test]
    fn find_by_properties_finds_by_type_and_colour() {
        let mut a = app();
        run(
            &mut a,
            "const r = ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             r.Fill.ApplyUniformFill(Color.RGB(255, 0, 0));\n\
             const e = ActiveLayer.CreateEllipse(20, 0, 10, 10);\n\
             e.Fill.ApplyUniformFill(Color.RGB(255, 0, 0));\n\
             ActiveLayer.CreateRectangle(40, 0, 10, 10);",
        )
        .unwrap();
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({type:'Rectangle'}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "2", "two rectangles");
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({fill: Color.RGB(255,0,0)}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "2", "two red objects");
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({fill: null}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "1", "one with no fill");
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({type:'Ellipse', fill: Color.RGB(255,0,0)}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "1", "the criteria are taken together");
    }

    /// ReplaceByProperties swaps one colour for another, and the count is
    /// how many it changed.
    #[test]
    fn replace_by_properties_swaps_the_colour() {
        let mut a = app();
        run(
            &mut a,
            "const r = ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             r.Fill.ApplyUniformFill(Color.RGB(255, 0, 0));\n\
             const e = ActiveLayer.CreateEllipse(20, 0, 10, 10);\n\
             e.Fill.ApplyUniformFill(Color.RGB(255, 0, 0));",
        )
        .unwrap();
        let out = run(
            &mut a,
            "print(ActiveDocument.ReplaceByProperties({fill: Color.RGB(255,0,0), new_fill: Color.RGB(0,0,255)}));",
        )
        .unwrap();
        assert_eq!(out[0], "2", "both changed");
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({fill: Color.RGB(255,0,0)}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "0", "no red left");
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({fill: Color.RGB(0,0,255)}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "2", "both blue now");
    }

    /// SelectByProperties puts what it found in the selection, so the
    /// calls that work on a selection use it.
    #[test]
    fn select_by_properties_selects_what_it_found() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveLayer.CreateEllipse(20, 0, 10, 10);\n\
             print(ActiveDocument.SelectByProperties({type:'Rectangle'}));",
        )
        .map(|out| assert_eq!(out[0], "1"))
        .unwrap();
        let out = run(&mut a, "print(ActiveSelection.Count);").unwrap();
        assert_eq!(out[0], "1", "one object selected");
        let out = run(&mut a, "print(ActiveSelection.Item(1).Type);").unwrap();
        assert_eq!(out[0], "Rectangle");
    }

    /// The whole drawing or just the page. `AddPages` adds a page but does
    /// not go to it, so the script says which page it means.
    #[test]
    fn find_by_properties_scopes_and_undoes() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveDocument.AddPages(1);\n\
             ActiveDocument.Pages.Item(2).Activate();\n\
             ActiveLayer.CreateRectangle(20, 20, 10, 10);",
        )
        .unwrap();
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({type:'Rectangle'}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "1", "the page the script is on");
        let out = run(
            &mut a,
            "print(ActiveDocument.FindByProperties({type:'Rectangle', whole_drawing:true}).length);",
        )
        .unwrap();
        assert_eq!(out[0], "2", "both pages");

        // Back to the first page for the rest.
        run(&mut a, "ActiveDocument.Pages.Item(1).Activate();").unwrap();
        run(
            &mut a,
            "ActiveDocument.ReplaceByProperties({type:'Rectangle', new_outline_width: 2});",
        )
        .unwrap();
        let antes = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Outline.Width);").unwrap();
        assert_eq!(antes[0], "2");
        a.undo();
        let depois = run(&mut a, "print(ActiveLayer.Shapes.Item(1).Outline.Width);").unwrap();
        assert_ne!(depois[0], "2", "one undo puts the width back");
    }

    /// A query with no criterion, or a replacement with nothing to put,
    /// is an error rather than a search that matches everything.
    #[test]
    fn properties_queries_that_cannot_work_are_refused() {
        let mut a = app();
        run(&mut a, "ActiveLayer.CreateRectangle(0, 0, 10, 10);").unwrap();
        assert!(
            run(&mut a, "ActiveDocument.FindByProperties({});").is_err(),
            "no criterion"
        );
        assert!(
            run(
                &mut a,
                "ActiveDocument.FindByProperties({type:'nonsense'});"
            )
            .is_err(),
            "no such type"
        );
        assert!(
            run(
                &mut a,
                "ActiveDocument.ReplaceByProperties({type:'Rectangle'});"
            )
            .is_err(),
            "nothing to put in place"
        );
    }

    /// After `Page.Activate()` a script draws on the page it chose, not on
    /// the one it started from.
    #[test]
    fn page_and_layer_follow_activation() {
        let mut a = app();
        run(
            &mut a,
            "ActiveLayer.CreateRectangle(0, 0, 10, 10);\n\
             ActiveDocument.AddPages(1);\n\
             ActiveDocument.Pages.Item(2).Activate();\n\
             ActiveLayer.CreateRectangle(20, 20, 10, 10);",
        )
        .unwrap();
        let out = run(
            &mut a,
            "print(ActiveDocument.Pages.Item(1).Shapes.Count + ',' + \
                    ActiveDocument.Pages.Item(2).Shapes.Count);",
        )
        .unwrap();
        assert_eq!(out[0], "1,1", "one object on each page");
    }
}
