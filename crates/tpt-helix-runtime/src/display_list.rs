//! Flattens a computed [`crate::layout`] tree into a paint-order list of
//! [`DisplayItem`]s: the boundary between "what to draw" (layout + style)
//! and "how to draw it" (the [`crate::gpu`] pipeline).

use lightningcss::traits::Parse;
use lightningcss::values::color::CssColor;
use markup5ever_rcdom::{Handle, NodeData};
use taffy::{NodeId, TaffyTree};

use crate::css::{DomElement, StyleRule, TextStyle, matches, text_style};
use crate::layout::text_content;

/// An RGBA color in `[0, 1]` linear-ish component range (no color management
/// yet — sRGB hex values are mapped straight through).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// Parses any CSS `<color>` value `lightningcss` understands (hex,
    /// named colors like `red`, `rgb()`/`hsl()`, ...), converted to sRGB.
    /// `currentColor` and other context-dependent colors aren't resolvable
    /// without a cascade, so those yield `None`.
    pub fn parse_css(value: &str) -> Option<Color> {
        let color = CssColor::parse_string(value).ok()?;
        let rgba = color.to_rgb().ok()?;
        let CssColor::RGBA(rgba) = rgba else {
            return None;
        };
        Some(Color {
            r: rgba.red as f32 / 255.0,
            g: rgba.green as f32 / 255.0,
            b: rgba.blue as f32 / 255.0,
            a: rgba.alpha as f32 / 255.0,
        })
    }
}

/// One paintable rectangle in the display list, in viewport-space pixels.
#[derive(Debug, Clone, Copy)]
pub struct DisplayItem {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: Color,
}

fn background_color(element: &DomElement, rules: &[StyleRule]) -> Option<Color> {
    let mut color = None;
    for rule in rules {
        if rule.selectors.slice().iter().any(|s| matches(s, element)) {
            color = background_from_declarations(&rule.declarations_css, color);
        }
    }
    // Inline style comes last in the cascade, so it overrides the stylesheet.
    if let Some(inline) = element.inline_style() {
        color = background_from_declarations(&inline, color);
    }
    color
}

/// Applies the background declarations in `declarations` to `current`. Handles
/// `background-color` and the `background` shorthand when its value names a
/// color (for example `background: #fff` or `background: url(a.png) red`).
/// `background: none` clears the color. A later declaration wins.
fn background_from_declarations(declarations: &str, current: Option<Color>) -> Option<Color> {
    let mut color = current;
    for declaration in declarations.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match property.trim() {
            "background-color" => {
                if let Some(parsed) = Color::parse_css(value) {
                    color = Some(parsed);
                }
            }
            "background" => {
                if value.eq_ignore_ascii_case("none") {
                    color = None;
                } else if let Some(parsed) = Color::parse_css(value)
                    .or_else(|| value.split_whitespace().rev().find_map(Color::parse_css))
                {
                    color = Some(parsed);
                }
            }
            _ => {}
        }
    }
    color
}

/// Walks `tree` in paint order (parents before children, matching source
/// order for siblings) and emits a [`DisplayItem`] for every box that has a
/// resolved `background-color`. Boxes with no background are layout-only and
/// contribute no paint.
pub fn build_display_list(
    tree: &TaffyTree<Handle>,
    root: NodeId,
    rules: &[StyleRule],
) -> Vec<DisplayItem> {
    let mut items = Vec::new();
    walk(tree, root, 0.0, 0.0, rules, &mut items);
    items
}

fn walk(
    tree: &TaffyTree<Handle>,
    node: NodeId,
    parent_x: f32,
    parent_y: f32,
    rules: &[StyleRule],
    items: &mut Vec<DisplayItem>,
) {
    let Ok(layout) = tree.layout(node) else {
        return;
    };
    let x = parent_x + layout.location.x;
    let y = parent_y + layout.location.y;

    if let Some(handle) = tree.get_node_context(node)
        && matches!(handle.data, NodeData::Element { .. })
        && let Some(color) = background_color(&DomElement(handle.clone()), rules)
    {
        items.push(DisplayItem {
            x,
            y,
            width: layout.size.width,
            height: layout.size.height,
            color,
        });
    }

    if let Ok(children) = tree.children(node) {
        for child in children {
            walk(tree, child, x, y, rules, items);
        }
    }
}

/// One run of text to paint, positioned by its top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub x: f32,
    pub y: f32,
    /// The box the text was laid out into. Lines wrap to this width.
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub font_size: f32,
    pub color: Color,
}

/// Collects the text runs of the laid-out tree in paint order. Each run takes the
/// `color` and `font-size` its element cascades to.
pub fn build_text_runs(
    tree: &TaffyTree<Handle>,
    root: NodeId,
    rules: &[StyleRule],
) -> Vec<TextRun> {
    let mut out = Vec::new();
    collect_text(tree, root, 0.0, 0.0, rules, TextStyle::default(), &mut out);
    out
}

fn collect_text(
    tree: &TaffyTree<Handle>,
    node: NodeId,
    parent_x: f32,
    parent_y: f32,
    rules: &[StyleRule],
    inherited: TextStyle,
    out: &mut Vec<TextRun>,
) {
    let Ok(layout) = tree.layout(node) else {
        return;
    };
    let x = parent_x + layout.location.x;
    let y = parent_y + layout.location.y;
    let mut style = inherited;
    if let Some(handle) = tree.get_node_context(node) {
        match &handle.data {
            NodeData::Element { .. } => {
                style = text_style(&DomElement(handle.clone()), inherited, rules);
            }
            NodeData::Text { .. } => {
                let text = text_content(handle);
                if !text.is_empty() {
                    out.push(TextRun {
                        x,
                        y,
                        width: layout.size.width,
                        height: layout.size.height,
                        text,
                        font_size: inherited.font_size,
                        color: inherited.color,
                    });
                }
            }
            _ => {}
        }
    }
    if let Ok(children) = tree.children(node) {
        for child in children {
            collect_text(tree, child, x, y, rules, style, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parse_stylesheet;
    use crate::html::parse_html;
    use crate::layout::{build_layout_tree, compute};

    #[test]
    fn parses_css_colors() {
        assert_eq!(
            Color::parse_css("#ff0000"),
            Some(Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            })
        );
        assert_eq!(
            Color::parse_css("#0f0"),
            Some(Color {
                r: 0.0,
                g: 1.0,
                b: 0.0,
                a: 1.0
            })
        );
        assert_eq!(
            Color::parse_css("red"),
            Some(Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            })
        );
        assert_eq!(Color::parse_css("not-a-color"), None);
    }

    #[test]
    fn emits_one_item_per_colored_box() {
        let dom =
            parse_html(r#"<html><body><div class="a"></div><div class="b"></div></body></html>"#);
        let rules = parse_stylesheet(
            "div.a { width: 10px; height: 10px; background-color: #ff0000; } \
             div.b { width: 20px; height: 20px; background-color: #00ff00; }",
        );
        let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
        compute(&mut layout, 800.0, 600.0).expect("compute layout");

        let items = build_display_list(&layout.tree, layout.root, &rules);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].color,
            Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            }
        );
        assert_eq!(
            items[1].color,
            Color {
                r: 0.0,
                g: 1.0,
                b: 0.0,
                a: 1.0
            }
        );
        // Both divs are block-level, so they stack vertically (each starts a new
        // line) rather than sitting side by side. `div.a` is 10px tall, so
        // `div.b` begins at y=10.
        assert_eq!((items[0].x, items[0].y), (0.0, 0.0));
        assert_eq!((items[1].x, items[1].y, items[1].width), (0.0, 10.0, 20.0));
    }
}
