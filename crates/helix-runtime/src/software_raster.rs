//! Headless software rasterization of a [`crate::display_list`] for visual
//! regression testing (Stage S3: Validate, spec §6.3).
//!
//! The GPU path ([`crate::gpu`]) is the production presenter, but CI runners
//! have no GPU. To still be able to *see* what the static pipeline produced,
//! this module paints a [`crate::display_list::DisplayItem`] list — colored
//! rectangles in viewport space — into an in-memory RGBA buffer using plain
//! alpha compositing (paint order = stacking order). The buffer can then be
//! encoded to PNG and diffed against a baseline by [`crate::screenshot_diff`].
//!
//! This is intentionally a minimal software fallback: no text, borders, or
//! images, just the background boxes. It is enough to catch gross layout /
//! styling regressions (an element moved, changed size, or lost its color)
//! without a display.

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use crate::display_list::{Color, DisplayItem};

/// Paint `items` (in the order they were emitted, which is paint order) onto a
/// `width`x`height` RGBA buffer, starting from a transparent black canvas.
///
/// Each item's color is alpha-composited over whatever is already there, so a
/// later item occludes an earlier one where they overlap (matching the
/// over-painting semantics of the GPU path).
pub fn rasterize_display_list(
    items: &[DisplayItem],
    width: u32,
    height: u32,
) -> RgbaImage {
    let mut buf = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));

    for item in items {
        let x0 = item.x.max(0.0).floor() as i64;
        let y0 = item.y.max(0.0).floor() as i64;
        let x1 = (item.x + item.width).ceil() as i64;
        let y1 = (item.y + item.height).ceil() as i64;
        if x1 <= 0 || y1 <= 0 || x0 >= width as i64 || y0 >= height as i64 {
            continue;
        }
        let x0 = x0.max(0) as u32;
        let y0 = y0.max(0) as u32;
        let x1 = (x1.min(width as i64)) as u32;
        let y1 = (y1.min(height as i64)) as u32;

        let (cr, cg, cb, ca) = to_bytes(item.color);
        for y in y0..y1 {
            for x in x0..x1 {
                let dst = buf.get_pixel_mut(x, y);
                let (dr, dg, db, da) = (dst[0], dst[1], dst[2], dst[3]);
                // Standard "over" operator in straight (non-premultiplied) alpha.
                let out_a = ca + (da as u32) * (255 - ca as u32) / 255;
                let out_r = (cr as u32 * ca as u32
                    + dr as u32 * da as u32 * (255 - ca as u32) / 255)
                    / out_a.max(1) as u32;
                let out_g = (cg as u32 * ca as u32
                    + dg as u32 * da as u32 * (255 - ca as u32) / 255)
                    / out_a.max(1) as u32;
                let out_b = (cb as u32 * ca as u32
                    + db as u32 * da as u32 * (255 - ca as u32) / 255)
                    / out_a.max(1) as u32;
                *dst = Rgba([out_r as u8, out_g as u8, out_b as u8, out_a as u8]);
            }
        }
    }

    buf
}

/// Encode an RGBA buffer as PNG bytes (used to persist a baseline / artifact).
pub fn encode_png(image: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .expect("PNG encoding is infallible for an in-memory buffer");
    out
}

/// Decode a PNG buffer back into an RGBA image (used to load a baseline).
pub fn decode_png(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .ok()
        .map(|d| d.to_rgba8())
}

fn to_bytes(c: Color) -> (u8, u8, u8, u8) {
    (
        (c.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.b.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::DisplayItem;

    fn rect(x: f32, y: f32, w: f32, h: f32, c: Color) -> DisplayItem {
        DisplayItem { x, y, width: w, height: h, color: c }
    }

    #[test]
    fn paints_opaque_rect() {
        let items = vec![rect(0.0, 0.0, 10.0, 10.0, Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 })];
        let img = rasterize_display_list(&items, 10, 10);
        assert_eq!(img.get_pixel(5, 5), &Rgba([255, 0, 0, 255]));
        // Outside the rect stays transparent.
        assert_eq!(img.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn later_items_occlude_earlier_ones() {
        let items = vec![
            rect(0.0, 0.0, 10.0, 10.0, Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }),
            rect(2.0, 2.0, 4.0, 4.0, Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }),
        ];
        let img = rasterize_display_list(&items, 10, 10);
        assert_eq!(img.get_pixel(4, 4), &Rgba([0, 0, 255, 255]), "blue on top");
        assert_eq!(img.get_pixel(0, 0), &Rgba([255, 0, 0, 255]), "red underneath");
    }

    #[test]
    fn alpha_composites_over_transparent() {
        let items = vec![rect(
            0.0,
            0.0,
            10.0,
            10.0,
            Color { r: 0.0, g: 0.0, b: 0.0, a: 0.5 },
        )];
        let img = rasterize_display_list(&items, 10, 10);
        // 50% black over transparent -> mid grey, 50% alpha.
        assert_eq!(img.get_pixel(5, 5), &Rgba([0, 0, 0, 127]), "half-alpha black");
    }

    #[test]
    fn round_trips_through_png() {
        let items = vec![rect(0.0, 0.0, 4.0, 4.0, Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 })];
        let img = rasterize_display_list(&items, 4, 4);
        let png = encode_png(&img);
        let back = decode_png(&png).expect("re-decode");
        assert_eq!(img, back);
    }
}
