//! The `egui` ↔ `taffy` bridge.
//!
//! A `HelixDocument` parses HTML/CSS, builds a `taffy` layout tree, and emits a
//! flat list of [`RenderItem`]s for a given surface size. This is the seam
//! between AppFront's `egui` widget tree (the host chrome) and the Helix
//! `taffy` layout tree (the web content).

use std::collections::HashMap;

use markup5ever_rcdom::{Handle, NodeData};
use taffy::{
    AvailableSpace, Dimension, Display, FlexDirection, NodeId, Rect, Size, Style, TaffyTree,
};

use crate::css::{
    StyleProps, resolve_element_props, {self},
};
use crate::render::{RectItem, RenderItem, TextItem};

/// Tags that default to block-level formatting if no `display` is set.
const BLOCK_TAGS: &[&str] = &[
    "html",
    "body",
    "div",
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "header",
    "footer",
    "main",
    "section",
    "article",
    "nav",
    "blockquote",
    "pre",
];

/// Per-node paint metadata attached during tree construction.
#[derive(Debug, Clone, Default)]
struct RenderMeta {
    /// Pre-order index of the DOM node (elements and text), used for hit testing.
    node: usize,
    /// Lower-case tag name for elements; `#text` for text nodes.
    tag: String,
    background: Option<[u8; 4]>,
    border_color: Option<[u8; 4]>,
    border_width: f32,
    radius: f32,
    color: [u8; 4],
    text: Option<TextMeta>,
}

#[derive(Debug, Clone)]
struct TextMeta {
    content: String,
    size: f32,
}

/// A parsed Helix document ready to be laid out and rendered.
pub struct HelixDocument {
    source: String,
    stylesheet: Vec<(css::Selector, StyleProps)>,
}

/// A built `taffy` tree plus the metadata needed to emit render items.
pub struct BuiltTree {
    tree: TaffyTree<()>,
    root: NodeId,
    metas: HashMap<NodeId, RenderMeta>,
    avail_w: f32,
    avail_h: f32,
}

impl HelixDocument {
    /// Parses `source`, extracting any `<style>` blocks for the stylesheet.
    pub fn parse(source: &str) -> Self {
        let css_body = css::extract_style_blocks(source);
        let stylesheet = css::parse_stylesheet(&css_body);
        HelixDocument {
            source: source.to_string(),
            stylesheet,
        }
    }

    /// Builds the `taffy` tree for a surface of `width`×`height`.
    pub fn build(&self, width: f32, height: f32) -> BuiltTree {
        let dom = tpt_helix_runtime::html::parse_html(&self.source);
        let mut tree: TaffyTree<()> = TaffyTree::new();
        let mut metas: HashMap<NodeId, RenderMeta> = HashMap::new();

        let root_style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            size: Size {
                width: Dimension::Auto,
                height: Dimension::Auto,
            },
            ..Default::default()
        };
        let root = tree.new_leaf(root_style).unwrap();

        let mut next_node = 0usize;
        let top: Vec<NodeId> = dom
            .document
            .children
            .borrow()
            .iter()
            .filter_map(|h| {
                build_node(
                    &mut tree,
                    h.clone(),
                    self,
                    width,
                    [0, 0, 0, 255],
                    &mut metas,
                    &mut next_node,
                )
            })
            .collect();
        tree.set_children(root, &top).unwrap();

        BuiltTree {
            tree,
            root,
            metas,
            avail_w: width,
            avail_h: height,
        }
    }

    /// Lays out and renders the document into a flat list of draw items for a
    /// surface of `width`×`height`.
    pub fn render(&self, width: f32, height: f32) -> Vec<RenderItem> {
        self.frame(width, height).items
    }

    /// Lays out the document for a `width`×`height` surface and returns both the
    /// paint items and the hit boxes for every element.
    pub fn frame(&self, width: f32, height: f32) -> Frame {
        let mut built = self.build(width, height);
        built.render_frame()
    }
}

/// One laid-out DOM box, reported for every element and text node (painted or not).
#[derive(Debug, Clone, PartialEq)]
pub struct HitBox {
    /// Pre-order index of the DOM node.
    pub node: usize,
    /// Lower-case tag name (`#text` for text nodes).
    pub tag: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl HitBox {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// The output of one layout pass: what to paint and what can be clicked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub items: Vec<RenderItem>,
    /// Boxes in pre-order: a parent always precedes its children.
    pub hits: Vec<HitBox>,
}

impl Frame {
    /// The deepest box containing `(x, y)` in surface coordinates. Pre-order
    /// puts children after parents, so the last match is the innermost box
    /// (and among siblings, the one painted on top).
    pub fn hit_test(&self, x: f32, y: f32) -> Option<&HitBox> {
        self.hits.iter().rev().find(|b| b.contains(x, y))
    }
}

/// Reuses the last layout while the surface size is unchanged.
///
/// Building and laying out the tree re-parses the HTML, which is far more
/// expensive than painting. A window redraws every frame, so the cache keeps a
/// frame until the surface size changes.
#[derive(Debug, Default)]
pub struct FrameCache {
    size: Option<(f32, f32)>,
    frame: Frame,
    layouts: usize,
}

impl FrameCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the frame for `doc` at `width`×`height`, laying out only if the
    /// size differs from the cached one.
    pub fn frame(&mut self, doc: &HelixDocument, width: f32, height: f32) -> &Frame {
        if self.size != Some((width, height)) {
            self.frame = doc.frame(width, height);
            self.size = Some((width, height));
            self.layouts += 1;
        }
        &self.frame
    }

    /// How many layout passes have run (for tests and diagnostics).
    pub fn layouts(&self) -> usize {
        self.layouts
    }
}

impl BuiltTree {
    /// Computes layout and emits render items rooted at the surface origin.
    pub fn render_items(&mut self) -> Vec<RenderItem> {
        self.render_frame().items
    }

    /// Computes layout and emits paint items and hit boxes.
    pub fn render_frame(&mut self) -> Frame {
        self.tree
            .compute_layout(
                self.root,
                Size {
                    width: AvailableSpace::Definite(self.avail_w),
                    height: AvailableSpace::Definite(self.avail_h),
                },
            )
            .expect("taffy layout is infallible for finite available space");
        let mut frame = Frame::default();
        emit(
            &self.tree,
            self.root,
            &self.metas,
            0.0,
            0.0,
            &mut frame.items,
            &mut frame.hits,
        );
        frame
    }

    /// The number of nodes in the built tree (useful for tests).
    pub fn node_count(&self) -> usize {
        self.metas.len() + 1
    }
}

/// Recursively builds a `taffy` node from a DOM handle. `inherited_color` is
/// threaded down so text inherits its parent's `color`.
fn build_node(
    tree: &mut TaffyTree<()>,
    handle: Handle,
    doc: &HelixDocument,
    max_width: f32,
    inherited_color: [u8; 4],
    metas: &mut HashMap<NodeId, RenderMeta>,
    next_node: &mut usize,
) -> Option<NodeId> {
    match &handle.data {
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.to_string().to_ascii_lowercase();
            // Assigned before the children so the index is pre-order.
            let node_id = *next_node;
            *next_node += 1;
            let mut classes = Vec::new();
            let mut inline = String::new();
            for attr in attrs.borrow().iter() {
                let an = attr.name.local.to_string().to_ascii_lowercase();
                let av = attr.value.to_string();
                if an == "class" {
                    classes.extend(av.split_whitespace().map(|s| s.to_string()));
                } else if an == "style" {
                    inline = av;
                }
            }
            let props = resolve_element_props(&tag, &classes, &doc.stylesheet, &inline);
            let is_block = BLOCK_TAGS.contains(&tag.as_str())
                || props.display == Some(crate::css::DisplayKind::Block);
            let style = element_style(&props, is_block);

            let child_color = props.color.unwrap_or(inherited_color);
            let child_handles: Vec<Handle> = handle.children.borrow().iter().cloned().collect();
            let mut child_nodes = Vec::new();
            for ch in child_handles {
                if let Some(n) = build_node(tree, ch, doc, max_width, child_color, metas, next_node)
                {
                    child_nodes.push(n);
                }
            }

            let meta = RenderMeta {
                node: node_id,
                tag: tag.clone(),
                background: props.background,
                border_color: props.border_color,
                border_width: props.border_width.unwrap_or(0.0),
                radius: props.radius.unwrap_or(0.0),
                color: child_color,
                text: None,
            };
            let node = tree.new_with_children(style, &child_nodes).unwrap();
            metas.insert(node, meta);
            Some(node)
        }
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            let node_id = *next_node;
            *next_node += 1;
            let size = 16.0;
            let w = estimate_text_width(trimmed, size)
                .min(max_width - 4.0)
                .max(size);
            let h = size * 1.25;
            let style = Style {
                display: Display::Block,
                size: Size {
                    width: Dimension::Length(w),
                    height: Dimension::Length(h),
                },
                ..Default::default()
            };
            let meta = RenderMeta {
                node: node_id,
                tag: "#text".to_string(),
                color: inherited_color,
                text: Some(TextMeta {
                    content: trimmed.to_string(),
                    size,
                }),
                ..Default::default()
            };
            let node = tree.new_leaf(style).unwrap();
            metas.insert(node, meta);
            Some(node)
        }
        _ => None,
    }
}

/// Maps resolved CSS props to a `taffy` `Style`.
fn element_style(props: &StyleProps, is_block: bool) -> Style {
    let display = match props.display {
        Some(crate::css::DisplayKind::Block) => Display::Block,
        Some(crate::css::DisplayKind::Flex) => Display::Flex,
        Some(crate::css::DisplayKind::Grid) => Display::Flex,
        None => {
            if is_block {
                Display::Block
            } else {
                Display::Flex
            }
        }
    };

    let mut style = Style {
        display,
        ..Default::default()
    };

    style.size.width = match props.width {
        Some(w) if w < 0.0 => Dimension::Percent((-w) / 100.0),
        Some(w) => Dimension::Length(w),
        None if is_block => Dimension::Percent(1.0),
        None => Dimension::Auto,
    };
    style.size.height = match props.height {
        Some(h) if h < 0.0 => Dimension::Percent((-h) / 100.0),
        Some(h) => Dimension::Length(h),
        None => Dimension::Auto,
    };

    if let Some(p) = props.padding {
        style.padding = uniform(p);
    }
    if let Some(m) = props.margin {
        style.margin = uniform(m);
    }
    match props.flex_direction {
        Some(crate::css::FlexDir::Column) => style.flex_direction = FlexDirection::Column,
        Some(crate::css::FlexDir::Row) => style.flex_direction = FlexDirection::Row,
        None => {}
    }

    style
}

fn uniform<T: taffy::style_helpers::FromLength>(v: f32) -> Rect<T> {
    Rect {
        left: T::from_length(v),
        right: T::from_length(v),
        top: T::from_length(v),
        bottom: T::from_length(v),
    }
}

/// Rough monospace-ish width estimate used to size text leaves (no shaping yet).
fn estimate_text_width(text: &str, size: f32) -> f32 {
    (text.chars().count() as f32) * size * 0.55
}

/// Emits render items for `node` and its descendants in pre-order.
fn emit(
    tree: &TaffyTree<()>,
    node: NodeId,
    metas: &HashMap<NodeId, RenderMeta>,
    offset_x: f32,
    offset_y: f32,
    out: &mut Vec<RenderItem>,
    hits: &mut Vec<HitBox>,
) {
    let layout = match tree.layout(node) {
        Ok(l) => l,
        Err(_) => return,
    };
    let x = offset_x + layout.location.x;
    let y = offset_y + layout.location.y;
    let w = layout.size.width;
    let h = layout.size.height;

    if let Some(meta) = metas.get(&node) {
        hits.push(HitBox {
            node: meta.node,
            tag: meta.tag.clone(),
            x,
            y,
            w,
            h,
        });
        if let Some(bg) = meta.background {
            out.push(RenderItem::Rect(RectItem {
                x,
                y,
                w,
                h,
                fill: bg,
                border_color: meta.border_color,
                border_width: meta.border_width,
                radius: meta.radius,
            }));
        }
        if let Some(t) = &meta.text {
            out.push(RenderItem::Text(TextItem {
                x: x + 2.0,
                y: y + t.size * 0.2,
                text: t.content.clone(),
                color: meta.color,
                size: t.size,
            }));
        }
    }

    if let Ok(children) = tree.children(node) {
        for child in children {
            emit(tree, child, metas, x, y, out, hits);
        }
    }
}
