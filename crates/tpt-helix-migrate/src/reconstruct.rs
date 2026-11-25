//! Rebuilds the HTML a generated guest produces, from its Stage S2 DOM operations
//! (spec §10.3). The confidence report renders this and the original page, then
//! compares the pixels. Anything the guest does not carry (CSS, scripts, and
//! handler bodies) is missing from the rebuild, so it shows up as a difference.

use std::collections::HashMap;
use std::fmt::Write;

use crate::transpile::{DomOp, TranspiledSite};

/// Elements with no closing tag.
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

#[derive(Default)]
struct Node {
    tag: String,
    attrs: Vec<(String, String)>,
    text: Option<String>,
    children: Vec<String>,
}

/// The HTML document the guest's operations build, serialized in creation order.
///
/// Elements appear in the order they were created. A root is an element that is
/// never appended to another element. Event-handler operations add no markup, so
/// they are skipped.
pub fn reconstruct_html(site: &TranspiledSite) -> String {
    let mut nodes: HashMap<&str, Node> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    let mut attached: Vec<&str> = Vec::new();
    for op in &site.ops {
        match op {
            DomOp::Create { var, tag } => {
                nodes.insert(
                    var,
                    Node {
                        tag: tag.to_ascii_lowercase(),
                        ..Node::default()
                    },
                );
                order.push(var);
            }
            DomOp::Text { var, text } => {
                if let Some(n) = nodes.get_mut(var.as_str()) {
                    n.text = Some(text.clone());
                }
            }
            DomOp::Attr { var, name, value } => {
                if let Some(n) = nodes.get_mut(var.as_str()) {
                    n.attrs.push((name.clone(), value.clone()));
                }
            }
            DomOp::Append { parent, child } => {
                if let Some(n) = nodes.get_mut(parent.as_str()) {
                    n.children.push(child.clone());
                }
                attached.push(child);
            }
            // Handlers add no markup.
            DomOp::OnClick { .. }
            | DomOp::OnSubmit { .. }
            | DomOp::OnChange { .. }
            | DomOp::OnInput { .. }
            | DomOp::OnPlay { .. }
            | DomOp::OnPause { .. }
            | DomOp::OnEnded { .. } => {}
        }
    }

    let mut out = String::new();
    for var in &order {
        if !attached.contains(var) {
            write_node(&nodes, var, &mut out);
        }
    }
    out
}

fn write_node(nodes: &HashMap<&str, Node>, var: &str, out: &mut String) {
    let Some(node) = nodes.get(var) else {
        return;
    };
    let _ = write!(out, "<{}", node.tag);
    for (name, value) in &node.attrs {
        let _ = write!(out, " {}=\"{}\"", name, escape(value));
    }
    out.push('>');
    if VOID_TAGS.contains(&node.tag.as_str()) {
        return;
    }
    if let Some(text) = &node.text {
        out.push_str(&escape(text));
    }
    for child in &node.children {
        write_node(nodes, child, out);
    }
    let _ = write!(out, "</{}>", node.tag);
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transpile::transpile_static_site;

    #[test]
    fn rebuilds_element_tree_in_order() {
        let site = transpile_static_site("<h1>Hello</h1><p>World</p>");
        let html = reconstruct_html(&site);
        assert!(html.contains("<h1>"), "{html}");
        assert!(html.contains("Hello"), "{html}");
        assert!(html.find("<h1>") < html.find("<p>"), "{html}");
    }

    #[test]
    fn style_bodies_are_not_carried_into_the_rebuild() {
        // The guest has no CSS: style bodies are dropped (`Node::Raw`), so the
        // rebuild must not contain them.
        let site = transpile_static_site("<style>p { color: red; }</style><p>x</p>");
        assert!(!reconstruct_html(&site).contains("color: red"));
    }

    #[test]
    fn text_and_attribute_values_are_escaped() {
        let site = TranspiledSite {
            ops: vec![
                DomOp::Create {
                    var: "el0".into(),
                    tag: "a".into(),
                },
                DomOp::Attr {
                    var: "el0".into(),
                    name: "title".into(),
                    value: "a \"b\" & c".into(),
                },
                DomOp::Text {
                    var: "el0".into(),
                    text: "1 < 2".into(),
                },
            ],
            ..empty_site()
        };
        let html = reconstruct_html(&site);
        assert_eq!(html, "<a title=\"a &quot;b&quot; &amp; c\">1 &lt; 2</a>");
    }

    #[test]
    fn void_elements_have_no_closing_tag() {
        let site = TranspiledSite {
            ops: vec![DomOp::Create {
                var: "el0".into(),
                tag: "input".into(),
            }],
            ..empty_site()
        };
        assert_eq!(reconstruct_html(&site), "<input>");
    }

    fn empty_site() -> TranspiledSite {
        TranspiledSite {
            ops: vec![],
            handlers: vec![],
            rust_source: String::new(),
            wit_world: String::new(),
            crud: Default::default(),
            dataviz: Default::default(),
            media: Default::default(),
            realtime: Default::default(),
            spa: Default::default(),
        }
    }
}
