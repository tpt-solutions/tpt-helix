//! Task: Integrate `cosmic-text` for font shaping and line breaking.

use std::cell::RefCell;
use std::collections::HashMap;

use cosmic_text::{Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache};
use image::{Rgba, RgbaImage};

/// Line height as a multiple of the font size. Layout and painting both use it,
/// so a text box is exactly as tall as the text it paints.
pub const LINE_HEIGHT_RATIO: f32 = 1.25;

/// The line height used for text at `font_size`.
pub fn line_height_for(font_size: f32) -> f32 {
    font_size * LINE_HEIGHT_RATIO
}

/// Per-thread font state. Loading system fonts is the expensive step, so it runs
/// once per thread, not once per text run.
struct Fonts {
    system: FontSystem,
    cache: SwashCache,
}

thread_local! {
    static FONTS: RefCell<Option<Fonts>> = const { RefCell::new(None) };
    static MEASURED: RefCell<HashMap<MeasureKey, (f32, f32)>> = RefCell::new(HashMap::new());
}

/// Most measurements kept before the cache is cleared. Layout asks for the same
/// text at the same width several times per pass, so a small cache removes most
/// of the shaping work, and clearing it bounds memory on long-running hosts.
const MEASURE_CACHE_LIMIT: usize = 4096;

/// Cache key for a measurement: text, font size, and wrap width, with the floats
/// stored as bit patterns so the key is hashable.
#[derive(Clone, PartialEq, Eq, Hash)]
struct MeasureKey {
    text: String,
    font_size: u32,
    wrap_width: Option<u32>,
}

fn with_fonts<R>(f: impl FnOnce(&mut FontSystem, &mut SwashCache) -> R) -> R {
    FONTS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let fonts = slot.get_or_insert_with(|| Fonts {
            system: FontSystem::new_with_locale_and_db(
                "en-US".to_string(),
                crate::fonts::load_system_fonts(),
            ),
            cache: SwashCache::new(),
        });
        f(&mut fonts.system, &mut fonts.cache)
    })
}

/// Shapes `text` on a single unwrapped line at `font_size` and returns its
/// `(width, height)` in pixels. Returns `(0.0, 0.0)` when the system has no fonts.
pub fn measure_text(text: &str, font_size: f32) -> (f32, f32) {
    measure_wrapped(text, font_size, None)
}

/// Shapes `text` at `font_size`, wrapping lines to `wrap_width` when it is given,
/// and returns the `(width, height)` of the laid-out block. The widest line sets the
/// width. Returns `(0.0, 0.0)` when the system has no fonts.
pub fn measure_wrapped(text: &str, font_size: f32, wrap_width: Option<f32>) -> (f32, f32) {
    let key = MeasureKey {
        text: text.to_string(),
        font_size: font_size.to_bits(),
        wrap_width: wrap_width.map(f32::to_bits),
    };
    if let Some(hit) = MEASURED.with(|cache| cache.borrow().get(&key).copied()) {
        return hit;
    }
    let measured = shape_block(text, font_size, wrap_width);
    MEASURED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= MEASURE_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, measured);
    });
    measured
}

fn shape_block(text: &str, font_size: f32, wrap_width: Option<f32>) -> (f32, f32) {
    with_fonts(|system, _| {
        let mut buffer = Buffer::new(system, Metrics::new(font_size, line_height_for(font_size)));
        buffer.set_size(system, wrap_width, None);
        buffer.set_text(system, text, Attrs::new(), Shaping::Advanced);
        buffer.shape_until_scroll(system, false);
        let mut width = 0.0f32;
        let mut height = 0.0f32;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height = height.max(run.line_top + run.line_height);
        }
        (width, height)
    })
}

/// Paints `text` into `image` with its top-left corner at `(x, y)`, wrapping lines
/// to `wrap_width` when given. `color` is straight (non-premultiplied) RGBA. Glyph
/// coverage is composited with the source-over operator.
pub fn draw_text(
    image: &mut RgbaImage,
    x: f32,
    y: f32,
    text: &str,
    font_size: f32,
    wrap_width: Option<f32>,
    color: [u8; 4],
) {
    let (ox, oy) = (x.round() as i64, y.round() as i64);
    with_fonts(|system, cache| {
        let mut buffer = Buffer::new(system, Metrics::new(font_size, line_height_for(font_size)));
        buffer.set_size(system, wrap_width, None);
        buffer.set_text(system, text, Attrs::new(), Shaping::Advanced);
        buffer.shape_until_scroll(system, false);
        let text_color = TextColor::rgba(color[0], color[1], color[2], color[3]);
        buffer.draw(system, cache, text_color, |px, py, w, h, c| {
            for dy in 0..h as i64 {
                for dx in 0..w as i64 {
                    let tx = ox + px as i64 + dx;
                    let ty = oy + py as i64 + dy;
                    if tx < 0 || ty < 0 || tx >= image.width() as i64 || ty >= image.height() as i64
                    {
                        continue;
                    }
                    let dst = image.get_pixel_mut(tx as u32, ty as u32);
                    blend_over(dst, [c.r(), c.g(), c.b()], c.a());
                }
            }
        });
    });
}

/// Source-over compositing of one straight-alpha source pixel onto `dst`.
fn blend_over(dst: &mut Rgba<u8>, src: [u8; 3], alpha: u8) {
    let a = alpha as f32 / 255.0;
    let da = dst.0[3] as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    if out_a <= 0.0 {
        return;
    }
    for (channel, &s) in dst.0.iter_mut().take(3).zip(src.iter()) {
        let v = (s as f32 * a + *channel as f32 * da * (1.0 - a)) / out_a;
        *channel = v.round().clamp(0.0, 255.0) as u8;
    }
    dst.0[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// One shaped glyph, positioned relative to the top-left of its line.
#[derive(Debug, Clone, Copy)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub x: f32,
    pub y: f32,
}

/// One line of shaped text: its glyphs plus the line's total width, after
/// `cosmic-text` line breaking has wrapped the source text to `wrap_width`.
#[derive(Debug, Clone)]
pub struct ShapedLine {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
}

/// Shapes and line-breaks `text` at `font_size`/`line_height`, wrapping to
/// fit within `wrap_width` logical pixels, using fonts from `db`.
///
/// Builds its own [`FontSystem`] per call rather than taking a shared one:
/// `FontSystem` construction (via [`crate::fonts::load_system_fonts`]) is the
/// expensive step, and callers doing many shaping passes should hoist that
/// and call `cosmic-text` directly once this module's approach is proven out.
pub fn shape_text(
    db: fontdb::Database,
    text: &str,
    font_size: f32,
    line_height: f32,
    wrap_width: f32,
) -> Vec<ShapedLine> {
    let mut font_system = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
    let metrics = Metrics::new(font_size, line_height);
    let mut buffer = Buffer::new(&mut font_system, metrics);
    buffer.set_size(&mut font_system, Some(wrap_width), None);
    buffer.set_text(&mut font_system, text, Attrs::new(), Shaping::Advanced);
    buffer.shape_until_scroll(&mut font_system, false);

    buffer
        .layout_runs()
        .map(|run| ShapedLine {
            width: run.line_w,
            glyphs: run
                .glyphs
                .iter()
                .map(|g| ShapedGlyph {
                    glyph_id: g.glyph_id,
                    x: g.x,
                    y: g.y,
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::load_system_fonts;

    #[test]
    fn shapes_short_text_into_a_single_line() {
        let lines = shape_text(load_system_fonts(), "hello", 16.0, 20.0, 1000.0);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].glyphs.is_empty());
        assert!(lines[0].width > 0.0);
    }

    #[test]
    fn narrow_wrap_width_breaks_into_multiple_lines() {
        let lines = shape_text(
            load_system_fonts(),
            "the quick brown fox jumps over the lazy dog",
            16.0,
            20.0,
            60.0,
        );
        assert!(
            lines.len() > 1,
            "expected wrapping to produce multiple lines"
        );
    }

    #[test]
    fn shapes_cjk_text_into_glyphs() {
        // CJK has no spaces, so shaping must still produce one glyph run per
        // character and not collapse the line.
        let lines = shape_text(load_system_fonts(), "汉字测试", 16.0, 20.0, 1000.0);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].glyphs.is_empty());
        assert!(lines[0].width > 0.0);
    }

    #[test]
    fn longer_text_produces_more_glyphs_than_shorter() {
        let short = shape_text(load_system_fonts(), "hi", 16.0, 20.0, 1000.0);
        let long = shape_text(
            load_system_fonts(),
            "the quick brown fox jumps",
            16.0,
            20.0,
            1000.0,
        );
        let short_glyphs: usize = short.iter().map(|l| l.glyphs.len()).sum();
        let long_glyphs: usize = long.iter().map(|l| l.glyphs.len()).sum();
        assert!(
            long_glyphs > short_glyphs,
            "more text should yield more glyphs"
        );
    }

    #[test]
    fn accented_and_ligature_text_shapes() {
        // Diacritics / ligatures must not crash shaping and must produce output.
        let lines = shape_text(load_system_fonts(), "café ﬁn", 16.0, 20.0, 1000.0);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].glyphs.is_empty());
    }

    #[test]
    fn empty_text_yields_no_glyphs() {
        let lines = shape_text(load_system_fonts(), "", 16.0, 20.0, 1000.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].glyphs.is_empty());
    }
}
