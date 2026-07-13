//! Task: Integrate `image` (PNG/JPEG/WebP/GIF) and `resvg` (SVG) decoding.
//!
//! Raster formats decode straight to RGBA8 pixels via the `image` crate.
//! SVG is a vector format with no fixed pixel size, so it's parsed with
//! `usvg` and rasterized to RGBA8 with `resvg` at a caller-chosen size.

use image::{DynamicImage, ImageError};
use resvg::{tiny_skia, usvg};

/// A decoded image, normalized to tightly-packed RGBA8 rows.
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// Decodes PNG/JPEG/WebP/GIF (first frame) bytes into RGBA8, auto-detecting
/// the format from the data.
pub fn decode_raster(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    let image = image::load_from_memory(bytes)?;
    to_decoded(image)
}

fn to_decoded(image: DynamicImage) -> Result<DecodedImage, ImageError> {
    let rgba = image.to_rgba8();
    Ok(DecodedImage { width: rgba.width(), height: rgba.height(), rgba8: rgba.into_raw() })
}

/// Parses and rasterizes an SVG document to RGBA8 at exactly
/// `width`x`height` pixels (the SVG's own viewBox is scaled to fit).
pub fn rasterize_svg(svg_text: &str, width: u32, height: u32) -> Option<DecodedImage> {
    let tree = usvg::Tree::from_str(svg_text, &usvg::Options::default()).ok()?;

    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let tree_size = tree.size();
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / tree_size.width(),
        height as f32 / tree_size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    Some(DecodedImage { width, height, rgba8: pixmap.take() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_1x1_png() {
        // A minimal 1x1 red PNG, base64-decoded at compile time would need an
        // extra dependency; encode one on the fly instead.
        let mut png_bytes = Vec::new();
        {
            let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
                .unwrap();
        }

        let decoded = decode_raster(&png_bytes).expect("valid PNG");
        assert_eq!((decoded.width, decoded.height), (1, 1));
        assert_eq!(decoded.rgba8, vec![255, 0, 0, 255]);
    }

    #[test]
    fn rasterizes_a_solid_color_svg() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect width="10" height="10" fill="#ff0000"/>
        </svg>"##;
        let decoded = rasterize_svg(svg, 10, 10).expect("valid SVG");
        assert_eq!((decoded.width, decoded.height), (10, 10));
        // Center pixel should be opaque red.
        let idx = ((5 * 10 + 5) * 4) as usize;
        assert_eq!(&decoded.rgba8[idx..idx + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn decode_rejects_garbage_bytes() {
        // Random non-image bytes are not a recognized raster format.
        assert!(decode_raster(b"this is definitely not an image").is_err());
    }

    #[test]
    fn decode_rejects_truncated_png() {
        // A valid PNG header but a truncated body fails to decode.
        let mut png = Vec::new();
        {
            let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
        }
        // Keep only the first 8 bytes (signature) — the rest is missing.
        let truncated = &png[..8];
        assert!(decode_raster(truncated).is_err());
    }

    #[test]
    fn rasterize_malformed_svg_returns_none() {
        // Unparseable SVG yields no tree, so rasterization returns None rather
        // than panicking.
        assert!(rasterize_svg("not <svg> at all <<<", 10, 10).is_none());
        assert!(rasterize_svg("<svg><rect", 10, 10).is_none());
    }
}
