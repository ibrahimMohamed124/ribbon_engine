//! Pure framebuffer scaling and pointer-to-image coordinate conversion.

//! CPU window presentation helpers and framebuffer coordinate conversion.

use crate::frame::Framebuffer;
pub use ribbon_platform::window::{viewport, window_to_framebuffer, Viewport};

/// Clear the destination and nearest-neighbor scale the image with centered black bars.
pub fn blit_scaled(fb: &Framebuffer, dst: &mut [u32], dst_w: u32, dst_h: u32) {
    dst.fill(0);
    if dst_w == 0 || dst_h == 0 || fb.width == 0 || fb.height == 0 {
        return;
    }
    let view = viewport((dst_w, dst_h), (fb.width, fb.height));
    let integer_scale = view.scale as usize;
    if view.scale == integer_scale as f32
        && view.x == view.x.floor()
        && view.y == view.y.floor()
        && integer_scale > 0
    {
        let offset_x = view.x as usize;
        let offset_y = view.y as usize;
        for sy in 0..fb.height {
            for repeat_y in 0..integer_scale {
                let row = (offset_y + sy * integer_scale + repeat_y) * dst_w as usize;
                for sx in 0..fb.width {
                    let color = fb.pixels[sy * fb.width + sx] & 0x00ff_ffff;
                    let left = row + offset_x + sx * integer_scale;
                    if let Some(span) = dst.get_mut(left..left + integer_scale) {
                        span.fill(color);
                    }
                }
            }
        }
        return;
    }
    let image_right = view.x + fb.width as f32 * view.scale;
    let image_bottom = view.y + fb.height as f32 * view.scale;
    for y in 0..dst_h {
        let py = y as f32 + 0.5;
        if py < view.y || py >= image_bottom {
            continue;
        }
        let sy = (((py - view.y) / view.scale).floor() as usize).min(fb.height - 1);
        for x in 0..dst_w {
            let px = x as f32 + 0.5;
            if px < view.x || px >= image_right {
                continue;
            }
            let sx = (((px - view.x) / view.scale).floor() as usize).min(fb.width - 1);
            let dst_index = y as usize * dst_w as usize + x as usize;
            if let (Some(&pixel), Some(output)) =
                (fb.pixels.get(sy * fb.width + sx), dst.get_mut(dst_index))
            {
                *output = pixel & 0x00ff_ffff;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{blit_scaled, viewport, window_to_framebuffer};
    use crate::frame::Framebuffer;

    #[test]
    fn viewport_centers_pillarbox_letterbox_and_integer_scale() {
        assert_eq!(
            viewport((500, 300), (4, 3)),
            super::Viewport {
                x: 50.0,
                y: 0.0,
                scale: 100.0
            }
        );
        assert_eq!(
            viewport((400, 500), (4, 3)),
            super::Viewport {
                x: 0.0,
                y: 100.0,
                scale: 100.0
            }
        );
        assert_eq!(
            viewport((1280, 720), (320, 180)),
            super::Viewport {
                x: 0.0,
                y: 0.0,
                scale: 4.0
            }
        );
        assert_eq!(viewport((1, 1), (1, 1)).scale, 1.0);
    }

    #[test]
    fn zero_sizes_and_pointer_edges_are_safe() {
        assert_eq!(viewport((0, 20), (4, 3)), super::Viewport::default());
        assert_eq!(viewport((20, 20), (0, 3)), super::Viewport::default());
        let view = viewport((500, 300), (4, 3));
        assert_eq!(
            window_to_framebuffer((50.0, 0.0), &view, (4, 3)),
            Some((0.0, 0.0))
        );
        let bottom_right = window_to_framebuffer((449.9, 299.9), &view, (4, 3)).unwrap();
        assert!((bottom_right.0 - 3.999).abs() < 1e-4);
        assert!((bottom_right.1 - 2.999).abs() < 1e-4);
        assert_eq!(window_to_framebuffer((450.0, 300.0), &view, (4, 3)), None);
        assert_eq!(
            window_to_framebuffer((50.0, 50.0), &view, (4, 3)),
            Some((0.0, 0.5))
        );
        assert_eq!(window_to_framebuffer((49.99, 20.0), &view, (4, 3)), None);
        assert_eq!(window_to_framebuffer((50.0, 0.0), &view, (0, 3)), None);
    }

    #[test]
    fn two_x_and_three_x_blits_are_pixel_exact() {
        let mut fb = Framebuffer::new(2, 2);
        fb.pixels = vec![0x0011_2233, 0x0044_5566, 0x0077_8899, 0x00aa_bbcc];
        for scale in [2, 3] {
            let mut dst = vec![u32::MAX; (2 * scale * 2 * scale) as usize];
            blit_scaled(&fb, &mut dst, 2 * scale, 2 * scale);
            for y in 0..2 * scale {
                for x in 0..2 * scale {
                    assert_eq!(
                        dst[(y * 2 * scale + x) as usize],
                        fb.pixels[((y / scale) * 2 + x / scale) as usize]
                    );
                }
            }
        }
    }

    #[test]
    fn blit_clears_bars_and_ignores_destination_high_byte() {
        let mut fb = Framebuffer::new(2, 2);
        fb.pixels.fill(0xff12_3456);
        let mut dst = vec![u32::MAX; 8 * 6];
        blit_scaled(&fb, &mut dst, 8, 6);
        assert_eq!(dst[0], 0);
        assert_eq!(dst[3], 0x0012_3456);
        assert_eq!(dst[4], 0x0012_3456);
        assert_eq!(dst[7], 0);
        let mut empty = vec![1; 20];
        blit_scaled(&fb, &mut empty, 0, 5);
        assert!(empty.iter().all(|&pixel| pixel == 0));
    }
}
