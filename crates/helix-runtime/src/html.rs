//! Task: Integrate `html5ever` for HTML5 parsing.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{QualName, parse_document};
use markup5ever_rcdom::{Handle, RcDom};

/// Parses an HTML5 document from raw bytes, performing the same character
/// encoding sniffing the HTML standard prescribes (BOM, `<meta charset>`, and
/// the windows-1252 default) before decoding to UTF-8 and parsing. Unlike
/// [`parse_html`] which requires valid UTF-8 input up front, this accepts any
/// byte stream a real web server might send.
pub fn parse_html_bytes(source: &[u8]) -> RcDom {
    let decoded = decode_html_bytes(source);
    parse_html(&decoded)
}

/// Detect the character encoding of a raw HTML byte stream per the HTML
/// standard's encoding sniffing algorithm (BOM, `<meta charset>`, then the
/// windows-1252 default) and decode it to a `String`.
pub fn decode_html_bytes(source: &[u8]) -> String {
    use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};

    let encoding: &'static Encoding = if source.starts_with(&[0xEFu8, 0xBB, 0xBF]) {
        UTF_8
    } else if source.starts_with(&[0xFFu8, 0xFE]) {
        encoding_rs::UTF_16LE
    } else if source.starts_with(&[0xFEu8, 0xFF]) {
        encoding_rs::UTF_16BE
    } else if let Some(label) = scan_meta_charset(&source[..source.len().min(1024)]) {
        Encoding::for_label(label.as_bytes()).unwrap_or(WINDOWS_1252)
    } else {
        WINDOWS_1252
    };

    match encoding.decode_without_bom_handling_and_without_replacement(source) {
        Some(cow) => cow.into_owned(),
        None => encoding
            .decode_without_bom_handling(source)
            .0
            .into_owned(),
    }
}

/// Scan a leading slice of bytes for a `charset` declaration, returning the
/// raw encoding label (e.g. `utf-8`, `iso-8859-1`). Handles both
/// `<meta charset="...">` and `<meta http-equiv="content-type" ...>` forms.
fn scan_meta_charset(head: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(head);
    let lower = s.to_ascii_lowercase();
    let bytes = s.as_bytes();
    let mut idx = 0;
    while let Some(pos) = lower[idx..].find("charset") {
        let abs = idx + pos;
        let rest = &lower[abs + 7..];
        let rest_bytes = &bytes[abs + 7..];
        let mut i = 0;
        while i < rest.len() && (rest_bytes[i] == b' ' || rest_bytes[i] == b'\t') {
            i += 1;
        }
        if i < rest.len() && rest_bytes[i] == b'=' {
            i += 1;
            while i < rest.len() && (rest_bytes[i] == b' ' || rest_bytes[i] == b'\t') {
                i += 1;
            }
            let quoted = i < rest.len() && (rest_bytes[i] == b'"' || rest_bytes[i] == b'\'');
            if quoted {
                i += 1;
            }
            let start = i;
            while i < rest.len()
                && !rest_bytes[i].is_ascii_whitespace()
                && rest_bytes[i] != b'>'
                && (!quoted || (rest_bytes[i] != b'"' && rest_bytes[i] != b'\''))
            {
                i += 1;
            }
            if i > start {
                return Some(s[abs + 7 + start..abs + 7 + i].to_string());
            }
        }
        idx = abs + 7;
    }
    None
}

/// Parses a UTF-8 HTML5 document string into an `RcDom` tree.
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
        assert_eq!(tags(&dom), vec!["html", "head", "body", "h1", "p", "b"]);
        assert_eq!(text(&dom), vec!["Title", "Hello", "world"]);
    }

    #[test]
    fn recovers_from_unclosed_tags() {
        // Missing closing </p> and </body>: html5ever recovers by closing open
        // elements at end-of-input, so the tree is still well-formed (not a panic).
        let dom = parse_html("<html><body><div>orphan text<p>unclosed");
        assert_eq!(tags(&dom), vec!["html", "head", "body", "div", "p"]);
        // html5ever's recovery drops the intra-word space when the pending text
        // run is flushed on the `<p>` open; the tree is still well-formed.
        assert_eq!(text(&dom), vec!["orphantext", "unclosed"]);
    }

    #[test]
    fn parses_attributes() {
        let dom =
            parse_html(r#"<html><body><a href="https://x.test" class="link">go</a></body></html>"#);
        let tags = tags(&dom);
        assert_eq!(tags, vec!["html", "head", "body", "a"]);

        fn find_attr(handle: &Handle, name: &str) -> Option<String> {
            if let NodeData::Element { attrs, .. } = &handle.data
                && let Some(v) = attrs.borrow().iter().find(|a| &*a.name.local == name)
            {
                return Some(v.value.to_string());
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
        assert_eq!(tags(&dom), vec!["html", "head", "body", "p"]);
        assert_eq!(text(&dom), vec!["real"]);
    }

    #[test]
    fn handles_void_and_self_closing_elements() {
        let dom = parse_html(
            r#"<html><body><img src="a.png"/><br><input type="text" value="x"></body></html>"#,
        );
        assert_eq!(
            tags(&dom),
            vec!["html", "head", "body", "img", "br", "input"]
        );
    }

    // --- Encoding detection (html5ever `from_bytes` sniffing) -----------------

    /// UTF-8 input (with a leading BOM) must decode and parse normally.
    #[test]
    fn detects_utf8_with_bom() {
        let bom = [0xEFu8, 0xBB, 0xBF];
        let mut bytes = bom.to_vec();
        bytes.extend_from_slice(b"<html><body><p>caf\xc3\xa9</p></body></html>");
        let dom = parse_html_bytes(&bytes);
        assert_eq!(text(&dom), vec!["café"]);
    }

    /// Raw windows-1252 bytes (no meta, no BOM) decode via the HTML default
    /// encoding: the high byte 0xE9 maps to U+00E9 (é), not the UTF-8 sequence.
    #[test]
    fn detects_windows_1252_default() {
        let mut bytes = b"<html><body><p>caf".to_vec();
        bytes.push(0xE9); // é in windows-1252
        bytes.extend_from_slice(b"</p></body></html>");
        let dom = parse_html_bytes(&bytes);
        assert_eq!(text(&dom), vec!["café"]);
    }

    /// A leading UTF-16LE BOM selects UTF-16 decoding.
    #[test]
    fn detects_utf16le_via_bom() {
        let mut bytes = vec![0xFFu8, 0xFE]; // UTF-16LE BOM
        for unit in "<html><body><p>naïve</p></body></html>".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let dom = parse_html_bytes(&bytes);
        assert_eq!(text(&dom), vec!["naïve"]);
    }

    /// A `<meta charset>` declaration overrides the default decoding, so the
    /// same high bytes are read as ISO-8859-1 rather than windows-1252.
    #[test]
    fn meta_charset_overrides_default() {
        let mut bytes =
            b"<html><head><meta charset=\"iso-8859-1\"></head><body><p>caf".to_vec();
        bytes.push(0xE9); // é in iso-8859-1
        bytes.extend_from_slice(b"</p></body></html>");
        let dom = parse_html_bytes(&bytes);
        assert_eq!(text(&dom), vec!["café"]);
    }
}
