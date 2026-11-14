//! Cutting a picture's pixels to an area: the Crop tool's "Crop picture
//! pixels" and Bitmaps > Crop to Frame (#1913), and exports, which leave
//! out the pixels of a picture in a clip frame that the frame hides, so
//! what a crop took away cannot come back from the exported file.

use std::borrow::Cow;
use tracedraw_core::document::{Shape, ShapeKind};
use tracedraw_core::geometry::{Affine, Rect, Shape as _};
use tracedraw_core::picture_inks::{fingerprint, PictureInks};
use tracedraw_core::{Document, Fill};

/// The whole pixels of a `width` by `height` picture laid on `rect` (its
/// own space, rows from the top) that `area` (the same space) touches:
/// columns `x0..x1` and rows `y0..y1`; `None` when it touches none.
pub fn pixel_window(
    rect: Rect,
    width: u32,
    height: u32,
    area: Rect,
) -> Option<(u32, u32, u32, u32)> {
    if !(rect.width() > 0.0 && rect.height() > 0.0) || width == 0 || height == 0 {
        return None;
    }
    let (w, h) = (f64::from(width), f64::from(height));
    // A hair's tolerance, so an area on a pixel edge keeps no extra
    // column of pixels.
    let lo = |v: f64| (v + 1e-6).floor();
    let hi = |v: f64| (v - 1e-6).ceil();
    let x0 = lo((area.x0 - rect.x0) / rect.width() * w).clamp(0.0, w);
    let x1 = hi((area.x1 - rect.x0) / rect.width() * w).clamp(0.0, w);
    let y0 = lo((rect.y1 - area.y1) / rect.height() * h).clamp(0.0, h);
    let y1 = hi((rect.y1 - area.y0) / rect.height() * h).clamp(0.0, h);
    if !(x0 < x1 && y0 < y1) {
        return None;
    }
    Some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}

/// The picture `s` (a bitmap) with only its pixels inside `area` (a
/// rectangle in the picture's own space, the space of its `rect`) kept,
/// on its whole pixels: its new kind and its ink channels cut alike
/// (#1913). The effects stack's original is cut the same way when it
/// lies on the same pixels, else the effects stay in the pixels. The
/// picture keeps its colour type (a grey picture stays grey). `None`
/// when the area keeps every pixel or none, or the picture cannot be
/// read.
pub fn crop_picture(s: &Shape, area: Rect) -> Option<(ShapeKind, Option<Box<PictureInks>>)> {
    let ShapeKind::Bitmap {
        rect,
        width_px,
        height_px,
        png,
        fx,
    } = &s.kind
    else {
        return None;
    };
    let rect = rect.abs();
    let (x0, y0, x1, y1) = pixel_window(rect, *width_px, *height_px, area)?;
    if (x0, y0, x1, y1) == (0, 0, *width_px, *height_px) {
        return None;
    }
    let cut = |png: &[u8]| -> Option<Vec<u8>> {
        let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
        if img.width() != *width_px || img.height() != *height_px {
            return None;
        }
        let part = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
        let mut buf = std::io::Cursor::new(Vec::new());
        part.write_to(&mut buf, image::ImageFormat::Png).ok()?;
        Some(buf.into_inner())
    };
    let new_png = cut(png)?;
    let (w, h) = (f64::from(*width_px), f64::from(*height_px));
    let new_rect = Rect::new(
        rect.x0 + f64::from(x0) / w * rect.width(),
        rect.y1 - f64::from(y1) / h * rect.height(),
        rect.x0 + f64::from(x1) / w * rect.width(),
        rect.y1 - f64::from(y0) / h * rect.height(),
    );
    let new_fx = fx.as_ref().and_then(|f| {
        let same = f.width_px == *width_px
            && f.height_px == *height_px
            && (f.rect.abs().x0 - rect.x0).abs() < 1e-6
            && (f.rect.abs().y0 - rect.y0).abs() < 1e-6
            && (f.rect.abs().x1 - rect.x1).abs() < 1e-6
            && (f.rect.abs().y1 - rect.y1).abs() < 1e-6;
        let png = cut(&f.png).filter(|_| same)?;
        let mut f = f.clone();
        f.rect = new_rect;
        f.width_px = x1 - x0;
        f.height_px = y1 - y0;
        f.png = png.into();
        Some(f)
    });
    // The ink planes, one under the other, each cut like the pixels.
    let inks = s
        .picture_inks
        .as_deref()
        .filter(|i| i.fits(png.as_slice()))
        .and_then(|i| {
            let planes = image::load_from_memory(&i.png).ok()?.to_luma8();
            let n = i.inks.len() as u32;
            if n == 0
                || planes.width() != *width_px
                || Some(planes.height()) != height_px.checked_mul(n)
            {
                return None;
            }
            let (cw, ch) = (x1 - x0, y1 - y0);
            let mut out = image::GrayImage::new(cw, ch.checked_mul(n)?);
            for k in 0..n {
                let part =
                    image::imageops::crop_imm(&planes, x0, k * height_px + y0, cw, ch).to_image();
                image::imageops::replace(&mut out, &part, 0, i64::from(k * ch));
            }
            let mut bytes = std::io::Cursor::new(Vec::new());
            out.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
            Some(Box::new(PictureInks {
                inks: i.inks.clone(),
                png: bytes.into_inner().into(),
                of: fingerprint(&new_png),
            }))
        });
    Some((
        ShapeKind::Bitmap {
            rect: new_rect,
            width_px: x1 - x0,
            height_px: y1 - y0,
            png: new_png.into(),
            fx: new_fx,
        },
        inks,
    ))
}

/// How many samples a side each pixel is tested with against an outline
/// ([`mask_picture`]): the edge comes out in steps of a quarter.
const MASK_SAMPLES: u32 = 2;

/// The picture `s` (a bitmap) with the pixels outside `outline` (a
/// closed path in the picture's own space, filled by the nonzero rule)
/// made see-through: its new kind, on the same pixels and box, and its
/// ink channels kept for the new pixels. A pixel partly inside keeps
/// that share of its opacity. The effects stack's original is cut the
/// same way when it lies on the same pixels. `None` when every pixel
/// lies inside, or the picture cannot be read (#1913).
pub fn mask_picture(
    s: &Shape,
    outline: &tracedraw_core::geometry::BezPath,
) -> Option<(ShapeKind, Option<Box<PictureInks>>)> {
    let ShapeKind::Bitmap {
        rect,
        width_px,
        height_px,
        png,
        fx,
    } = &s.kind
    else {
        return None;
    };
    let rect = rect.abs();
    let (w, h) = (*width_px, *height_px);
    if w == 0 || h == 0 || !(rect.width() > 0.0 && rect.height() > 0.0) {
        return None;
    }
    // The share of each pixel inside the outline, rows from the top.
    let (px, py) = (rect.width() / f64::from(w), rect.height() / f64::from(h));
    let n = MASK_SAMPLES;
    let mut cover = vec![0u8; (w as usize) * (h as usize)];
    let mut all = true;
    for y in 0..h {
        for x in 0..w {
            let mut inside = 0u32;
            for j in 0..n {
                for i in 0..n {
                    let p = tracedraw_core::geometry::Point::new(
                        rect.x0 + (f64::from(x) + (f64::from(i) + 0.5) / f64::from(n)) * px,
                        rect.y1 - (f64::from(y) + (f64::from(j) + 0.5) / f64::from(n)) * py,
                    );
                    if outline.winding(p) != 0 {
                        inside += 1;
                    }
                }
            }
            all &= inside == n * n;
            cover[(y * w + x) as usize] = (inside * 255 / (n * n)) as u8;
        }
    }
    if all {
        return None;
    }
    let cut = |png: &[u8]| -> Option<Vec<u8>> {
        let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
        if img.width() != w || img.height() != h {
            return None;
        }
        let mut img = img.to_rgba8();
        for (x, y, p) in img.enumerate_pixels_mut() {
            let c = u32::from(cover[(y * w + x) as usize]);
            p.0[3] = ((u32::from(p.0[3]) * c + 127) / 255) as u8;
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
        Some(buf.into_inner())
    };
    let new_png = cut(png)?;
    let new_fx = fx.as_ref().and_then(|f| {
        let same = f.width_px == w
            && f.height_px == h
            && (f.rect.abs().x0 - rect.x0).abs() < 1e-6
            && (f.rect.abs().y0 - rect.y0).abs() < 1e-6
            && (f.rect.abs().x1 - rect.x1).abs() < 1e-6
            && (f.rect.abs().y1 - rect.y1).abs() < 1e-6;
        let png = cut(&f.png).filter(|_| same)?;
        let mut f = f.clone();
        f.png = png.into();
        Some(f)
    });
    // The ink planes hold no opacity: they stay, named for the new pixels.
    let inks = s
        .picture_inks
        .as_deref()
        .filter(|i| i.fits(png.as_slice()))
        .map(|i| {
            Box::new(PictureInks {
                inks: i.inks.clone(),
                png: i.png.clone(),
                of: fingerprint(&new_png),
            })
        });
    Some((
        ShapeKind::Bitmap {
            rect: *match &s.kind {
                ShapeKind::Bitmap { rect, .. } => rect,
                _ => unreachable!(),
            },
            width_px: w,
            height_px: h,
            png: new_png.into(),
            fx: new_fx,
        },
        inks,
    ))
}

/// Whether the live effects of `s` look the same on any box: none, or
/// only a uniform transparency. A gradient transparency, say, lies on
/// the object's box and moves when the box changes.
pub fn effects_fit_any_box(s: &Shape) -> bool {
    s.effects.iter().all(|e| {
        matches!(
            e,
            tracedraw_core::live::Effect::Transparency {
                mask: Fill::Solid(_) | Fill::None,
                ..
            }
        )
    })
}

/// Whether a picture can lose pixels without changing how it looks in
/// its frame: nothing on it reaches past the pixels or is laid on its
/// box (an outline round it, a shadow, a gradient transparency, other
/// live effects).
fn trimmable(s: &Shape) -> bool {
    s.visible
        && s.stroke.is_none()
        && s.shadow.is_none()
        && s.raster_fx.is_none()
        && s.extra_fills.is_empty()
        && s.extra_strokes.is_empty()
        && effects_fit_any_box(s)
}

/// Trim the pictures among `contents` (and inside their groups), whose
/// parent maps to the frame's space by `parent`, to `frame` (the
/// frame's outline in that space). True when one changed.
fn trim_contents(contents: &mut [Shape], parent: Affine, frame: &tracedraw_core::BezPath) -> bool {
    let mut changed = false;
    for c in contents {
        let m = parent * c.transform;
        match &mut c.kind {
            ShapeKind::Group { children } => {
                changed |= trim_contents(children, m, frame);
                continue;
            }
            ShapeKind::ClipFrame { .. } => {
                changed |= trim_shape(c);
                continue;
            }
            ShapeKind::Bitmap { .. } => {}
            _ => continue,
        }
        let det = m.determinant();
        if !det.is_finite() || det.abs() < 1e-12 || !trimmable(c) {
            continue;
        }
        // The frame in the picture's own space: the pixels outside its
        // box never show.
        let area = (m.inverse() * frame.clone()).bounding_box();
        if !(area.x0.is_finite()
            && area.y0.is_finite()
            && area.x1.is_finite()
            && area.y1.is_finite())
        {
            continue;
        }
        if let Some((kind, inks)) = crop_picture(c, area) {
            c.kind = kind;
            c.picture_inks = inks;
            changed = true;
        }
    }
    changed
}

/// Trim the pictures inside every clip frame of `s`. True when one
/// changed.
fn trim_shape(s: &mut Shape) -> bool {
    match &mut s.kind {
        ShapeKind::Group { children } => {
            let mut changed = false;
            for c in children {
                changed |= trim_shape(c);
            }
            changed
        }
        ShapeKind::ClipFrame { frame, contents } => {
            // The frame and the contents share the clip frame's space.
            let outline = frame.page_path();
            let mut changed = trim_shape(frame);
            if !outline.elements().is_empty() {
                changed |= trim_contents(contents, Affine::IDENTITY, &outline);
            }
            changed
        }
        _ => false,
    }
}

/// Whether `s` holds a picture in a clip frame anywhere.
fn holds_framed_picture(s: &Shape) -> bool {
    fn has_picture(s: &Shape) -> bool {
        match &s.kind {
            ShapeKind::Bitmap { .. } => true,
            ShapeKind::Group { children } => children.iter().any(has_picture),
            ShapeKind::ClipFrame { contents, .. } => contents.iter().any(has_picture),
            _ => false,
        }
    }
    match &s.kind {
        ShapeKind::Group { children } => children.iter().any(holds_framed_picture),
        ShapeKind::ClipFrame { frame, contents } => {
            contents.iter().any(has_picture) || holds_framed_picture(frame)
        }
        _ => false,
    }
}

/// The document with the pixels of every picture in a clip frame that
/// lie outside the frame's box (in the picture's own space) taken out,
/// as exports should see it: a picture cropped into a frame shows the
/// same, and what the crop hid is not in the file to be found by taking
/// the frame apart or by a reader that ignores clipping. Pictures with
/// an outline, a shadow or effects laid on their box keep their pixels.
/// With `page`, only that page (by index) and the master page are
/// trimmed: the export of one page reads no more.
pub fn trim_framed_pictures(doc: &Document, page: Option<usize>) -> Cow<'_, Document> {
    let wanted = |i: usize| page.is_none_or(|p| p == i);
    let any = doc
        .pages
        .iter()
        .enumerate()
        .filter(|(i, _)| wanted(*i))
        .flat_map(|(_, p)| &p.layers)
        .chain(&doc.master)
        .flat_map(|l| &l.shapes)
        .any(holds_framed_picture);
    if !any {
        return Cow::Borrowed(doc);
    }
    let mut out = doc.clone();
    let mut changed = false;
    for (i, p) in out.pages.iter_mut().enumerate() {
        if !wanted(i) {
            continue;
        }
        for layer in &mut p.layers {
            for s in &mut layer.shapes {
                changed |= trim_shape(s);
            }
        }
    }
    for layer in &mut out.master {
        for s in &mut layer.shapes {
            changed |= trim_shape(s);
        }
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::geometry::Size;
    use tracedraw_core::ShapeId;

    /// A 4 by 2 picture on (0, 0) to (40, 20), each pixel 10 mm: columns
    /// red, green, blue and white.
    fn picture(id: u64) -> Shape {
        let cols = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
        let img = image::RgbaImage::from_fn(4, 2, |x, _| {
            let [r, g, b] = cols[x as usize];
            image::Rgba([r, g, b, 255])
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png)
            .expect("png");
        let mut s = Shape::new(
            ShapeId(id),
            ShapeKind::Bitmap {
                rect: Rect::new(0.0, 0.0, 40.0, 20.0),
                width_px: 4,
                height_px: 2,
                png: png.into_inner().into(),
                fx: None,
            },
        );
        s.stroke = None;
        s
    }

    /// The pixels outside an outline go see-through, a pixel half inside
    /// keeps half its opacity, and an outline round every pixel changes
    /// nothing (#1913).
    #[test]
    fn a_picture_is_masked_to_an_outline() {
        use tracedraw_core::geometry::{BezPath, Point};
        let pic = picture(1);
        // The triangle under the diagonal from the top left to the
        // bottom right corner.
        let mut tri = BezPath::new();
        tri.move_to(Point::new(0.0, 0.0));
        tri.line_to(Point::new(40.0, 0.0));
        tri.line_to(Point::new(0.0, 20.0));
        tri.close_path();
        let (kind, inks) = mask_picture(&pic, &tri).expect("masked");
        assert!(inks.is_none());
        let ShapeKind::Bitmap {
            rect,
            width_px,
            height_px,
            png,
            ..
        } = &kind
        else {
            panic!("a picture");
        };
        assert_eq!((*width_px, *height_px), (4, 2));
        assert_eq!(*rect, Rect::new(0.0, 0.0, 40.0, 20.0));
        let img = image::load_from_memory(png).expect("png").to_rgba8();
        // Bottom left: all inside, colour kept. Top right: all outside.
        assert_eq!(img.get_pixel(0, 1).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(3, 0).0[3], 0);
        // On the diagonal: partly in.
        let a = img.get_pixel(1, 0).0[3];
        assert!(a > 0 && a < 255, "{a}");
        // Every pixel inside: nothing to do.
        let all = Rect::new(-1.0, -1.0, 41.0, 21.0).to_path(0.01);
        assert!(mask_picture(&pic, &all).is_none());
    }

    /// The picture cropped as the Crop tool frames it: a clip frame of
    /// the area (12, 0) to (28, 20), which shows the green and blue
    /// columns, moved 50 mm right on the page.
    fn cropped_doc(pic: Shape) -> Document {
        let mut frame = Shape::new(
            ShapeId(2),
            ShapeKind::Path {
                path: Rect::new(12.0, 0.0, 28.0, 20.0).to_path(0.01),
                closed: true,
            },
        );
        frame.fill = Fill::None;
        frame.stroke = None;
        let mut clip = Shape::new(
            ShapeId(3),
            ShapeKind::ClipFrame {
                frame: Box::new(frame),
                contents: vec![pic],
            },
        );
        clip.stroke = None;
        clip.transform = Affine::translate((50.0, 0.0));
        let mut doc = Document::new("t", Size::new(100.0, 40.0));
        let layer = doc.pages[0].layers[0].id;
        doc.layer_mut(layer).expect("layer").shapes.push(clip);
        doc
    }

    fn framed_picture(doc: &Document, page: usize) -> &Shape {
        match &doc.pages[page].layers[0].shapes[0].kind {
            ShapeKind::ClipFrame { contents, .. } => &contents[0],
            other => panic!("a clip frame: {other:?}"),
        }
    }

    fn pixels(s: &Shape) -> (Rect, image::RgbaImage) {
        let ShapeKind::Bitmap { rect, png, .. } = &s.kind else {
            panic!("a picture");
        };
        let img = image::load_from_memory(png).expect("decode").to_rgba8();
        (*rect, img)
    }

    /// A picture cropped into a frame (live test of 0.41): the SVG, PDF
    /// and EPS exports are still clipped to the frame and hold only the
    /// pixels it shows, so the part the crop hid is not in the file.
    #[test]
    fn exports_leave_out_what_a_crop_frame_hides() {
        let doc = cropped_doc(picture(1));
        let trimmed = trim_framed_pictures(&doc, None);
        let (rect, img) = pixels(framed_picture(&trimmed, 0));
        // The columns the frame shows, on whole pixels.
        assert_eq!((img.width(), img.height()), (2, 2));
        assert_eq!(rect, Rect::new(10.0, 0.0, 30.0, 20.0));
        assert_eq!(img.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(img.get_pixel(1, 1).0, [0, 0, 255, 255]);
        // The document itself keeps every pixel.
        assert_eq!(pixels(framed_picture(&doc, 0)).1.width(), 4);

        // SVG: still clipped to the frame, and the picture written is
        // the 2 by 2 part, no red in it.
        let svg = crate::svg::page_to_svg(&doc, 0);
        assert!(svg.contains("<clipPath"), "{svg}");
        assert!(svg.contains("width=\"2\" height=\"2\""), "{svg}");
        let b64 = svg
            .split("data:image/png;base64,")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("an embedded picture");
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64");
        let shown = image::load_from_memory(&png).expect("png").to_rgba8();
        assert_eq!((shown.width(), shown.height()), (2, 2));
        assert!(shown.pixels().all(|p| p.0 != [255, 0, 0, 255]));

        // PDF: clipped, and the image is 2 by 2 pixels.
        let pdf = crate::pdf::document_to_pdf(&doc);
        // The file as text, every zlib stream inflated after it.
        let mut text = String::from_utf8_lossy(&pdf).to_string();
        let mut rest: &[u8] = &pdf;
        while let Some(i) = rest.windows(7).position(|w| w == b"stream\n") {
            let data = &rest[i + 7..];
            use std::io::Read;
            let mut out = Vec::new();
            if flate2::read::ZlibDecoder::new(data)
                .read_to_end(&mut out)
                .is_ok()
            {
                text.push_str(&String::from_utf8_lossy(&out));
            }
            rest = data;
        }
        assert!(text.contains("/Width 2 /Height 2"), "cropped image");
        assert!(!text.contains("/Width 4 "), "no whole picture");
        assert!(text.contains("W n"), "clipped");

        // EPS: the picture written is 2 by 2 pixels.
        let eps = crate::eps::page_to_eps(&doc, 0);
        assert!(eps.contains("2 2 8 [2 0 0 -2 0 2]"), "{eps}");
        assert!(!eps.contains("4 2 8 ["), "no whole picture");
    }

    /// A picture whose look depends on its box (a gradient transparency,
    /// an outline round it) keeps its pixels, still clipped; a document
    /// with no framed picture is not copied; one page's export trims that
    /// page only.
    #[test]
    fn pictures_laid_on_their_box_and_plain_documents_stay() {
        let mut pic = picture(1);
        pic.stroke = Some(Default::default());
        let doc = cropped_doc(pic);
        assert!(matches!(trim_framed_pictures(&doc, None), Cow::Borrowed(_)));
        let mut pic = picture(1);
        pic.effects
            .push(tracedraw_core::live::Effect::Transparency {
                mask: Fill::Fountain(tracedraw_core::style::Fountain::two(
                    tracedraw_core::style::FountainKind::Linear,
                    tracedraw_core::Color::BLACK,
                    tracedraw_core::Color::WHITE,
                    0.0,
                )),
                merge: Default::default(),
                target: 2,
            });
        assert!(!effects_fit_any_box(&pic));
        let doc = cropped_doc(pic);
        assert!(matches!(trim_framed_pictures(&doc, None), Cow::Borrowed(_)));
        // A uniform transparency does not stop it.
        let mut pic = picture(1);
        pic.effects
            .push(tracedraw_core::live::Effect::Transparency {
                mask: Fill::Solid(tracedraw_core::Color::rgb8(128, 128, 128)),
                merge: Default::default(),
                target: 2,
            });
        assert!(effects_fit_any_box(&pic));
        let doc = cropped_doc(pic);
        assert!(matches!(trim_framed_pictures(&doc, None), Cow::Owned(_)));
        // A free picture: nothing to trim.
        let mut doc = Document::new("t", Size::new(100.0, 40.0));
        let layer = doc.pages[0].layers[0].id;
        doc.layer_mut(layer).expect("layer").shapes.push(picture(1));
        assert!(matches!(trim_framed_pictures(&doc, None), Cow::Borrowed(_)));
        // Only the page asked for is trimmed.
        let mut two = cropped_doc(picture(1));
        two.pages.push(two.pages[0].clone());
        let one = trim_framed_pictures(&two, Some(1));
        assert_eq!(pixels(framed_picture(&one, 0)).1.width(), 4);
        assert_eq!(pixels(framed_picture(&one, 1)).1.width(), 2);
    }

    /// A damaged picture (no pixels, an area that is not a number, a box
    /// of no size) is left alone, not a panic.
    #[test]
    fn damaged_pictures_are_left_alone() {
        let mut pic = picture(1);
        if let ShapeKind::Bitmap { png, .. } = &mut pic.kind {
            *png = Vec::new().into();
        }
        assert!(crop_picture(&pic, Rect::new(12.0, 0.0, 28.0, 20.0)).is_none());
        let doc = cropped_doc(pic);
        assert!(matches!(trim_framed_pictures(&doc, None), Cow::Borrowed(_)));
        let pic = picture(1);
        assert!(crop_picture(&pic, Rect::new(f64::NAN, 0.0, 28.0, 20.0)).is_none());
        assert_eq!(
            pixel_window(Rect::new(0.0, 0.0, 0.0, 20.0), 4, 2, Rect::ZERO),
            None
        );
    }
}
