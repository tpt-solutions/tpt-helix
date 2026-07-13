//! Stage S2 (Transpile) — P1 pattern: static content sites.
//!
//! `transpile_static_site` takes a static HTML document (blogs, documentation,
//! marketing pages — the lowest-complexity migration pattern, spec §6.2 P1) and
//! emits a [`TranspiledSite`]: a structured list of [`DomOp`]s plus the
//! generated guest Rust source and a WIT `world` snippet that rebuild the same
//! DOM through the Helix `dom` capability interface (see `wit/helix.wit`).
//!
//! The emitted op list is the contract Stage S3 (Validate) checks for
//! equivalence: the number of element/text operations must match the source
//! tree, and concatenation of text runs must be preserved byte-for-byte.
//!
//! Text-modeling note: the `dom` interface has no dedicated text node, so a
//! text run becomes either `set-text` on its element (when the element has only
//! that text child) or a `<span>` child carrying the text (mixed content). This
//! is a faithful, renderer-agnostic representation of the static content.

use std::fmt::Write;

/// A node in the parsed static HTML tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    /// Non-whitespace text run (whitespace-only runs are dropped on parse).
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// A single DOM operation in the transpiled output, in execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomOp {
    /// `let <var> = dom::create_element(&"<tag>");`
    Create { var: String, tag: String },
    /// `dom::set_text(<var>, &"<text>");`
    Text { var: String, text: String },
    /// `dom::set_attribute(<var>, &"<name>", &"<value>");`
    Attr { var: String, name: String, value: String },
    /// `dom::append_child(<parent>, <child>);`
    Append { parent: String, child: String },
    /// `dom::on_click(<var>, <handler>);`
    OnClick { var: String, handler: u64 },
}

/// The full transpilation output for one static site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranspiledSite {
    /// Ordered DOM operations rebuilding the page.
    pub ops: Vec<DomOp>,
    /// Generated guest Rust source (componentizes against `helix-guest`).
    pub rust_source: String,
    /// Generated WIT `world` snippet the guest is built from.
    pub wit_world: String,
}

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void(tag: &str) -> bool {
    VOID_TAGS.contains(&tag)
}

/// Parse a static HTML document into a node forest (top-level nodes).
///
/// Tolerant of comments and doctype declarations (skipped). Whitespace-only
/// text runs are discarded so equivalence checks operate on meaningful content.
pub fn parse_html(input: &str) -> Vec<Node> {
    let chars: Vec<char> = input.chars().collect();
    let (nodes, _) = parse_nodes(&chars, 0, None);
    nodes
}

fn parse_nodes(chars: &[char], mut pos: usize, stop: Option<&str>) -> (Vec<Node>, usize) {
    let mut nodes = Vec::new();
    while pos < chars.len() {
        if chars[pos] == '<' {
            // Comment / doctype / declaration.
            if pos + 1 < chars.len() && (chars[pos + 1] == '!' || chars[pos + 1] == '?') {
                if pos + 3 < chars.len() && &chars[pos + 1..pos + 4] == &['!', '-', '-'] {
                    if let Some(end) = find_substr(chars, pos + 4, "-->") {
                        pos = end;
                        continue;
                    }
                }
                if let Some(end) = find_char(chars, pos + 1, '>') {
                    pos = end + 1;
                    continue;
                }
                pos += 1;
                continue;
            }
            // Closing tag.
            if pos + 1 < chars.len() && chars[pos + 1] == '/' {
                let (name, end) = read_tag_name(chars, pos + 2);
                // `end` points at '>'; advance past it so the caller does not
                // treat the '>' as a stray text node.
                if let Some(stop) = stop {
                    if name.eq_ignore_ascii_case(stop) {
                        return (nodes, end + 1);
                    }
                }
                pos = end + 1;
                continue;
            }
            // Opening tag.
            let (el, end, self_close) = parse_element(chars, pos);
            pos = end;
            if self_close || is_void(&el.tag) {
                nodes.push(Node::Element(el));
                continue;
            }
            let (children, after) = parse_nodes(chars, pos, Some(&el.tag));
            pos = after;
            let mut el = el;
            el.children = children;
            nodes.push(Node::Element(el));
        } else {
            let (text, end) = read_text(chars, pos);
            pos = end;
            let trimmed: String = text.split_whitespace().collect();
            if !trimmed.is_empty() {
                nodes.push(Node::Text(trimmed));
            }
        }
    }
    (nodes, pos)
}

fn parse_element(chars: &[char], start: usize) -> (Element, usize, bool) {
    let (tag, mut pos) = read_tag_name(chars, start + 1);
    let mut attrs = Vec::new();
    let mut self_close = false;
    while pos < chars.len() {
        match chars[pos] {
            '>' => {
                pos += 1;
                break;
            }
            '/' => {
                self_close = true;
                if pos + 1 < chars.len() && chars[pos + 1] == '>' {
                    pos += 2;
                    break;
                }
                pos += 1;
            }
            c if c.is_whitespace() => pos += 1,
            _ => {
                let (name, val, end) = read_attr(chars, pos);
                pos = end;
                attrs.push((name, val));
            }
        }
    }
    (Element { tag, attrs, children: Vec::new() }, pos, self_close)
}

fn read_tag_name(chars: &[char], start: usize) -> (String, usize) {
    let mut pos = start;
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    let begin = pos;
    while pos < chars.len() && !chars[pos].is_whitespace() && chars[pos] != '>' && chars[pos] != '/' {
        pos += 1;
    }
    (chars[begin..pos].iter().collect(), pos)
}

fn read_attr(chars: &[char], start: usize) -> (String, String, usize) {
    let begin = start;
    let mut pos = start;
    while pos < chars.len() && !chars[pos].is_whitespace() && chars[pos] != '=' && chars[pos] != '>' && chars[pos] != '/' {
        pos += 1;
    }
    let name: String = chars[begin..pos].iter().collect();
    // Skip whitespace before '='.
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    let mut value = String::new();
    if pos < chars.len() && chars[pos] == '=' {
        pos += 1;
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos < chars.len() && (chars[pos] == '"' || chars[pos] == '\'') {
            let q = chars[pos];
            pos += 1;
            let vbegin = pos;
            while pos < chars.len() && chars[pos] != q {
                pos += 1;
            }
            value = chars[vbegin..pos].iter().collect();
            if pos < chars.len() {
                pos += 1; // closing quote
            }
        } else {
            // Unquoted value.
            let vbegin = pos;
            while pos < chars.len() && !chars[pos].is_whitespace() && chars[pos] != '>' && chars[pos] != '/' {
                pos += 1;
            }
            value = chars[vbegin..pos].iter().collect();
        }
    }
    (name, value, pos)
}

fn read_text(chars: &[char], start: usize) -> (String, usize) {
    let begin = start;
    let mut pos = start;
    while pos < chars.len() && chars[pos] != '<' {
        pos += 1;
    }
    (chars[begin..pos].iter().collect(), pos)
}

fn find_char(chars: &[char], start: usize, target: char) -> Option<usize> {
    (start..chars.len()).find(|&i| chars[i] == target)
}

fn find_substr(chars: &[char], start: usize, pat: &str) -> Option<usize> {
    let pat: Vec<char> = pat.chars().collect();
    if pat.is_empty() {
        return Some(start);
    }
    (start..chars.len())
        .find(|&i| chars[i..].starts_with(&pat))
        .map(|i| i + pat.len())
}

/// Count element and text nodes in a parsed forest (whitespace text excluded).
pub fn count_nodes(nodes: &[Node]) -> (usize, usize) {
    let mut elements = 0;
    let mut texts = 0;
    fn rec(nodes: &[Node], e: &mut usize, t: &mut usize) {
        for n in nodes {
            match n {
                Node::Element(el) => {
                    *e += 1;
                    rec(&el.children, e, t);
                }
                Node::Text(_) => *t += 1,
            }
        }
    }
    rec(nodes, &mut elements, &mut texts);
    (elements, texts)
}

/// Collect text runs in document order.
pub fn collect_text(nodes: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    fn rec(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Element(el) => rec(&el.children, out),
                Node::Text(s) => out.push(s.clone()),
            }
        }
    }
    rec(nodes, &mut out);
    out
}

fn rust_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

struct Emitter {
    ops: Vec<DomOp>,
    counter: usize,
}

impl Emitter {
    fn next_var(&mut self) -> String {
        let v = format!("el{}", self.counter);
        self.counter += 1;
        v
    }

    fn emit(&mut self, nodes: &[Node], parent: Option<&str>) {
        for node in nodes {
            match node {
                Node::Element(el) => {
                    let var = self.next_var();
                    self.ops.push(DomOp::Create {
                        var: var.clone(),
                        tag: el.tag.clone(),
                    });
                    for (name, value) in &el.attrs {
                        self.ops.push(DomOp::Attr {
                            var: var.clone(),
                            name: name.clone(),
                            value: value.clone(),
                        });
                    }
                    // Text handling: pure-text leaf -> set-text on element;
                    // mixed content -> span-wrapped text children.
                    let text_runs: Vec<&String> = el
                        .children
                        .iter()
                        .filter_map(|c| match c {
                            Node::Text(t) => Some(t),
                            _ => None,
                        })
                        .collect();
                    let has_elements = el.children.iter().any(|c| matches!(c, Node::Element(_)));
                    if text_runs.len() == 1 && !has_elements {
                        self.ops.push(DomOp::Text {
                            var: var.clone(),
                            text: text_runs[0].clone(),
                        });
                    } else {
                        for t in text_runs {
                            let span = self.next_var();
                            self.ops.push(DomOp::Create {
                                var: span.clone(),
                                tag: "span".to_string(),
                            });
                            self.ops.push(DomOp::Text {
                                var: span.clone(),
                                text: t.clone(),
                            });
                            self.ops.push(DomOp::Append {
                                parent: var.clone(),
                                child: span,
                            });
                        }
                    }
                    // Element children.
                    let child_elements: Vec<Node> = el
                        .children
                        .iter()
                        .filter(|c| matches!(c, Node::Element(_)))
                        .cloned()
                        .collect();
                    self.emit(&child_elements, Some(&var));
                    if let Some(p) = parent {
                        self.ops.push(DomOp::Append {
                            parent: p.to_string(),
                            child: var,
                        });
                    }
                }
                Node::Text(_) => {
                    // Top-level text (no parent) — wrap in a span root.
                    let span = self.next_var();
                    self.ops.push(DomOp::Create {
                        var: span.clone(),
                        tag: "span".to_string(),
                    });
                    if let Node::Text(t) = node {
                        self.ops.push(DomOp::Text {
                            var: span.clone(),
                            text: t.clone(),
                        });
                    }
                }
            }
        }
    }

    fn generate_rust(&self) -> String {
        let mut s = String::new();
        s.push_str("wit_bindgen::generate!({\n    world: \"helix-guest\",\n    path: \"../helix-wit/wit\",\n});\n\n");
        s.push_str("#[unsafe(no_mangle)]\npub extern \"C\" fn run() {\n    use helix::runtime::dom;\n");
        for op in &self.ops {
            match op {
                DomOp::Create { var, tag } => {
                    let _ = writeln!(s, "    let {var} = dom::create_element(&\"{}\");", rust_escape(tag));
                }
                DomOp::Text { var, text } => {
                    let _ = writeln!(s, "    dom::set_text({var}, &\"{}\");", rust_escape(text));
                }
                DomOp::Attr { var, name, value } => {
                    let _ = writeln!(
                        s,
                        "    dom::set_attribute({var}, &\"{}\", &\"{}\");",
                        rust_escape(name),
                        rust_escape(value)
                    );
                }
                DomOp::Append { parent, child } => {
                    let _ = writeln!(s, "    dom::append_child({parent}, {child});");
                }
                DomOp::OnClick { var, handler } => {
                    let _ = writeln!(s, "    dom::on_click({var}, {handler});");
                }
            }
        }
        s.push_str("}\n");
        s
    }
}

/// Transpile a static HTML document into a Helix guest component.
pub fn transpile_static_site(html: &str) -> TranspiledSite {
    let nodes = parse_html(html);
    let mut emitter = Emitter { ops: Vec::new(), counter: 0 };
    emitter.emit(&nodes, None);
    let ops = emitter.ops.clone();
    let rust_source = emitter.generate_rust();
    let wit_world = r#"world helix-guest {
    import dom;
    export run: func();
}
"#
    .to_string();
    TranspiledSite {
        ops,
        rust_source,
        wit_world,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_elements_and_attributes() {
        let html = r#"<html><head><title>T</title></head><body class="x"><p>Hi <b>there</b></p></body></html>"#;
        let nodes = parse_html(html);
        let (elements, texts) = count_nodes(&nodes);
        assert_eq!(elements, 6); // html, head, title, body, p, b
        assert_eq!(texts, 3); // T, Hi, there
        let text = collect_text(&nodes);
        assert_eq!(text, vec!["T", "Hi", "there"]);
    }

    #[test]
    fn skips_comments_and_doctype_and_whitespace() {
        let html = r#"<!doctype html><!-- ignore --><div>   </div><p>real</p>"#;
        let nodes = parse_html(html);
        let (elements, texts) = count_nodes(&nodes);
        assert_eq!(elements, 2); // div, p
        assert_eq!(texts, 1); // real
    }

    #[test]
    fn transpile_emits_ops_matching_source_shape() {
        let html = r#"<body><h1>Title</h1><p>Hello <b>world</b></p></body>"#;
        let site = transpile_static_site(html);
        let (elements, texts) = count_nodes(&parse_html(html));
        // Each element yields one Create; mixed-content text runs add a span.
        let creates = site.ops.iter().filter(|o| matches!(o, DomOp::Create { .. })).count();
        let set_texts = site.ops.iter().filter(|o| matches!(o, DomOp::Text { .. })).count();
        assert!(creates >= elements, "every element must be created");
        // Every text run maps to exactly one set_text (on element or span).
        assert_eq!(set_texts, texts, "text runs must be preserved 1:1");
        // Every Append has a defined parent (parent index < child index).
        let mut created = std::collections::HashSet::new();
        for op in &site.ops {
            if let DomOp::Create { var, .. } = op {
                created.insert(var.clone());
            }
            if let DomOp::Append { parent, child } = op {
                assert!(created.contains(parent), "append references undefined parent");
                assert!(created.contains(child), "append references undefined child");
            }
        }
    }

    #[test]
    fn generated_rust_is_well_formed() {
        let site = transpile_static_site(r#"<div class="a"><span>hi</span></div>"#);
        assert!(site.rust_source.contains("wit_bindgen::generate!"));
        assert!(site.rust_source.contains("pub extern \"C\" fn run()"));
        assert!(site.rust_source.contains("dom::create_element(&\"div\")"));
        assert!(site.rust_source.contains("dom::set_attribute(el0, &\"class\", &\"a\")"));
        assert!(site.rust_source.contains("dom::append_child(el0, el1)"));
        assert!(site.wit_world.contains("export run: func()"));
    }
}
