# Graphics card and processor

Tools > Options > TraceDraw > General, under "Use hardware acceleration".

## What each one does

- **The graphics card** (wgpu: Direct3D 12, Vulkan or Metal, with OpenGL
  and the system's software adapter as fallbacks, see
  `about-and-startup.md`) shows the window: the menus, panels and the
  drawing's picture, and while you zoom or scroll it moves and scales the
  last picture until the new one is ready.
- **The processor** draws the page itself (`crates/render`, tiny-skia):
  fills, outlines, drop shadows, blurs, lenses, transparencies, bitmap
  effects, tracing, the full-screen preview, page thumbnails and every
  export and print.

The drawing is worked out on the processor so it is the same on every
computer, whatever its card or driver, and so the screen shows exactly
what an export or a print holds. Options says so under the graphics
card choice (`options.gpu_explain`).

## Blur shadows on the graphics card

Off by default; can be turned on while hardware acceleration is on, and
takes effect at once (no restart).

- With it on and the window drawn on a graphics card (not the software
  adapter, which is the processor itself), the blur of a drop shadow on
  screen whose layer has at least 65,536 pixels (`accel::MIN_PIXELS`,
  256 x 256) is worked out by a compute shader (`apps/tracedraw/src/gpu_fx.rs`).
  Smaller shadows stay on the processor: sending them to the card and
  back costs more than blurring them.
- The shader does the same box blur as the processor: each pass, for
  every pixel, adds the `2r + 1` pixels about it along the row (then the
  column), the edge pixel repeated past the edge, and divides rounding
  down; three passes of radius `r` (at most 64) for a shadow, one pass of
  `1.7 r` (at most 110) for an even ("Average") feather. The result is
  the same pixels, byte for byte (`gpu_fx::tests::the_card_blurs_exactly_as_the_cpu`).
- Only renders shown on screen use it (`RenderOptions::draft`): the
  canvas, its backdrop, object thumbnails and the tray. Exports,
  printing, Convert to Bitmap and the `.cdr` shadow pictures always blur
  on the processor.
- The processor is the fallback: a layer larger than the card's storage
  buffers, a radius past 128, a lost device or a driver error. After one
  failure the processor blurs until the next start, and the log says so.
- A card that cannot run the shader is never asked to: the device's
  limits are read first (two storage buffers in a compute shader,
  workgroups of 256, the shader's shared memory), and when they fall
  short, as on an older card drawn through OpenGL, no shader is made,
  the processor blurs and Options says the blur is not available
  (`gpu_fx::tests::a_card_without_compute_shaders_is_not_asked`, #2568).
- When the option is on but the window is not drawn on a graphics card,
  Options says it is not available now (`options.gpu_effects_unavailable`).
