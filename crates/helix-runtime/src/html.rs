//! Task: Integrate `html5ever` for HTML5 parsing.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{QualName, parse_document};
use markup5ever_rcdom::{Handle, RcDom};

/// Parses an HTML5 document string into an `RcDom` tree.
pub fn parse_html(source: &str) -> RcDom {
    parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut source.as_bytes())
        .expect("html5ever parsing is infallible for well-formed byte streams")
}

/// Returns the qualified name of the root `<html>` element, if present.
pub fn root_element_name(dom: &RcDom) -> Option<QualName> {
    fn find(handle: &Handle) -> Option<QualName> {
        use markup5ever_rcdom::NodeData;
        if let NodeData::Element { name, .. } = &handle.data {
            return Some(name.clone());
        }
        handle.children.borrow().iter().find_map(find)
    }
    find(&dom.document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use markup5ever_rcdom::{Handle, NodeData};

    /// All element local names in document order (depth-first, pre-order).
    fn collect_tags(handle: &Handle, out: &mut Vec<String>) {
        if let NodeData::Element { name, .. } = &handle.data {
            out.push(name.local.to_string());
        }
        for child in handle.children.borrow().iter() {
            collect_tags(child, out);
        }
    }

    fn tags(dom: &RcDom) -> Vec<String> {
        let mut out = Vec::new();
        collect_tags(&dom.document, &mut out);
        out
    }

    /// All non-whitespace text content in document order.
    fn collect_text(handle: &Handle, out: &mut Vec<String>) {
        if let NodeData::Text { contents } = &handle.data {
            let t: String = contents.borrow().chars().collect();
            let trimmed: String = t.split_whitespace().collect();
            if !trimmed.is_empty() {
                out.push(trimmed);
            }
        }
        for child in handle.children.borrow().iter() {
            collect_text(child, out);
        }
    }

    fn text(dom: &RcDom) -> Vec<String> {
        let mut out = Vec::new();
        collect_text(&dom.document, &mut out);
        out
    }

    #[test]
    fn parses_minimal_document() {
        let dom = parse_html("<html><body><p>hi</p></body></html>");
        let name = root_element_name(&dom).expect("root element");
        assert_eq!(&*name.local, "html");
    }

    #[test]
    fn parses_nested_elements_and_text() {
        let dom = parse_html("<html><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>");
        assert_eq!(tags(&dom), vec!["html", "body", "h1", "p", "b"]);
        assert_eq!(text(&dom), vec!["Title", "Hello", "world"]);
    }

    #[test]
    fn recovers_from_unclosed_tags() {
        // Missing closing </p> and </body>: html5ever recovers by closing open
        // elements at end-of-input, so the tree is still well-formed (not a panic).
        let dom = parse_html("<html><body><div>orphan text<p>unclosed");
        assert_eq!(tags(&dom), vec!["html", "body", "div", "p"]);
        assert_eq!(text(&dom), vec!["orphan", "text", "unclosed"]);
    }

    #[test]
    fn parses_attributes() {
        let dom =
            parse_html(r#"<html><body><a href="https://x.test" class="link">go</a></body></html>"#);
        let tags = tags(&dom);
        assert_eq!(tags, vec!["html", "body", "a"]);

        fn find_attr(handle: &Handle, name: &str) -> Option<String> {
            if let NodeData::Element { attrs, .. } = &handle.data {
                return attrs
                    .borrow()
                    .iter()
                    .find(|a| &*a.name.local == name)
                    .map(|a| a.value.to_string());
            }
            for child in handle.children.borrow().iter() {
                if let Some(v) = find_attr(child, name) {
                    return Some(v);
                }
            }
            None
        }
        let href = find_attr(&dom.document, "href").expect("href attr");
        assert_eq!(href, "https://x.test");
        assert_eq!(find_attr(&dom.document, "class"), Some("link".to_string()));
    }

    #[test]
    fn ignores_doctype_and_comments() {
        // Doctype and comments are non-element nodes and must not appear as tags.
        let dom =
            parse_html("<!doctype html><!-- a comment --><html><body><p>real</p></body></html>");
        assert_eq!(tags(&dom), vec!["html", "body", "p"]);
        assert_eq!(text(&dom), vec!["real"]);
    }

    #[test]
    fn handles_void_and_self_closing_elements() {
        let dom = parse_html(
            r#"<html><body><img src="a.png"/><br><input type="text" value="x"></body></html>"#,
        );
        assert_eq!(tags(&dom), vec!["html", "body", "img", "br", "input"]);
    }
}
