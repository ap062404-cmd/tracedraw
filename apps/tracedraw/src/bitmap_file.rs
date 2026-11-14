//! The file of one picture (#1834): Save Bitmap As writes a picture as
//! PNG, JPEG or TIFF, and Edit Bitmap hands it to another program and
//! reads it back.
//!
//! A drawing keeps a picture's pixels as a PNG, RGB for the screen, and
//! for a print picture (a `.cdr` CMYK image, or a picture in the CMYK
//! colour mode) also its mode and the ink channels it prints from
//! (`Shape::picture_inks`, #1659). A CMYK picture saved as JPEG or TIFF
//! is written in CMYK from those inks (or its pixels through the colour
//! management when it keeps none), with the CMYK writers of the
//! JPEG and TIFF export (#1763); PNG holds no CMYK and is written as the
//! drawing keeps it. A CMYK TIFF read back gives the picture its inks
//! again.

use tracedraw_core::document::ShapeKind;
use tracedraw_core::picture_inks::PictureInks;
use tracedraw_core::{Color, Document, Shape};

/// The kinds of file a picture is written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitmapFile {
    Png,
    Jpeg,
    Tiff,
}

impl BitmapFile {
    /// In the order the save dialog lists them.
    pub const ALL: [BitmapFile; 3] = [BitmapFile::Png, BitmapFile::Jpeg, BitmapFile::Tiff];

    /// The file name extension written.
    pub fn ext(self) -> &'static str {
        match self {
            BitmapFile::Png => "png",
            BitmapFile::Jpeg => "jpg",
            BitmapFile::Tiff => "tif",
        }
    }

    /// The extensions the save dialog's filter takes.
    pub fn exts(self) -> &'static [&'static str] {
        match self {
            BitmapFile::Png => &["png"],
            BitmapFile::Jpeg => &["jpg", "jpeg"],
            BitmapFile::Tiff => &["tif", "tiff"],
        }
    }

    /// The name of the save dialog's filter (a format name).
    pub fn filter_name(self) -> &'static str {
        match self {
            BitmapFile::Png => "PNG",
            BitmapFile::Jpeg => "JPEG",
            BitmapFile::Tiff => "TIFF",
        }
    }

    /// The kind a file name's extension names, if one of these.
    pub fn from_name(name: &str) -> Option<BitmapFile> {
        let ext = std::path::Path::new(name)
            .extension()?
            .to_str()?
            .to_ascii_lowercase();
        BitmapFile::ALL
            .into_iter()
            .find(|k| k.exts().contains(&ext.as_str()))
    }
}

/// Whether `s` is a picture in the CMYK colour mode or keeping ink
/// channels that fit its pixels.
pub fn is_cmyk(s: &Shape) -> bool {
    let ShapeKind::Bitmap { png, .. } = &s.kind else {
        return false;
    };
    s.cmyk_bitmap
        || s.picture_inks
            .as_deref()
            .is_some_and(|i| i.fits(png.as_slice()))
}

/// The kind of file a picture is saved as unless the user picks another:
/// the kind of the file it is linked to or was imported from (its
/// name), else TIFF for a CMYK picture, which keeps its inks without
/// loss, else PNG.
pub fn own_kind(s: &Shape) -> BitmapFile {
    let link = s
        .data
        .iter()
        .find(|(k, _)| k == "bitmap.link")
        .map(|(_, v)| v.as_str());
    link.and_then(BitmapFile::from_name)
        .or_else(|| s.name.as_deref().and_then(BitmapFile::from_name))
        .unwrap_or(if is_cmyk(s) {
            BitmapFile::Tiff
        } else {
            BitmapFile::Png
        })
}

/// The picture's resolution in dots per inch, from its pixel width and
/// its width on the page.
fn dpi(s: &Shape) -> f64 {
    let ShapeKind::Bitmap { rect, width_px, .. } = &s.kind else {
        return 0.0;
    };
    let c = s.transform.as_coeffs();
    let mm = rect.width().abs() * c[0].hypot(c[1]);
    if mm > 0.0 && mm.is_finite() {
        f64::from(*width_px) * 25.4 / mm
    } else {
        0.0
    }
}

/// JPEG quality of a saved picture.
const QUALITY: u8 = 92;

/// The file of picture `s` of `doc` as `kind`; `icc` is the CMYK
/// profile a CMYK file carries.
pub fn write(
    doc: &Document,
    s: &Shape,
    kind: BitmapFile,
    icc: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let ShapeKind::Bitmap { png, .. } = &s.kind else {
        return Err("not a bitmap".into());
    };
    let dpi = dpi(s);
    if kind == BitmapFile::Png {
        return Ok(png.to_vec());
    }
    if is_cmyk(s) {
        let spots: Vec<(u32, [f32; 4])> = doc.inks.iter().map(|i| (i.id, i.cmyk)).collect();
        let cmyk = |alpha| {
            crate::picture_plates::picture_cmyk(s, &spots, alpha)
                .ok_or("the picture cannot be read")
        };
        let out = match kind {
            BitmapFile::Jpeg => {
                // No transparency in a JPEG: the inks over white paper.
                let (w, h, ink) = cmyk(false)?;
                tracedraw_io::cmyk_bitmap::cmyk_jpeg(w, h, &ink, QUALITY, dpi, icc)
            }
            _ => {
                // A TIFF keeps the transparency as an alpha channel
                // beside the inks (#1925); an opaque picture stays four
                // channels.
                let (w, h, inka) = cmyk(true)?;
                if inka.chunks_exact(5).all(|p| p[4] == 255) {
                    let ink: Vec<u8> = inka
                        .chunks_exact(5)
                        .flat_map(|p| [p[0], p[1], p[2], p[3]])
                        .collect();
                    tracedraw_io::cmyk_bitmap::cmyk_tiff(w, h, &ink, dpi, icc)
                } else {
                    tracedraw_io::cmyk_bitmap::cmyka_tiff(w, h, &inka, dpi, icc)
                }
            }
        };
        return out.ok_or_else(|| "CMYK encoding failed".to_string());
    }
    let img = crate::bitmap_fx::decode(png).ok_or("the picture cannot be read")?;
    let gray = crate::bitmap_fx::png_is_gray(png);
    let mut out = std::io::Cursor::new(Vec::new());
    match kind {
        BitmapFile::Jpeg => {
            // No transparency in a JPEG: the picture over white paper.
            let flat = image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
                let p = img.get_pixel(x, y).0;
                let a = u16::from(p[3]);
                image::Rgb(
                    [p[0], p[1], p[2]]
                        .map(|v| ((u16::from(v) * a + 255 * (255 - a) + 127) / 255) as u8),
                )
            });
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY);
            if gray {
                image::DynamicImage::ImageRgb8(flat)
                    .to_luma8()
                    .write_with_encoder(enc)
            } else {
                flat.write_with_encoder(enc)
            }
            .map_err(|e| e.to_string())?;
            Ok(crate::export::record_dpi(out.into_inner(), "jpg", dpi))
        }
        _ => {
            let opaque = img.pixels().all(|p| p[3] == 255);
            let dynamic = image::DynamicImage::ImageRgba8(img);
            let dynamic = match (gray, opaque) {
                (true, true) => image::DynamicImage::ImageLuma8(dynamic.to_luma8()),
                (true, false) => image::DynamicImage::ImageLumaA8(dynamic.to_luma_alpha8()),
                (false, true) => image::DynamicImage::ImageRgb8(dynamic.to_rgb8()),
                (false, false) => dynamic,
            };
            dynamic
                .write_to(&mut out, image::ImageFormat::Tiff)
                .map_err(|e| e.to_string())?;
            Ok(out.into_inner())
        }
    }
}

/// A picture read from a file: its pixels and, from a CMYK TIFF, its
/// inks.
pub struct ReadPicture {
    pub rgba: image::RgbaImage,
    /// The ink channels (cyan, magenta, yellow, black) as the grey PNG
    /// `PictureInks` keeps, when the file is CMYK.
    pub planes: Option<Vec<u8>>,
}

impl ReadPicture {
    /// The pixels as the PNG a bitmap keeps, and the ink channels that
    /// belong to it.
    pub fn into_png(self) -> Option<(Vec<u8>, Option<Box<PictureInks>>)> {
        let png = crate::bitmap_fx::encode(&self.rgba)?;
        let inks = self
            .planes
            .map(|p| Box::new(PictureInks::cmyk(&png, p.into())));
        Some((png, inks))
    }
}

/// Read a picture file: a CMYK TIFF (8 bits per ink, with or without
/// alpha) or a CMYK JPEG keeps its inks, and its pixels show them
/// through the colour management, as a CMYK colour of the drawing
/// shows (#2163); every other file is read as pixels.
pub fn read(bytes: &[u8]) -> Result<ReadPicture, String> {
    if let Some(p) = read_cmyk_tiff(bytes) {
        return Ok(p);
    }
    if let Some(p) = read_cmyk_jpeg(bytes) {
        return Ok(p);
    }
    let rgba = crate::files::decode_picture(bytes)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    Ok(ReadPicture { rgba, planes: None })
}

fn read_cmyk_tiff(bytes: &[u8]) -> Option<ReadPicture> {
    if !(bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")) {
        return None;
    }
    // Print-size pictures too (#2000): the reader's own guard is 256 MiB.
    let limit = usize::try_from(crate::files::picture_decode_limit()).unwrap_or(usize::MAX);
    let mut limits = tiff::decoder::Limits::default();
    limits.decoding_buffer_size = limits.decoding_buffer_size.max(limit);
    limits.intermediate_buffer_size = limits.intermediate_buffer_size.max(limit);
    let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes))
        .ok()?
        .with_limits(limits);
    let step = match dec.colortype().ok()? {
        tiff::ColorType::CMYK(8) => 4,
        tiff::ColorType::CMYKA(8) => 5,
        _ => return None,
    };
    let (w, h) = dec.dimensions().ok()?;
    let tiff::decoder::DecodingResult::U8(data) = dec.read_image().ok()? else {
        return None;
    };
    let n = (w as usize).checked_mul(h as usize)?;
    if data.len() < n.checked_mul(step)? {
        return None;
    }
    let px = data.chunks_exact(step).take(n).map(|px| {
        let a = if step == 5 { px[4] } else { 255 };
        ([px[0], px[1], px[2], px[3]], a)
    });
    let (rgba, planes) = inks_to_picture(w, h, px)?;
    picture_with_planes(rgba, planes)
}

/// The pixels of `w` by `h` ink coverages (cyan, magenta, yellow,
/// black, 0 = no ink) with their alpha, row by row: the colours they
/// show through the colour management, and the four ink planes one
/// under the other (the layout `PictureInks` keeps).
fn inks_to_picture(
    w: u32,
    h: u32,
    px: impl Iterator<Item = ([u8; 4], u8)>,
) -> Option<(image::RgbaImage, image::GrayImage)> {
    let n = (w as usize).checked_mul(h as usize)?;
    let mut planes = vec![0u8; n.checked_mul(4)?];
    let mut rgba = Vec::with_capacity(n * 4);
    let mut known: std::collections::HashMap<[u8; 4], [u8; 3]> = std::collections::HashMap::new();
    for (i, (ink, a)) in px.take(n).enumerate() {
        for (k, v) in ink.iter().enumerate() {
            if let Some(p) = planes.get_mut(k * n + i) {
                *p = *v;
            }
        }
        let rgb = *known.entry(ink).or_insert_with(|| {
            let f = |v: u8| f32::from(v) / 255.0;
            Color::Cmyk {
                c: f(ink[0]),
                m: f(ink[1]),
                y: f(ink[2]),
                k: f(ink[3]),
            }
            .to_rgb8()
        });
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
    }
    let rgba = image::RgbaImage::from_raw(w, h, rgba)?;
    let grey = image::GrayImage::from_raw(w, h.checked_mul(4)?, planes)?;
    Some((rgba, grey))
}

/// A read picture from its pixels and its ink planes (as
/// [`inks_to_picture`] lays them out).
fn picture_with_planes(rgba: image::RgbaImage, planes: image::GrayImage) -> Option<ReadPicture> {
    let mut png = std::io::Cursor::new(Vec::new());
    planes.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(ReadPicture {
        rgba,
        planes: Some(png.into_inner()),
    })
}

/// How the four channels of a JPEG file are stored, from its markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JpegInks {
    /// C, M, Y, K as they are (0 = no ink): no Adobe `APP14` marker.
    Plain,
    /// C, M, Y, K inverted (255 = no ink): an Adobe `APP14` marker with
    /// transform 0 (Adobe Technical Note 5116).
    Inverted,
    /// Y, Cb, Cr of the inverted C, M, Y, then the inverted K: an Adobe
    /// `APP14` marker with transform 2 (YCCK).
    Ycck,
}

/// The way a JPEG file with four components stores its inks, from the
/// marker segments before its first scan (ITU-T T.81 annex B: the frame
/// header's component count; Adobe Technical Note 5116: the `APP14`
/// segment, "Adobe", version, two flag words, then the transform byte).
/// None for a JPEG of another number of components, an unknown
/// transform, or something else.
fn jpeg_inks(b: &[u8]) -> Option<JpegInks> {
    if !b.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2usize;
    let mut components = None;
    let mut transform = None;
    loop {
        // Fill bytes before a marker.
        while b.get(at) == Some(&0xFF) && b.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if *b.get(at)? != 0xFF {
            return None;
        }
        let marker = *b.get(at + 1)?;
        // Markers without a length.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            at += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = usize::from(u16::from_be_bytes([*b.get(at + 2)?, *b.get(at + 3)?]));
        let body = b.get(at + 4..(at + 2).checked_add(len)?)?;
        match marker {
            // Start of frame (every coding process but DHT 0xC4, JPG 0xC8
            // and DAC 0xCC): precision, height, width, components.
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                components = body.get(5).copied();
            }
            0xEE if body.starts_with(b"Adobe") => {
                transform = Some(*body.get(11)?);
            }
            _ => {}
        }
        at += 2 + len;
    }
    if components? != 4 {
        return None;
    }
    match transform {
        None => Some(JpegInks::Plain),
        Some(0) => Some(JpegInks::Inverted),
        Some(2) => Some(JpegInks::Ycck),
        Some(_) => None,
    }
}

/// The ink coverages of one stored JPEG pixel (four channels as `how`
/// stores them).
fn jpeg_pixel_inks(px: [u8; 4], how: JpegInks) -> [u8; 4] {
    match how {
        JpegInks::Plain => px,
        JpegInks::Inverted => px.map(|v| 255 - v),
        JpegInks::Ycck => {
            // T.81 is silent on colour; the YCbCr of JFIF (ITU-T T.871):
            // the colour transform taken back gives the inverted C, M, Y.
            let y = f32::from(px[0]);
            let cb = f32::from(px[1]) - 128.0;
            let cr = f32::from(px[2]) - 128.0;
            let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            let r = byte(y + 1.402 * cr);
            let g = byte(y - 0.344_136 * cb - 0.714_136 * cr);
            let b = byte(y + 1.772 * cb);
            [255 - r, 255 - g, 255 - b, 255 - px[3]]
        }
    }
}

/// A CMYK JPEG (four components, baseline or progressive, plain,
/// inverted under an Adobe marker, or YCCK): its inks, and its pixels
/// through the colour management. The image library turns such a file
/// into RGB by itself, taking every file as inverted and leaving the
/// colour management out, so pictures came in brighter than the same
/// inks show in the drawing (#2163). Turned as its Exif orientation
/// says. None for any other file, or one too large to read.
fn read_cmyk_jpeg(bytes: &[u8]) -> Option<ReadPicture> {
    use zune_core::colorspace::ColorSpace;
    let how = jpeg_inks(bytes)?;
    let stored = if how == JpegInks::Ycck {
        ColorSpace::YCCK
    } else {
        ColorSpace::CMYK
    };
    let options = zune_core::options::DecoderOptions::default()
        .set_strict_mode(false)
        .set_max_width(usize::MAX)
        .set_max_height(usize::MAX);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(
        zune_core::bytestream::ZCursor::new(bytes),
        options,
    );
    dec.decode_headers().ok()?;
    // The decoder must see the colour space the markers say, so that
    // the channels come out as stored, with no conversion of its own.
    if dec.input_colorspace()? != stored {
        return None;
    }
    let (w, h) = dec.dimensions()?;
    // The decoded channels, the pixels and the planes side by side.
    let n = w.checked_mul(h)?;
    let need = u64::try_from(n.checked_mul(12)?).ok()?;
    if need > crate::files::picture_decode_limit() {
        return None;
    }
    dec.set_options(dec.options().jpeg_set_out_colorspace(stored));
    let data = dec.decode().ok()?;
    if data.len() < n.checked_mul(4)? {
        return None;
    }
    let (w, h) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    let px = data
        .chunks_exact(4)
        .map(|p| (jpeg_pixel_inks([p[0], p[1], p[2], p[3]], how), 255));
    let (rgba, planes) = inks_to_picture(w, h, px)?;
    let (rgba, planes) = oriented(bytes, rgba, planes)?;
    picture_with_planes(rgba, planes)
}

/// The pixels and the ink planes of picture file `bytes` turned the way
/// its Exif orientation says, as the pixels of other files are.
fn oriented(
    bytes: &[u8],
    rgba: image::RgbaImage,
    planes: image::GrayImage,
) -> Option<(image::RgbaImage, image::GrayImage)> {
    use image::ImageDecoder;
    let orientation = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .and_then(|mut d| d.orientation().ok())
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    if orientation == image::metadata::Orientation::NoTransforms {
        return Some((rgba, planes));
    }
    let (w, h) = rgba.dimensions();
    let mut pixels = image::DynamicImage::ImageRgba8(rgba);
    pixels.apply_orientation(orientation);
    let pixels = pixels.into_rgba8();
    let (tw, th) = pixels.dimensions();
    // Each plane turned alone, then laid one under the other again.
    let mut out = image::GrayImage::new(tw, th.checked_mul(4)?);
    for k in 0..4u32 {
        let plane = image::imageops::crop_imm(&planes, 0, k * h, w, h).to_image();
        let mut plane = image::DynamicImage::ImageLuma8(plane);
        plane.apply_orientation(orientation);
        image::imageops::replace(&mut out, &plane.into_luma8(), 0, i64::from(k * th));
    }
    Some((pixels, out))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::app::App;
    use tracedraw_core::geometry::Rect;
    use tracedraw_core::{Command, ShapeId};

    /// Four by two pixels: cyan ink on the left half, black on the
    /// right, as a `.cdr` CMYK image is read (pixels for the screen and
    /// its ink channels).
    fn cmyk_picture(app: &mut App) -> ShapeId {
        let rgb = |x: u32| if x < 2 { [0, 160, 230] } else { [30, 30, 30] };
        let img = image::RgbaImage::from_fn(4, 2, |x, _| {
            let [r, g, b] = rgb(x);
            image::Rgba([r, g, b, 255])
        });
        let png = crate::bitmap_fx::encode(&img).expect("png");
        // Planes C, M, Y, K one under the other.
        let planes = image::GrayImage::from_fn(4, 8, |x, y| {
            let ink = y / 2;
            image::Luma([match (ink, x < 2) {
                (0, true) | (3, false) => 255,
                _ => 0,
            }])
        });
        let mut planes_png = std::io::Cursor::new(Vec::new());
        planes
            .write_to(&mut planes_png, image::ImageFormat::Png)
            .expect("planes");
        let inks = PictureInks::cmyk(&png, planes_png.into_inner().into());
        let id = app
            .new_shape(ShapeKind::Bitmap {
                rect: Rect::new(10.0, 10.0, 10.0 + 4.0 * 25.4 / 300.0, 20.0),
                width_px: 4,
                height_px: 2,
                png: png.into(),
                fx: None,
            })
            .expect("bitmap");
        app.run(Command::SetPictureInks {
            shape: id,
            cmyk: true,
            inks: Some(Box::new(inks)),
        });
        id
    }

    fn rgb_picture(app: &mut App) -> ShapeId {
        let img = image::RgbaImage::from_fn(5, 3, |x, _| image::Rgba([200, 40 * x as u8, 10, 255]));
        app.new_shape(ShapeKind::Bitmap {
            rect: Rect::new(10.0, 10.0, 15.0, 13.0),
            width_px: 5,
            height_px: 3,
            png: crate::bitmap_fx::encode(&img).expect("png").into(),
            fx: None,
        })
        .expect("bitmap")
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tracedraw-1834-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("dir");
        d
    }

    fn planes(s: &Shape) -> Vec<u8> {
        let inks = s.picture_inks.as_deref().expect("inks");
        let ShapeKind::Bitmap { png, .. } = &s.kind else {
            panic!("not a bitmap");
        };
        assert!(inks.fits(png.as_slice()), "the inks fit the pixels");
        image::load_from_memory(&inks.png)
            .expect("planes")
            .to_luma8()
            .into_raw()
    }

    /// Save Bitmap As (#1834): a CMYK picture is offered as TIFF and
    /// keeps its ink channels as they are in a CMYK TIFF; as JPEG it is a
    /// four-component CMYK file; PNG stays the pixels as kept.
    #[test]
    fn save_bitmap_as_keeps_the_inks_of_a_cmyk_picture() {
        let mut app = App::headless();
        let id = cmyk_picture(&mut app);
        let s = app.doc().find_shape(id).expect("picture").clone();
        assert!(is_cmyk(&s));
        assert_eq!(own_kind(&s), BitmapFile::Tiff);
        let d = dir("save");

        let tif = d.join("picture.tif");
        app.save_bitmap_to(id, &tif, BitmapFile::Png).expect("tiff");
        let bytes = std::fs::read(&tif).expect("read");
        let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes)).expect("tiff");
        assert_eq!(dec.colortype().expect("type"), tiff::ColorType::CMYK(8));
        let back = read(&bytes).expect("read back");
        assert_eq!(back.planes.is_some(), true);
        let (_, inks) = back.into_png().expect("png");
        let got = image::load_from_memory(&inks.expect("inks").png)
            .expect("planes")
            .to_luma8()
            .into_raw();
        assert_eq!(got, planes(&s), "the inks as the drawing keeps them");
        assert_eq!(
            crate::image_dpi::resolution(&bytes).map(|(x, _)| x.round()),
            Some(300.0)
        );

        let jpg = d.join("picture.jpeg");
        app.save_bitmap_to(id, &jpg, BitmapFile::Png).expect("jpeg");
        let bytes = std::fs::read(&jpg).expect("read");
        let sof = bytes
            .windows(2)
            .position(|w| w == [0xFF, 0xC0])
            .expect("SOF0");
        assert_eq!(bytes[sof + 9], 4, "four components (CMYK)");
        let img = image::load_from_memory(&bytes).expect("decodes").to_rgb8();
        assert_eq!(img.dimensions(), (4, 2));
        // Cyan ink alone shows as no red, full green and blue.
        let p = img.get_pixel(0, 0).0;
        assert!(p[0] < 40 && p[1] > 215 && p[2] > 215, "cyan: {p:?}");

        let png = d.join("picture.png");
        app.save_bitmap_to(id, &png, BitmapFile::Tiff).expect("png");
        let ShapeKind::Bitmap { png: kept, .. } = &s.kind else {
            panic!("bitmap");
        };
        assert_eq!(std::fs::read(&png).expect("read"), kept.to_vec());

        // No extension: the kind asked for.
        let plain = d.join("picture");
        app.save_bitmap_to(id, &plain, BitmapFile::Tiff)
            .expect("plain");
        assert!(std::fs::read(&plain).expect("read").starts_with(b"II*\0"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #1925: a CMYK picture with transparency (here its right half is
    /// clear) keeps it in Save Bitmap As TIFF, as an alpha channel
    /// beside the full inks, and in Edit Bitmap: the file read back
    /// leaves the picture transparent where it was.
    #[test]
    fn a_cmyk_picture_keeps_its_transparency_in_tiff_and_edit_bitmap() {
        let mut app = App::headless();
        let id = cmyk_picture(&mut app);
        // The same picture with its right half (the black ink) clear.
        let s = app.doc().find_shape(id).expect("picture").clone();
        let ShapeKind::Bitmap { png, .. } = &s.kind else {
            panic!("bitmap");
        };
        let mut px = crate::bitmap_fx::decode(png).expect("pixels");
        for (x, _, p) in px.enumerate_pixels_mut() {
            if x >= 2 {
                p.0[3] = 0;
            }
        }
        let clear = crate::bitmap_fx::encode(&px).expect("png");
        let inks = PictureInks::cmyk(&clear, s.picture_inks.as_deref().expect("inks").png.clone());
        let id = app
            .new_shape(ShapeKind::Bitmap {
                rect: Rect::new(30.0, 10.0, 30.0 + 4.0 * 25.4 / 300.0, 20.0),
                width_px: 4,
                height_px: 2,
                png: clear.into(),
                fx: None,
            })
            .expect("bitmap");
        app.run(Command::SetPictureInks {
            shape: id,
            cmyk: true,
            inks: Some(Box::new(inks)),
        });
        let before = app.doc().find_shape(id).expect("picture").clone();
        let d = dir("alpha");

        let tif = d.join("clear.tif");
        app.save_bitmap_to(id, &tif, BitmapFile::Png).expect("tiff");
        let bytes = std::fs::read(&tif).expect("read");
        let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes)).expect("tiff");
        assert_eq!(dec.colortype().expect("type"), tiff::ColorType::CMYKA(8));
        let back = read(&bytes).expect("read back");
        let alpha: Vec<u8> = back.rgba.pixels().map(|p| p.0[3]).collect();
        assert_eq!(alpha, [255, 255, 0, 0, 255, 255, 0, 0], "clear half kept");
        let (_, inks) = back.into_png().expect("png");
        let got = image::load_from_memory(&inks.expect("inks").png)
            .expect("planes")
            .to_luma8()
            .into_raw();
        assert_eq!(got, planes(&before), "the full inks, not faded");

        // Edit Bitmap: hand the file over, the other program saves it as
        // it is, the picture is read back still clear on the right.
        let path = app.write_bitmap_for_editing(id, &d).expect("written");
        app.run(Command::SetObjectData {
            shape: id,
            data: vec![("bitmap.link".into(), path.display().to_string())],
        });
        let resaved = std::fs::read(&path).expect("read");
        std::fs::write(&path, resaved).expect("write");
        assert!(app.reload_linked_bitmap(id));
        let s = app.doc().find_shape(id).expect("picture");
        assert!(s.cmyk_bitmap);
        let ShapeKind::Bitmap { png, .. } = &s.kind else {
            panic!("bitmap");
        };
        let px = crate::bitmap_fx::decode(png).expect("pixels");
        let alpha: Vec<u8> = px.pixels().map(|p| p.0[3]).collect();
        assert_eq!(alpha, [255, 255, 0, 0, 255, 255, 0, 0], "still clear");
        assert_eq!(planes(s), planes(&before));

        // An opaque CMYK picture stays a four channel file.
        let tif = d.join("opaque.tif");
        let opaque = cmyk_picture(&mut app);
        app.save_bitmap_to(opaque, &tif, BitmapFile::Png)
            .expect("tiff");
        let bytes = std::fs::read(&tif).expect("read");
        let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(&bytes)).expect("tiff");
        assert_eq!(dec.colortype().expect("type"), tiff::ColorType::CMYK(8));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An RGB picture is offered as the kind of file it came from, else
    /// PNG, and is written as JPEG or TIFF when asked.
    #[test]
    fn save_bitmap_as_writes_an_rgb_picture_as_jpeg_or_tiff() {
        let mut app = App::headless();
        let id = rgb_picture(&mut app);
        let s = app.doc().find_shape(id).expect("picture").clone();
        assert!(!is_cmyk(&s));
        assert_eq!(own_kind(&s), BitmapFile::Png);
        let mut named = s.clone();
        named.name = Some("Photo.JPG".into());
        assert_eq!(own_kind(&named), BitmapFile::Jpeg);
        named
            .data
            .push(("bitmap.link".into(), "/pictures/scan.tiff".into()));
        assert_eq!(own_kind(&named), BitmapFile::Tiff, "the linked file first");

        let d = dir("rgb");
        for (name, format) in [
            ("p.jpg", image::ImageFormat::Jpeg),
            ("p.tif", image::ImageFormat::Tiff),
        ] {
            let path = d.join(name);
            app.save_bitmap_to(id, &path, BitmapFile::Png).expect(name);
            let bytes = std::fs::read(&path).expect("read");
            assert_eq!(image::guess_format(&bytes).expect("format"), format);
            let img = image::load_from_memory(&bytes).expect("decodes").to_rgb8();
            assert_eq!(img.dimensions(), (5, 3));
            let p = img.get_pixel(2, 1).0;
            assert!(
                (p[0] as i32 - 200).abs() < 8 && (p[1] as i32 - 80).abs() < 8,
                "{name}: {p:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Edit Bitmap (#1834): a CMYK picture goes to the other program as
    /// a CMYK TIFF with its inks, and the file it saves back gives the
    /// picture its new inks (and the pixels that show them); undo puts
    /// the old ones back. An RGB picture still goes as a PNG.
    #[test]
    fn edit_bitmap_hands_over_and_reads_back_the_inks() {
        let mut app = App::headless();
        let id = cmyk_picture(&mut app);
        let before = app.doc().find_shape(id).expect("picture").clone();
        let d = dir("edit");
        let path = app.write_bitmap_for_editing(id, &d).expect("written");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("tif"));
        let back = read(&std::fs::read(&path).expect("read")).expect("tiff");
        let (_, inks) = back.into_png().expect("png");
        assert_eq!(
            image::load_from_memory(&inks.expect("inks").png)
                .expect("planes")
                .to_luma8()
                .into_raw(),
            planes(&before)
        );
        app.run(Command::SetObjectData {
            shape: id,
            data: vec![("bitmap.link".into(), path.display().to_string())],
        });
        // The other program saves magenta ink everywhere.
        let magenta: Vec<u8> = (0..8).flat_map(|_| [0u8, 255, 0, 0]).collect();
        let tif = tracedraw_io::cmyk_bitmap::cmyk_tiff(4, 2, &magenta, 300.0, None).expect("tif");
        std::fs::write(&path, tif).expect("write");
        assert!(app.reload_linked_bitmap(id));
        let s = app.doc().find_shape(id).expect("picture").clone();
        assert!(s.cmyk_bitmap);
        let p = planes(&s);
        assert!(p[..8].iter().all(|v| *v == 0), "no cyan");
        assert!(p[8..16].iter().all(|v| *v == 255), "magenta");
        assert!(p[16..].iter().all(|v| *v == 0), "no yellow or black");
        let ShapeKind::Bitmap { png, .. } = &s.kind else {
            panic!("bitmap");
        };
        let px = crate::bitmap_fx::decode(png).expect("pixels");
        let c = px.get_pixel(1, 1).0;
        assert!(
            c[0] > 180 && c[1] < 120 && c[2] > 100,
            "shows magenta: {c:?}"
        );
        app.engine.undo().expect("undo");
        let s = app.doc().find_shape(id).expect("picture");
        assert_eq!(planes(s), planes(&before));

        let rgb = rgb_picture(&mut app);
        let path = app.write_bitmap_for_editing(rgb, &d).expect("written");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `jpg` with an Exif `APP1` segment right after its start, holding
    /// orientation `o` (CIPA DC-008: a TIFF header and IFD0 with tag
    /// 274).
    fn with_orientation(jpg: &[u8], o: u16) -> Vec<u8> {
        let mut tiff = b"II*\0".to_vec();
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&274u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&o.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let mut body = b"Exif\0\0".to_vec();
        body.extend(tiff);
        let mut out = jpg[..2].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        out.extend(body);
        // A file has one Exif segment: the one the writer made for the
        // resolution goes.
        let mut at = 2;
        while jpg.get(at) == Some(&0xFF)
            && jpg.get(at + 1).is_some_and(|m| (0xE0..=0xEF).contains(m))
        {
            let len = usize::from(u16::from_be_bytes([jpg[at + 2], jpg[at + 3]]));
            if !(jpg[at + 1] == 0xE1 && jpg[at + 4..].starts_with(b"Exif")) {
                out.extend_from_slice(&jpg[at..at + 2 + len]);
            }
            at += 2 + len;
        }
        out.extend_from_slice(&jpg[at..]);
        out
    }

    /// `jpg` without its Adobe `APP14` segment.
    fn without_adobe(jpg: &[u8]) -> Vec<u8> {
        let at = jpg
            .windows(2)
            .position(|w| w[0] == 0xFF && w[1] == 0xEE)
            .expect("APP14");
        let len = usize::from(u16::from_be_bytes([jpg[at + 2], jpg[at + 3]]));
        let mut out = jpg[..at].to_vec();
        out.extend_from_slice(&jpg[at + 2 + len..]);
        out
    }

    fn near(a: u8, b: u8, by: u8) -> bool {
        a.abs_diff(b) <= by
    }

    /// The ink planes of a read picture, one under the other.
    fn read_planes(p: &ReadPicture) -> image::GrayImage {
        image::load_from_memory(p.planes.as_deref().expect("inks"))
            .expect("planes")
            .to_luma8()
    }

    /// The bitmap File > Import placed from `bytes` written as `name`.
    fn imported(app: &mut App, d: &std::path::Path, name: &str, bytes: &[u8]) -> Shape {
        let path = d.join(name);
        std::fs::write(&path, bytes).expect("write");
        app.import_bitmap(&path);
        let s = app.selected_shapes();
        assert_eq!(s.len(), 1, "{name} imported: {}", app.status);
        assert!(
            matches!(s[0].kind, ShapeKind::Bitmap { .. }),
            "{name} is a bitmap"
        );
        s[0].clone()
    }

    fn pixel(s: &Shape, x: u32, y: u32) -> [u8; 4] {
        let ShapeKind::Bitmap { png, .. } = &s.kind else {
            panic!("not a bitmap");
        };
        image::load_from_memory(png.as_slice())
            .expect("pixels")
            .to_rgba8()
            .get_pixel(x, y)
            .0
    }

    fn size_px(s: &Shape) -> (u32, u32) {
        match &s.kind {
            ShapeKind::Bitmap {
                width_px,
                height_px,
                ..
            } => (*width_px, *height_px),
            _ => (0, 0),
        }
    }

    /// #2163: a CMYK JPEG (inverted under an Adobe marker, as the CMYK
    /// export writes it) comes in as a CMYK picture with its inks, its
    /// pixels the colour those inks have in the drawing, not the bright
    /// colour the image library makes of them.
    #[test]
    fn a_cmyk_jpeg_imports_with_its_inks_as_the_drawing_shows_them() {
        let d = dir("2163-import");
        let ink: Vec<u8> = (0..16 * 16).flat_map(|_| [153, 153, 0, 0]).collect();
        let jpg = tracedraw_io::cmyk_bitmap::cmyk_jpeg(16, 16, &ink, 95, 300.0, None).expect("jpg");
        let mut app = App::headless();
        let s = imported(&mut app, &d, "cmyk.jpg", &jpg);
        assert!(s.cmyk_bitmap, "a CMYK picture");
        let p = planes(&s);
        let n = 16 * 16;
        assert!(
            p[..2 * n].iter().all(|v| near(*v, 153, 3)),
            "cyan and magenta"
        );
        assert!(
            p[2 * n..].iter().all(|v| near(*v, 0, 3)),
            "no yellow or black"
        );
        let want = Color::Cmyk {
            c: 0.6,
            m: 0.6,
            y: 0.0,
            k: 0.0,
        }
        .to_rgb8();
        // What the image library shows: (255 - ink) for each colour.
        let naive = [102u8, 102, 255];
        assert!(
            (0..3).any(|i| want[i].abs_diff(naive[i]) > 20),
            "the drawing's colour is not the plain inversion: {want:?}"
        );
        let got = pixel(&s, 8, 8);
        assert!(
            (0..3).all(|i| near(got[i], want[i], 4)),
            "{got:?} shows as the drawing's C60 M60 {want:?}"
        );
        // At the resolution the file records.
        let ShapeKind::Bitmap { rect, .. } = &s.kind else {
            panic!("bitmap");
        };
        assert!(
            (rect.width() - 16.0 * 25.4 / 300.0).abs() < 1e-3,
            "{rect:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #2163: without the Adobe marker the four channels are the inks as
    /// they are; with it they are inverted.
    #[test]
    fn a_cmyk_jpeg_without_the_adobe_marker_keeps_its_inks_as_stored() {
        let ink: Vec<u8> = (0..8 * 8).flat_map(|_| [40, 200, 10, 90]).collect();
        let inverted: Vec<u8> = ink.iter().map(|v| 255 - v).collect();
        // Written inverted under the marker, then the marker taken out:
        // the stored channels are the inks.
        let jpg =
            tracedraw_io::cmyk_bitmap::cmyk_jpeg(8, 8, &inverted, 100, 72.0, None).expect("jpg");
        let plain = without_adobe(&jpg);
        assert_eq!(jpeg_inks(&plain), Some(JpegInks::Plain));
        assert_eq!(jpeg_inks(&jpg), Some(JpegInks::Inverted));
        let p = read_planes(&read(&plain).expect("read"));
        assert_eq!(p.dimensions(), (8, 32));
        for (k, want) in [40u8, 200, 10, 90].iter().enumerate() {
            let v = p.get_pixel(3, k as u32 * 8 + 3).0[0];
            assert!(near(v, *want, 2), "ink {k}: {v} for {want}");
        }
    }

    /// A progressive CMYK JPEG (made for this test, inverted under an
    /// Adobe marker): left half C60 M60, right half Y100.
    #[test]
    fn a_progressive_cmyk_jpeg_reads_with_its_inks() {
        let bytes = include_bytes!("../testdata/progressive-cmyk.jpg");
        assert_eq!(jpeg_inks(bytes), Some(JpegInks::Inverted));
        let pic = read(bytes).expect("read");
        assert_eq!(pic.rgba.dimensions(), (16, 16));
        let p = read_planes(&pic);
        let at = |k: u32, x: u32| p.get_pixel(x, k * 16 + 8).0[0];
        assert!(near(at(0, 2), 153, 4) && near(at(1, 2), 153, 4), "C60 M60");
        assert!(near(at(2, 2), 0, 4) && near(at(3, 2), 0, 4));
        assert!(near(at(2, 13), 255, 4) && near(at(0, 13), 0, 4), "Y100");
    }

    /// YCCK (Adobe transform 2): the colour transform taken back gives
    /// the inverted cyan, magenta and yellow; black is inverted.
    #[test]
    fn ycck_pixels_give_their_inks() {
        assert_eq!(
            jpeg_pixel_inks([200, 128, 128, 55], JpegInks::Ycck),
            [55, 55, 55, 200]
        );
        // Stored red (R 255, G 0, B 0): no cyan, full magenta and yellow.
        let (r, g, b) = (255.0f32, 0.0f32, 0.0f32);
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        let cb = 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b;
        let cr = 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b;
        let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        let got = jpeg_pixel_inks([byte(y), byte(cb), byte(cr), 255], JpegInks::Ycck);
        assert!(
            near(got[0], 0, 2) && near(got[1], 255, 2) && near(got[2], 255, 2),
            "{got:?}"
        );
        assert_eq!(got[3], 0);
        assert_eq!(jpeg_pixel_inks([1, 2, 3, 4], JpegInks::Plain), [1, 2, 3, 4]);
        assert_eq!(
            jpeg_pixel_inks([1, 2, 3, 4], JpegInks::Inverted),
            [254, 253, 252, 251]
        );
    }

    /// Broken or other JPEG files are no CMYK JPEG, and reading them
    /// never panics.
    #[test]
    fn broken_jpeg_markers_give_no_inks() {
        let ink: Vec<u8> = (0..8 * 8).flat_map(|_| [10, 20, 30, 40]).collect();
        let jpg = tracedraw_io::cmyk_bitmap::cmyk_jpeg(8, 8, &ink, 90, 72.0, None).expect("jpg");
        let mut rgb = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .write_to(&mut rgb, image::ImageFormat::Jpeg)
            .expect("rgb jpeg");
        assert_eq!(jpeg_inks(&rgb.into_inner()), None, "three components");
        for b in [
            &b""[..],
            &[0xFF, 0xD8],
            &[0xFF, 0xD8, 0xFF],
            &[0xFF, 0xD8, 0xFF, 0xEE, 0x00],
            &[0xFF, 0xD8, 0xFF, 0xEE, 0x00, 0x00, 0xFF, 0xDA],
            &[0xFF, 0xD8, 0xFF, 0xEE, 0x00, 0x01, 0xFF, 0xDA],
            &[0xFF, 0xD8, 0xFF, 0xEE, 0xFF, 0xFF, b'A'],
            &[0xFF, 0xD8, 0x00, 0x00],
            b"II*\0",
        ] {
            assert_eq!(jpeg_inks(b), None, "{b:?}");
            assert!(read_cmyk_jpeg(b).is_none());
        }
        // Cut short anywhere: read or refused, never a panic.
        let turned = with_orientation(&jpg, 6);
        for file in [&jpg, &turned] {
            for cut in [3, 20, 40, file.len() / 2, file.len() - 3] {
                let _ = read(&file[..cut]);
            }
        }
        // An unknown Adobe transform with four components.
        let mut odd = jpg.clone();
        let at = odd.windows(5).position(|w| w == b"Adobe").expect("Adobe");
        odd[at + 11] = 7;
        assert_eq!(jpeg_inks(&odd), None);
    }

    /// #2167: a photo whose Exif orientation says it was taken with the
    /// camera on its side comes in upright, its CMYK inks turned with it.
    #[test]
    fn an_exif_orientation_stands_the_picture_upright() {
        let d = dir("2167-orient");
        let mut app = App::headless();
        // 16 by 8: red on the left, blue on the right.
        let img = image::RgbImage::from_fn(16, 8, |x, _| {
            image::Rgb(if x < 8 { [230, 20, 20] } else { [20, 20, 230] })
        });
        let mut jpg = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 95)
            .encode_image(&img)
            .expect("jpeg");
        // 6: turned a quarter turn clockwise to stand upright.
        let photo = with_orientation(&jpg.into_inner(), 6);
        let s = imported(&mut app, &d, "photo.jpg", &photo);
        assert_eq!(size_px(&s), (8, 16));
        let top = pixel(&s, 4, 2);
        let bottom = pixel(&s, 4, 13);
        assert!(top[0] > 150 && top[2] < 100, "red on top: {top:?}");
        assert!(bottom[2] > 150 && bottom[0] < 100, "blue below: {bottom:?}");
        let ShapeKind::Bitmap { rect, .. } = &s.kind else {
            panic!("bitmap");
        };
        assert!(rect.height() > rect.width() * 1.9, "{rect:?}");
        // A CMYK JPEG: cyan on the left, black on the right.
        let ink: Vec<u8> = (0..16 * 8)
            .flat_map(|i| {
                if i % 16 < 8 {
                    [255, 0, 0, 0]
                } else {
                    [0, 0, 0, 255]
                }
            })
            .collect();
        let cmyk = tracedraw_io::cmyk_bitmap::cmyk_jpeg(16, 8, &ink, 95, 72.0, None).expect("jpg");
        let s = imported(&mut app, &d, "cmyk.jpg", &with_orientation(&cmyk, 6));
        assert_eq!(size_px(&s), (8, 16));
        let p = planes(&s);
        let at = |k: usize, x: usize, y: usize| p[k * 128 + y * 8 + x];
        assert!(at(0, 4, 2) > 200 && at(3, 4, 2) < 50, "cyan on top");
        assert!(at(3, 4, 13) > 200 && at(0, 4, 13) < 50, "black below");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #2167: File > Import reads every common kind of PNG and JPEG:
    /// 8 and 16 bits, grey, with and without alpha, a palette with a
    /// transparent entry, baseline, progressive and CMYK JPEG.
    #[test]
    fn every_common_png_and_jpeg_imports() {
        use image::DynamicImage as Dyn;
        let d = dir("2167-kinds");
        let mut app = App::headless();
        let encode = |img: Dyn, f: image::ImageFormat| {
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, f).expect("encode");
            out.into_inner()
        };
        let rgb = image::RgbImage::from_fn(6, 4, |x, _| {
            image::Rgb(if x < 3 { [230, 20, 20] } else { [20, 20, 230] })
        });
        let dynamic = Dyn::ImageRgb8(rgb);
        let mut half_clear = dynamic.to_rgba8();
        for (x, _, p) in half_clear.enumerate_pixels_mut() {
            if x >= 3 {
                p.0[3] = 0;
            }
        }
        let half_clear = Dyn::ImageRgba8(half_clear);
        let png = image::ImageFormat::Png;
        let jpeg = image::ImageFormat::Jpeg;
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("rgb8.png", encode(dynamic.clone(), png)),
            ("rgba8.png", encode(half_clear.clone(), png)),
            (
                "rgb16.png",
                encode(Dyn::ImageRgb16(dynamic.to_rgb16()), png),
            ),
            (
                "rgba16.png",
                encode(Dyn::ImageRgba16(half_clear.to_rgba16()), png),
            ),
            (
                "grey8.png",
                encode(Dyn::ImageLuma8(dynamic.to_luma8()), png),
            ),
            (
                "grey16.png",
                encode(Dyn::ImageLuma16(dynamic.to_luma16()), png),
            ),
            (
                "greya8.png",
                encode(Dyn::ImageLumaA8(half_clear.to_luma_alpha8()), png),
            ),
            ("baseline.jpg", encode(dynamic.clone(), jpeg)),
            (
                "grey.jpg",
                encode(Dyn::ImageLuma8(dynamic.to_luma8()), jpeg),
            ),
        ];
        for (name, bytes) in &cases {
            let s = imported(&mut app, &d, name, bytes);
            assert_eq!(size_px(&s), (6, 4), "{name}");
            let (left, right) = (pixel(&s, 0, 1), pixel(&s, 5, 1));
            if name.starts_with("grey") {
                assert!(left[0] > right[0] + 20, "{name}: {left:?} {right:?}");
            } else {
                assert!(left[0] > 150 && left[2] < 100, "{name}: red {left:?}");
                if !name.starts_with("rgba") {
                    assert!(right[2] > 150 && right[0] < 100, "{name}: blue {right:?}");
                }
            }
            if name.starts_with("rgba") || name.starts_with("greya") {
                assert_eq!(left[3], 255, "{name}");
                assert_eq!(right[3], 0, "{name}: clear");
            }
            assert!(!s.cmyk_bitmap, "{name} is RGB");
        }
        // Made for these tests: a palette PNG (green, a clear entry) and
        // progressive JPEG files.
        let palette = include_bytes!("../testdata/palette.png");
        let s = imported(&mut app, &d, "palette.png", palette);
        assert_eq!(size_px(&s), (8, 4));
        assert_eq!(pixel(&s, 1, 1), [0, 200, 0, 255]);
        assert_eq!(pixel(&s, 6, 1)[3], 0);
        let progressive = include_bytes!("../testdata/progressive-rgb.jpg");
        let s = imported(&mut app, &d, "progressive.jpg", progressive);
        assert_eq!(size_px(&s), (24, 16));
        let (left, right) = (pixel(&s, 2, 8), pixel(&s, 21, 8));
        assert!(left[0] > 180 && left[2] < 70, "{left:?}");
        assert!(right[2] > 180 && right[0] < 70, "{right:?}");
        let cmyk = include_bytes!("../testdata/progressive-cmyk.jpg");
        let s = imported(&mut app, &d, "progressive-cmyk.jpg", cmyk);
        assert!(s.cmyk_bitmap && s.picture_inks.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }
}
