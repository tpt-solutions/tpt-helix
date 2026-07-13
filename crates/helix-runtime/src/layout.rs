//! Task: Integrate `taffy` for flexbox/grid layout.
//!
//! Builds a `taffy` layout tree that mirrors the element structure produced
//! by [`crate::html::parse_html`], with per-element `taffy::Style` resolved
//! from matched [`crate::css`] rules, then computes final box positions and
//! sizes for a given viewport.

use markup5ever_rcdom::{Handle, NodeData, RcDom};
use taffy::prelude::{auto, length, percent};
use taffy::{AvailableSpace, Dimension, Display, NodeId, Size, Style, TaffyTree};

use crate::css::{matches, DomElement, StyleRule};

/// The result of laying out a parsed HTML document: a `taffy` tree plus a
/// map from each DOM element to its `taffy` node, so callers can look up
/// computed [`taffy::Layout`] boxes per element after [`TaffyTree::compute_layout`].
pub struct DocumentLayout {
    pub tree: TaffyTree<Handle>,
    pub root: NodeId,
}

/// Resolves the `taffy::Style` for `element` from whichever of `rules`
/// match it, last rule wins (a minimal stand-in for CSS cascade order).
fn resolve_style(element: &DomElement, rules: &[StyleRule]) -> Style {
    let mut style = Style::default();
    for rule in rules {
        if rule.selectors.slice().iter().any(|s| matches(s, element)) {
            apply_declarations(&mut style, &rule.declarations_css);
        }
    }
    style
}

/// Applies a handful of layout-relevant CSS declarations (`display`, `width`,
/// `height`) found in `declarations_css` onto `style`. Declarations this
/// runtime doesn't understand yet are ignored rather than erroring, since
/// most CSS properties (color, font, ...) aren't layout inputs.
fn apply_declarations(style: &mut Style, declarations_css: &str) {
    for declaration in declarations_css.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim();
        let value = value.trim();
        match property {
            "display" => {
                style.display = match value {
                    "flex" => Display::Flex,
                    "grid" => Display::Grid,
                    "none" => Display::None,
                    _ => style.display,
                }
            }
            "width" => {
                if let Some(dim) = parse_dimension(value) {
                    style.size.width = dim;
                }
            }
            "height" => {
                if let Some(dim) = parse_dimension(value) {
                    style.size.height = dim;
                }
            }
            _ => {}
        }
    }
}

fn parse_dimension(value: &str) -> Option<Dimension> {
    if let Some(pct) = value.strip_suffix('%') {
        return pct.trim().parse::<f32>().ok().map(|p| percent(p / 100.0));
    }
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse::<f32>().ok().map(length);
    }
    if value == "auto" {
        return Some(auto());
    }
    None
}

fn is_element(handle: &Handle) -> bool {
    matches!(handle.data, NodeData::Element { .. })
}

fn build_node(
    tree: &mut TaffyTree<Handle>,
    handle: &Handle,
    rules: &[StyleRule],
) -> taffy::TaffyResult<NodeId> {
    let style = resolve_style(&DomElement(handle.clone()), rules);
    let children: Vec<NodeId> = handle
        .children
        .borrow()
        .iter()
        .filter(|c| is_element(c))
        .map(|c| build_node(tree, c, rules))
        .collect::<Result<_, _>>()?;

    let node = tree.new_with_children(style, &children)?;
    tree.set_node_context(node, Some(handle.clone()))?;
    Ok(node)
}

/// Builds a `taffy` tree for `dom`'s root element, styled by `rules`.
pub fn build_layout_tree(dom: &RcDom, rules: &[StyleRule]) -> taffy::TaffyResult<DocumentLayout> {
    fn find_root_element(handle: &Handle) -> Option<Handle> {
        if is_element(handle) {
            return Some(handle.clone());
        }
        handle.children.borrow().iter().find_map(find_root_element)
    }
    let root_element = find_root_element(&dom.document).expect("document has a root element");

    let mut tree = TaffyTree::new();
    let root = build_node(&mut tree, &root_element, rules)?;
    Ok(DocumentLayout { tree, root })
}

/// Computes layout for `layout.tree` against a viewport of `width`x`height`
/// logical pixels.
pub fn compute(layout: &mut DocumentLayout, width: f32, height: f32) -> taffy::TaffyResult<()> {
    layout.tree.compute_layout(
        layout.root,
        Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parse_stylesheet;
    use crate::html::parse_html;

    #[test]
    fn lays_out_flex_children_side_by_side() {
        let dom = parse_html(
            r#"<html><body><div class="row">
                 <div class="cell"></div>
                 <div class="cell"></div>
               </div></body></html>"#,
        );
        let rules = parse_stylesheet(
            "div.row { display: flex; width: 200px; height: 100px; } \
             div.cell { width: 50px; height: 50px; }",
        );

        let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
        compute(&mut layout, 800.0, 600.0).expect("compute layout");

        fn find_row(tree: &TaffyTree<Handle>, node: NodeId) -> Option<NodeId> {
            let handle = tree.get_node_context(node)?;
            if let NodeData::Element { attrs, .. } = &handle.data {
                if attrs.borrow().iter().any(|a| &*a.value == "row") {
                    return Some(node);
                }
            }
            tree.children(node)
                .ok()?
                .into_iter()
                .find_map(|c| find_row(tree, c))
        }

        let row = find_row(&layout.tree, layout.root).expect("row node");
        let row_layout = layout.tree.layout(row).expect("row layout");
        assert_eq!(row_layout.size.width, 200.0);

        let children = layout.tree.children(row).expect("row children");
        assert_eq!(children.len(), 2);
        let first = layout.tree.layout(children[0]).expect("first cell layout");
        let second = layout.tree.layout(children[1]).expect("second cell layout");
        assert_eq!(first.location.x, 0.0);
        assert_eq!(second.location.x, 50.0);
    }

    #[test]
    fn percentage_width_resolves_against_viewport() {
        let dom = parse_html(r#"<html><body><div class="half"></div></body></html>"#);
        let rules = parse_stylesheet("div.half { width: 50%; height: 10px; }");
        let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
        compute(&mut layout, 800.0, 600.0).expect("compute layout");

        fn find(tree: &TaffyTree<Handle>, node: NodeId, target: &str) -> Option<NodeId> {
            let handle = tree.get_node_context(node)?;
            if let NodeData::Element { attrs, .. } = &handle.data {
                if attrs.borrow().iter().any(|a| &*a.value == target) {
                    return Some(node);
                }
            }
            tree.children(node).ok()?.into_iter().find_map(|c| find(tree, c, target))
        }

        let half = find(&layout.tree, layout.root, "half").expect("half node");
        let half_layout = layout.tree.layout(half).expect("half layout");
        // 50% of an 800px viewport.
        assert_eq!(half_layout.size.width, 400.0);
    }

    #[test]
    fn cascade_last_matching_rule_wins() {
        let dom = parse_html(r#"<html><body><div class="box"></div></body></html>"#);
        // Two rules match; the later one must win (minimal cascade order).
        let rules = parse_stylesheet(
            "div.box { width: 100px; } \
             div.box { width: 250px; }",
        );
        let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
        compute(&mut layout, 800.0, 600.0).expect("compute layout");

        fn find(tree: &TaffyTree<Handle>, node: NodeId, target: &str) -> Option<NodeId> {
            let handle = tree.get_node_context(node)?;
            if let NodeData::Element { attrs, .. } = &handle.data {
                if attrs.borrow().iter().any(|a| &*a.value == target) {
                    return Some(node);
                }
            }
            tree.children(node).ok()?.into_iter().find_map(|c| find(tree, c, target))
        }

        let box_ = find(&layout.tree, layout.root, "box").expect("box node");
        let box_layout = layout.tree.layout(box_).expect("box layout");
        assert_eq!(box_layout.size.width, 250.0);
    }

    #[test]
    fn grid_container_stacks_children_in_rows() {
        let dom = parse_html(
            r#"<html><body><div class="grid">
                 <div class="cell"></div>
                 <div class="cell"></div>
               </div></body></html>"#,
        );
        let rules = parse_stylesheet(
            "div.grid { display: grid; width: 200px; height: 200px; } \
             div.cell { width: 50px; height: 50px; }",
        );
        let mut layout = build_layout_tree(&dom, &rules).expect("build layout tree");
        compute(&mut layout, 800.0, 600.0).expect("compute layout");

        fn find(tree: &TaffyTree<Handle>, node: NodeId, target: &str) -> Option<NodeId> {
            let handle = tree.get_node_context(node)?;
            if let NodeData::Element { attrs, .. } = &handle.data {
                if attrs.borrow().iter().any(|a| &*a.value == target) {
                    return Some(node);
                }
            }
            tree.children(node).ok()?.into_iter().find_map(|c| find(tree, c, target))
        }

        let grid = find(&layout.tree, layout.root, "grid").expect("grid node");
        let children = layout.tree.children(grid).expect("grid children");
        assert_eq!(children.len(), 2);
        // With no explicit columns, grid auto-places items in a single column,
        // so the second cell sits below the first.
        let first = layout.tree.layout(children[0]).expect("first cell");
        let second = layout.tree.layout(children[1]).expect("second cell");
        assert_eq!(first.location.x, second.location.x);
        assert!(second.location.y > first.location.y);
    }
}
