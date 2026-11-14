//! egui's meshes rasterised in software: what a painter draws, as
//! pixels, without a window or a graphics card. The Help's screenshots
//! are made with it (`help_shots`), and the pointers the tools draw are
//! turned into the bitmaps the system shows as the pointer
//! (`ui::cursors`, #2136).

use egui::epaint::{Color32, Primitive, TextureId, Vertex};
use std::collections::HashMap;

/// A premultiplied RGBA canvas in 0..1.
pub(crate) struct Canvas {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) px: Vec<[f32; 4]>,
}

pub(crate) struct Texture {
    w: usize,
    h: usize,
    px: Vec<Color32>,
}

#[derive(Default)]
pub(crate) struct Textures(pub(crate) HashMap<TextureId, Texture>);

impl Textures {
    pub(crate) fn apply(&mut self, delta: &egui::TexturesDelta) {
        for (id, ds) in &delta.set {
            for d in ds.iter() {
                let egui::ImageData::Color(img) = &d.image;
                let [w, h] = img.size;
                match d.pos {
                    None => {
                        self.0.insert(
                            *id,
                            Texture {
                                w,
                                h,
                                px: img.pixels.clone(),
                            },
                        );
                    }
                    Some([x0, y0]) => {
                        if let Some(t) = self.0.get_mut(id) {
                            for y in 0..h {
                                for x in 0..w {
                                    if x0 + x < t.w && y0 + y < t.h {
                                        t.px[(y0 + y) * t.w + x0 + x] = img.pixels[y * w + x];
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn sample(&self, id: TextureId, u: f32, v: f32) -> [f32; 4] {
        let Some(t) = self.0.get(&id) else {
            return [1.0; 4];
        };
        let fx = (u * t.w as f32 - 0.5).clamp(0.0, (t.w - 1) as f32);
        let fy = (v * t.h as f32 - 0.5).clamp(0.0, (t.h - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(t.w - 1), (y0 + 1).min(t.h - 1));
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let c = |x: usize, y: usize| t.px[y * t.w + x].to_array().map(|b| b as f32 / 255.0);
        let (a, b, cc, d) = (c(x0, y0), c(x1, y0), c(x0, y1), c(x1, y1));
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * ax;
            let bot = cc[i] + (d[i] - cc[i]) * ax;
            out[i] = top + (bot - top) * ay;
        }
        out
    }
}

impl Canvas {
    pub(crate) fn new(w: usize, h: usize) -> Self {
        Canvas {
            w,
            h,
            px: vec![[0.0; 4]; w * h],
        }
    }

    fn triangle(&mut self, tex: &Textures, id: TextureId, v: [&Vertex; 3], clip: egui::Rect) {
        let (p0, p1, p2) = (v[0].pos, v[1].pos, v[2].pos);
        let area = (p1.x - p0.x) * (p2.y - p0.y) - (p2.x - p0.x) * (p1.y - p0.y);
        if area.abs() < 1e-6 {
            return;
        }
        let minx = p0.x.min(p1.x).min(p2.x).max(clip.min.x).max(0.0).floor() as i64;
        let maxx =
            p0.x.max(p1.x)
                .max(p2.x)
                .min(clip.max.x)
                .min(self.w as f32)
                .ceil() as i64;
        let miny = p0.y.min(p1.y).min(p2.y).max(clip.min.y).max(0.0).floor() as i64;
        let maxy =
            p0.y.max(p1.y)
                .max(p2.y)
                .min(clip.max.y)
                .min(self.h as f32)
                .ceil() as i64;
        let col = v.map(|x| x.color.to_array().map(|b| b as f32 / 255.0));
        for y in miny..maxy {
            for x in minx..maxx {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                if px < clip.min.x || px > clip.max.x || py < clip.min.y || py > clip.max.y {
                    continue;
                }
                let w0 = ((p1.x - px) * (p2.y - py) - (p2.x - px) * (p1.y - py)) / area;
                let w1 = ((p2.x - px) * (p0.y - py) - (p0.x - px) * (p2.y - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                let vv = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                let t = tex.sample(id, u, vv);
                let mut src = [0.0; 4];
                for i in 0..4 {
                    src[i] = (w0 * col[0][i] + w1 * col[1][i] + w2 * col[2][i]) * t[i];
                }
                let d = &mut self.px[y as usize * self.w + x as usize];
                for i in 0..4 {
                    d[i] = src[i] + d[i] * (1.0 - src[3]);
                }
            }
        }
    }

    pub(crate) fn paint(&mut self, tex: &Textures, prims: &[egui::ClippedPrimitive]) {
        for p in prims {
            if let Primitive::Mesh(m) = &p.primitive {
                for tri in m.indices.chunks_exact(3) {
                    let v = [
                        &m.vertices[tri[0] as usize],
                        &m.vertices[tri[1] as usize],
                        &m.vertices[tri[2] as usize],
                    ];
                    self.triangle(tex, m.texture_id, v, p.clip_rect);
                }
            }
        }
    }
}

/// What `body` paints with `ctx` on a `w` x `h` point area at `ppp`
/// device pixels per point, as device pixels. `ctx` is one kept for
/// this alone: the frame it runs shows nothing else.
pub(crate) fn rasterise(
    ctx: &egui::Context,
    w: usize,
    h: usize,
    ppp: f32,
    body: impl FnOnce(&egui::Painter),
) -> Canvas {
    // (The scale comes with the frame's input; a context used before
    // keeps none of its own.)
    ctx.set_zoom_factor(1.0);
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w as f32, h as f32),
        )),
        ..Default::default()
    };
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .native_pixels_per_point = Some(ppp);
    let mut body = Some(body);
    let mut out = ctx.run_ui(input, |ui| {
        if let Some(b) = body.take() {
            b(ui.painter());
        }
    });
    let mut tex = Textures::default();
    tex.apply(&out.textures_delta);
    out.textures_delta.clear();
    let shapes = std::mem::take(&mut out.shapes);
    let prims = ctx.tessellate(shapes, out.pixels_per_point);
    // Device pixels: the meshes are in points.
    let scale = out.pixels_per_point;
    let prims: Vec<egui::ClippedPrimitive> = prims
        .into_iter()
        .map(|mut p| {
            if let Primitive::Mesh(m) = &mut p.primitive {
                for v in &mut m.vertices {
                    v.pos = egui::pos2(v.pos.x * scale, v.pos.y * scale);
                }
            }
            p.clip_rect = egui::Rect::from_min_max(
                (p.clip_rect.min.to_vec2() * scale).to_pos2(),
                (p.clip_rect.max.to_vec2() * scale).to_pos2(),
            );
            p
        })
        .collect();
    let mut canvas = Canvas::new(
        (w as f32 * scale).round() as usize,
        (h as f32 * scale).round() as usize,
    );
    canvas.paint(&tex, &prims);
    canvas
}
