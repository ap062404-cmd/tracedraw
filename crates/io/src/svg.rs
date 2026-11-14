//! SVG export. One page per SVG, in millimetres, with the Y axis flipped
//! from our bottom-up page space to SVG's top-down space.

use std::fmt::Write;
use tracedraw_core::{
    document::{Shape, ShapeKind},
    geometry::{Affine, PathEl, Shape as _},
    Color, Document, Fill, LineCap, LineJoin,
};

thread_local! {
    /// The spot inks of the drawing being written (id, name), so a
    /// spot colour keeps its ink's name; set by [`page_to_svg`].
    static INKS: std::cell::RefCell<Vec<(u32, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The name of the spot ink `c` is a tint of, in the drawing being
/// written.
fn spot_name(c: tracedraw_core::Color) -> Option<String> {
    let tracedraw_core::Color::Spot { ink, .. } = c else {
        return None;
    };
    INKS.with(|i| {
        i.borrow()
            .iter()
            .find(|(id, _)| *id == ink)
            .map(|(_, n)| n.clone())
    })
}

/// SVG has no spot colours: an object painted with a spot ink is written
/// in the ink's look and carries the ink's name in `data-spot-fill` and
/// `data-spot-stroke`, so a cutter or RIP reading the file finds, say,
/// its CutContour lines.
fn spot_attrs(fill: &Fill, stroke: Option<&tracedraw_core::Stroke>) -> String {
    let mut out = String::new();
    if let Fill::Solid(c) = fill {
        if let Some(n) = spot_name(*c) {
            let _ = write!(out, " data-spot-fill=\"{}\"", escape(&n));
        }
    }
    if let Some(n) = stroke.and_then(|s| spot_name(s.color)) {
        let _ = write!(out, " data-spot-stroke=\"{}\"", escape(&n));
    }
    out
}

pub fn page_to_svg(doc: &Document, page_index: usize) -> String {
    let doc = &*crate::for_export(doc, Some(page_index));
    INKS.with(|i| {
        *i.borrow_mut() = doc.inks.iter().map(|k| (k.id, k.name.clone())).collect();
    });
    let Some(page) = doc.pages.get(page_index) else {
        return String::from("<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
    };
    let w = page.size.width;
    let h = page.size.height;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">"
    );
    let _ = writeln!(out, "  <title>{}</title>", escape(&doc.title));
    // Flip Y: page space has origin bottom-left.
    let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, h]);
    let mut gradient_defs = String::new();
    let mut body = String::new();
    let mut ctx = SvgCtx { next_grad: 0, flip };
    let layers = doc.layers_drawn_for_page(page.id).unwrap_or_default();
    for layer in layers {
        if !layer.visible {
            continue;
        }
        let _ = writeln!(
            body,
            "  <g id=\"{}\" data-name=\"{}\">",
            layer.id.raw(),
            escape(&layer.name)
        );
        for shape in &layer.shapes {
            write_shape(
                shape,
                &doc.symbols,
                flip,
                &mut body,
                &mut gradient_defs,
                &mut ctx,
                2,
            );
        }
        let _ = writeln!(body, "  </g>");
    }
    if !gradient_defs.is_empty() {
        let _ = writeln!(out, "  <defs>\n{gradient_defs}  </defs>");
    }
    out.push_str(&body);
    out.push_str("</svg>\n");
    out
}

/// What one page's writer carries along: the numbering of gradients and
/// patterns in `defs`, and the page flip (page space, Y up, to the
/// output's Y down), which anchors patterns to the page.
struct SvgCtx {
    next_grad: usize,
    flip: Affine,
}

impl SvgCtx {
    /// The next id with `prefix`.
    fn next(&mut self, prefix: &str) -> String {
        self.next_grad += 1;
        format!("{prefix}{}", self.next_grad)
    }
}

/// A linked object inside a link (`<a>`) to its address; links of the
/// objects inside it are dropped, since links cannot nest. Page links
/// (`#page=n`) have no target in a one-page SVG and are left out.
fn write_shape(
    shape: &Shape,
    symbols: &[tracedraw_core::Symbol],
    parent: Affine,
    out: &mut String,
    defs: &mut String,
    ctx: &mut SvgCtx,
    indent: usize,
) {
    // Bitmap effects on a vector object: its picture in its place.
    if let Some(pic) = shape.drawn_as() {
        return write_shape(&pic, symbols, parent, out, defs, ctx, indent);
    }
    let link = shape
        .link
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("#page="));
    let Some(link) = link.filter(|_| shape.visible) else {
        write_unlinked(shape, symbols, parent, out, defs, ctx, indent);
        return;
    };
    let pad = "  ".repeat(indent);
    let _ = writeln!(out, "{pad}<a href=\"{}\">", escape(link));
    let mut inner = shape.clone();
    drop_links(&mut inner);
    write_unlinked(&inner, symbols, parent, out, defs, ctx, indent + 1);
    let _ = writeln!(out, "{pad}</a>");
}

/// `shape` and everything inside it without links.
fn drop_links(shape: &mut Shape) {
    shape.link = None;
    match &mut shape.kind {
        ShapeKind::Group { children } => children.iter_mut().for_each(drop_links),
        ShapeKind::ClipFrame { frame, contents } => {
            drop_links(frame);
            contents.iter_mut().for_each(drop_links);
        }
        _ => {}
    }
}

fn write_unlinked(
    shape: &Shape,
    symbols: &[tracedraw_core::Symbol],
    parent: Affine,
    out: &mut String,
    defs: &mut String,
    ctx: &mut SvgCtx,
    indent: usize,
) {
    if !shape.visible {
        return;
    }
    let pad = "  ".repeat(indent);
    if !shape.effects.is_empty() {
        let ev = tracedraw_core::live::evaluate(shape);
        let _ = writeln!(
            out,
            "{pad}<g id=\"{}\" data-effects=\"{}\">",
            shape.id.raw(),
            shape.effects.len()
        );
        for b in &ev.below {
            let mut b = b.clone();
            b.effects.clear();
            write_shape(&b, symbols, parent, out, defs, ctx, indent + 1);
        }
        let mut main = ev.main.clone();
        main.effects.clear();
        // Lens and non-uniform transparency have no SVG equivalent here;
        // approximate with the average opacity of the mask. The merge
        // mode goes out as the CSS blend mode when one matches it.
        let mut blend = None;
        // The transparency's opacity and its target (the fill, the
        // outline or the whole object, live test of 0.41).
        let mut fade: Option<(f64, u8)> = None;
        for e in &shape.effects {
            match e {
                tracedraw_core::live::Effect::Transparency {
                    mask,
                    merge,
                    target,
                } => {
                    fade = Some((average_luminance(mask) as f64, *target));
                    blend = svg_blend_mode(*merge);
                }
                tracedraw_core::live::Effect::Lens(tracedraw_core::live::Lens::Transparency {
                    rate,
                    color,
                }) => {
                    main.fill = Fill::Solid(*color);
                    main.opacity *= rate / 100.0;
                }
                tracedraw_core::live::Effect::Lens(_) => {
                    main.opacity *= 0.5;
                }
                _ => {}
            }
        }
        let parts = match fade {
            Some((_, target)) => tracedraw_core::live::transparency_parts(&main, target),
            None => vec![(main, false)],
        };
        for (mut part, faded) in parts {
            if !faded {
                write_shape(&part, symbols, parent, out, defs, ctx, indent + 1);
                continue;
            }
            if let Some((opacity, _)) = fade {
                part.opacity *= opacity;
            }
            match blend {
                Some(b) => {
                    let _ = writeln!(out, "{pad}  <g style=\"mix-blend-mode:{b}\">");
                    write_shape(&part, symbols, parent, out, defs, ctx, indent + 2);
                    let _ = writeln!(out, "{pad}  </g>");
                }
                None => write_shape(&part, symbols, parent, out, defs, ctx, indent + 1),
            }
        }
        for a in &ev.above {
            let mut a = a.clone();
            a.effects.clear();
            write_shape(&a, symbols, parent, out, defs, ctx, indent + 1);
        }
        let _ = writeln!(out, "{pad}</g>");
        return;
    }
    // Characters with their own fill or outline: one text a look.
    if let Some(parts) = shape.text_parts() {
        let _ = writeln!(out, "{pad}<g id=\"{}\">", shape.id.raw());
        for p in &parts {
            write_unlinked(p, symbols, parent, out, defs, ctx, indent + 1);
        }
        let _ = writeln!(out, "{pad}</g>");
        return;
    }
    let transform = parent * shape.transform;
    if let ShapeKind::Group { children } = &shape.kind {
        let _ = writeln!(out, "{pad}<g id=\"{}\">", shape.id.raw());
        for c in children {
            // A group's uniform transparency goes on each member, as the
            // canvas draws it: members that overlap show through each
            // other (#1386).
            if shape.opacity < 1.0 {
                let mut c = c.clone();
                c.absorb_opacity(shape.opacity);
                write_shape(&c, symbols, transform, out, defs, ctx, indent + 1);
            } else {
                write_shape(c, symbols, transform, out, defs, ctx, indent + 1);
            }
        }
        let _ = writeln!(out, "{pad}</g>");
        return;
    }
    if matches!(
        shape.kind,
        ShapeKind::Table(_) | ShapeKind::SymbolInstance { .. }
    ) {
        let _ = writeln!(out, "{pad}<g id=\"{}\">", shape.id.raw());
        for c in shape.expand(symbols) {
            write_shape(&c, symbols, transform, out, defs, ctx, indent + 1);
        }
        let _ = writeln!(out, "{pad}</g>");
        return;
    }
    if let (ShapeKind::ClipFrame { frame, contents }, Some(om)) = (&shape.kind, shape.opacity_mask)
    {
        // Opacity mask: the frame's fill is a luminance mask over the
        // contents (inverted through a colour matrix); the frame is not
        // drawn.
        let id = shape.id.raw();
        let mut fill_only = (**frame).clone();
        fill_only.stroke = None;
        let mut m = String::new();
        write_shape(&fill_only, symbols, transform, &mut m, defs, ctx, 3);
        let mut mask = format!(
            "    <mask id=\"mask{id}\" maskUnits=\"userSpaceOnUse\" x=\"-100000\" y=\"-100000\" width=\"200000\" height=\"200000\">\n"
        );
        if om.invert {
            let _ = writeln!(
                defs,
                "    <filter id=\"maskinv{id}\"><feColorMatrix type=\"matrix\" values=\"-1 0 0 0 1 0 -1 0 0 1 0 0 -1 0 1 0 0 0 1 0\"/></filter>"
            );
            let _ = writeln!(mask, "      <g filter=\"url(#maskinv{id})\">");
            mask.push_str(&m);
            mask.push_str("      </g>\n");
        } else {
            mask.push_str(&m);
        }
        mask.push_str("    </mask>\n");
        defs.push_str(&mask);
        let opacity = if shape.opacity < 1.0 {
            format!(" opacity=\"{}\"", fmt(shape.opacity))
        } else {
            String::new()
        };
        let _ = writeln!(out, "{pad}<g id=\"{id}\" mask=\"url(#mask{id})\"{opacity}>");
        for c in contents {
            write_shape(c, symbols, transform, out, defs, ctx, indent + 1);
        }
        let _ = writeln!(out, "{pad}</g>");
        return;
    }
    if let ShapeKind::ClipFrame { frame, contents } = &shape.kind {
        // Frame fill, then the contents clipped to the frame, then its outline.
        let mut fill_only = (**frame).clone();
        fill_only.stroke = None;
        write_shape(&fill_only, symbols, transform, out, defs, ctx, indent);
        let clip_id = format!("clip{}", shape.id.raw());
        let _ = writeln!(
            defs,
            "    <clipPath id=\"{clip_id}\"><path d=\"{}\"/></clipPath>",
            path_data(&(transform * frame.page_path()))
        );
        let _ = writeln!(
            out,
            "{pad}<g id=\"{}\" clip-path=\"url(#{clip_id})\">",
            shape.id.raw()
        );
        for c in contents {
            write_shape(c, symbols, transform, out, defs, ctx, indent + 1);
        }
        let _ = writeln!(out, "{pad}</g>");
        let mut outline = (**frame).clone();
        outline.fill = Fill::None;
        if outline.stroke.is_some() {
            write_shape(&outline, symbols, transform, out, defs, ctx, indent);
        }
        return;
    }
    // Further fills and outlines: the parts in a group, bottom first.
    if let Some(parts) = shape.layers() {
        let _ = writeln!(out, "{pad}<g id=\"{}\">", shape.id.raw());
        for p in &parts {
            write_shape(p, symbols, parent, out, defs, ctx, indent + 1);
        }
        let _ = writeln!(out, "{pad}</g>");
        return;
    }
    if let ShapeKind::Bitmap {
        rect,
        width_px,
        height_px,
        png,
        fx: _,
    } = &shape.kind
    {
        // Image space (pixels, y down) to the local rect to the page.
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let m = transform
            * Affine::new([
                rect.width() / (*width_px).max(1) as f64,
                0.0,
                0.0,
                -(rect.height() / (*height_px).max(1) as f64),
                rect.x0,
                rect.y1,
            ]);
        let c = m.as_coeffs();
        let opacity = if shape.opacity < 1.0 {
            format!(" opacity=\"{}\"", fmt(shape.opacity))
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "{pad}<image id=\"{}\" width=\"{width_px}\" height=\"{height_px}\" transform=\"matrix({} {} {} {} {} {})\" preserveAspectRatio=\"none\"{opacity} href=\"data:image/png;base64,{b64}\"/>",
            shape.id.raw(),
            fmt(c[0]),
            fmt(c[1]),
            fmt(c[2]),
            fmt(c[3]),
            fmt(c[4]),
            fmt(c[5])
        );
        return;
    }
    let path = transform * shape.local_path();
    let d = path_data(&path);
    let bounds = path.bounding_box();

    let fill_attr = format!(
        "fill=\"{}\"",
        paint_value(&shape.fill, bounds, transform, symbols, defs, ctx)
    );

    let drawn = shape.stroke.as_ref().map(|s| drawn_stroke(s, transform));
    // A calligraphic, variable, inside or outside outline, or one with
    // dashes fitted to the corners, has no SVG equivalent: its area is
    // written as a filled path after the object.
    let calligraphic = shape
        .stroke
        .as_ref()
        .zip(drawn.as_ref())
        .and_then(|(s, (_, scale))| {
            tracedraw_core::shaping::outline_band(&shape.page_path(), s, *scale).map(|b| {
                let band = parent * b;
                let paint =
                    outline_paint_value(s, band.bounding_box(), transform, symbols, defs, ctx);
                (band, paint)
            })
        });
    let stroke_attr = match drawn.as_ref().map(|(s, _)| s) {
        None => "stroke=\"none\"".to_string(),
        Some(s) if calligraphic.is_some() || s.painterly.is_some() => "stroke=\"none\"".to_string(),
        Some(s) => {
            let cap = match s.cap {
                LineCap::Butt => "butt",
                LineCap::Round => "round",
                LineCap::Square => "square",
            };
            let join = match s.join {
                LineJoin::Miter => "miter",
                LineJoin::Round => "round",
                LineJoin::Bevel => "bevel",
            };
            // The outline's own paint spreads over the outline's bounds,
            // as on screen.
            let half = s.width / 2.0;
            let paint =
                outline_paint_value(s, bounds.inflate(half, half), transform, symbols, defs, ctx);
            let mut a = format!(
                "stroke=\"{paint}\" stroke-width=\"{}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\"",
                s.width
            );
            if s.join == LineJoin::Miter {
                let _ = write!(a, " stroke-miterlimit=\"{}\"", fmt(s.miter_ratio()));
            }
            if !s.dash.is_empty() {
                let dashes: Vec<String> = s.dash.iter().map(|d| fmt(d * s.width)).collect();
                let _ = write!(a, " stroke-dasharray=\"{}\"", dashes.join(" "));
            }
            // An outline behind the fill: painted first, the fill over it.
            if s.behind_fill {
                a.push_str(" paint-order=\"stroke\"");
            }
            a
        }
    };

    let name = shape
        .name
        .as_deref()
        .map(|n| format!(" data-name=\"{}\"", escape(n)))
        .unwrap_or_default();
    // Shapes fill even-odd, as the renderer draws them (holes in combined
    // curves, the middle of a complex star); text, and a curve whose fill
    // rule is nonzero (#1919), keep the default (nonzero).
    let rule = if matches!(shape.kind, ShapeKind::Text { .. })
        || shape.fill_rule == tracedraw_core::shaping::FillRule::NonZero
    {
        ""
    } else {
        " fill-rule=\"evenodd\""
    };
    // Uniform transparency: on the path, or on a group around it when
    // the outline, its marks or its arrowheads are paths of their own.
    let heads: Vec<_> = shape
        .stroke
        .as_ref()
        .map(|s| tracedraw_core::arrowhead_paths(&shape.page_path(), s))
        .unwrap_or_default();
    let painterly = drawn.as_ref().is_some_and(|(s, _)| s.painterly.is_some());
    let see_through = shape.opacity < 1.0;
    let wrapped = see_through && (calligraphic.is_some() || painterly || !heads.is_empty());
    let opacity = if see_through && !wrapped {
        format!(" opacity=\"{}\"", fmt(shape.opacity.max(0.0)))
    } else {
        String::new()
    };
    let spot = spot_attrs(&shape.fill, shape.stroke.as_ref());
    if wrapped {
        let _ = writeln!(out, "{pad}<g opacity=\"{}\">", fmt(shape.opacity.max(0.0)));
    }
    let _ = writeln!(
        out,
        "{pad}<path id=\"{}\"{name} d=\"{d}\" {fill_attr}{rule} {stroke_attr}{spot}{opacity}/>",
        shape.id.raw()
    );
    if let Some((band, paint)) = calligraphic {
        let _ = writeln!(
            out,
            "{pad}<path d=\"{}\" fill=\"{paint}\" stroke=\"none\"/>",
            path_data(&band)
        );
    }
    // A painterly outline: its marks, one path per opacity step.
    if let Some((s, brush)) = drawn
        .as_ref()
        .and_then(|(s, _)| s.painterly.as_ref().map(|b| (s, b)))
    {
        for (alpha, marks) in
            tracedraw_core::painterly::marks_by_opacity(&shape.page_path(), s.width, brush, 12)
        {
            let _ = writeln!(
                out,
                "{pad}<path d=\"{}\" fill=\"{}\" fill-opacity=\"{}\" stroke=\"none\"/>",
                path_data(&(parent * marks)),
                s.color.to_hex(),
                fmt(alpha)
            );
        }
    }
    // Arrowheads (presets and custom) are filled outlines in the stroke
    // colour. They are built in page space, where the curve direction is
    // unmirrored, and then taken through the parent transform and the flip.
    if let Some(s) = &shape.stroke {
        for head in heads {
            let _ = writeln!(
                out,
                "{pad}<path d=\"{}\" fill=\"{}\" stroke=\"none\"/>",
                path_data(&(parent * head)),
                s.color.to_hex()
            );
        }
    }
    if wrapped {
        let _ = writeln!(out, "{pad}</g>");
    }
}

/// The outline as the drawing window draws it under `transform`, and the
/// factor its width took: one that scales with its object ("Scale with
/// object") takes the object's scale factor, so a 10 mm outline on an
/// object scaled to a quarter is 2.5 mm wide. Others keep their width
/// (factor 1), and a hairline stays a hairline.
pub(crate) fn drawn_stroke(
    s: &tracedraw_core::Stroke,
    transform: Affine,
) -> (tracedraw_core::Stroke, f64) {
    let scale = if s.scale_with_object {
        let c = transform.as_coeffs();
        (c[0] * c[3] - c[1] * c[2]).abs().sqrt().max(1e-6)
    } else {
        1.0
    };
    let mut drawn = s.clone();
    if drawn.width > tracedraw_core::Stroke::HAIRLINE + 1e-9 {
        drawn.width *= scale;
    }
    (drawn, scale)
}

/// The SVG paint of an outline: its own paint (a gradient, pattern,
/// texture or mesh) over `bounds`, or its colour.
fn outline_paint_value(
    s: &tracedraw_core::Stroke,
    bounds: tracedraw_core::Rect,
    object: Affine,
    symbols: &[tracedraw_core::Symbol],
    defs: &mut String,
    ctx: &mut SvgCtx,
) -> String {
    match s.fill.as_deref() {
        Some(fill) if !matches!(fill, Fill::None | Fill::Solid(_)) => {
            paint_value(fill, bounds, object, symbols, defs, ctx)
        }
        _ => s.color.to_hex(),
    }
}

/// Most stops a gradient sampled along a hue path or a smooth transition
/// is written with (a stepped gradient keeps a pair per band).
pub const MAX_SAMPLED_STOPS: usize = 64;

/// `stops` (sorted, over 0..1) as `n` stops spaced evenly, each the
/// colour and opacity the run has there.
pub(crate) fn resample_stops(
    stops: &[tracedraw_core::Stop],
    n: usize,
) -> Vec<tracedraw_core::Stop> {
    let n = n.max(2);
    let k = (n - 1) as f64;
    (0..n)
        .map(|i| {
            let pos = i as f64 / k;
            tracedraw_core::Stop {
                pos,
                ..tracedraw_core::style::sample_stop(stops, pos)
            }
        })
        .collect()
}

/// `matrix(a b c d e f)` for an SVG transform attribute.
fn matrix(m: Affine) -> String {
    let c = m.as_coeffs();
    format!(
        "matrix({} {} {} {} {} {})",
        fmt(c[0]),
        fmt(c[1]),
        fmt(c[2]),
        fmt(c[3]),
        fmt(c[4]),
        fmt(c[5])
    )
}

/// The `<pattern>` of a pattern fill, written into `defs`: its tile is
/// the block of tiles that repeats (one tile, or the mirrored and
/// shifted tiles of the placement) in millimetres, Y down, and
/// `patternTransform` carries the placement (tile size, offset,
/// rotation, skew, the row or column lean) into the output space, from
/// the page origin, or from the object (`object`, its transform into the
/// output space) when the pattern transforms with its object. Returns
/// the paint reference.
fn pattern_paint(
    p: &tracedraw_core::Pattern,
    object: Affine,
    symbols: &[tracedraw_core::Symbol],
    defs: &mut String,
    ctx: &mut SvgCtx,
) -> String {
    let id = ctx.next("pat");
    let placement = p.placement().sanitized();
    let tile = p.tile_size();
    let (cols, rows) = placement.cell_tiles();
    // The block in pattern space: `cw` by `ch` mm, Y down.
    let (cw, ch) = (tile.width * f64::from(cols), tile.height * f64::from(rows));
    let cell = placement.cell(tile);
    // Pattern space to the block's unit square (Y up), and back.
    let q = Affine::new([1.0 / cw, 0.0, 0.0, -1.0 / ch, 0.0, 1.0]);
    let q_inv = Affine::new([cw, 0.0, 0.0, -ch, 0.0, ch]);
    let to_output = if placement.transform_with_object {
        object * placement.place()
    } else {
        ctx.flip * placement.place()
    };
    let m = to_output * cell * q;
    let identity = m
        .as_coeffs()
        .iter()
        .zip(Affine::IDENTITY.as_coeffs())
        .all(|(a, b)| (a - b).abs() < 1e-9);
    let pattern_transform = if identity {
        String::new()
    } else {
        format!(" patternTransform=\"{}\"", matrix(m))
    };
    let mut inner = String::new();
    match p {
        tracedraw_core::Pattern::TwoColor { .. } => {
            // Rasterised at 32 px per tile (the block holds two where
            // mirrored) into a data URI.
            let dpi = (32.0 / tile.width.max(1e-6) * 25.4).clamp(1.0, 2400.0);
            if let Some((pm, _)) = tracedraw_render::render_pattern_cell(p, dpi) {
                let png = pm.encode_png().unwrap_or_default();
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(png);
                let _ = writeln!(
                    inner,
                    "      <image width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{b64}\"/>",
                    fmt(cw),
                    fmt(ch)
                );
            }
        }
        tracedraw_core::Pattern::Bitmap {
            png,
            width_px,
            height_px,
            ..
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(png);
            // The picture's pixels (Y down) to one tile's millimetres (Y up).
            let content = Affine::new([
                tile.width / f64::from((*width_px).max(1)),
                0.0,
                0.0,
                -tile.height / f64::from((*height_px).max(1)),
                0.0,
                tile.height,
            ]);
            for copy in placement.tile_copies(tile) {
                let t = q_inv * cell.inverse() * copy * content;
                let _ = writeln!(
                    inner,
                    "      <image width=\"{width_px}\" height=\"{height_px}\" transform=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{b64}\"/>",
                    matrix(t)
                );
            }
        }
        tracedraw_core::Pattern::Vector {
            shapes: tile_shapes,
            tile: own,
            ..
        } => {
            // The tile's content stays vector: its shapes are written into
            // the pattern once per copy with the tile's own Y flip (tile
            // space is Y up, origin bottom-left). Gradients they use land
            // in `defs` too.
            let content = Affine::scale_non_uniform(
                tile.width / own.width.max(1e-9),
                tile.height / own.height.max(1e-9),
            );
            for copy in placement.tile_copies(tile) {
                let t = q_inv * cell.inverse() * copy * content;
                for s in tile_shapes {
                    write_shape(s, symbols, t, &mut inner, defs, ctx, 3);
                }
            }
        }
    }
    let _ = writeln!(
        defs,
        "    <pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{}\" height=\"{}\"{pattern_transform}>\n{inner}    </pattern>",
        fmt(cw),
        fmt(ch)
    );
    format!("url(#{id})")
}

/// The SVG paint of `fill` over `bounds` (output space): `none`, a
/// colour, or a reference to a gradient or pattern written into `defs`.
/// `object` is the object's transform into the output space (a pattern
/// set to transform with its object follows it). Fills SVG cannot draw
/// (textures, meshes) give a representative colour.
fn paint_value(
    fill: &Fill,
    bounds: tracedraw_core::Rect,
    object: Affine,
    symbols: &[tracedraw_core::Symbol],
    defs: &mut String,
    ctx: &mut SvgCtx,
) -> String {
    match fill {
        Fill::None => "none".to_string(),
        Fill::Solid(c) => c.to_hex(),
        Fill::Fountain(f) => {
            let id = ctx.next("grad");
            // The edge pad, repeats, mirror, inversion and blend settings
            // as one run of stops: a stepped fountain gets a pair of stops
            // per band (hard edges), a hue path or a smooth transition is
            // sampled, at most MAX_SAMPLED_STOPS of them; a shaped
            // fountain (0 on the edge) runs from the centre out as a
            // radial one.
            let mut stops = f.resolved_stops();
            if !f.stepped() && stops.len() > MAX_SAMPLED_STOPS {
                stops = resample_stops(&stops, MAX_SAMPLED_STOPS);
            }
            if f.kind == tracedraw_core::FountainKind::Shaped {
                stops.reverse();
                for s in &mut stops {
                    s.pos = 1.0 - s.pos;
                }
            }
            let stop_xml: String = stops
                .iter()
                .map(|st| {
                    // A node's transparency as the stop's opacity.
                    let opacity = if st.opacity < 1.0 {
                        format!(" stop-opacity=\"{}\"", fmt(f64::from(st.opacity.max(0.0))))
                    } else {
                        String::new()
                    };
                    format!(
                        "      <stop offset=\"{}\" stop-color=\"{}\"{opacity}/>\n",
                        fmt(st.pos),
                        st.color.to_hex()
                    )
                })
                .collect();
            // Nodes placed freely (Interactive Fill): the fill's own
            // frame, start + u (end - start) + v (skew - start), as the
            // gradient's transform; the output's y runs down.
            if let (Some(h), true) = (f.handles, f.kind != tracedraw_core::FountainKind::Shaped) {
                let c = bounds.center();
                let at = |r: tracedraw_core::geometry::Point| {
                    (
                        c.x + r.x * bounds.width() / 2.0,
                        c.y - r.y * bounds.height() / 2.0,
                    )
                };
                let (sx, sy) = at(h.start);
                let (ex, ey) = at(h.end);
                let (kx, ky) = at(h.skew);
                let m = [ex - sx, ey - sy, kx - sx, ky - sy, sx, sy];
                let det = m[0] * m[3] - m[1] * m[2];
                if det.is_finite() && det.abs() > 1e-12 {
                    let matrix = m.iter().map(|v| fmt(*v)).collect::<Vec<_>>().join(" ");
                    if f.kind == tracedraw_core::FountainKind::Radial {
                        let _ = writeln!(defs, "    <radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"0\" cy=\"0\" r=\"1\" gradientTransform=\"matrix({matrix})\">\n{stop_xml}    </radialGradient>");
                    } else {
                        // Conical and square: SVG has neither; linear along
                        // the same axis.
                        let _ = writeln!(defs, "    <linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\" gradientTransform=\"matrix({matrix})\">\n{stop_xml}    </linearGradient>");
                    }
                    return format!("url(#{id})");
                }
            }
            match f.kind {
                tracedraw_core::FountainKind::Linear
                | tracedraw_core::FountainKind::Conical
                | tracedraw_core::FountainKind::Square => {
                    // SVG has no conical/square gradients; they become linear.
                    let a = f.angle.to_radians();
                    let (x1, y1, x2, y2) = (
                        bounds.x0 + bounds.width() * (0.5 - a.cos() / 2.0),
                        bounds.y0 + bounds.height() * (0.5 + a.sin() / 2.0),
                        bounds.x0 + bounds.width() * (0.5 + a.cos() / 2.0),
                        bounds.y0 + bounds.height() * (0.5 - a.sin() / 2.0),
                    );
                    let _ = writeln!(defs, "    <linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\">\n{stop_xml}    </linearGradient>");
                }
                tracedraw_core::FountainKind::Radial => {
                    let cx = bounds.center().x + f.offset.x * bounds.width() / 2.0;
                    let cy = bounds.center().y - f.offset.y * bounds.height() / 2.0;
                    let r = bounds.width().max(bounds.height()) / 2.0 * std::f64::consts::SQRT_2;
                    let _ = writeln!(defs, "    <radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\">\n{stop_xml}    </radialGradient>");
                }
                tracedraw_core::FountainKind::Shaped => {
                    // SVG has no gradient that follows an outline: a radial
                    // one reaching the nearer sides stands in for it.
                    let c = bounds.center();
                    let r = bounds.width().min(bounds.height()).max(1e-6) / 2.0;
                    let _ = writeln!(defs, "    <radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\">\n{stop_xml}    </radialGradient>", c.x, c.y, r);
                }
            }
            format!("url(#{id})")
        }
        Fill::Pattern(p) => pattern_paint(p, object, symbols, defs, ctx),
        Fill::Texture(t) => {
            // Approximate with the average of the two colours.
            tracedraw_core::style::lerp_color(t.color_a, t.color_b, 0.5).to_hex()
        }
        Fill::Procedural(p) => {
            let id = ctx.next("pat");
            // The tile's paths as they are (vector), with the tile's Y
            // flip (tile space is Y up, origin bottom-left), over a
            // rectangle of the background colour when there is one.
            let tile = tracedraw_core::procedural::procedural_tile(p);
            let (w, h) = (tile.size.width, tile.size.height);
            let tile_flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, h]);
            let mut inner = String::new();
            if let Some(bg) = p.background {
                let _ = writeln!(
                    inner,
                    "      <rect width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                    fmt(w),
                    fmt(h),
                    bg.to_hex()
                );
            }
            let fg = p.foreground.to_hex();
            for path in &tile.paths {
                let _ = writeln!(
                    inner,
                    "      <path d=\"{}\" fill=\"{fg}\"/>",
                    path_data(&(tile_flip * path))
                );
            }
            // The tiles start at the page origin, as on screen.
            let origin = ctx.flip * tracedraw_core::geometry::Point::ZERO;
            let anchor = if origin.x == 0.0 && origin.y == 0.0 {
                String::new()
            } else {
                format!(
                    " patternTransform=\"{}\"",
                    matrix(Affine::translate(origin.to_vec2()))
                )
            };
            let _ = writeln!(
                defs,
                "    <pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"{}\" height=\"{}\"{anchor}>\n{inner}    </pattern>",
                fmt(w),
                fmt(h)
            );
            format!("url(#{id})")
        }
        Fill::Mesh(m) => {
            // Average of the node colours; the renderer image path is exact.
            let n = m.nodes.len().max(1) as f32;
            let mut acc = [0.0f32; 3];
            for node in &m.nodes {
                let [r, g, b] = node.color.to_rgb_f32();
                acc[0] += r;
                acc[1] += g;
                acc[2] += b;
            }
            Color::Rgb {
                r: acc[0] / n,
                g: acc[1] / n,
                b: acc[2] / n,
            }
            .to_hex()
        }
    }
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// The CSS `mix-blend-mode` that draws a merge mode as the drawing
/// window does (see `tracedraw_render::merge_blend`); None for the modes
/// CSS has no blend for, which draw normally.
pub fn svg_blend_mode(merge: tracedraw_core::live::MergeMode) -> Option<&'static str> {
    use tracedraw_core::live::MergeMode as M;
    Some(match merge {
        M::Multiply | M::Texturize | M::Subtract => "multiply",
        M::Difference => "difference",
        M::Divide | M::Screen => "screen",
        M::IfLighter => "lighten",
        M::IfDarker => "darken",
        M::Hue => "hue",
        M::Saturation => "saturation",
        M::Color => "color",
        M::Lightness => "luminosity",
        M::Invert | M::Exclusion => "exclusion",
        M::Overlay => "overlay",
        M::SoftLight => "soft-light",
        M::HardLight => "hard-light",
        M::ColorDodge => "color-dodge",
        M::ColorBurn => "color-burn",
        M::Normal | M::Add | M::And | M::Or | M::Xor | M::Red | M::Green | M::Blue | M::Behind => {
            return None
        }
    })
}

/// Mean luminance (0..1) of a mask fill, for exporters without masks.
pub fn average_luminance(fill: &Fill) -> f32 {
    match fill {
        Fill::None => 1.0,
        Fill::Solid(c) => c.luminance(),
        Fill::Fountain(f) => {
            let n = f.stops.len().max(1) as f32;
            f.stops.iter().map(|s| s.color.luminance()).sum::<f32>() / n
        }
        Fill::Pattern(tracedraw_core::Pattern::TwoColor { front, back, .. }) => {
            (front.luminance() + back.luminance()) / 2.0
        }
        Fill::Pattern(_) => 0.5,
        Fill::Texture(t) => (t.color_a.luminance() + t.color_b.luminance()) / 2.0,
        Fill::Procedural(p) => {
            let back = p.background.map_or(1.0, |b| b.luminance());
            (p.foreground.luminance() + back) / 2.0
        }
        Fill::Mesh(m) => {
            let n = m.nodes.len().max(1) as f32;
            m.nodes.iter().map(|x| x.color.luminance()).sum::<f32>() / n
        }
    }
}

pub fn path_data(path: &tracedraw_core::BezPath) -> String {
    let mut d = String::new();
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                let _ = write!(d, "M{} {}", fmt(p.x), fmt(p.y));
            }
            PathEl::LineTo(p) => {
                let _ = write!(d, "L{} {}", fmt(p.x), fmt(p.y));
            }
            PathEl::QuadTo(c, p) => {
                let _ = write!(d, "Q{} {} {} {}", fmt(c.x), fmt(c.y), fmt(p.x), fmt(p.y));
            }
            PathEl::CurveTo(c1, c2, p) => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {}",
                    fmt(c1.x),
                    fmt(c1.y),
                    fmt(c2.x),
                    fmt(c2.y),
                    fmt(p.x),
                    fmt(p.y)
                );
            }
            PathEl::ClosePath => d.push('Z'),
        }
    }
    d
}

/// Minimal PNG encoder (RGBA8) via tiny-skia.
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut pm = match tiny_skia::Pixmap::new(w, h) {
        Some(p) => p,
        None => return Vec::new(),
    };
    for (i, px) in pm.pixels_mut().iter_mut().enumerate() {
        let o = i * 4;
        if o + 3 < rgba.len() {
            *px = tiny_skia::ColorU8::from_rgba(rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3])
                .premultiply();
        }
    }
    pm.encode_png().unwrap_or_default()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tracedraw_core::{geometry::Rect, Color, Document, Stroke};

    /// A rounded rectangle, red with a blue outline, with a uniform
    /// transparency of half on `target` (0 the fill, 1 the outline, 2 all).
    pub(crate) fn doc_with_targeted_transparency(target: u8) -> Document {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut s = Shape::new(
            doc.ids_mut().shape(),
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 6.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = Some(Stroke::new(Color::rgb8(0, 0, 255), 2.0));
        s.effects.push(tracedraw_core::live::Effect::Transparency {
            mask: Fill::Solid(Color::Gray { v: 0.5 }),
            merge: tracedraw_core::live::MergeMode::Normal,
            target,
        });
        doc.layer_mut(layer).unwrap().shapes.push(s);
        doc
    }

    /// A live test of 0.41: the transparency's target made no difference,
    /// the whole object faded. The fill alone, the outline alone or both
    /// go out see-through as the target says.
    #[test]
    fn a_transparency_target_fades_the_fill_the_outline_or_both() {
        for target in 0..=2u8 {
            let svg = page_to_svg(&doc_with_targeted_transparency(target), 0);
            let line = |paint: &str| {
                svg.lines()
                    .filter(|l| l.contains("<path") && l.contains(paint))
                    .map(|l| l.contains(" opacity=\"0."))
                    .collect::<Vec<_>>()
            };
            let (fill, outline) = (line("fill=\"#ff0000\""), line("stroke=\"#0000ff\""));
            assert!(!fill.is_empty() && !outline.is_empty(), "{svg}");
            assert!(fill.iter().all(|f| *f == (target != 1)), "{target}: {svg}");
            assert!(
                outline.iter().all(|o| *o == (target != 0)),
                "{target}: {svg}"
            );
        }
    }

    #[test]
    fn opacity_masks_are_svg_masks() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let contents = Shape::new(
            doc.ids_mut().shape(),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                radius: 0.0,
                corners: None,
            },
        );
        let mut frame = Shape::new(
            doc.ids_mut().shape(),
            ShapeKind::Rect {
                rect: Rect::new(5.0, 5.0, 35.0, 35.0),
                radius: 0.0,
                corners: None,
            },
        );
        frame.fill = Fill::Solid(Color::WHITE);
        let mut clip = Shape::new(
            doc.ids_mut().shape(),
            ShapeKind::ClipFrame {
                frame: Box::new(frame),
                contents: vec![contents],
            },
        );
        let id = clip.id.raw();
        clip.opacity_mask = Some(tracedraw_core::document::OpacityMask { invert: true });
        doc.layer_mut(layer).unwrap().shapes.push(clip);
        let svg = page_to_svg(&doc, 0);
        assert!(svg.contains(&format!("<mask id=\"mask{id}\"")), "{svg}");
        assert!(svg.contains(&format!("mask=\"url(#mask{id})\"")));
        assert!(svg.contains("feColorMatrix"), "inverted");
        assert!(!svg.contains("clip-path"));
    }

    /// An object's uniform transparency is written: SVG export left it
    /// out, so a see-through object came out opaque (#1386).
    #[test]
    fn an_objects_uniform_transparency_is_written() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut s = Shape::new(
            doc.ids_mut().shape(),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.opacity = 0.25;
        let id = s.id.raw();
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        let path = svg
            .lines()
            .find(|l| l.contains(&format!("<path id=\"{id}\"")))
            .unwrap_or_else(|| panic!("{svg}"));
        assert!(path.contains("opacity=\"0.25\""), "{svg}");
    }

    #[test]
    fn a_groups_uniform_transparency_goes_on_each_member() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let child = Shape::new(
            doc.ids_mut().shape(),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            },
        );
        let mut half = child.clone();
        half.id = doc.ids_mut().shape();
        half.opacity = 0.5;
        let mut group = Shape::new(
            doc.ids_mut().shape(),
            ShapeKind::Group {
                children: vec![child, half],
            },
        );
        let id = group.id.raw();
        group.opacity = 0.4;
        doc.layer_mut(layer).unwrap().shapes.push(group);
        let svg = page_to_svg(&doc, 0);
        // The group's <g> has none: the members carry it, so they show
        // through each other as on the canvas.
        assert!(svg.contains(&format!("<g id=\"{id}\">")), "{svg}");
        assert!(svg.contains("opacity=\"0.4\""), "{svg}");
        assert!(svg.contains("opacity=\"0.2\""), "{svg}");
    }

    #[test]
    fn spot_colours_carry_their_ink_name() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let ink = tracedraw_core::document::Ink {
            id: 3,
            name: "CutContour".into(),
            cmyk: [0.0, 1.0, 0.0, 0.0],
        };
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 30.0, 30.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::None;
        s.stroke = Some(tracedraw_core::Stroke::hairline(ink.color(1.0)));
        doc.inks.push(ink);
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        assert!(svg.contains(" data-spot-stroke=\"CutContour\""), "{svg}");
        assert!(!svg.contains("data-spot-fill"), "{svg}");
        // The next drawing without the ink names nothing.
        let mut plain = doc.clone();
        plain.inks.clear();
        assert!(!page_to_svg(&plain, 0).contains("data-spot"));
    }

    #[test]
    fn linked_objects_sit_inside_links() {
        let mut doc = Document::default();
        let layer = &mut doc.pages[0].layers[0];
        let rect = |id: u64, link: Option<&str>| {
            let mut s = Shape::new(
                tracedraw_core::ShapeId(id),
                ShapeKind::Rect {
                    rect: Rect::new(10.0, 10.0, 20.0, 20.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.link = link.map(String::from);
            s
        };
        let mut group = Shape::new(
            tracedraw_core::ShapeId(10),
            ShapeKind::Group {
                children: vec![rect(11, Some("https://inner.example")), rect(12, None)],
            },
        );
        group.link = Some("https://example.com/?a=1&b=2".into());
        layer.shapes = vec![
            group,
            rect(20, Some("#page=2")),
            rect(21, Some("mailto:a@b.c")),
        ];
        let svg = page_to_svg(&doc, 0);
        assert!(
            svg.contains("<a href=\"https://example.com/?a=1&amp;b=2\">"),
            "{svg}"
        );
        // Links do not nest; page links have no target in one page.
        assert!(!svg.contains("inner.example") && !svg.contains("#page"));
        assert!(svg.contains("<a href=\"mailto:a@b.c\">"));
        assert_eq!(svg.matches("<a ").count(), 2);
        assert_eq!(svg.matches("</a>").count(), 2);
    }

    /// Characters with their own colour: the text goes out as one part
    /// a look, in a group, each part with its colour.
    #[test]
    fn characters_with_their_own_colour_are_written_in_parts() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut red = tracedraw_core::TextSpan::new("b", "Any", 12.0);
        red.fill = Some(Fill::Solid(Color::rgb8(255, 0, 0)));
        let mut s = Shape::new(
            id,
            ShapeKind::Text {
                spans: vec![tracedraw_core::TextSpan::new("a", "Any", 12.0), red],
                origin: tracedraw_core::Point::ZERO,
                frame: None,
                align: Default::default(),
                para: Default::default(),
                on_path: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(0, 0, 255));
        s.stroke = None;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        assert!(svg.contains(&format!("<g id=\"{}\">", id.raw())), "{svg}");
        assert!(svg.contains("fill=\"#0000ff\""), "{svg}");
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    }

    #[test]
    fn an_outline_behind_the_fill_paints_first() {
        let svg_of = |behind: bool| {
            let mut doc = Document::default();
            let layer = doc.pages[0].layers[0].id;
            let id = doc.ids_mut().shape();
            let mut s = Shape::new(
                id,
                ShapeKind::Rect {
                    rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = Fill::Solid(Color::rgb8(0, 0, 0));
            s.stroke = Some(Stroke {
                behind_fill: behind,
                ..Stroke::new(Color::rgb8(255, 255, 255), 2.0)
            });
            doc.layer_mut(layer).unwrap().shapes.push(s);
            page_to_svg(&doc, 0)
        };
        assert!(svg_of(true).contains("paint-order=\"stroke\""));
        assert!(!svg_of(false).contains("paint-order"));
    }

    /// An outline that scales with its object is as wide in the SVG as in
    /// the drawing window: its width times the object's scale factor.
    #[test]
    fn an_outline_that_scales_with_its_object_takes_the_object_scale() {
        let svg_of = |scale_with_object: bool| {
            let mut doc = Document::default();
            let layer = doc.pages[0].layers[0].id;
            let id = doc.ids_mut().shape();
            let mut s = Shape::new(
                id,
                ShapeKind::Rect {
                    rect: Rect::new(0.0, 0.0, 200.0, 100.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.transform = Affine::translate((20.0, 20.0)) * Affine::scale(0.25);
            s.stroke = Some(Stroke {
                scale_with_object,
                ..Stroke::new(Color::rgb8(0, 0, 0), 10.0)
            });
            doc.layer_mut(layer).unwrap().shapes.push(s);
            page_to_svg(&doc, 0)
        };
        assert!(svg_of(true).contains("stroke-width=\"2.5\""));
        assert!(svg_of(false).contains("stroke-width=\"10\""));
    }

    #[test]
    fn exports_a_filled_rect() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.stroke = Some(Stroke::new(Color::BLACK, 0.5));
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        assert!(svg.contains("viewBox=\"0 0 210 297\""));
        assert!(svg.contains("fill=\"#ff0000\""));
        // Bottom-left origin: y=10 in page space is y=287 in SVG space.
        assert!(svg.contains("M10 287"));
        assert!(svg.contains("stroke-width=\"0.5\""));
    }

    #[test]
    fn shapes_fill_even_odd_like_the_renderer() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Polygon {
                rect: Rect::new(10.0, 10.0, 60.0, 60.0),
                points: 5,
                sharpness: 0.0,
                complex: Some(1),
                twist: 0.0,
            },
        );
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        let line = svg
            .lines()
            .find(|l| l.contains("fill=\"#ff0000\""))
            .expect("the star");
        assert!(line.contains("fill-rule=\"evenodd\""), "{line}");
    }

    /// A curve whose fill rule is nonzero (#1919) keeps the SVG default,
    /// so its overlapping parts stay filled in the SVG too.
    #[test]
    fn a_nonzero_curve_keeps_the_default_fill_rule() {
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
        s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        s.fill_rule = tracedraw_core::shaping::FillRule::NonZero;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        let line = svg
            .lines()
            .find(|l| l.contains("fill=\"#ff0000\""))
            .expect("the square");
        assert!(!line.contains("fill-rule"), "{line}");
    }

    #[test]
    fn calligraphic_outline_becomes_a_filled_band() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::None;
        let mut st = Stroke::new(Color::rgb8(0, 0, 255), 3.0);
        st.stretch = 0.2;
        st.nib_angle = 45.0;
        s.stroke = Some(st);
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        // The object itself carries no stroke; the band is a blue fill.
        assert!(!svg.contains("stroke-width=\"3\""));
        assert!(svg.contains("fill=\"#0000ff\" stroke=\"none\""));
        let band_start = svg.find("fill=\"#0000ff\"").unwrap();
        let line = svg[..band_start].rsplit('\n').next().unwrap();
        // A real outline of the rectangle: dozens of vertices.
        assert!(line.matches('L').count() > 20, "{line}");
    }

    #[test]
    fn variable_outlines_become_a_filled_band_and_survive_the_native_format() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut path = tracedraw_core::geometry::BezPath::new();
        path.move_to(tracedraw_core::Point::new(10.0, 50.0));
        path.line_to(tracedraw_core::Point::new(110.0, 50.0));
        let mut s = Shape::new(
            id,
            ShapeKind::Path {
                path,
                closed: false,
            },
        );
        let mut st = Stroke::new(Color::rgb8(255, 0, 0), 1.0);
        st.widths
            .push(tracedraw_core::WidthNode::symmetric(0.5, 8.0));
        s.stroke = Some(st.clone());
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        assert!(svg.contains("fill=\"#ff0000\" stroke=\"none\""), "{svg}");
        assert!(!svg.contains("stroke-width=\"1\""));
        // The native format keeps the nodes.
        let file = std::env::temp_dir().join(format!("vo-{}.tdraw", std::process::id()));
        crate::save_native(&doc, &file).unwrap();
        let back = crate::load_native(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        let widths = back.pages[0].layers[0].shapes[0]
            .stroke
            .as_ref()
            .map(|s| s.widths.clone());
        assert_eq!(widths, Some(st.widths));
    }

    #[test]
    fn painterly_outlines_are_written_as_marks_by_opacity() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut path = tracedraw_core::geometry::BezPath::new();
        path.move_to(tracedraw_core::Point::new(10.0, 50.0));
        path.line_to(tracedraw_core::Point::new(110.0, 50.0));
        let mut s = Shape::new(
            id,
            ShapeKind::Path {
                path,
                closed: false,
            },
        );
        let mut st = Stroke::new(Color::rgb8(0, 128, 0), 6.0);
        st.painterly = Some(tracedraw_core::painterly::Painterly::new(
            tracedraw_core::painterly::PainterlyBrush::Watercolor,
        ));
        s.stroke = Some(st);
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        assert!(svg.contains("fill=\"#008000\" fill-opacity=\""), "{svg}");
        assert!(!svg.contains("stroke-width=\"6\""));
        let pdf = crate::pdf::document_to_pdf(&doc);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/ca "));
    }

    /// A line from (10, 50) to (60, 50), 2 mm wide, with a custom
    /// triangular end head, moved 100 mm up by its transform.
    fn arrow_line() -> Shape {
        let mut tri = tracedraw_core::geometry::BezPath::new();
        tri.move_to((0.0, 0.0));
        tri.line_to((20.0, 10.0));
        tri.line_to((0.0, 20.0));
        tri.close_path();
        let mut line = Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Path {
                path: {
                    let mut p = tracedraw_core::geometry::BezPath::new();
                    p.move_to((10.0, 50.0));
                    p.line_to((60.0, 50.0));
                    p
                },
                closed: false,
            },
        );
        line.fill = Fill::None;
        line.stroke = Some(Stroke {
            end_arrow: tracedraw_core::Arrowhead::from_shape_path(&tri, "Tri"),
            ..Stroke::new(Color::rgb8(0, 0, 255), 2.0)
        });
        line.transform = Affine::translate((0.0, 100.0));
        line
    }

    #[test]
    fn custom_arrowhead_is_written_as_a_filled_path_at_the_line_end() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        doc.layer_mut(layer).unwrap().shapes.push(arrow_line());
        let svg = page_to_svg(&doc, 0);
        // The head is 8 mm (four widths) long and tall: base corners at
        // (52, 154) and (52, 146) in page space, tip at (60, 150); SVG
        // flips y against the 297 mm page.
        let head = svg
            .lines()
            .find(|l| l.contains("fill=\"#0000ff\" stroke=\"none\""))
            .expect("head path");
        assert!(head.contains("52 143"), "{head}");
        assert!(head.contains("60 147"), "{head}");
        assert!(head.contains("52 151"), "{head}");
        assert!(
            head.ends_with("Z\" fill=\"#0000ff\" stroke=\"none\"/>"),
            "{head}"
        );
        // No head: no extra path.
        if let Some(s) = doc.pages[0].layers[0].shapes.first_mut() {
            if let Some(st) = s.stroke.as_mut() {
                st.end_arrow = tracedraw_core::Arrowhead::None;
            }
        }
        let svg = page_to_svg(&doc, 0);
        assert!(!svg.contains("fill=\"#0000ff\" stroke=\"none\""));
    }

    /// #1210: a head sized to 200 % and moved 5 mm out past the end is
    /// written where the editor put it: 16 mm long from x 49 to the tip
    /// at 65, 12.8 mm tall about the line (page y 150, flipped against
    /// the 297 mm page).
    #[test]
    fn a_moved_and_scaled_arrowhead_lands_where_the_editor_put_it() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut line = arrow_line();
        if let Some(st) = line.stroke.as_mut() {
            st.end_arrow = tracedraw_core::Arrowhead::Arrow;
            st.end_head.size = 200.0;
            st.end_head.offset = 5.0;
        }
        doc.layer_mut(layer).unwrap().shapes.push(line);
        let svg = page_to_svg(&doc, 0);
        let head = svg
            .lines()
            .find(|l| l.contains("fill=\"#0000ff\" stroke=\"none\""))
            .expect("head path");
        let d = head
            .split("d=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("path data");
        let nums: Vec<f64> = d
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter_map(|t| t.parse::<f64>().ok())
            .collect();
        assert!(nums.len() >= 8 && nums.len() % 2 == 0, "{d}");
        let xs: Vec<f64> = nums.iter().step_by(2).copied().collect();
        let ys: Vec<f64> = nums.iter().skip(1).step_by(2).copied().collect();
        let (x0, x1) = xs
            .iter()
            .fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
        let (y0, y1) = ys
            .iter()
            .fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
        assert!((x0 - 49.0).abs() < 0.01 && (x1 - 65.0).abs() < 0.01, "{d}");
        assert!(
            (y0 - (297.0 - 156.4)).abs() < 0.01 && (y1 - (297.0 - 143.6)).abs() < 0.01,
            "{d}"
        );
    }

    #[test]
    fn vector_pattern_exports_as_svg_pattern_with_vector_content() {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let mut tile_square = Shape::new(
            tracedraw_core::ShapeId(7),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 5.0, 5.0),
                radius: 0.0,
                corners: None,
            },
        );
        tile_square.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        tile_square.stroke = None;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 30.0, 30.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = Fill::Pattern(tracedraw_core::Pattern::Vector {
            shapes: vec![tile_square],
            tile: tracedraw_core::geometry::Size::new(10.0, 10.0),
            placement: Default::default(),
        });
        s.stroke = None;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        let svg = page_to_svg(&doc, 0);
        let start = svg.find("<pattern id=\"pat1\"").expect("pattern element");
        let end = svg[start..].find("</pattern>").expect("pattern end") + start;
        let pattern = &svg[start..end];
        assert!(pattern.contains("patternUnits=\"userSpaceOnUse\" width=\"10\" height=\"10\""));
        // The tile content is a vector path, not an image, flipped into the
        // tile's top-down space (y 0..5 becomes 5..10).
        assert!(pattern.contains("<path id=\"7\""), "{pattern}");
        assert!(pattern.contains("fill=\"#ff0000\""), "{pattern}");
        assert!(pattern.contains("M0 10L5 10L5 5L0 5Z"), "{pattern}");
        assert!(!pattern.contains("<image"));
        assert!(svg.contains("fill=\"url(#pat1)\""));
    }

    /// A transparency's merge mode goes out as the CSS blend mode on a
    /// group around the object; modes CSS has no blend for draw normally
    /// (#1203).
    #[test]
    fn merge_modes_go_out_as_mix_blend_mode() {
        use tracedraw_core::live::{Effect, MergeMode};
        let svg_with = |merge: MergeMode| {
            let mut doc = Document::default();
            let layer = doc.pages[0].layers[0].id;
            let id = doc.ids_mut().shape();
            let mut s = Shape::new(
                id,
                ShapeKind::Rect {
                    rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                    radius: 0.0,
                    corners: None,
                },
            );
            s.fill = Fill::Solid(Color::rgb8(255, 0, 0));
            s.effects.push(Effect::Transparency {
                mask: Fill::Solid(Color::WHITE),
                merge,
                target: 2,
            });
            doc.layer_mut(layer).unwrap().shapes.push(s);
            page_to_svg(&doc, 0)
        };
        for (merge, css) in [
            (MergeMode::Screen, "screen"),
            (MergeMode::Overlay, "overlay"),
            (MergeMode::SoftLight, "soft-light"),
            (MergeMode::HardLight, "hard-light"),
            (MergeMode::ColorDodge, "color-dodge"),
            (MergeMode::ColorBurn, "color-burn"),
            (MergeMode::Exclusion, "exclusion"),
            (MergeMode::Multiply, "multiply"),
            (MergeMode::IfLighter, "lighten"),
        ] {
            let svg = svg_with(merge);
            assert!(
                svg.contains(&format!("style=\"mix-blend-mode:{css}\"")),
                "{merge:?}: {svg}"
            );
            assert!(svg.contains("fill=\"#ff0000\""), "{merge:?}");
        }
        for merge in [MergeMode::Normal, MergeMode::Behind, MergeMode::Xor] {
            assert!(!svg_with(merge).contains("mix-blend-mode"), "{merge:?}");
        }
    }

    /// The placement goes out as `patternTransform`; mirrored tiles make
    /// the pattern's tile the two-tile block with the picture flipped in
    /// its second half; Transform with object puts the object's transform
    /// in front.
    #[test]
    fn pattern_transform_carries_the_placement() {
        use tracedraw_core::style::PatternPlacement;
        let checker = |placement| {
            Fill::Pattern(tracedraw_core::Pattern::TwoColor {
                tile: tracedraw_core::PatternTile::Checker,
                front: Color::BLACK,
                back: Color::WHITE,
                size_mm: 10.0,
                placement,
            })
        };
        // The default page is 297 mm tall: the tiles anchor at its
        // bottom-left corner, so the pattern's origin is 287 mm down.
        let svg = one_shape_svg(checker(PatternPlacement::default()), None);
        assert!(
            svg.contains("width=\"10\" height=\"10\" patternTransform=\"matrix(1 0 0 1 0 287)\""),
            "{svg}"
        );
        let svg = one_shape_svg(
            checker(PatternPlacement {
                mirror_horizontal: true,
                offset_x: 2.0,
                offset_y: 3.0,
                rotation: 90.0,
                ..PatternPlacement::default()
            }),
            None,
        );
        // Two tiles wide; turned a quarter turn counter-clockwise on the
        // page around the offset point (2, 3): the block's bottom-left
        // corner, pattern space (0, 10), lands on the offset, output
        // (2, 294), and its 10 mm of height run left along the page.
        assert!(
            svg.contains("width=\"20\" height=\"10\" patternTransform=\"matrix(0 -1 1 0 -8 294)\""),
            "{svg}"
        );
        assert!(svg.contains("<image width=\"20\" height=\"10\""), "{svg}");

        // A vector tile with a row offset: the leaning block holds the
        // tile's shapes once per copy, and the lean is in the transform.
        let mut tile_square = Shape::new(
            tracedraw_core::ShapeId(7),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 5.0, 5.0),
                radius: 0.0,
                corners: None,
            },
        );
        tile_square.fill = Fill::Solid(Color::rgb8(255, 0, 0));
        tile_square.stroke = None;
        let svg = one_shape_svg(
            Fill::Pattern(tracedraw_core::Pattern::Vector {
                shapes: vec![tile_square],
                tile: tracedraw_core::geometry::Size::new(10.0, 10.0),
                placement: PatternPlacement {
                    shift_percent: 50.0,
                    transform_with_object: true,
                    ..PatternPlacement::default()
                },
            }),
            None,
        );
        // With the object (an untransformed rectangle): the page flip
        // alone, so the tiles start at the page's bottom-left corner, the
        // rows leaning half a tile (the block's top edge starts 5 mm
        // along, and pattern space runs down from there).
        assert!(
            svg.contains("patternTransform=\"matrix(1 0 -0.5 1 5 287)\""),
            "{svg}"
        );
        assert_eq!(svg.matches("<path id=\"7\"").count(), 4, "{svg}");
    }

    fn one_shape_svg(fill: Fill, stroke: Option<Stroke>) -> String {
        let mut doc = Document::default();
        let layer = doc.pages[0].layers[0].id;
        let id = doc.ids_mut().shape();
        let mut s = Shape::new(
            id,
            ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            },
        );
        s.fill = fill;
        s.stroke = stroke;
        doc.layer_mut(layer).unwrap().shapes.push(s);
        page_to_svg(&doc, 0)
    }

    /// A procedural fill is a `<pattern>` holding the tile's paths as
    /// vectors over its background, never a picture (#1212).
    #[test]
    fn procedural_fill_is_a_vector_pattern() {
        use tracedraw_core::{Procedural, ProceduralPattern};
        let mut p = Procedural::new(ProceduralPattern::Honeycomb, Color::rgb8(255, 0, 0));
        p.params.size_mm = 6.0;
        let svg = one_shape_svg(Fill::Procedural(p.clone()), None);
        assert!(svg.contains("fill=\"url(#pat1)\""), "{svg}");
        let start = svg.find("<pattern id=\"pat1\"").expect("a pattern");
        let end = svg[start..].find("</pattern>").expect("closed") + start;
        let pat = &svg[start..end];
        assert!(pat.contains("patternUnits=\"userSpaceOnUse\""), "{pat}");
        assert!(pat.contains("width=\"6\""), "{pat}");
        // Two rows of hexagons: the tile is sqrt(3) times as high.
        assert!(pat.contains("height=\"10.3923\""), "{pat}");
        assert!(pat.contains("<rect width=\"6\" height=\"10.3923\" fill=\"#ffffff\"/>"));
        assert_eq!(pat.matches("<path d=\"").count(), 7, "{pat}");
        assert!(pat.contains("fill=\"#ff0000\""), "{pat}");
        assert!(!pat.contains("<image"), "{pat}");
        // No background: no rectangle, what is beneath shows through.
        p.background = None;
        let svg = one_shape_svg(Fill::Procedural(p), None);
        assert!(!svg.contains("<rect width=\"6\""), "{svg}");
        assert!(svg.contains("<pattern id=\"pat1\""), "{svg}");
    }

    /// A fill whose nodes were placed freely goes out through a gradient
    /// transform: the unit gradient mapped onto its start, end and skew
    /// handle (#330).
    #[test]
    fn node_opacity_goes_out_as_stop_opacity() {
        use tracedraw_core::style::FountainKind;
        let mut f =
            tracedraw_core::Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        f.stops[1].opacity = 0.25;
        let svg = one_shape_svg(Fill::Fountain(f), None);
        assert!(svg.contains("stop-opacity=\"0.25\""), "{svg}");
        assert_eq!(svg.matches("stop-opacity").count(), 1, "{svg}");
    }

    #[test]
    fn free_fill_nodes_go_out_as_a_gradient_transform() {
        use tracedraw_core::style::{FountainHandles, FountainKind};
        let mut f =
            tracedraw_core::Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        f.handles = Some(FountainHandles {
            start: tracedraw_core::geometry::Point::new(-1.0, 0.0),
            end: tracedraw_core::geometry::Point::new(0.0, 0.0),
            skew: tracedraw_core::geometry::Point::new(-1.0, 1.0),
        });
        let svg = one_shape_svg(Fill::Fountain(f.clone()), None);
        assert!(
            svg.contains("x2=\"1\" y2=\"0\" gradientTransform=\"matrix("),
            "{svg}"
        );
        f.kind = FountainKind::Radial;
        let svg = one_shape_svg(Fill::Fountain(f), None);
        assert!(svg.contains("r=\"1\" gradientTransform=\"matrix("), "{svg}");
    }

    #[test]
    fn positioned_outlines_are_written_as_filled_areas() {
        let mut st = Stroke::new(Color::rgb8(255, 0, 0), 2.0);
        // Centred: a stroke with its mitre limit.
        let svg = one_shape_svg(Fill::None, Some(st.clone()));
        assert!(svg.contains("stroke-miterlimit=\"2.61"), "{svg}");
        // Inside: no stroke on the object, a filled path in the colour.
        st.position = tracedraw_core::OutlinePosition::Inside;
        let svg = one_shape_svg(Fill::None, Some(st.clone()));
        assert!(!svg.contains("stroke-width"), "{svg}");
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
        // Aligned dashes too.
        st.position = tracedraw_core::OutlinePosition::Center;
        st.dash = vec![3.0, 2.0];
        st.align_dashes = true;
        let svg = one_shape_svg(Fill::None, Some(st));
        assert!(!svg.contains("stroke-dasharray"), "{svg}");
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    }

    #[test]
    fn outline_paints_are_written_as_the_stroke_paint() {
        use tracedraw_core::{Fountain, FountainKind};
        let red = Color::rgb8(255, 0, 0);
        let blue = Color::rgb8(0, 0, 255);
        // A gradient outline strokes with the gradient.
        let mut st = Stroke::new(Color::rgb8(0, 255, 0), 2.0);
        st.fill = Some(Box::new(Fill::linear(red, blue, 0.0)));
        let svg = one_shape_svg(Fill::None, Some(st.clone()));
        assert!(svg.contains("stroke=\"url(#grad1)\""), "{svg}");
        assert!(svg.contains("<linearGradient id=\"grad1\""), "{svg}");
        assert!(!svg.contains("#00ff00"), "{svg}");
        // A pattern outline strokes with the pattern.
        st.fill = Some(Box::new(Fill::Pattern(tracedraw_core::Pattern::TwoColor {
            tile: tracedraw_core::PatternTile::Checker,
            front: red,
            back: blue,
            size_mm: 5.0,
            placement: Default::default(),
        })));
        let svg = one_shape_svg(Fill::None, Some(st.clone()));
        assert!(svg.contains("stroke=\"url(#pat1)\""), "{svg}");
        // A texture outline: a colour between its two, still an outline.
        st.fill = Some(Box::new(Fill::Texture(tracedraw_core::Texture {
            kind: tracedraw_core::TextureKind::Clouds,
            color_a: red,
            color_b: blue,
            scale: 5.0,
            seed: 1,
        })));
        let svg = one_shape_svg(Fill::None, Some(st.clone()));
        assert!(
            svg.contains("stroke=\"#800080\" stroke-width=\"2\""),
            "{svg}"
        );
        // A calligraphic band takes the paint as its fill.
        st.stretch = 0.3;
        st.fill = Some(Box::new(Fill::radial(red, blue)));
        let svg = one_shape_svg(Fill::None, Some(st));
        assert!(svg.contains("stroke=\"none\"/>\n"), "{svg}");
        assert!(
            svg.contains("fill=\"url(#grad1)\" stroke=\"none\""),
            "{svg}"
        );
        // Gradient options are worked into the stops; a shaped gradient
        // is written as a radial one, its inside colour at the centre.
        let mut f = Fountain::two(FountainKind::Shaped, red, blue, 0.0);
        f.invert = true;
        let svg = one_shape_svg(Fill::Fountain(f), None);
        assert!(svg.contains("<radialGradient id=\"grad1\""), "{svg}");
        let first = svg.find("<stop offset=\"0\" stop-color=\"").expect("stop");
        assert!(
            svg[first..].starts_with("<stop offset=\"0\" stop-color=\"#ff0000\""),
            "{svg}"
        );
        let mut saw = Fountain::two(FountainKind::Linear, red, blue, 0.0);
        saw.repeat = tracedraw_core::FountainRepeat::Repeat;
        saw.repeats = 3;
        let svg = one_shape_svg(Fill::Fountain(saw), None);
        assert_eq!(svg.matches("<stop ").count(), 6, "{svg}");
    }

    /// A stepped gradient goes out as a pair of stops per band; a hue
    /// path or a smooth transition as a sampled run of stops, at most
    /// MAX_SAMPLED_STOPS of them (#1208).
    #[test]
    fn blend_settings_go_out_as_stops() {
        use tracedraw_core::style::{Fountain, FountainKind};
        let (red, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255));
        let mut f = Fountain::two(FountainKind::Linear, Color::BLACK, Color::WHITE, 0.0);
        f.steps = 4;
        let svg = one_shape_svg(Fill::Fountain(f), None);
        assert_eq!(svg.matches("<stop ").count(), 8, "{svg}");
        assert!(
            svg.contains("<stop offset=\"0.25\" stop-color=\"#000000\""),
            "{svg}"
        );
        assert!(
            svg.contains("<stop offset=\"0.25\" stop-color=\"#555555\""),
            "{svg}"
        );
        assert!(
            svg.contains("<stop offset=\"0.5\" stop-color=\"#aaaaaa\""),
            "{svg}"
        );
        // Two stops around the wheel: 17 sampled stops, green half way.
        let mut h = Fountain::two(FountainKind::Linear, red, blue, 0.0);
        h.blend_direction = tracedraw_core::ColorBlend::Clockwise;
        let svg = one_shape_svg(Fill::Fountain(h), None);
        assert_eq!(
            svg.matches("<stop ").count(),
            Fountain::MID_STEPS + 1,
            "{svg}"
        );
        assert!(
            svg.contains("<stop offset=\"0.5\" stop-color=\"#00ff00\""),
            "{svg}"
        );
        // Many stops smoothed: sampled down to the cap.
        let mut s = Fountain {
            stops: (0..=6)
                .map(|i| {
                    tracedraw_core::Stop::new(i as f64 / 6.0, if i % 2 == 0 { red } else { blue })
                })
                .collect(),
            ..Default::default()
        };
        s.smooth = true;
        assert!(s.resolved_stops().len() > MAX_SAMPLED_STOPS);
        let svg = one_shape_svg(Fill::Fountain(s), None);
        assert_eq!(svg.matches("<stop ").count(), MAX_SAMPLED_STOPS, "{svg}");
        // A plain gradient keeps its two stops.
        let svg = one_shape_svg(Fill::linear(red, blue, 0.0), None);
        assert_eq!(svg.matches("<stop ").count(), 2, "{svg}");
    }
}
