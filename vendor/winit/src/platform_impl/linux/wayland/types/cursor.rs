use cursor_icon::CursorIcon;

use sctk::reexports::client::protocol::wl_shm::Format;
use sctk::shm::slot::{Buffer, SlotPool};

use crate::cursor::CursorImage;

#[derive(Debug)]
pub enum SelectedCursor {
    Named(CursorIcon),
    Custom(CustomCursor),
}

impl Default for SelectedCursor {
    fn default() -> Self {
        Self::Named(Default::default())
    }
}

#[derive(Debug)]
pub struct CustomCursor {
    pub buffer: Buffer,
    pub w: i32,
    pub h: i32,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
}

impl CustomCursor {
    pub(crate) fn new(pool: &mut SlotPool, image: &CursorImage) -> Self {
        // TraceDraw: the buffer's sides are whole multiples of any integer
        // buffer scale up to 6 (12 is a multiple of 1, 2, 3, 4 and 6), the
        // image at its top left and the rest transparent. Without a
        // viewport the cursor surface takes the output's buffer scale, and
        // a compositor ends the connection when a buffer's size is not a
        // multiple of it (wl_surface version 6, #2557).
        let (iw, ih) = (image.width as usize, image.height as usize);
        let (w, h) = (iw.div_ceil(12) * 12, ih.div_ceil(12) * 12);
        let (buffer, canvas) = pool
            .create_buffer(w as i32, h as i32, 4 * (w as i32), Format::Argb8888)
            .unwrap();

        canvas.fill(0);
        for (row, line) in image.rgba.chunks_exact(4 * iw.max(1)).enumerate().take(ih) {
            let out = &mut canvas[row * 4 * w..row * 4 * w + 4 * iw];
            for (canvas_chunk, rgba) in out.chunks_exact_mut(4).zip(line.chunks_exact(4)) {
                // Alpha in buffer is premultiplied.
                let alpha = rgba[3] as f32 / 255.;
                let r = (rgba[0] as f32 * alpha) as u32;
                let g = (rgba[1] as f32 * alpha) as u32;
                let b = (rgba[2] as f32 * alpha) as u32;
                let color = ((rgba[3] as u32) << 24) + (r << 16) + (g << 8) + b;
                let array: &mut [u8; 4] = canvas_chunk.try_into().unwrap();
                *array = color.to_le_bytes();
            }
        }

        CustomCursor {
            buffer,
            w: w as i32,
            h: h as i32,
            hotspot_x: image.hotspot_x as i32,
            hotspot_y: image.hotspot_y as i32,
        }
    }
}
