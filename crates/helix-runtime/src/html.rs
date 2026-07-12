//! Task: Integrate `html5ever` for HTML5 parsing.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, QualName};
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

    #[test]
    fn parses_minimal_document() {
        let dom = parse_html("<html><body><p>hi</p></body></html>");
        let name = root_element_name(&dom).expect("root element");
        assert_eq!(&*name.local, "html");
    }
}
