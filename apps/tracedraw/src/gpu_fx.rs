//! Drop shadows blurred on the graphics card (Options > General, "Blur
//! shadows on the graphics card"). The page is drawn on the CPU; with the
//! option on and the window drawn on a graphics card, the blur of a large
//! drop shadow shown on screen, the slowest step of drawing it, goes to
//! the card as a compute shader. The shader adds up the same pixels and
//! rounds down as the CPU does, so the screen shows exactly the pixels an
//! export (always drawn on the CPU) holds. Whatever the card cannot do (a
//! layer larger than it takes, a lost device, a driver error) falls back
//! to the CPU. See docs/behavior/gpu-effects.md.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::wgpu;

/// Pixels each workgroup works on along a row or a column.
const TILE: u32 = 256;
/// The widest radius the shader takes (the renderer asks at most 110).
const MAX_RADIUS: u32 = 128;
/// Pixels a workgroup keeps in shared memory (`cache` in the shader): a
/// tile and the widest radius either side.
const CACHE: u32 = TILE + 2 * MAX_RADIUS;

/// One pass of the box blur: each pixel of row (or column) `o` the
/// average, rounded down, of the `2r + 1` pixels about it, the edge
/// pixels repeated, as `tracedraw_render::box_blur_passes` works it out.
/// A workgroup first reads its tile and `r` pixels either side into
/// shared memory.
const SHADER: &str = r#"
struct Params { w: u32, h: u32, r: u32, horizontal: u32 }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

const TILE: u32 = 256u;
var<workgroup> cache: array<vec4<u32>, 512>;

fn unpack(v: u32) -> vec4<u32> {
    return vec4<u32>(v & 255u, (v >> 8u) & 255u, (v >> 16u) & 255u, v >> 24u);
}

fn index(o: u32, i: u32) -> u32 {
    if (p.horizontal == 1u) {
        return o * p.w + i;
    }
    return i * p.w + o;
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let inner = select(p.h, p.w, p.horizontal == 1u);
    let o = wg.y;
    let start = wg.x * TILE;
    let n = TILE + 2u * p.r;
    for (var k = li; k < n; k += TILE) {
        let j = i32(start) + i32(k) - i32(p.r);
        let c = u32(clamp(j, 0, i32(inner) - 1));
        cache[k] = unpack(src[index(o, c)]);
    }
    workgroupBarrier();
    let i = start + li;
    if (i >= inner) {
        return;
    }
    var acc = vec4<u32>(0u);
    for (var k = 0u; k <= 2u * p.r; k++) {
        acc += cache[li + k];
    }
    let q = acc / vec4<u32>(2u * p.r + 1u);
    dst[index(o, i)] = q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
}
"#;

/// The box blur of the graphics card the window is drawn on.
pub struct GpuBlur {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// Whether Options asks for it; read at every blur.
    enabled: AtomicBool,
    /// Set once the card failed: the CPU blurs from then on.
    broken: AtomicBool,
    /// One blur at a time: the bands of a page drawn at once take turns.
    turn: Mutex<()>,
}

/// Whether a device with `limits` can run the blur's compute shader: two
/// storage buffers in the compute stage, workgroups of a whole tile and
/// the shared memory a tile reads into. An adapter without compute
/// shaders (an older card on OpenGL) reports these as zero, and making
/// the shader's layout on it is an error that stops the program (#2568).
fn limits_allow(limits: &wgpu::Limits) -> bool {
    limits.max_storage_buffers_per_shader_stage >= 2
        && limits.max_compute_workgroup_size_x >= TILE
        && limits.max_compute_invocations_per_workgroup >= TILE
        && limits.max_compute_workgroup_storage_size >= CACHE * 16
        && limits.max_compute_workgroups_per_dimension >= 1
}

impl GpuBlur {
    /// The blur on `device`, `None` when the device cannot run its
    /// shader or the shader cannot be made.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Option<GpuBlur> {
        if !limits_allow(&device.limits()) {
            return None;
        }
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shadow blur"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
            let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty,
                count: None,
            };
            let storage = |read_only| wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            };
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow blur"),
                entries: &[
                    entry(
                        0,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                    ),
                    entry(1, storage(true)),
                    entry(2, storage(false)),
                ],
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow blur"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("shadow blur"),
                layout: Some(&pl),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            (pipeline, layout)
        }));
        let (pipeline, layout) = made.ok()?;
        Some(GpuBlur {
            device,
            queue,
            pipeline,
            layout,
            enabled: AtomicBool::new(false),
            broken: AtomicBool::new(false),
            turn: Mutex::new(()),
        })
    }

    /// Whether a layer `w` x `h` blurred with `radius` fits the card.
    fn fits(&self, w: u32, h: u32, radius: u32) -> bool {
        let limits = self.device.limits();
        let bytes = (w as u64) * (h as u64) * 4;
        let groups = limits.max_compute_workgroups_per_dimension;
        w > 0
            && h > 0
            && radius <= MAX_RADIUS
            && bytes <= limits.max_storage_buffer_binding_size
            && bytes <= limits.max_buffer_size
            && w.div_ceil(TILE).max(h.div_ceil(TILE)) <= groups
            && w.max(h) <= groups
    }

    /// The blur, `None` when the card did not do it.
    fn run(&self, data: &[u8], w: u32, h: u32, radius: u32, passes: u32) -> Option<Vec<u8>> {
        use wgpu::util::DeviceExt as _;
        let d = &self.device;
        let size = data.len() as u64;
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let a = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow blur a"),
            contents: data,
            usage: usage | wgpu::BufferUsages::COPY_DST,
        });
        let b = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow blur b"),
            size,
            usage,
            mapped_at_creation: false,
        });
        let params = |horizontal: u32| {
            let words = [w, h, radius, horizontal];
            let bytes: Vec<u8> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow blur params"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };
        let (ph, pv) = (params(1), params(0));
        let group = |p: &wgpu::Buffer, from: &wgpu::Buffer, to: &wgpu::Buffer| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow blur"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: p.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: from.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: to.as_entire_binding(),
                    },
                ],
            })
        };
        // Horizontal from a to b, vertical back to a.
        let (gh, gv) = (group(&ph, &a, &b), group(&pv, &b, &a));
        let read = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow blur read"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("shadow blur"),
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("shadow blur"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            for _ in 0..passes {
                pass.set_bind_group(0, &gh, &[]);
                pass.dispatch_workgroups(w.div_ceil(TILE), h, 1);
                pass.set_bind_group(0, &gv, &[]);
                pass.dispatch_workgroups(h.div_ceil(TILE), w, 1);
            }
        }
        enc.copy_buffer_to_buffer(&a, 0, &read, 0, size);
        let index = self.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r.is_ok());
        });
        d.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: Some(std::time::Duration::from_secs(5)),
        })
        .ok()?;
        if !rx.recv_timeout(std::time::Duration::from_secs(5)).ok()? {
            return None;
        }
        let out = read.get_mapped_range(..).ok()?.to_vec();
        read.unmap();
        (out.len() == data.len()).then_some(out)
    }
}

impl tracedraw_render::accel::BlurAccelerator for GpuBlur {
    fn box_blur(&self, data: &mut [u8], w: u32, h: u32, radius: u32, passes: u32) -> bool {
        if !self.enabled.load(Ordering::Relaxed)
            || self.broken.load(Ordering::Relaxed)
            || data.len() as u64 != (w as u64) * (h as u64) * 4
            || !self.fits(w, h, radius)
        {
            return false;
        }
        let _turn = self.turn.lock();
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.run(data, w, h, radius, passes)
        }));
        match done {
            Ok(Some(out)) => {
                data.copy_from_slice(&out);
                true
            }
            _ => {
                log::warn!("the graphics card could not blur a shadow; the CPU blurs from now on");
                self.broken.store(true, Ordering::Relaxed);
                false
            }
        }
    }
}

static BLUR: OnceLock<Arc<GpuBlur>> = OnceLock::new();

/// Make the blur on the graphics card the window is drawn on and hand it
/// to the renderer. Only for a window drawn on a graphics card: on the
/// software adapter the "card" is the CPU itself.
pub fn install(device: &wgpu::Device, queue: &wgpu::Queue) {
    if BLUR.get().is_some() {
        return;
    }
    let Some(blur) = GpuBlur::new(device.clone(), queue.clone()) else {
        log::info!("the graphics card cannot blur shadows; they are blurred on the CPU");
        return;
    };
    let blur = Arc::new(blur);
    if BLUR.set(blur.clone()).is_ok() {
        tracedraw_render::accel::set_blur_accelerator(Some(blur));
        log::info!("shadows can be blurred on the graphics card");
    }
}

/// Follow Options: blur on the card or not. Cheap; called every frame.
pub fn set_enabled(on: bool) {
    if let Some(b) = BLUR.get() {
        b.enabled.store(on, Ordering::Relaxed);
    }
}

/// Whether the card is there to blur shadows (the window is drawn on a
/// graphics card and the shader was made).
pub fn available() -> bool {
    BLUR.get()
        .is_some_and(|b| !b.broken.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_render::accel::BlurAccelerator as _;

    /// A device on any Vulkan, Metal or Direct3D 12 adapter, a CPU one
    /// such as lavapipe included (a test machine often has no graphics
    /// card); `None` when there is none.
    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(desc);
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        };
        let adapter = pollster::block_on(instance.request_adapter(&options)).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    /// A device that takes no storage buffers in a compute shader, as an
    /// older card on OpenGL, is left alone: no layout is asked of it, so
    /// the program starts and the CPU blurs (#2568).
    #[test]
    fn a_card_without_compute_shaders_is_not_asked() {
        assert!(limits_allow(&wgpu::Limits::default()));
        assert!(!limits_allow(&wgpu::Limits::downlevel_webgl2_defaults()));
        let none = wgpu::Limits {
            max_storage_buffers_per_shader_stage: 0,
            ..wgpu::Limits::default()
        };
        assert!(!limits_allow(&none));
        let small = wgpu::Limits {
            max_compute_workgroup_size_x: 128,
            ..wgpu::Limits::default()
        };
        assert!(!limits_allow(&small));
        // The shader's shared memory is the size checked.
        assert!(SHADER.contains(&format!("array<vec4<u32>, {CACHE}>")));
    }

    /// The same on a real device opened with such limits: no blur is
    /// made, and nothing is asked of the device that it refuses.
    #[test]
    fn a_device_opened_without_storage_buffers_gives_no_blur() {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(desc);
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("no wgpu adapter; skipped");
            return;
        };
        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_storage_buffers_per_shader_stage = 0;
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            }))
        else {
            eprintln!("no device with these limits; skipped");
            return;
        };
        let failed = std::sync::Arc::new(AtomicBool::new(false));
        let seen = failed.clone();
        device.on_uncaptured_error(std::sync::Arc::new(move |_| {
            seen.store(true, Ordering::Relaxed);
        }));
        assert!(GpuBlur::new(device.clone(), queue).is_none());
        let _ = device.poll(wgpu::PollType::Poll);
        assert!(!failed.load(Ordering::Relaxed), "the device was asked");
    }

    /// A layer like a shadow's: a soft-edged silhouette in white.
    fn layer(w: u32, h: u32) -> tiny_skia::Pixmap {
        let mut pm = tiny_skia::Pixmap::new(w, h).expect("pixmap");
        let mut pb = tiny_skia::PathBuilder::new();
        pb.push_circle(w as f32 * 0.4, h as f32 * 0.5, h as f32 * 0.3);
        pb.push_rect(
            tiny_skia::Rect::from_xywh(w as f32 * 0.55, 10.0, 60.0, h as f32 * 0.7).expect("rect"),
        );
        let path = pb.finish().expect("path");
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(255, 255, 255, 200);
        pm.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
        pm
    }

    /// The card's blur gives exactly the CPU's pixels, for the three
    /// passes of an ordinary shadow and the one wide pass of an even
    /// feather, on layers whose sides are not multiples of the tile.
    /// Skipped on a machine where wgpu finds no adapter at all.
    #[test]
    fn the_card_blurs_exactly_as_the_cpu() {
        let Some((device, queue)) = device() else {
            eprintln!("no wgpu adapter; skipped");
            return;
        };
        let blur = GpuBlur::new(device, queue).expect("shader");
        blur.enabled.store(true, Ordering::Relaxed);
        for (w, h, r, passes) in [
            (301, 257, 4, 3),
            (640, 300, 64, 3),
            (300, 513, 110, 1),
            (517, 260, 1, 3),
        ] {
            let mut cpu = layer(w, h);
            let mut gpu = cpu.clone();
            tracedraw_render::box_blur_passes(&mut cpu, r as i32, passes as usize);
            assert!(
                blur.box_blur(gpu.data_mut(), w, h, r, passes),
                "{w}x{h} r{r}"
            );
            assert!(cpu.data() == gpu.data(), "{w}x{h} r{r} x{passes} differs");
        }
    }

    /// Off in Options, too wide or the wrong size: the card declines and
    /// the layer is left as it was for the CPU.
    #[test]
    fn the_card_declines_what_it_should_not_do() {
        let Some((device, queue)) = device() else {
            return;
        };
        let blur = GpuBlur::new(device, queue).expect("shader");
        let mut pm = layer(300, 300);
        let before = pm.clone();
        assert!(!blur.box_blur(pm.data_mut(), 300, 300, 4, 3), "off");
        blur.enabled.store(true, Ordering::Relaxed);
        assert!(!blur.box_blur(pm.data_mut(), 300, 300, MAX_RADIUS + 1, 1));
        assert!(!blur.box_blur(pm.data_mut(), 300, 299, 4, 3));
        assert!(pm.data() == before.data());
    }

    /// The whole page drawn as the canvas draws it, with shadows blurred
    /// by the card and by the processor: written as PNG files to
    /// `$TRACEDRAW_COMPARE` with the time each took, and checked pixel
    /// for pixel. Run with `cargo test -p tracedraw --release --lib
    /// gpu_fx::tests::compare -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn compare_a_page_of_shadows() {
        use tracedraw_core::geometry::{Rect, Size, Vec2};
        use tracedraw_core::{Color, Document, Fill, Shape, ShapeKind};
        let Some((device, queue)) = device() else {
            eprintln!("no wgpu adapter; skipped");
            return;
        };
        let mut doc = Document::new("shadows", Size::new(210.0, 297.0));
        let layer = doc.pages[0].layers[0].id;
        let colours = [(220, 60, 60), (60, 140, 220), (240, 180, 40), (80, 180, 90)];
        for (i, (r, g, b)) in colours.into_iter().enumerate() {
            let id = doc.ids_mut().shape();
            let (x, y) = (20.0 + (i % 2) as f64 * 95.0, 30.0 + (i / 2) as f64 * 130.0);
            let kind = if i % 2 == 0 {
                ShapeKind::Rect {
                    rect: Rect::new(x, y, x + 75.0, y + 100.0),
                    radius: 8.0,
                    corners: None,
                }
            } else {
                ShapeKind::Ellipse {
                    rect: Rect::new(x, y, x + 75.0, y + 100.0),
                    arc: None,
                }
            };
            let mut s = Shape::new(id, kind);
            s.fill = Fill::Solid(Color::rgb8(r, g, b));
            s.stroke = None;
            s.shadow = Some(tracedraw_core::Shadow {
                offset: Vec2::new(4.0, -4.0),
                opacity: 0.75,
                blur: 3.0 + 2.0 * i as f64,
                color: Color::BLACK,
                ..Default::default()
            });
            doc.layer_mut(layer).expect("layer").shapes.push(s);
        }
        let page = doc.pages[0].id;
        let zoom = 150.0 / 25.4;
        let opts = tracedraw_render::RenderOptions {
            width: (210.0 * zoom) as u32,
            height: (297.0 * zoom) as u32,
            view: tracedraw_render::ViewTransform {
                zoom,
                origin_x: 0.0,
                origin_y: 297.0 * zoom,
            },
            draft: true,
            ..Default::default()
        };
        let time = |label: &str| {
            let t = std::time::Instant::now();
            let pm = tracedraw_render::render_page(&doc, page, &opts).expect("render");
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            eprintln!("{label}: {ms:.1} ms");
            pm
        };
        tracedraw_render::accel::set_blur_accelerator(None);
        let _ = time("processor (warm-up)");
        let cpu = time("processor");
        let blur = Arc::new(GpuBlur::new(device, queue).expect("shader"));
        blur.enabled.store(true, Ordering::Relaxed);
        tracedraw_render::accel::set_blur_accelerator(Some(blur));
        let _ = time("graphics card (warm-up)");
        let gpu = time("graphics card");
        tracedraw_render::accel::set_blur_accelerator(None);
        if let Ok(dir) = std::env::var("TRACEDRAW_COMPARE") {
            let _ = cpu.save_png(format!("{dir}/cpu.png"));
            let _ = gpu.save_png(format!("{dir}/gpu.png"));
        }
        let differ = cpu
            .data()
            .iter()
            .zip(gpu.data())
            .filter(|(a, b)| a != b)
            .count();
        eprintln!("bytes that differ: {differ}");
        assert_eq!(differ, 0);
    }
}
