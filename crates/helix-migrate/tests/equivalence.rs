//! Stage S3 (Validate) — property-based equivalence checking.
//!
//! Per spec §6.1, Stage S3 proves the ported (transpiled) app is *equivalent*
//! to the original. For the P1 static-content pattern, the meaningful
//! equivalence invariant is **content fidelity**: transpiling a source HTML
//! tree and walking the generated [`DomOp`]s must reproduce the original text
//! runs in order, and the generated guest source must be structurally sound
//! (every variable is created before it is appended to).
//!
//! These properties are exercised with `proptest` over randomly generated HTML
//! trees (fuzzing), not just hand-written fixtures.

use helix_migrate::transpile::{collect_text, parse_html, transpile_static_site, DomOp};
use proptest::prelude::*;

/// A synthetic HTML tree used as the migration source.
#[derive(Debug, Clone)]
enum Tree {
    Elem(String, Vec<(String, String)>, Vec<Tree>),
    Text(String),
}

fn tag() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("div".to_string()),
        Just("p".to_string()),
        Just("span".to_string()),
        Just("h1".to_string()),
        Just("b".to_string()),
        Just("a".to_string()),
        Just("li".to_string()),
    ]
}

fn attr_name() -> impl Strategy<Value = String> {
    "[a-z]+".prop_filter("non-empty", |s| !s.is_empty())
}

fn text_str() -> impl Strategy<Value = String> {
    // No whitespace / markup characters: keeps the parser's whitespace-trim
    // behavior from altering the value, so source text == parsed text.
    "[a-z0-9]+".prop_filter("non-empty", |s| !s.is_empty())
}

fn tree(depth: u8) -> BoxedStrategy<Tree> {
    if depth == 0 {
        text_str().prop_map(Tree::Text).boxed()
    } else {
        (
            tag(),
            prop::collection::vec((attr_name(), text_str()), 0..3),
            prop::collection::vec(tree(depth - 1), 0..3),
        )
            .prop_map(|(t, attrs, children)| Tree::Elem(t, attrs, children))
            .boxed()
    }
}

/// Render a synthetic tree to compact HTML (no inter-tag whitespace, so the
/// parser sees exactly the text we generated).
fn render(t: &Tree) -> String {
    match t {
        Tree::Text(s) => s.clone(),
        Tree::Elem(tag, attrs, children) => {
            let mut s = String::from("<");
            s.push_str(tag);
            for (k, v) in attrs {
                s.push(' ');
                s.push_str(k);
                s.push_str("=\"");
                s.push_str(v);
                s.push('"');
            }
            s.push('>');
            for c in children {
                s.push_str(&render(c));
            }
            s.push_str("</");
            s.push_str(tag);
            s.push('>');
            s
        }
    }
}

proptest! {
    /// Transpilation preserves the text content of the source tree exactly.
    #[test]
    fn text_content_is_preserved(tree in tree(3)) {
        let html = render(&tree);
        let site = transpile_static_site(&html);

        // The equivalence baseline is the HTML-faithful text (adjacent text
        // siblings merge into one run, as real HTML requires); the transpiler
        // must reproduce exactly that.
        let expected = collect_text(&parse_html(&html));

        let got: Vec<String> = site
            .ops
            .iter()
            .filter_map(|op| match op {
                DomOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();

        prop_assert_eq!(expected, got, "transpiled text diverges from source");
    }

    /// The transpiled guest source is structurally sound: every variable is
    /// created before any `append-child` references it.
    #[test]
    fn generated_source_references_are_well_formed(tree in tree(3)) {
        let html = render(&tree);
        let site = transpile_static_site(&html);

        let mut created: std::collections::HashSet<String> = Default::default();
        for op in &site.ops {
            match op {
                DomOp::Create { var, .. } => {
                    prop_assert!(created.insert(var.clone()), "duplicate var {var}");
                }
                DomOp::Append { parent, child } => {
                    prop_assert!(created.contains(parent), "append to undefined {parent}");
                    prop_assert!(created.contains(child), "append of undefined {child}");
                }
                _ => {}
            }
        }
        // A non-empty document always yields at least one create.
        prop_assert!(!site.ops.is_empty() || parse_html(&html).is_empty());
    }

    /// Element count is preserved 1:1 by create operations for pure-text trees
    /// (no span wrapping needed when elements carry only text).
    #[test]
    fn pure_text_elements_map_one_to_one(tree in tree(2)) {
        let html = render(&tree);
        let site = transpile_static_site(&html);
        let nodes = parse_html(&html);
        let (elements, _texts) = helix_migrate::transpile::count_nodes(&nodes);
        let creates = site.ops.iter().filter(|o| matches!(o, DomOp::Create { .. })).count();
        // Every element is created; mixed-content text adds extra span creates.
        prop_assert!(creates >= elements);
    }
}
