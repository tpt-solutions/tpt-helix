//! Minimal CSS subset parsing.
//!
//! AppFront does not yet run the full `lightningcss` pipeline; it parses a
//! small, well-defined subset of CSS that is enough to demonstrate the
//! `egui` ↔ `taffy` bridge with real styled content:
//!
//! * inline `style="..."` attributes on elements
//! * `<style>` blocks with `tag`, `.class`, and `*` selectors
//!
//! Supported properties: `display`, `flex-direction`, `width`, `height`,
//! `padding`, `margin`, `background`, `color`, `font-size`, `border`,
//! `border-radius`.

use std::collections::HashMap;

/// A CSS selector supported by the minimal parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Universal,
    Tag(String),
    Class(String),
}

/// Resolved CSS property values for a single element (pre-layout).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleProps {
    pub display: Option<DisplayKind>,
    pub flex_direction: Option<FlexDir>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub padding: Option<f32>,
    pub margin: Option<f32>,
    pub background: Option<[u8; 4]>,
    pub color: Option<[u8; 4]>,
    pub font_size: Option<f32>,
    pub border_width: Option<f32>,
    pub border_color: Option<[u8; 4]>,
    pub radius: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayKind {
    Block,
    Flex,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDir {
    Row,
    Column,
}

/// Parses an inline `style="..."` attribute string.
pub fn parse_inline(input: &str) -> StyleProps {
    let mut props = StyleProps::default();
    for decl in input.split(';') {
        apply_decl(decl, &mut props);
    }
    props
}

/// Parses a `<style>` block body into `(selector, props)` rules.
pub fn parse_stylesheet(input: &str) -> Vec<(Selector, StyleProps)> {
    let mut rules = Vec::new();
    for block in input.split('}') {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let Some((selectors, decls)) = block.split_once('{') else {
            continue;
        };
        let mut props = StyleProps::default();
        for decl in decls.split(';') {
            apply_decl(decl, &mut props);
        }
        for sel in selectors.split(',') {
            let sel = sel.trim();
            if sel.is_empty() {
                continue;
            }
            rules.push((parse_selector(sel), props.clone()));
        }
    }
    rules
}

fn parse_selector(s: &str) -> Selector {
    let s = s.trim();
    if s == "*" {
        Selector::Universal
    } else if let Some(class) = s.strip_prefix('.') {
        Selector::Class(class.to_string())
    } else {
        Selector::Tag(s.to_ascii_lowercase())
    }
}

fn apply_decl(decl: &str, props: &mut StyleProps) {
    let decl = decl.trim();
    let Some((raw_name, raw_value)) = decl.split_once(':') else {
        return;
    };
    let name = raw_name.trim().to_ascii_lowercase();
    let value = raw_value.trim();

    match name.as_str() {
        "display" => {
            props.display = match value {
                "block" => Some(DisplayKind::Block),
                "flex" => Some(DisplayKind::Flex),
                "grid" => Some(DisplayKind::Grid),
                _ => None,
            };
        }
        "flex-direction" => {
            props.flex_direction = match value {
                "row" => Some(FlexDir::Row),
                "column" => Some(FlexDir::Column),
                _ => None,
            };
        }
        "width" => props.width = parse_length(value),
        "height" => props.height = parse_length(value),
        "padding" => props.padding = parse_length(value),
        "margin" => props.margin = parse_length(value),
        "font-size" => props.font_size = parse_length(value),
        "border-radius" => props.radius = parse_length(value),
        "background" | "background-color" => props.background = parse_color(value),
        "color" => props.color = parse_color(value),
        "border" => {
            if let Some((w, c)) = parse_border_shorthand(value) {
                props.border_width = Some(w);
                props.border_color = Some(c);
            }
        }
        _ => {}
    }
}

/// Parses a CSS length: a number (px) or a percentage (`50%`). Returns points.
/// Percentages are returned as negative sentinel-free `Option` via `Percent`.
/// We keep them as points here for the subset; `layout` maps them later.
fn parse_length(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.is_empty() || value == "auto" {
        return None;
    }
    if let Some(pct) = value.strip_suffix('%') {
        // Store percentage widths as negative to flag "percent" downstream.
        if let Ok(v) = pct.trim().parse::<f32>() {
            return Some(-v.clamp(0.0, 100.0).abs());
        }
        return None;
    }
    value.trim_end_matches("px").parse::<f32>().ok()
}

/// Parses a color: `#rgb`, `#rrggbb`, or a small set of named colors.
pub fn parse_color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() || value == "transparent" {
        return None;
    }
    if let Some(hex) = value.strip_prefix('#') {
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some([r, g, b, 255])
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some([r, g, b, 255])
            }
            _ => None,
        }
    } else {
        named_color(&value)
    }
}

fn named_color(name: &str) -> Option<[u8; 4]> {
    let c = match name {
        "black" => [0, 0, 0, 255],
        "white" => [255, 255, 255, 255],
        "red" => [220, 50, 47, 255],
        "green" => [50, 170, 70, 255],
        "blue" => [40, 90, 220, 255],
        "gray" | "grey" => [120, 120, 120, 255],
        "darkgray" | "darkgrey" => [80, 80, 80, 255],
        "lightgray" | "lightgrey" => [200, 200, 200, 255],
        _ => return None,
    };
    Some(c)
}

/// Parses `1px solid #rrggbb` style border shorthand into (width, color).
fn parse_border_shorthand(value: &str) -> Option<(f32, [u8; 4])> {
    let mut width = 1.0f32;
    let mut color = [0, 0, 0, 255];
    let mut found_color = false;
    for part in value.split_whitespace() {
        if let Some(w) = parse_length(part) {
            if w >= 0.0 {
                width = w;
            }
        } else if let Some(c) = parse_color(part) {
            color = c;
            found_color = true;
        }
    }
    if found_color {
        Some((width, color))
    } else {
        None
    }
}

/// Collects all `<style>` blocks from a document's text content.
pub fn extract_style_blocks(source: &str) -> String {
    let mut out = String::new();
    let bytes = source.as_bytes();
    let marker = b"<style";
    let mut i = 0;
    while let Some(pos) = find_at(bytes, marker, i) {
        // find '>' closing the opening tag
        let open_end = match find_at(bytes, b">", pos) {
            Some(e) => e + 1,
            None => break,
        };
        if let Some(close) = find_at(bytes, b"</style>", open_end) {
            let css = &source[open_end..close];
            out.push_str(css);
            out.push('\n');
            i = close + 8;
        } else {
            break;
        }
    }
    out
}

fn find_at(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Matches an element (by tag + classes) against a parsed selector.
pub fn selector_matches(sel: &Selector, tag: &str, classes: &[String]) -> bool {
    match sel {
        Selector::Universal => true,
        Selector::Tag(t) => t == tag,
        Selector::Class(c) => classes.iter().any(|cl| cl == c),
    }
}

/// Merges a list of `StyleProps` (lowest priority first) into one, returning
/// the combined result. Later entries win over earlier ones.
pub fn merge_props(rules: &[StyleProps]) -> StyleProps {
    let mut acc = StyleProps::default();
    for r in rules {
        macro_rules! pick {
            ($field:ident) => {
                if r.$field.is_some() {
                    acc.$field = r.$field.clone();
                }
            };
        }
        pick!(display);
        pick!(flex_direction);
        pick!(width);
        pick!(height);
        pick!(padding);
        pick!(margin);
        pick!(background);
        pick!(color);
        pick!(font_size);
        pick!(border_width);
        pick!(border_color);
        pick!(radius);
    }
    acc
}

/// Computes the effective `StyleProps` for an element given stylesheet rules
/// and an inline style override. Stylesheet rules are applied in order; the
/// inline style overrides everything.
pub fn resolve_element_props(
    tag: &str,
    classes: &[String],
    stylesheet: &[(Selector, StyleProps)],
    inline: &str,
) -> StyleProps {
    let mut matched: Vec<StyleProps> = stylesheet
        .iter()
        .filter(|(sel, _)| selector_matches(sel, tag, classes))
        .map(|(_, p)| p.clone())
        .collect();
    let inline_props = parse_inline(inline);
    matched.push(inline_props);
    merge_props(&matched)
}

/// Convenience: build a `tag -> class list` plus inline style lookup is the
/// caller's job; this just re-exports a HashMap alias for clarity.
pub type ClassMap = HashMap<String, Vec<String>>;
