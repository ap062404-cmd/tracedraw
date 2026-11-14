//! SVG import: parses with `usvg` (which resolves CSS, `use`, units and
//! nested transforms into a flat tree) and converts the result into
//! document shapes. Groups keep their structure, clip paths become
//! clip frames, images become bitmaps, text becomes editable text
//! objects (read by `svg_text`, since usvg is built without its text
//! engine and drops text elements).
//!
//! Coordinates: SVG is in user units at 96 dpi with Y down; the document is
//! in millimetres with Y up, origin at the bottom-left of the page.

use std::sync::Arc;

use tracedraw_core::{
    document::{Shape, ShapeKind},
    geometry::{Affine, BezPath, PathEl, Point, Rect, Size},
    id::IdSource,
    style::{Fountain, FountainKind, LineCap, LineJoin, Stop, Stroke},
    Color, Fill, ShapeId,
};

const PX_TO_MM: f64 = 25.4 / 96.0;

/// Result of an import: shapes in page space (mm, Y up) and the page size
/// the SVG declared.
pub struct Imported {
    pub shapes: Vec<Shape>,
    pub size: Size,
}

/// How deep elements may nest in an SVG file. The XML reader walks the
/// elements by calling itself once per level, with no bound of its own,
/// so a file of nothing but opening `<g>` tags (50 000 of them in
/// 350 KB) ran the stack out, which cannot be caught, and closed the
/// program. Drawings nest a few dozen levels at most; a file deeper
/// than this is refused with a clear error before it is read.
pub const MAX_ELEMENT_DEPTH: usize = 256;

/// How deep the elements of XML text nest: the most elements open at
/// once, counted from the text alone (comments, CDATA sections,
/// processing instructions and the document type skipped, self-closing
/// tags not counted). Text that is not XML gives some number; the XML
/// reader then says what is wrong with it.
fn element_depth(xml: &str) -> usize {
    let b = xml.as_bytes();
    let (mut depth, mut deepest) = (0usize, 0usize);
    let mut i = 0;
    // The end of the section starting at `i` with `open`, after `close`.
    let skip_to = |i: usize, close: &[u8]| -> usize {
        b[i..]
            .windows(close.len())
            .position(|w| w == close)
            .map_or(b.len(), |p| i + p + close.len())
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        if rest.starts_with(b"<!--") {
            i = skip_to(i + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = skip_to(i + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = skip_to(i + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            // The document type, whose internal subset may hold `>`.
            let mut j = i + 2;
            let mut in_subset = false;
            while j < b.len() {
                match b[j] {
                    b'[' => in_subset = true,
                    b']' => in_subset = false,
                    b'>' if !in_subset => break,
                    _ => {}
                }
                j += 1;
            }
            i = j + 1;
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = skip_to(i + 2, b">");
        } else {
            // A start tag: to its `>` outside quoted attribute values;
            // `/>` closes the element at once.
            let mut j = i + 1;
            let mut quote: Option<u8> = None;
            let mut self_closing = false;
            while j < b.len() {
                match (quote, b[j]) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, c @ (b'"' | b'\'')) => quote = Some(c),
                    (None, b'>') => {
                        self_closing = j > 0 && b[j - 1] == b'/';
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            if !self_closing {
                depth += 1;
                deepest = deepest.max(depth);
            }
            i = j + 1;
        }
    }
    deepest
}

/// Parse SVG text. `ids` allocates shape identifiers so the result can be
/// added to an existing document.
pub fn parse(svg: &str, ids: &mut IdSource) -> std::result::Result<Imported, String> {
    let depth = element_depth(svg);
    if depth > MAX_ELEMENT_DEPTH {
        return Err(format!(
            "the file nests {depth} elements deep; at most {MAX_ELEMENT_DEPTH} are read"
        ));
    }
    let opt = usvg::Options::default();
    // Text elements become marker paths that usvg places; should the
    // changed source not read, the file is read as it is, without text.
    let (prepared, mut texts) = crate::svg_text::prepare(svg);
    let (prepared, mut links) = mark_links(&prepared);
    let tree = match usvg::Tree::from_str(&prepared, &opt) {
        Ok(tree) => tree,
        Err(_) if prepared != svg => {
            texts.clear();
            links.clear();
            usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    };
    let size = tree.size();
    let height_px = size.height() as f64;
    // SVG px, Y down -> mm, Y up.
    let to_page = Affine::new([PX_TO_MM, 0.0, 0.0, -PX_TO_MM, 0.0, height_px * PX_TO_MM]);
    let cx = Cx {
        to_page,
        texts: &texts,
        links: &links,
    };
    let mut shapes = Vec::new();
    for node in tree.root().children() {
        if let Some(s) = convert_node(node, &cx, ids) {
            shapes.push(s);
        }
    }
    Ok(Imported {
        shapes,
        size: Size::new(size.width() as f64 * PX_TO_MM, height_px * PX_TO_MM),
    })
}

/// What every node is converted with.
struct Cx<'a> {
    /// SVG px, Y down -> mm, Y up.
    to_page: Affine,
    /// The text elements, by the number their marker path carries.
    texts: &'a [crate::svg_text::SvgText],
    /// The links, by the id of the group their `<a>` element became.
    links: &'a std::collections::HashMap<String, Link>,
}

/// A link of the file: an `<a>` element around objects.
struct Link {
    href: String,
    /// The element had no id: the one it carries was made here and is
    /// not a name.
    made_id: bool,
}

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Turn the `<a>` elements of `svg` that link objects into groups with
/// an id, and give their addresses by that id (#2191). usvg reads an
/// `<a>` as a group but keeps neither its address nor its id, so the
/// links of a file were lost. Links inside text, clips, masks, patterns
/// and markers stay as they are, and so does an element not spelled
/// `<a ...> ... </a>`. A source that cannot be read is given back
/// unchanged.
fn mark_links(svg: &str) -> (String, std::collections::HashMap<String, Link>) {
    let mut links = std::collections::HashMap::new();
    let opt = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let Ok(doc) = roxmltree::Document::parse_with_options(svg, opt) else {
        return (svg.to_string(), links);
    };
    let named = |n: roxmltree::Node, name: &str| {
        n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(SVG_NS)
    };
    let taken: std::collections::HashSet<&str> = doc
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .collect();
    // What takes the place of which bytes of the source.
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut made = 0usize;
    for node in doc.descendants().filter(|n| named(*n, "a")) {
        let Some(href) = node
            .attribute("href")
            .or_else(|| node.attribute((XLINK_NS, "href")))
            .map(str::trim)
            .filter(|h| !h.is_empty())
        else {
            continue;
        };
        if node.ancestors().skip(1).any(|a| {
            ["text", "clipPath", "mask", "pattern", "marker"]
                .iter()
                .any(|name| named(a, name))
        }) {
            continue;
        }
        let range = node.range();
        let Some(text) = svg.get(range.clone()) else {
            continue;
        };
        let opens = text
            .strip_prefix("<a")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_whitespace() || c == '>');
        // The end tag: `</a`, blanks, `>` at the very end.
        let close = text
            .rfind("</a")
            .filter(|c| text.get(c + 3..).is_some_and(|rest| rest.trim() == ">"));
        let (true, Some(close)) = (opens, close) else {
            continue;
        };
        let own = node.attribute("id").filter(|id| !id.is_empty());
        let id = match own {
            Some(id) => id.to_string(),
            None => loop {
                made += 1;
                let id = format!("tracedraw-link-{made}");
                if !taken.contains(id.as_str()) {
                    break id;
                }
            },
        };
        let open = match own {
            Some(_) => "g".to_string(),
            None => format!("g id=\"{id}\""),
        };
        edits.push((range.start + 1..range.start + 2, open));
        edits.push((
            range.start + close + 2..range.start + close + 3,
            "g".to_string(),
        ));
        links.insert(
            id,
            Link {
                href: href.to_string(),
                made_id: own.is_none(),
            },
        );
    }
    if edits.is_empty() {
        return (svg.to_string(), links);
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(svg.len() + edits.len() * 24);
    let mut at = 0;
    for (range, rep) in &edits {
        out.push_str(&svg[at..range.start]);
        out.push_str(rep);
        at = range.end;
    }
    out.push_str(&svg[at..]);
    (out, links)
}

fn convert_node(node: &usvg::Node, cx: &Cx, ids: &mut IdSource) -> Option<Shape> {
    let to_page = cx.to_page;
    match node {
        usvg::Node::Group(g) => convert_group(g, cx, ids),
        usvg::Node::Path(p) => match crate::svg_text::marker_index(p) {
            Some(n) => {
                let text = cx.texts.get(n)?;
                crate::svg_text::to_shape(text, to_page * to_affine(p.abs_transform()), ids)
            }
            None => convert_path(p, to_page, ids),
        },
        usvg::Node::Image(img) => convert_image(img, to_page, ids),
        // usvg is built without its text engine: text elements reach
        // the tree as marker paths (above), never as text nodes.
        usvg::Node::Text(_) => None,
    }
}

fn convert_group(g: &usvg::Group, cx: &Cx, ids: &mut IdSource) -> Option<Shape> {
    let to_page = cx.to_page;
    // usvg already pushed the group transform into the children's
    // absolute transforms, so children are converted with `to_page` only.
    let mut children: Vec<Shape> = g
        .children()
        .iter()
        .filter_map(|c| convert_node(c, cx, ids))
        .collect();
    if children.is_empty() {
        return None;
    }
    // A link's group carries the address; an id made for it is no name.
    let link = cx.links.get(g.id());
    let name = match link {
        Some(l) if l.made_id => "",
        _ => g.id(),
    };
    let opacity = g.opacity().get() as f64;
    let mut shape = if let Some(clip) = g.clip_path() {
        // Clip path -> ClipFrame with the union of the clip's paths as frame.
        // Clip content is in the user space of the clipped group: its
        // nodes' absolute transforms start at the clip's root, so the
        // group's own absolute transform and the clip's transform come
        // first.
        let mut frame_path = BezPath::new();
        collect_paths(clip.root(), &mut frame_path);
        let frame_path =
            to_page * to_affine(g.abs_transform()) * to_affine(clip.transform()) * frame_path;
        if frame_path.elements().is_empty() {
            group_of(children, ids)
        } else {
            let mut frame = Shape::new(
                ShapeId(ids.shape().0),
                ShapeKind::Path {
                    path: frame_path,
                    closed: true,
                },
            );
            frame.fill = Fill::None;
            frame.stroke = None;
            Shape::new(
                ShapeId(ids.shape().0),
                ShapeKind::ClipFrame {
                    frame: Box::new(frame),
                    contents: std::mem::take(&mut children),
                },
            )
        }
    } else if children.len() == 1 && name.is_empty() {
        children.pop().unwrap()
    } else {
        group_of(children, ids)
    };
    if opacity < 1.0 {
        shape.opacity *= opacity;
    }
    if !name.is_empty() {
        shape.name = Some(name.to_string());
    }
    if let Some(link) = link {
        shape.link = Some(link.href.clone());
    }
    Some(shape)
}

fn group_of(children: Vec<Shape>, ids: &mut IdSource) -> Shape {
    Shape::new(ShapeId(ids.shape().0), ShapeKind::Group { children })
}

fn collect_paths(group: &usvg::Group, out: &mut BezPath) {
    for n in group.children() {
        match n {
            usvg::Node::Path(p) => {
                let t = p.abs_transform();
                let path = bez_path(p.data(), to_affine(t));
                out.extend(path.elements().iter().copied());
            }
            usvg::Node::Group(g) => collect_paths(g, out),
            _ => {}
        }
    }
}

fn to_affine(t: usvg::Transform) -> Affine {
    Affine::new([
        t.sx as f64,
        t.ky as f64,
        t.kx as f64,
        t.sy as f64,
        t.tx as f64,
        t.ty as f64,
    ])
}

fn bez_path(data: &usvg::tiny_skia_path::Path, t: Affine) -> BezPath {
    use usvg::tiny_skia_path::PathSegment;
    let mut out = BezPath::new();
    for seg in data.segments() {
        match seg {
            PathSegment::MoveTo(p) => out.move_to(t * Point::new(p.x as f64, p.y as f64)),
            PathSegment::LineTo(p) => out.line_to(t * Point::new(p.x as f64, p.y as f64)),
            PathSegment::QuadTo(a, p) => out.quad_to(
                t * Point::new(a.x as f64, a.y as f64),
                t * Point::new(p.x as f64, p.y as f64),
            ),
            PathSegment::CubicTo(a, b, p) => out.curve_to(
                t * Point::new(a.x as f64, a.y as f64),
                t * Point::new(b.x as f64, b.y as f64),
                t * Point::new(p.x as f64, p.y as f64),
            ),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}

fn convert_path(p: &usvg::Path, to_page: Affine, ids: &mut IdSource) -> Option<Shape> {
    if !p.is_visible() {
        return None;
    }
    let t = to_page * to_affine(p.abs_transform());
    let path = bez_path(p.data(), t);
    if path.elements().is_empty() {
        return None;
    }
    let closed = path
        .elements()
        .iter()
        .any(|e| matches!(e, PathEl::ClosePath));
    let bounds = bbox(&path);
    let mut shape = Shape::new(ShapeId(ids.shape().0), ShapeKind::Path { path, closed });
    shape.fill = match p.fill() {
        Some(f) => convert_paint(f.paint(), f.opacity().get() as f64, bounds, to_page),
        None => Fill::None,
    };
    shape.stroke = p.stroke().map(|s| convert_stroke(s, t));
    if !p.id().is_empty() {
        shape.name = Some(p.id().to_string());
    }
    Some(shape)
}

fn bbox(path: &BezPath) -> Rect {
    use tracedraw_core::geometry::Shape as _;
    path.bounding_box()
}

fn rgb(c: usvg::Color) -> Color {
    Color::rgb8(c.red, c.green, c.blue)
}

fn convert_paint(paint: &usvg::Paint, opacity: f64, bounds: Rect, to_page: Affine) -> Fill {
    let _ = opacity; // per-shape fill opacity is folded into the colour below
    match paint {
        usvg::Paint::Color(c) => Fill::Solid(rgb(*c)),
        usvg::Paint::LinearGradient(g) => {
            let stops = convert_stops(g.stops());
            // Angle from the gradient vector, mapped through the SVG transform
            // and the page flip (which negates the Y component).
            let t = to_page * to_affine(g.transform());
            let a = t * Point::new(g.x1() as f64, g.y1() as f64);
            let b = t * Point::new(g.x2() as f64, g.y2() as f64);
            let angle = (b - a).atan2().to_degrees();
            Fill::Fountain(Fountain {
                kind: FountainKind::Linear,
                stops,
                angle,
                offset: Point::ZERO,
                edge_pad: 0.0,
                ..Default::default()
            })
        }
        usvg::Paint::RadialGradient(g) => {
            let stops = convert_stops(g.stops());
            let t = to_page * to_affine(g.transform());
            let c = t * Point::new(g.cx() as f64, g.cy() as f64);
            let centre = bounds.center();
            let off = if bounds.width() > 0.0 && bounds.height() > 0.0 {
                Point::new(
                    ((c.x - centre.x) / (bounds.width() / 2.0)).clamp(-1.0, 1.0),
                    ((c.y - centre.y) / (bounds.height() / 2.0)).clamp(-1.0, 1.0),
                )
            } else {
                Point::ZERO
            };
            Fill::Fountain(Fountain {
                kind: FountainKind::Radial,
                stops,
                angle: 0.0,
                offset: off,
                edge_pad: 0.0,
                ..Default::default()
            })
        }
        usvg::Paint::Pattern(_) => Fill::Solid(Color::rgb8(128, 128, 128)),
    }
}

fn convert_stops(stops: &[usvg::Stop]) -> Vec<Stop> {
    let mut out: Vec<Stop> = stops
        .iter()
        .map(|s| Stop {
            opacity: s.opacity().get(),
            ..Stop::new(s.offset().get() as f64, rgb(s.color()))
        })
        .collect();
    if out.len() < 2 {
        let c = out.first().map(|s| s.color).unwrap_or(Color::BLACK);
        out = vec![Stop::new(0.0, c), Stop::new(1.0, c)];
    }
    out
}

fn convert_stroke(s: &usvg::Stroke, t: Affine) -> Stroke {
    let color = match s.paint() {
        usvg::Paint::Color(c) => rgb(*c),
        usvg::Paint::LinearGradient(g) => g
            .stops()
            .first()
            .map(|st| rgb(st.color()))
            .unwrap_or(Color::BLACK),
        usvg::Paint::RadialGradient(g) => g
            .stops()
            .first()
            .map(|st| rgb(st.color()))
            .unwrap_or(Color::BLACK),
        usvg::Paint::Pattern(_) => Color::BLACK,
    };
    // Uniform scale of the transform, for the stroke width.
    let scale = (t.as_coeffs()[0].abs() * t.as_coeffs()[3].abs()).sqrt();
    let width = (s.width().get() as f64 * scale).max(Stroke::HAIRLINE);
    let dash = s
        .dasharray()
        .map(|d| {
            d.iter()
                .map(|v| *v as f64 / s.width().get() as f64)
                .collect()
        })
        .unwrap_or_default();
    Stroke {
        color,
        width,
        cap: match s.linecap() {
            usvg::LineCap::Butt => LineCap::Butt,
            usvg::LineCap::Round => LineCap::Round,
            usvg::LineCap::Square => LineCap::Square,
        },
        join: match s.linejoin() {
            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
            usvg::LineJoin::Round => LineJoin::Round,
            usvg::LineJoin::Bevel => LineJoin::Bevel,
        },
        dash,
        miter_limit: Stroke::miter_angle_from_ratio(s.miterlimit().get() as f64),
        ..Default::default()
    }
}

fn convert_image(img: &usvg::Image, to_page: Affine, ids: &mut IdSource) -> Option<Shape> {
    if !img.is_visible() {
        return None;
    }
    let (png, width_px, height_px) = match img.kind() {
        usvg::ImageKind::PNG(data) => {
            let data: Arc<Vec<u8>> = data.clone();
            let (w, h) = png_size(&data)?;
            (data.as_ref().clone(), w, h)
        }
        // Other encodings are left to the host application's image decoder;
        // here only PNG is embedded directly.
        _ => return None,
    };
    let t = to_page * to_affine(img.abs_transform());
    let sz = img.size();
    let r = Rect::new(0.0, 0.0, sz.width() as f64, sz.height() as f64);
    let rect = t.transform_rect_bbox(r);
    let mut shape = Shape::new(
        ShapeId(ids.shape().0),
        ShapeKind::Bitmap {
            rect,
            width_px,
            height_px,
            png: png.into(),
            fx: None,
        },
    );
    shape.stroke = None;
    if !img.id().is_empty() {
        shape.name = Some(img.id().to_string());
    }
    Some(shape)
}

/// Width and height from a PNG header (IHDR).
fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 || &data[1..4] != b"PNG" {
        return None;
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_and_gradient_round_trip_to_page_space() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="192" viewBox="0 0 96 192">
            <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient></defs>
            <g id="layer"><rect id="r" x="0" y="0" width="48" height="48" fill="url(#g)" stroke="#000" stroke-width="2"/>
            <circle cx="48" cy="144" r="24" fill="#0f0"/></g>
        </svg>"##;
        let mut ids = IdSource::default();
        let out = parse(svg, &mut ids).unwrap();
        assert!((out.size.width - 25.4).abs() < 1e-6 && (out.size.height - 50.8).abs() < 1e-6);
        assert_eq!(out.shapes.len(), 1);
        let ShapeKind::Group { children } = &out.shapes[0].kind else {
            panic!("expected a group");
        };
        assert_eq!(children.len(), 2);
        // The rect sits at the top-left of the SVG: in page space its top is at y = 50.8.
        let b = children[0].bounds();
        assert!((b.x0).abs() < 1e-6 && (b.y1 - 50.8).abs() < 1e-6, "{b:?}");
        assert!((b.width() - 12.7).abs() < 1e-6);
        assert!(matches!(children[0].fill, Fill::Fountain(_)));
        let st = children[0].stroke.as_ref().unwrap();
        assert!((st.width - 2.0 * PX_TO_MM).abs() < 1e-6);
        assert_eq!(children[0].name.as_deref(), Some("r"));
        assert_eq!(children[1].fill, Fill::Solid(Color::rgb8(0, 255, 0)));
    }

    /// Objects inside `<a>` come in with the link's address, a lone
    /// object carrying it itself, several as a linked group; what the
    /// SVG export writes for linked objects reads back (#2191).
    #[test]
    fn links_around_objects_are_read() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="96" height="96">
            <a href="https://example.com/one"><rect x="0" y="0" width="10" height="10" fill="#f00"/></a>
            <a xlink:href="mailto:a@b.c" id="mail">
              <rect x="20" y="0" width="10" height="10" fill="#0f0"/>
              <rect x="40" y="0" width="10" height="10" fill="#00f"/>
            </a >
            <rect x="60" y="0" width="10" height="10" fill="#000"/>
        </svg>"##;
        let mut ids = IdSource::default();
        let out = parse(svg, &mut ids).unwrap();
        assert_eq!(out.shapes.len(), 3);
        let one = &out.shapes[0];
        assert!(matches!(one.kind, ShapeKind::Path { .. }), "{:?}", one.kind);
        assert_eq!(one.link.as_deref(), Some("https://example.com/one"));
        assert_eq!(one.name, None, "no name from the id made for the link");
        let two = &out.shapes[1];
        assert!(matches!(&two.kind, ShapeKind::Group { children } if children.len() == 2));
        assert_eq!(two.link.as_deref(), Some("mailto:a@b.c"));
        assert_eq!(two.name.as_deref(), Some("mail"));
        assert_eq!(out.shapes[2].link, None);
    }

    /// A file of 50 000 nested groups is refused before the XML reader
    /// sees it: the reader's recursion ran the stack out, which cannot
    /// be caught, and closed the program. Drawings nested a hundred
    /// deep are read whole, and the count skips what is not an element.
    #[test]
    fn deeply_nested_elements_are_refused_not_a_crash() {
        let nested = |depth: usize| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{}<rect x="1" y="1" width="10" height="10"/>{}</svg>"#,
                "<g>".repeat(depth),
                "</g>".repeat(depth)
            )
        };
        let mut ids = IdSource::default();
        let Err(err) = parse(&nested(50_000), &mut ids) else {
            panic!("a file nested 50 000 deep was read");
        };
        assert!(err.contains("50001 elements deep"), "{err}");
        let imp = parse(&nested(100), &mut ids).expect("a hundred deep is fine");
        let mut shape = &imp.shapes[0];
        while let ShapeKind::Group { children } = &shape.kind {
            shape = &children[0];
        }
        assert!(
            matches!(shape.kind, ShapeKind::Path { .. }),
            "{:?}",
            shape.kind
        );
        // Comments, CDATA, the prologue, the document type, attribute
        // values and self-closing tags hold no elements.
        let text = r#"<?xml version="1.0"?><!DOCTYPE svg [ <!ENTITY a "<g><g>"> ]>
<!-- <g><g><g> --><svg title="<g><g>" xmlns="http://www.w3.org/2000/svg"><g><![CDATA[<g><g><g>]]><rect/><path d='M 0 0'/></g></svg>"#;
        assert_eq!(element_depth(text), 2);
        // The svg, three groups and a self-closing rect.
        assert_eq!(element_depth(&nested(3)), 4);
    }
}

#[cfg(test)]
mod clip_tests {
    use super::*;
    use tracedraw_core::document::ShapeKind;

    #[test]
    fn clip_paths_follow_the_root_scale_and_become_clip_frames() {
        // Millimetre page with a mm viewBox: the root scale is 96/25.4.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
            <defs><clipPath id="c"><path d="M 60 10 H 80 V 30 H 60 Z"/></clipPath></defs>
            <g clip-path="url(#c)"><rect x="0" y="0" width="100" height="50" fill="#f00"/></g>
        </svg>"##;
        let mut ids = IdSource::default();
        let imp = parse(svg, &mut ids).expect("parse");
        assert_eq!(imp.shapes.len(), 1);
        match &imp.shapes[0].kind {
            ShapeKind::ClipFrame { frame, .. } => {
                let b = frame.bounds();
                // y is flipped: SVG y 10..30 on a 50 mm page is 20..40 up.
                assert!(
                    (b.x0 - 60.0).abs() < 1e-4 && (b.x1 - 80.0).abs() < 1e-4,
                    "{b:?}"
                );
                assert!(
                    (b.y0 - 20.0).abs() < 1e-4 && (b.y1 - 40.0).abs() < 1e-4,
                    "{b:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
