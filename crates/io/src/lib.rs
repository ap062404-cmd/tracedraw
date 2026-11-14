//! tracedraw-io: export the document model to interchange formats and read the
//! native `.tdraw` file.

#![allow(clippy::field_reassign_with_default)]
pub mod ai;
pub mod cmyk_bitmap;
pub mod dxf;
pub mod emf;
pub mod eps;
pub mod eps_import;
pub mod ggr;
pub mod html;
pub mod pdf;
pub mod pdf_crypt;
pub(crate) mod pdf_glyphs;
pub mod pdf_import;
mod pdf_indic;
pub mod pdf_paragraphs;
pub mod picture_crop;
pub mod plt;
pub mod psd;
pub mod svg;
pub mod svg_import;
mod svg_text;
pub mod text_import;
pub mod xlsx;

use std::path::Path;
use tracedraw_core::Document;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Core(#[from] tracedraw_core::Error),
    #[error("{0}")]
    Format(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The document as vector exports should see it: unless the document
/// fills open curves, an open curve's fill is kept for its closed subpaths
/// only (the curve becomes a group of the filled closed part and the
/// outlined whole), or dropped when it has none, as the screen shows it.
/// An ellipse made an arc (not a pie) is an open curve too.
pub fn resolve_open_fills(doc: &Document) -> std::borrow::Cow<'_, Document> {
    use tracedraw_core::document::{Shape, ShapeKind};
    use tracedraw_core::Fill;
    fn needs(s: &Shape) -> bool {
        match &s.kind {
            ShapeKind::Group { children } => children.iter().any(needs),
            ShapeKind::ClipFrame { frame, contents } => needs(frame) || contents.iter().any(needs),
            // A curve left open, or an ellipse made an arc (not a pie).
            k => k.is_open_curve() && !matches!(s.fill, Fill::None),
        }
    }
    fn fix(s: &mut Shape) {
        // An arc is an open curve: unfilled, as on screen (a live test of
        // 0.41). One whose ends meet is a whole ellipse and keeps it.
        if matches!(s.kind, ShapeKind::Ellipse { .. }) {
            let open = s.kind.is_open_curve()
                && tracedraw_render::closed_subpaths(&s.local_path())
                    .elements()
                    .is_empty();
            if open {
                s.fill = Fill::None;
            }
            return;
        }
        match &mut s.kind {
            ShapeKind::Path {
                path,
                closed: false,
            } if !matches!(s.fill, Fill::None) => {
                let closed = tracedraw_render::closed_subpaths(path);
                if closed.elements().is_empty() {
                    s.fill = Fill::None;
                    return;
                }
                let mut filled = s.clone();
                filled.kind = ShapeKind::Path {
                    path: closed,
                    closed: true,
                };
                filled.stroke = None;
                filled.effects.clear();
                filled.transform = tracedraw_core::geometry::Affine::IDENTITY;
                let mut outlined = s.clone();
                outlined.fill = Fill::None;
                outlined.effects.clear();
                outlined.transform = tracedraw_core::geometry::Affine::IDENTITY;
                s.kind = ShapeKind::Group {
                    children: vec![filled, outlined],
                };
                s.fill = Fill::None;
                s.stroke = None;
            }
            ShapeKind::Group { children } => children.iter_mut().for_each(fix),
            ShapeKind::ClipFrame { frame, contents } => {
                fix(frame);
                contents.iter_mut().for_each(fix);
            }
            _ => {}
        }
    }
    if doc.metadata.fill_open_curves || !doc.all_layers().flat_map(|l| &l.shapes).any(needs) {
        return std::borrow::Cow::Borrowed(doc);
    }
    let mut out = doc.clone();
    for page in &mut out.pages {
        for layer in &mut page.layers {
            layer.shapes.iter_mut().for_each(fix);
        }
    }
    for layer in &mut out.master {
        layer.shapes.iter_mut().for_each(fix);
    }
    std::borrow::Cow::Owned(out)
}

/// The document as vector exports should see it: open curves' fills as
/// the screen shows them ([`resolve_open_fills`]) and the pictures in
/// clip frames without the pixels their frames hide
/// ([`picture_crop::trim_framed_pictures`]), of page `page` (by index)
/// alone or of every page.
pub fn for_export(doc: &Document, page: Option<usize>) -> std::borrow::Cow<'_, Document> {
    match resolve_open_fills(doc) {
        std::borrow::Cow::Borrowed(d) => picture_crop::trim_framed_pictures(d, page),
        std::borrow::Cow::Owned(d) => {
            let trimmed = match picture_crop::trim_framed_pictures(&d, page) {
                std::borrow::Cow::Owned(t) => Some(t),
                std::borrow::Cow::Borrowed(_) => None,
            };
            std::borrow::Cow::Owned(trimmed.unwrap_or(d))
        }
    }
}

/// Native format: compact JSON with a `.tdraw` extension.
pub fn save_native(doc: &Document, path: impl AsRef<Path>) -> Result<()> {
    // Written as it is made, not gathered in memory first.
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    doc.write_json_version(&mut w, tracedraw_core::document::TDRAW_FORMAT_VERSION)?;
    std::io::Write::flush(&mut w)?;
    Ok(())
}

pub fn load_native(path: impl AsRef<Path>) -> Result<Document> {
    let s = std::fs::read_to_string(path)?;
    Ok(Document::from_json(&s)?)
}

/// Write every page to a PDF file.
pub fn save_pdf(doc: &Document, path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path, pdf::document_to_pdf(doc))?;
    Ok(())
}

/// Write one page as an ASCII DXF file.
pub fn save_dxf(doc: &Document, page_index: usize, path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path, dxf::page_to_dxf(doc, page_index))?;
    Ok(())
}

/// Write one page as an HPGL plotter file.
pub fn save_plt(doc: &Document, page_index: usize, path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path, plt::page_to_plt(doc, page_index))?;
    Ok(())
}

/// Write one page as a Photoshop file (one raster layer per document layer).
pub fn save_psd(doc: &Document, page_index: usize, dpi: f64, path: impl AsRef<Path>) -> Result<()> {
    let bytes = psd::page_to_psd(doc, page_index, dpi)
        .ok_or_else(|| Error::Format("page could not be rendered".into()))?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Write one page as an enhanced metafile (EMF).
pub fn save_emf(doc: &Document, page_index: usize, path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path, emf::page_to_emf(doc, page_index))?;
    Ok(())
}

/// Write one page as a placeable Windows metafile (WMF).
pub fn save_wmf(doc: &Document, page_index: usize, path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path, emf::page_to_wmf(doc, page_index))?;
    Ok(())
}

/// Write the first page (or `page_index`) as an SVG file.
pub fn save_svg(doc: &Document, page_index: usize, path: impl AsRef<Path>) -> Result<()> {
    std::fs::write(path, svg::page_to_svg(doc, page_index))?;
    Ok(())
}

/// A PNG picture of each page, its longer side `max_side` pixels on a
/// white background, as `(png, width, height)` in page order (the
/// pictures `.cdr` packages store with the drawing). Pages that cannot
/// be rendered are left out of the list's end.
pub fn page_pictures(doc: &Document, max_side: u32) -> Vec<(Vec<u8>, u32, u32)> {
    let mut out = Vec::new();
    for page in &doc.pages {
        let longest = page.size.width.max(page.size.height);
        if !longest.is_finite() || longest <= 0.0 {
            break;
        }
        // Pixels per inch that make the longer side `max_side` pixels.
        let dpi = f64::from(max_side.max(1)) / (longest / 25.4);
        let Some(pm) = tracedraw_render::render_page_image(doc, page.id, dpi) else {
            break;
        };
        let (w, h) = (pm.width(), pm.height());
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for p in pm.pixels() {
            let c = p.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        let png = svg::encode_png(w, h, &rgba);
        if png.is_empty() {
            break;
        }
        out.push((png, w, h));
    }
    out
}

/// The resolution of the drop shadow pictures [`shadow_pictures`] makes
/// for files: enough for a printed page, small enough for a file.
pub const SHADOW_PICTURE_DPI: f64 = 150.0;

/// The drop shadows of the drawing's visible objects as pictures at
/// `dpi`, for file formats that keep no shadows (`.cdr` writes each as a
/// bitmap beneath its object): every object with a shadow, on any page,
/// in any group or clip frame, as `(shape, page rectangle, png, width,
/// height)`, the picture holding the shadow alone with its transparency.
pub fn shadow_pictures(
    doc: &Document,
    dpi: f64,
) -> Vec<(
    tracedraw_core::ShapeId,
    tracedraw_core::Rect,
    Vec<u8>,
    u32,
    u32,
)> {
    use tracedraw_core::{Affine, Shape, ShapeKind};
    fn walk(
        shapes: &[Shape],
        parent: Affine,
        dpi: f64,
        out: &mut Vec<(
            tracedraw_core::ShapeId,
            tracedraw_core::Rect,
            Vec<u8>,
            u32,
            u32,
        )>,
    ) {
        for s in shapes {
            if !s.visible {
                continue;
            }
            if s.shadow.is_some_and(|sh| sh.opacity > 0.0) {
                let mut on_page = s.clone();
                on_page.absorb(parent);
                if let Some((rect, pm)) = tracedraw_render::render_shadow_image(&on_page, dpi) {
                    let (w, h) = (pm.width(), pm.height());
                    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                    for p in pm.pixels() {
                        let c = p.demultiply();
                        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
                    }
                    let png = svg::encode_png(w, h, &rgba);
                    if !png.is_empty() {
                        out.push((s.id, rect, png, w, h));
                    }
                }
            }
            let inner = parent * s.transform;
            match &s.kind {
                ShapeKind::Group { children } => walk(children, inner, dpi, out),
                ShapeKind::ClipFrame { frame, contents } => {
                    walk(std::slice::from_ref(frame), inner, dpi, out);
                    walk(contents, inner, dpi, out);
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for page in &doc.pages {
        for layer in &page.layers {
            walk(&layer.shapes, Affine::IDENTITY, dpi, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::{
        document::{Shape, ShapeKind},
        geometry::{Rect, Size},
        Color, Fill, Pattern, Stroke,
    };

    /// #1331: an inverted star (negative sharpness) reopens as it was
    /// saved, and the SVG export
    /// draws the inverted outline.
    #[test]
    fn inverted_stars_round_trip_in_native_files_and_export_their_outline() {
        use tracedraw_core::geometry::{Affine, Shape as _};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Polygon {
                rect: Rect::new(20.0, 20.0, 80.0, 80.0),
                points: 5,
                sharpness: -0.7,
                complex: None,
                twist: 0.0,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        doc.layer_mut(layer).unwrap().shapes.push(s.clone());
        let path = std::env::temp_dir().join(format!(
            "tracedraw-inverted-star-{}-{:?}.tdraw",
            std::process::id(),
            std::thread::current().id()
        ));
        save_native(&doc, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let back = load_native(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(text.contains("\"sharpness\":-0.7"), "{text}");
        assert_eq!(back, doc);
        // SVG: the outline as the model draws it, inner points past the
        // centre (y flipped to the SVG's top-left origin).
        let h = doc.pages[0].size.height;
        let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, h]);
        let want = svg::path_data(&(flip * s.page_path()));
        let out = svg::page_to_svg(&doc, 0);
        assert!(out.contains(&want), "{want}\n{out}");
        let second = s.page_path().elements()[1].end_point().unwrap();
        assert!(second.y < 50.0, "{second:?}");
        assert!(s.page_path().bounding_box().width() > 0.0);
    }

    #[test]
    fn open_curves_lose_their_fill_in_vector_exports() {
        use tracedraw_core::geometry::{BezPath, Point};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut path = BezPath::new();
        path.move_to(Point::new(10.0, 10.0));
        path.line_to(Point::new(50.0, 10.0));
        path.line_to(Point::new(50.0, 50.0));
        let mut open = Shape::new(
            tracedraw_core::ShapeId(5),
            ShapeKind::Path {
                path,
                closed: false,
            },
        );
        open.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        open.stroke = Some(Stroke::hairline(Color::BLACK));
        doc.layer_mut(layer).unwrap().shapes.push(open);
        let resolved = resolve_open_fills(&doc);
        let s = &resolved.pages[0].layers[0].shapes[0];
        assert!(matches!(s.fill, Fill::None));
        assert!(s.stroke.is_some());
        let svg = svg::page_to_svg(&doc, 0);
        assert!(!svg.contains("fill=\"#ff0000\""), "{svg}");
        // A document that fills open curves exports the fill.
        doc.metadata.fill_open_curves = true;
        assert!(matches!(
            resolve_open_fills(&doc),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    /// A live test of 0.41: an ellipse made an arc with the Shape tool
    /// kept its fill. It is an open curve: unfilled in SVG and PDF while
    /// the document does not fill open curves; a pie keeps its fill.
    #[test]
    fn an_ellipse_arc_loses_its_fill_in_vector_exports_and_a_pie_keeps_it() {
        use tracedraw_core::document::EllipseArc;
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let ellipse = |id: u64, pie: bool, x: f64| {
            let mut s = Shape::new(
                tracedraw_core::ShapeId(id),
                ShapeKind::Ellipse {
                    rect: Rect::new(x, 10.0, x + 40.0, 50.0),
                    arc: Some(EllipseArc {
                        start_deg: 200.0,
                        end_deg: 90.0,
                        pie,
                    }),
                },
            );
            s.stroke = Some(Stroke::hairline(Color::BLACK));
            s
        };
        let mut arc = ellipse(5, false, 10.0);
        arc.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        let mut pie = ellipse(6, true, 60.0);
        pie.fill = Fill::Solid(Color::rgb8(0, 0, 255));
        doc.layer_mut(layer).unwrap().shapes.extend([arc, pie]);
        let resolved = resolve_open_fills(&doc);
        let shapes = &resolved.pages[0].layers[0].shapes;
        assert!(matches!(shapes[0].fill, Fill::None), "{:?}", shapes[0].fill);
        assert!(shapes[0].stroke.is_some());
        assert_eq!(shapes[1].fill, Fill::Solid(Color::rgb8(0, 0, 255)));
        let svg = svg::page_to_svg(&doc, 0);
        assert!(!svg.contains("#ff0000"), "{svg}");
        assert!(svg.contains("#0000ff"), "{svg}");
        // A document that fills open curves keeps the arc's fill.
        doc.metadata.fill_open_curves = true;
        assert!(svg::page_to_svg(&doc, 0).contains("#ff0000"));
    }

    #[test]
    fn vector_pattern_fill_round_trips_through_the_native_format() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        // A tile with a filled square and an outlined group, built from
        // page-space shapes so the helper has to move them to the origin.
        let mut square = Shape::new(
            tracedraw_core::ShapeId(7),
            ShapeKind::Rect {
                rect: Rect::new(40.0, 40.0, 45.0, 45.0),
                radius: 0.0,
                corners: None,
            },
        );
        square.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        square.stroke = None;
        let mut circle = Shape::new(
            tracedraw_core::ShapeId(8),
            ShapeKind::Ellipse {
                rect: Rect::new(45.0, 45.0, 50.0, 50.0),
                arc: None,
            },
        );
        circle.fill = Fill::linear(Color::rgb8(0, 0, 255), Color::WHITE, 45.0);
        circle.stroke = Some(Stroke::new(Color::BLACK, 0.5));
        let group = Shape::new(
            tracedraw_core::ShapeId(9),
            ShapeKind::Group {
                children: vec![circle],
            },
        );
        let pattern = Pattern::vector_from_shapes(&[square, group]);
        assert!(matches!(&pattern, Pattern::Vector { tile, .. } if *tile == Size::new(10.0, 10.0)));
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 60.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Pattern(pattern);
        doc.layer_mut(layer).unwrap().shapes.push(s);

        let path = std::env::temp_dir().join(format!(
            "tracedraw-vector-pattern-{}-{:?}.tdraw",
            std::process::id(),
            std::thread::current().id()
        ));
        save_native(&doc, &path).unwrap();
        let back = load_native(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, doc);
        let shapes = &back.pages[0].layers[0].shapes;
        assert!(matches!(
            &shapes[0].fill,
            Fill::Pattern(Pattern::Vector { shapes, .. }) if shapes.len() == 2
        ));
    }

    /// A pattern's placement is kept in the native file, and a fill without
    /// one writes as before (no `placement` key).
    #[test]
    fn pattern_placement_round_trips_through_the_native_format() {
        use tracedraw_core::style::{PatternPlacement, PatternTile, TileShift};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut make = |x: f64, placement: PatternPlacement| {
            let id = doc.ids_mut().shape();
            let mut s = Shape::new(
                id,
                ShapeKind::Rect {
                    rect: Rect::new(x, 10.0, x + 20.0, 30.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = Fill::Pattern(Pattern::TwoColor {
                tile: PatternTile::Bricks,
                front: Color::rgb8(255, 0, 0),
                back: Color::WHITE,
                size_mm: 8.0,
                placement,
            });
            doc.layer_mut(layer).unwrap().shapes.push(s);
        };
        make(10.0, PatternPlacement::default());
        make(
            40.0,
            PatternPlacement {
                mirror_horizontal: true,
                mirror_vertical: true,
                offset_x: 1.5,
                offset_y: -2.0,
                rotation: 15.0,
                skew: 5.0,
                shift_percent: 25.0,
                shift: TileShift::Column,
                size: Some(Size::new(8.0, 4.0)),
                transform_with_object: true,
            },
        );
        let path = std::env::temp_dir().join(format!(
            "tracedraw-pattern-placement-{}-{:?}.tdraw",
            std::process::id(),
            std::thread::current().id()
        ));
        save_native(&doc, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let back = load_native(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, doc);
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(
            text.matches("placement").count(),
            1,
            "only the placed fill writes one"
        );
    }

    /// A procedural fill with every number set, a seed and no background
    /// comes back from the native file as it was (#1212).
    #[test]
    fn procedural_fill_round_trips_through_the_native_format() {
        use tracedraw_core::{Procedural, ProceduralParams, ProceduralPattern};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 60.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Procedural(Procedural {
            pattern: ProceduralPattern::Speckle,
            params: ProceduralParams {
                size_mm: 12.5,
                line_width_mm: 0.75,
                share: 0.4,
                amplitude: 0.3,
                turns: 4.5,
                density: 55.0,
            },
            foreground: Color::cmyk_pct(0.0, 100.0, 100.0, 0.0),
            background: None,
            seed: 42,
        });
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let path = std::env::temp_dir().join(format!(
            "tracedraw-procedural-{}-{:?}.tdraw",
            std::process::id(),
            std::thread::current().id()
        ));
        save_native(&doc, &path).unwrap();
        let back = load_native(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, doc);
        assert!(matches!(
            &back.pages[0].layers[0].shapes[0].fill,
            Fill::Procedural(p) if p.seed == 42 && p.params.density == 55.0
        ));
    }

    /// The axis values of a variable font set on text are kept in the
    /// native file (#1013): a weight of 600 reopens at 600.
    #[test]
    fn variable_font_axes_round_trip_in_native_files() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut span = tracedraw_core::TextSpan::new("Weight", "MiSans VF", 24.0);
        span.set_variation("wght", Some(600.0));
        span.set_variation("wdth", Some(87.5));
        let s = Shape::new(
            id,
            ShapeKind::Text {
                spans: vec![span],
                origin: tracedraw_core::geometry::Point::new(10.0, 20.0),
                frame: None,
                align: tracedraw_core::TextAlign::Left,
                para: tracedraw_core::document::ParagraphStyle::default(),
                on_path: None,
            },
        );
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let path = std::env::temp_dir().join(format!(
            "tracedraw-font-axes-{}-{:?}.tdraw",
            std::process::id(),
            std::thread::current().id()
        ));
        save_native(&doc, &path).unwrap();
        let back = load_native(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, doc);
        let ShapeKind::Text { spans, .. } = &back.pages[0].layers[0].shapes[0].kind else {
            panic!("not text");
        };
        assert_eq!(spans[0].variation("wght"), Some(600.0));
        assert_eq!(spans[0].variation("wdth"), Some(87.5));
    }

    /// Character styles (#1209) are kept in the native file: the
    /// underline and strike line styles, all caps, enclosed and the
    /// overprints; a single thin underline is still written as `true`.
    #[test]
    fn character_styles_round_trip_in_native_files() {
        use tracedraw_core::LineStyle;
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut a = tracedraw_core::TextSpan::new("Styled ", "Selawik", 24.0);
        a.underline = LineStyle::DoubleThinWord;
        a.strikethrough = LineStyle::SingleThick;
        a.all_caps = true;
        a.enclosed = true;
        a.overprint_fill = Some(true);
        a.overprint_outline = Some(false);
        let mut b = tracedraw_core::TextSpan::new("plain", "Selawik", 24.0);
        b.underline = LineStyle::SingleThin;
        let s = Shape::new(
            id,
            ShapeKind::Text {
                spans: vec![a, b],
                origin: tracedraw_core::geometry::Point::new(10.0, 20.0),
                frame: None,
                align: tracedraw_core::TextAlign::Left,
                para: tracedraw_core::document::ParagraphStyle::default(),
                on_path: None,
            },
        );
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let path = std::env::temp_dir().join(format!(
            "tracedraw-character-styles-{}-{:?}.tdraw",
            std::process::id(),
            std::thread::current().id()
        ));
        save_native(&doc, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let back = load_native(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, doc);
        assert!(text.contains("\"double_thin_word\""), "{text}");
        assert!(text.contains("\"underline\":true"), "{text}");
        let ShapeKind::Text { spans, .. } = &back.pages[0].layers[0].shapes[0].kind else {
            panic!("not text");
        };
        assert_eq!(spans[0].underline, LineStyle::DoubleThinWord);
        assert_eq!(spans[0].strikethrough, LineStyle::SingleThick);
        assert!(spans[0].all_caps && spans[0].enclosed);
        assert_eq!(spans[0].overprint_fill, Some(true));
        assert_eq!(spans[0].overprint_outline, Some(false));
        assert_eq!(spans[1].underline, LineStyle::SingleThin);
        assert_eq!(spans[1].strikethrough, LineStyle::None);
    }

    #[test]
    fn outline_position_mitre_limit_dashes_and_shadows_round_trip() {
        use tracedraw_core::shadow::{FeatherDirection, FeatherEdges, Perspective, ShadowEdge};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: tracedraw_core::Rect::new(10.0, 10.0, 40.0, 30.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.stroke = Some(Stroke {
            position: tracedraw_core::OutlinePosition::Outside,
            miter_limit: 17.5,
            align_dashes: true,
            dash: vec![5.0, 1.0, 1.0, 1.0],
            ..Stroke::new(Color::BLACK, 1.0)
        });
        s.shadow = Some(tracedraw_core::Shadow {
            merge: tracedraw_core::live::MergeMode::IfDarker,
            feather_direction: FeatherDirection::Outside,
            feather_edges: FeatherEdges::InvertedSquared,
            perspective: Some(Perspective {
                edge: ShadowEdge::Top,
                angle: 75.0,
                stretch: 0.4,
                fade: 0.3,
            }),
            ..Default::default()
        });
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let json = doc.to_json().unwrap();
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, doc);
    }

    /// Every merge mode of a transparency survives the native format, the
    /// new ones under their lower-case names (#1203).
    #[test]
    fn transparency_merge_modes_round_trip_in_native_files() {
        use tracedraw_core::live::{Effect, MergeMode};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        for merge in MergeMode::ALL {
            let id = doc.ids_mut().shape();
            let mut s = Shape::new(
                id,
                ShapeKind::Rect {
                    rect: tracedraw_core::Rect::new(10.0, 10.0, 40.0, 30.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.effects.push(Effect::Transparency {
                mask: Fill::linear(Color::WHITE, Color::BLACK, 0.0),
                merge,
                target: 2,
            });
            doc.layer_mut(layer).unwrap().shapes.push(s);
        }
        let json = doc.to_json().unwrap();
        for name in [
            "\"texturize\"",
            "\"lightness\"",
            "\"behind\"",
            "\"screen\"",
            "\"overlay\"",
            "\"softlight\"",
            "\"hardlight\"",
            "\"colordodge\"",
            "\"colorburn\"",
            "\"exclusion\"",
            "\"iflighter\"",
        ] {
            assert!(json.contains(name), "{name}");
        }
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, doc);
    }

    /// A gradient's blend direction, steps, smooth transition and
    /// acceleration are kept in the native file, and left out while they
    /// are the defaults (#1208).
    #[test]
    fn gradient_blend_settings_round_trip_in_native_files() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: tracedraw_core::Rect::new(10.0, 10.0, 40.0, 30.0),
                radius: 0.0,
                corners: None,
            },
        );
        let mut f = tracedraw_core::Fountain::two(
            tracedraw_core::FountainKind::Radial,
            Color::rgb8(255, 0, 0),
            Color::rgb8(0, 0, 255),
            0.0,
        );
        f.blend_direction = tracedraw_core::ColorBlend::Clockwise;
        f.steps = 24;
        f.smooth = true;
        f.acceleration = 35;
        s.fill = Fill::Fountain(f);
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let json = doc.to_json().unwrap();
        for what in [
            "\"blend_direction\"",
            "\"steps\":24",
            "\"smooth\":true",
            "\"acceleration\":35",
        ] {
            assert!(json.contains(what), "{what} in {json}");
        }
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, doc);
        // The defaults are not written.
        let mut plain = Document::default();
        let layer = plain.pages[0].layers[0].id;
        let id = plain.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: tracedraw_core::Rect::new(10.0, 10.0, 40.0, 30.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::linear(Color::BLACK, Color::WHITE, 0.0);
        plain.layer_mut(layer).unwrap().shapes.push(s);
        let json = plain.to_json().unwrap();
        for what in ["blend_direction", "steps", "smooth", "acceleration"] {
            assert!(!json.contains(what), "{what} in {json}");
        }
    }

    #[test]
    fn corners_that_keep_their_size_survive_native_svg_and_pdf() {
        use tracedraw_core::geometry::{Affine, Shape as _};
        use tracedraw_core::{CornerKind, Corners};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut c = Corners::uniform(5.0, CornerKind::Round);
        c.fixed = true;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::rect_with_corners(Rect::new(0.0, 0.0, 50.0, 30.0), c),
        );
        // Stretched to 100 x 30 mm: the corners stay 5 mm circles.
        s.transform = Affine::translate((20.0, 20.0)) * Affine::scale_non_uniform(2.0, 1.0);
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = None;
        let area = s.page_path().area().abs();
        let circles = 100.0 * 30.0 - (4.0 - std::f64::consts::PI) * 25.0;
        assert!((area - circles).abs() < 0.05, "{area}");
        doc.layer_mut(layer).unwrap().shapes.push(s);
        // Native: the flag is written and read back.
        let json = doc.to_json().unwrap();
        assert!(json.contains("\"fixed\":true"), "{json}");
        assert_eq!(Document::from_json(&json).unwrap(), doc);
        // SVG and PDF draw the outline the rectangle shows.
        let mut ids = tracedraw_core::id::IdSource::default();
        let svg = svg_import::parse(&svg::page_to_svg(&doc, 0), &mut ids).expect("svg");
        let pdf = pdf_import::to_document(
            pdf_import::parse(&pdf::document_to_pdf(&doc), &mut ids).expect("pdf"),
            "t",
        );
        let pdf_shapes: Vec<Shape> = pdf.all_layers().flat_map(|l| l.shapes.clone()).collect();
        for (what, shapes) in [("svg", svg.shapes), ("pdf", pdf_shapes)] {
            let got: f64 = shapes.iter().map(|s| s.page_path().area().abs()).sum();
            assert!((got - area).abs() < 1.0, "{what}: {got} vs {area}");
        }
    }

    /// A corner past half the shorter side, where its neighbours leave it
    /// room, is kept by the native format and drawn the same in SVG and
    /// PDF (#1757).
    #[test]
    fn a_corner_taking_a_whole_side_survives_native_svg_and_pdf() {
        use tracedraw_core::geometry::{Affine, Shape as _};
        use tracedraw_core::{CornerKind, Corners};
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut c = Corners::uniform(0.0, CornerKind::Round);
        c.fixed = true;
        c.radii[Corners::TOP_LEFT] = 10.0;
        c.radii[Corners::TOP_RIGHT] = 80.0;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::rect_with_corners(Rect::new(0.0, 0.0, 100.0, 80.0), c),
        );
        s.transform = Affine::translate((20.0, 20.0));
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = None;
        // Quarter circles of 10 and 80 mm, nothing shrunk.
        let area = s.page_path().area().abs();
        let want = 8000.0 - (4.0 - std::f64::consts::PI) / 4.0 * (100.0 + 6400.0);
        assert!((area - want).abs() < 2.0, "{area} vs {want}");
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let json = doc.to_json().unwrap();
        assert_eq!(Document::from_json(&json).unwrap(), doc);
        let mut ids = tracedraw_core::id::IdSource::default();
        let svg = svg_import::parse(&svg::page_to_svg(&doc, 0), &mut ids).expect("svg");
        let pdf = pdf_import::to_document(
            pdf_import::parse(&pdf::document_to_pdf(&doc), &mut ids).expect("pdf"),
            "t",
        );
        let pdf_shapes: Vec<Shape> = pdf.all_layers().flat_map(|l| l.shapes.clone()).collect();
        for (what, shapes) in [("svg", svg.shapes), ("pdf", pdf_shapes)] {
            let got: f64 = shapes.iter().map(|s| s.page_path().area().abs()).sum();
            assert!((got - area).abs() < 1.0, "{what}: {got} vs {area}");
        }
    }

    #[test]
    fn custom_arrowhead_round_trips_through_the_native_format() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut tri = tracedraw_core::BezPath::new();
        tri.move_to((0.0, 0.0));
        tri.line_to((20.0, 10.0));
        tri.line_to((0.0, 20.0));
        tri.close_path();
        let id = doc.ids_mut().shape();
        let mut line = Shape::new(
            id,
            ShapeKind::Path {
                path: {
                    let mut p = tracedraw_core::BezPath::new();
                    p.move_to((10.0, 50.0));
                    p.line_to((60.0, 50.0));
                    p
                },
                closed: false,
            },
        );
        line.stroke = Some(Stroke {
            start_arrow: tracedraw_core::Arrowhead::Circle,
            end_arrow: tracedraw_core::Arrowhead::from_shape_path(&tri, "My head"),
            ..Stroke::new(Color::BLACK, 1.0)
        });
        doc.layer_mut(layer).unwrap().shapes.push(line);
        let json = doc.to_json().unwrap();
        assert!(json.contains("\"custom\""));
        assert!(json.contains("My head"));
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, doc);
        let s = &back.pages[0].layers[0].shapes[0];
        let st = s.stroke.as_ref().unwrap();
        assert!(st.end_arrow.is_custom());
        assert_eq!(st.end_arrow.name(), "My head");
        assert_eq!(st.start_arrow, tracedraw_core::Arrowhead::Circle);
    }

    /// A feature switched off ("-liga") and a superscript are kept in the
    /// native file (#1201); a file written without them reads as before.
    #[test]
    fn switched_off_features_and_superscripts_round_trip_in_native_files() {
        use tracedraw_core::TextPosition;
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut span = tracedraw_core::TextSpan::new("fi", "Quicksand", 24.0);
        span.set_feature("liga", Some(false));
        span.set_feature("smcp", Some(true));
        let mut sup = tracedraw_core::TextSpan::new("2", "Quicksand", 24.0);
        sup.position = TextPosition::Superscript;
        let mut sub = tracedraw_core::TextSpan::new("3", "Quicksand", 24.0);
        sub.position = TextPosition::Subscript;
        let s = Shape::new(
            id,
            ShapeKind::Text {
                spans: vec![span, sup, sub],
                origin: tracedraw_core::geometry::Point::new(10.0, 20.0),
                frame: None,
                align: tracedraw_core::TextAlign::Left,
                para: tracedraw_core::document::ParagraphStyle::default(),
                on_path: None,
            },
        );
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let path = std::env::temp_dir().join(format!(
            "tracedraw-features-{}-{:?}.tdraw",
            std::process::id(),
            std::thread::current().id()
        ));
        save_native(&doc, &path).unwrap();
        let back = load_native(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, doc);
        let ShapeKind::Text { spans, .. } = &back.pages[0].layers[0].shapes[0].kind else {
            panic!("not text");
        };
        assert!(!spans[0].feature_on("liga"));
        assert!(spans[0].feature_on("smcp"));
        assert_eq!(
            spans[0].features,
            vec!["-liga".to_string(), "smcp".to_string()]
        );
        assert_eq!(spans[1].position, TextPosition::Superscript);
        assert_eq!(spans[2].position, TextPosition::Subscript);
        // Written by a version before #1201: the features as they were,
        // the characters on the baseline.
        let json = doc.to_json().unwrap();
        let old = json.replace("\"position\":\"superscript\"", "\"features\":[]");
        assert_ne!(old, json);
        let back = Document::from_json(&old).unwrap();
        let ShapeKind::Text { spans, .. } = &back.pages[0].layers[0].shapes[0].kind else {
            panic!("not text");
        };
        assert_eq!(spans[1].position, TextPosition::Normal);
    }

    /// #1210: the arrowhead placements and Share attributes round trip
    /// through the native format, and a drawing without them is written
    /// as before.
    #[test]
    fn arrowhead_placement_round_trips_through_the_native_format() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut line = Shape::new(
            id,
            ShapeKind::Path {
                path: {
                    let mut p = tracedraw_core::BezPath::new();
                    p.move_to((10.0, 50.0));
                    p.line_to((60.0, 50.0));
                    p
                },
                closed: false,
            },
        );
        line.stroke = Some(Stroke {
            start_arrow: tracedraw_core::Arrowhead::Arrow,
            ..Stroke::new(Color::BLACK, 1.0)
        });
        doc.layer_mut(layer).unwrap().shapes.push(line);
        let plain = doc.to_json().unwrap();
        assert!(!plain.contains("start_head") && !plain.contains("share_arrowheads"));
        let shape = &mut doc.layer_mut(layer).unwrap().shapes[0];
        let st = shape.stroke.as_mut().unwrap();
        st.start_head = tracedraw_core::ArrowheadPlacement {
            size: 250.0,
            offset: -1.25,
            mirror_h: false,
            mirror_v: true,
            rotation: 12.5,
        };
        st.end_head.offset = 3.0;
        st.share_arrowheads = true;
        let json = doc.to_json().unwrap();
        assert!(json.contains("\"start_head\""));
        assert!(json.contains("\"share_arrowheads\""));
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, doc);
        let s = back.pages[0].layers[0].shapes[0].stroke.as_ref().unwrap();
        assert_eq!(s.start_head.size, 250.0);
        assert_eq!(s.start_head.rotation, 12.5);
        assert!(s.start_head.mirror_v);
        assert_eq!(s.end_head.offset, 3.0);
        assert!(s.share_arrowheads);
    }
}

#[cfg(test)]
mod shadow_picture_tests {
    use super::*;
    use tracedraw_core::{
        document::{Shape, ShapeKind},
        geometry::{Affine, Rect, Size, Vec2},
        style::Fill,
        Color, Shadow,
    };

    /// Every object with a shadow gets a picture over the shadow's area,
    /// also inside a moved group (its place on the page), and objects
    /// without one get none (#1031).
    #[test]
    fn shadow_pictures_cover_each_shadow_on_the_page() {
        let mut doc = Document::new("s", Size::new(200.0, 100.0));
        let mut ids = doc.ids().clone();
        let shadow = Shadow {
            offset: Vec2::new(4.0, -4.0),
            opacity: 0.7,
            blur: 0.0,
            color: Color::BLACK,
            ..Default::default()
        };
        let mut a = Shape::new(
            ids.shape(),
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 30.0, 20.0),
                radius: 0.0,
                corners: None,
            },
        );
        a.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        a.shadow = Some(shadow);
        let mut b = Shape::new(
            ids.shape(),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            },
        );
        b.fill = Fill::Solid(Color::rgb8(0, 0, 255));
        b.shadow = Some(shadow);
        let plain = Shape::new(
            ids.shape(),
            ShapeKind::Ellipse {
                rect: Rect::new(50.0, 50.0, 60.0, 60.0),
                arc: None,
            },
        );
        let (a_id, b_id) = (a.id, b.id);
        let mut group = Shape::new(
            ids.shape(),
            ShapeKind::Group {
                children: vec![b, plain],
            },
        );
        group.transform = Affine::translate((100.0, 50.0));
        doc.pages[0].layers[0].shapes = vec![a, group];
        doc.set_ids(ids);
        let pictures = shadow_pictures(&doc, 150.0);
        assert_eq!(pictures.len(), 2, "{pictures:?}");
        let of = |id| pictures.iter().find(|p| p.0 == id).expect("picture");
        let (_, rect, png, w, h) = of(a_id);
        // The shadow of `a` lies 4 mm right of and below it.
        assert!(
            rect.x0 >= 10.0 && rect.x1 >= 34.0 - 0.01 && rect.y0 <= 6.0 + 0.01,
            "{rect:?}"
        );
        assert!(*w > 0 && *h > 0 && png.starts_with(&[0x89, b'P', b'N', b'G']));
        let (_, rect, ..) = of(b_id);
        // `b` sits at (100, 50) on the page.
        assert!(
            rect.x0 >= 100.0 && rect.x1 >= 114.0 - 0.01 && rect.y0 <= 46.0 + 0.01,
            "{rect:?}"
        );
        assert!(shadow_pictures(&doc, 0.0).is_empty());
    }
}
