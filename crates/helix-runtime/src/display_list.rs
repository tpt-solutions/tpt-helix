//! Flattens a computed [`crate::layout`] tree into a paint-order list of
//! [`DisplayItem`]s: the boundary between "what to draw" (layout + style)
//! and "how to draw it" (the [`crate::gpu`] pipeline).

use lightningcss::traits::Parse;
use lightningcss::values::color::CssColor;
use markup5ever_rcdom::{Handle, NodeData};
use taffy::{NodeId, TaffyTree};

use crate::css::{DomElement, StyleRule, matches};

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
            for declaration in rule.declarations_css.split(';') {
                if let Some((property, value)) = declaration.split_once(':')
                    && property.trim() == "background-color"
                    && let Some(parsed) = Color::parse_css(value.trim())
                {
                    color = Some(parsed);
                }
            }
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
