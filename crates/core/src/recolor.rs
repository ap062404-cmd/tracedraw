//! Recolor: every colour of the objects chosen (fills, gradient stops,
//! mesh nodes, outlines, text, table cells, effects' colours, shadows)
//! turned round the hue circle, made more or less saturated, lighter or
//! darker, or tinted with one colour, keeping how light each colour is
//! next to the others, so the shading of a drawing (its highlights and
//! shadows) stays while its colours change.
//!
//! The work is done in CIE LCh (the polar form of L*a*b*): a colour's
//! lightness L, chroma C and hue h.
//!
//! - Hue: h + `hue` degrees.
//! - Saturation: C x (1 + `saturation` / 100); -100 gives grey.
//! - Lightness: L moves `lightness` percent of the way to white (above
//!   0) or to black (below 0), so the order of the colours' lightness
//!   stays.
//! - Tint (`tint`): h and C become those of the tint colour, L stays the
//!   colour's own, so a drawing shaded in one colour comes out shaded the
//!   same way in another.
//!
//! A colour outside the screen's gamut after the change keeps its
//! lightness and hue and loses chroma until it fits. Each colour comes
//! back in its own colour model (a CMYK colour stays CMYK, through the
//! colour management like Convert Colors); a grey that gains a hue
//! becomes RGB. Registration colour never changes; spot colours stay
//! unless `spots`, and then become process colours. Bitmaps' pixels are
//! left as they are. See docs/behavior/recolor.md.

use crate::color::{lab_to_rgb, rgb_to_lab};
use crate::{Color, Document, ShapeId};
use serde::{Deserialize, Serialize};

/// What Recolor does to each colour.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Recolor {
    /// Degrees round the hue circle, -180 to 180.
    #[serde(default)]
    pub hue: f32,
    /// Percent, -100 (grey) to 100 (twice the chroma).
    #[serde(default)]
    pub saturation: f32,
    /// Percent, -100 (black) to 100 (white).
    #[serde(default)]
    pub lightness: f32,
    /// Every colour takes this colour's hue and chroma, keeping its own
    /// lightness; applied before the three changes above.
    #[serde(default)]
    pub tint: Option<Color>,
    /// Spot colours change too (as process colours).
    #[serde(default)]
    pub spots: bool,
}

/// Chroma under which a colour counts as grey (no hue to turn).
const GREY_CHROMA: f32 = 0.5;

impl Recolor {
    /// Nothing would change.
    pub fn is_identity(&self) -> bool {
        self.tint.is_none()
            && self.hue.abs() < 1e-3
            && self.saturation.abs() < 1e-3
            && self.lightness.abs() < 1e-3
    }

    /// `c` recoloured.
    pub fn apply(&self, c: Color) -> Color {
        if self.is_identity() {
            return c;
        }
        let source = match c {
            Color::Registration => return c,
            Color::Spot { .. } if !self.spots => return c,
            Color::Spot { .. } => c.process(),
            _ => c,
        };
        let [r, g, b] = source.to_rgb_f32();
        let (l0, a0, b0) = rgb_to_lab(r, g, b);
        let (mut l, mut chroma, mut h) = (l0, a0.hypot(b0), b0.atan2(a0).to_degrees());
        if let Some(t) = self.tint {
            let [tr, tg, tb] = t.process().to_rgb_f32();
            let (_, ta, tb) = rgb_to_lab(tr, tg, tb);
            chroma = ta.hypot(tb);
            h = tb.atan2(ta).to_degrees();
        }
        let grey = chroma < GREY_CHROMA;
        h += self.hue.clamp(-180.0, 180.0);
        chroma *= 1.0 + self.saturation.clamp(-100.0, 100.0) / 100.0;
        let k = self.lightness.clamp(-100.0, 100.0) / 100.0;
        if k > 0.0 {
            l += (100.0 - l) * k;
        } else {
            l += l * k;
        }
        // A grey turned round the hue circle alone stays exactly as it was
        // (no drift through the conversions).
        if grey && self.tint.is_none() && self.saturation.abs() < 1e-3 && k == 0.0 {
            return c;
        }
        let rgb = in_gamut(l, chroma.max(0.0), h);
        let out = Color::Rgb {
            r: rgb[0],
            g: rgb[1],
            b: rgb[2],
        };
        let (_, oa, ob) = rgb_to_lab(rgb[0], rgb[1], rgb[2]);
        match source {
            Color::Rgb { .. } => out,
            // A grey stays grey unless it took a hue.
            Color::Gray { .. } if oa.hypot(ob) < GREY_CHROMA => out.convert_to("Gray"),
            Color::Gray { .. } => out,
            other => out.convert_to(other.model_name()),
        }
    }
}

/// The screen colour of lightness `l`, chroma `c` and hue `h` (degrees),
/// with the chroma lowered until it lies inside the RGB gamut, so the
/// lightness and the hue stay.
fn in_gamut(l: f32, c: f32, h: f32) -> [f32; 3] {
    let l = l.clamp(0.0, 100.0);
    let (s, co) = h.to_radians().sin_cos();
    let fits = |c: f32| {
        let (a, b) = (c * co, c * s);
        let rgb = lab_to_rgb(l, a, b);
        let (l2, a2, b2) = rgb_to_lab(rgb[0], rgb[1], rgb[2]);
        let d = ((l - l2).powi(2) + (a - a2).powi(2) + (b - b2).powi(2)).sqrt();
        (d < 0.5, rgb)
    };
    let (ok, rgb) = fits(c);
    if ok {
        return rgb;
    }
    let (mut lo, mut hi) = (0.0f32, c);
    let mut best = fits(0.0).1;
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        let (ok, rgb) = fits(mid);
        if ok {
            lo = mid;
            best = rgb;
        } else {
            hi = mid;
        }
    }
    best
}

/// Recolour every colour of `shapes` (any depth: the objects in groups and
/// clip frames too) by `r`; the number of colours changed.
pub fn recolor_document(doc: &mut Document, shapes: &[ShapeId], r: &Recolor) -> usize {
    if r.is_identity() {
        return 0;
    }
    let f = |c: Color| r.apply(c);
    let mut n = 0;
    for id in shapes {
        if let Some(s) = doc.find_shape_mut(*id) {
            n += crate::color_convert::map_shape_colors(s, &f);
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ShapeKind;
    use crate::geometry::{Rect, Size};
    use crate::style::{Fill, Fountain, FountainKind, Stroke};
    use crate::Shape;

    fn lab(c: Color) -> (f32, f32, f32) {
        let [r, g, b] = c.to_rgb_f32();
        rgb_to_lab(r, g, b)
    }

    fn hue(c: Color) -> f32 {
        let (_, a, b) = lab(c);
        b.atan2(a).to_degrees().rem_euclid(360.0)
    }

    fn rect(id: u64) -> Shape {
        Shape::new(
            ShapeId(id),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            },
        )
    }

    #[test]
    fn nothing_asked_changes_nothing() {
        let r = Recolor::default();
        assert!(r.is_identity());
        let c = Color::cmyk_pct(10.0, 20.0, 30.0, 40.0);
        assert_eq!(r.apply(c), c);
    }

    #[test]
    fn hue_turns_and_lightness_stays() {
        let r = Recolor {
            hue: 120.0,
            ..Default::default()
        };
        let red = Color::rgb8(200, 40, 40);
        let out = r.apply(red);
        let (l0, ..) = lab(red);
        let (l1, ..) = lab(out);
        assert!((l0 - l1).abs() < 1.0, "{l0} {l1}");
        let turned = (hue(out) - hue(red)).rem_euclid(360.0);
        assert!((turned - 120.0).abs() < 2.0, "{turned}");
    }

    #[test]
    fn tint_keeps_each_colours_lightness_so_shading_stays() {
        let r = Recolor {
            tint: Some(Color::rgb8(30, 90, 220)),
            ..Default::default()
        };
        let light = Color::rgb8(250, 200, 200);
        let mid = Color::rgb8(200, 60, 60);
        let dark = Color::rgb8(90, 10, 10);
        let outs: Vec<Color> = [light, mid, dark].iter().map(|c| r.apply(*c)).collect();
        for (a, b) in [light, mid, dark].iter().zip(&outs) {
            assert!((lab(*a).0 - lab(*b).0).abs() < 1.0);
        }
        // All blue now, light to dark in the same order.
        let target = hue(Color::rgb8(30, 90, 220));
        for o in &outs {
            let d = (hue(*o) - target).rem_euclid(360.0);
            assert!(!(4.0..=356.0).contains(&d), "{d}");
        }
        assert!(lab(outs[0]).0 > lab(outs[1]).0 && lab(outs[1]).0 > lab(outs[2]).0);
    }

    #[test]
    fn saturation_minus_100_gives_grey_and_lightness_moves_toward_white() {
        let grey = Recolor {
            saturation: -100.0,
            ..Default::default()
        }
        .apply(Color::rgb8(40, 160, 60));
        let (_, a, b) = lab(grey);
        assert!(a.hypot(b) < 1.0);
        let c = Color::rgb8(40, 80, 120);
        let lighter = Recolor {
            lightness: 50.0,
            ..Default::default()
        }
        .apply(c);
        let (l0, l1) = (lab(c).0, lab(lighter).0);
        assert!((l1 - (l0 + (100.0 - l0) * 0.5)).abs() < 1.5, "{l0} {l1}");
    }

    #[test]
    fn models_stay_registration_and_spots_stay_greys_keep_still() {
        let r = Recolor {
            hue: 90.0,
            ..Default::default()
        };
        assert_eq!(
            r.apply(Color::cmyk_pct(0.0, 90.0, 80.0, 0.0)).model_name(),
            "CMYK"
        );
        assert_eq!(r.apply(Color::Registration), Color::Registration);
        let spot = Color::Spot {
            ink: 1,
            tint: 1.0,
            c: 0.0,
            m: 1.0,
            y: 1.0,
            k: 0.0,
        };
        assert_eq!(r.apply(spot), spot);
        let spots = Recolor { spots: true, ..r };
        assert!(!matches!(spots.apply(spot), Color::Spot { .. }));
        let g = Color::Gray { v: 0.4 };
        assert_eq!(r.apply(g), g);
        let white = Color::rgb8(255, 255, 255);
        assert_eq!(r.apply(white), white);
    }

    #[test]
    fn out_of_gamut_keeps_lightness_and_hue() {
        let r = Recolor {
            saturation: 100.0,
            ..Default::default()
        };
        let c = Color::rgb8(230, 30, 30);
        let out = r.apply(c);
        assert!((lab(c).0 - lab(out).0).abs() < 1.5);
        let d = (hue(out) - hue(c)).rem_euclid(360.0);
        assert!(!(4.0..=356.0).contains(&d), "{d}");
    }

    #[test]
    fn every_colour_of_the_objects_and_their_groups_changes() {
        let mut doc = Document::new("t", Size::new(100.0, 100.0));
        let mut a = rect(1);
        a.fill = Fill::Fountain(Fountain::two(
            FountainKind::Linear,
            Color::rgb8(255, 0, 0),
            Color::rgb8(0, 0, 255),
            0.0,
        ));
        a.stroke = Some(Stroke::new(Color::rgb8(0, 120, 0), 0.2));
        let mut b = rect(2);
        b.fill = Fill::Solid(Color::rgb8(250, 200, 0));
        let mut group = Shape::new(ShapeId(3), ShapeKind::Group { children: vec![b] });
        group.fill = Fill::None;
        group.stroke = None;
        let mut other = rect(4);
        other.fill = Fill::Solid(Color::rgb8(250, 200, 0));
        let layer = &mut doc.pages[0].layers[0];
        layer.shapes.push(a);
        layer.shapes.push(group);
        layer.shapes.push(other);
        let r = Recolor {
            hue: 60.0,
            ..Default::default()
        };
        let n = recolor_document(&mut doc, &[ShapeId(1), ShapeId(3)], &r);
        assert_eq!(n, 4);
        let fill = |id: u64| doc.find_shape(ShapeId(id)).map(|s| s.fill.clone());
        assert_ne!(fill(2), Some(Fill::Solid(Color::rgb8(250, 200, 0))));
        assert_eq!(fill(4), Some(Fill::Solid(Color::rgb8(250, 200, 0))));
    }
}
