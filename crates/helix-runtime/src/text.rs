//! Task: Integrate `cosmic-text` for font shaping and line breaking.

use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};

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
                .map(|g| ShapedGlyph { glyph_id: g.glyph_id, x: g.x, y: g.y })
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
        assert!(lines.len() > 1, "expected wrapping to produce multiple lines");
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
        assert!(long_glyphs > short_glyphs, "more text should yield more glyphs");
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
