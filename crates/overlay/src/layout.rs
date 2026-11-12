use tiny_skia::Pixmap;
use wlmg_core::geometry::Vector;

const BYTES_PER_PIXEL: usize = 4;

/// Region of a buffer in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Centres the wheel on the cursor, shifted along each axis so it stays entirely on the output when it fits.
pub(crate) fn wheel_centre(cursor: Vector, output: Vector, radius: f32) -> Vector {
    let axis = |position: f32, length: f32| {
        if length >= 2.0 * radius { position.clamp(radius, length - radius) } else { length / 2.0 }
    };
    Vector::new(axis(cursor.x, output.x), axis(cursor.y, output.y))
}

/// Copies `pixmap` (premultiplied RGBA) into a little-endian ARGB8888 `canvas` with its top-left corner at
/// `origin`, clipped to the canvas; returns the region written.
pub(crate) fn blit(canvas: &mut [u8], canvas_width: i32, pixmap: &Pixmap, origin: (i32, i32)) -> Option<Region> {
    let canvas_height = (canvas.len() / BYTES_PER_PIXEL) as i32 / canvas_width;
    let (left, top) = (origin.0.max(0), origin.1.max(0));
    let right = (origin.0 + pixmap.width() as i32).min(canvas_width);
    let bottom = (origin.1 + pixmap.height() as i32).min(canvas_height);
    if left >= right || top >= bottom {
        return None;
    }
    for y in top..bottom {
        let source_row = &pixmap.data()[((y - origin.1) as u32 * pixmap.width()) as usize * BYTES_PER_PIXEL..];
        let row_start = (y * canvas_width) as usize * BYTES_PER_PIXEL;
        for x in left..right {
            let source = &source_row[(x - origin.0) as usize * BYTES_PER_PIXEL..][..BYTES_PER_PIXEL];
            let target = row_start + x as usize * BYTES_PER_PIXEL;
            canvas[target..target + BYTES_PER_PIXEL].copy_from_slice(&[source[2], source[1], source[0], source[3]]);
        }
    }
    Some(Region { x: left, y: top, width: right - left, height: bottom - top })
}

#[cfg(test)]
mod tests {
    use tiny_skia::Color;

    use super::*;

    const OUTPUT: Vector = Vector::new(1920.0, 1080.0);

    #[test]
    fn wheel_follows_cursor_away_from_edges() {
        assert_eq!(wheel_centre(Vector::new(800.0, 500.0), OUTPUT, 150.0), Vector::new(800.0, 500.0));
    }

    #[test]
    fn wheel_is_pushed_back_inside_near_edges() {
        assert_eq!(wheel_centre(Vector::new(10.0, 1075.0), OUTPUT, 150.0), Vector::new(150.0, 930.0));
    }

    #[test]
    fn wheel_larger_than_output_is_centred() {
        assert_eq!(wheel_centre(Vector::new(10.0, 10.0), Vector::new(200.0, 1080.0), 150.0), Vector::new(100.0, 150.0));
    }

    #[test]
    fn blit_swaps_to_bgra_and_clips() {
        let mut pixmap = Pixmap::new(2, 2).unwrap();
        pixmap.fill(Color::from_rgba8(0x11, 0x22, 0x33, 0xFF));
        let mut canvas = vec![0; 3 * 3 * BYTES_PER_PIXEL];
        let region = blit(&mut canvas, 3, &pixmap, (2, -1));
        assert_eq!(region, Some(Region { x: 2, y: 0, width: 1, height: 1 }));
        assert_eq!(canvas[2 * BYTES_PER_PIXEL..3 * BYTES_PER_PIXEL], [0x33, 0x22, 0x11, 0xFF]);
        assert_eq!(canvas.iter().filter(|&&byte| byte != 0).count(), BYTES_PER_PIXEL);
    }

    #[test]
    fn blit_outside_canvas_writes_nothing() {
        let pixmap = Pixmap::new(2, 2).unwrap();
        assert_eq!(blit(&mut [0; 4 * BYTES_PER_PIXEL], 2, &pixmap, (5, 0)), None);
    }
}
