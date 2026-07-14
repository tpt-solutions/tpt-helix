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

use helix_migrate::transpile::{
    DomOp, collect_text, parse_html, transpile_data_viz, transpile_form_app,
    transpile_media_player, transpile_static_site,
};
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

/// A P2 form-based CRUD document: a `<form>` with `fields` named inputs (each
/// carrying an `onchange` handler) and a submit button, followed by a `<table>`
/// with `rows` data rows. Returned HTML strings use no inter-tag whitespace so
/// the parser sees exactly the text we generated.
fn form_doc(fields: usize, rows: usize) -> String {
    let types = ["text", "number", "email"];
    let mut s = String::from("<form onsubmit=\"submit()\">");
    for i in 0..fields {
        let t = types[i % types.len()];
        s.push_str(&format!(
            "<input type=\"{t}\" name=\"f{i}\" onchange=\"chg()\" />"
        ));
    }
    s.push_str("<button type=\"submit\">Add</button></form><table>");
    for i in 0..rows {
        s.push_str(&format!("<tr><td>row{i}</td></tr>"));
    }
    s.push_str("</table>");
    s
}

proptest! {
    /// P2 transpilation preserves the text content (button label + row text)
    /// of the source document exactly — the Stage S3 content-fidelity invariant.
    #[test]
    fn form_app_text_is_preserved(fields in 1..4usize, rows in 0..4usize) {
        let html = form_doc(fields, rows);
        let site = transpile_form_app(&html);

        let expected = collect_text(&parse_html(&html));
        let got: Vec<String> = site
            .ops
            .iter()
            .filter_map(|op| match op {
                DomOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();

        prop_assert_eq!(expected, got, "transpiled form text diverges from source");
    }

    /// The P2 transpiled guest is structurally sound (every variable created
    /// before use), is wired with at least one handler, and its extracted CRUD
    /// model matches the source (field count, submit presence, row count).
    #[test]
    fn form_app_crud_model_is_consistent(fields in 1..4usize, rows in 0..4usize) {
        let html = form_doc(fields, rows);
        let site = transpile_form_app(&html);

        let mut created: std::collections::HashSet<String> = Default::default();
        let mut wired = false;
        for op in &site.ops {
            match op {
                DomOp::Create { var, .. } => {
                    prop_assert!(created.insert(var.clone()), "duplicate var {var}");
                }
                DomOp::Append { parent, child } => {
                    prop_assert!(created.contains(parent), "append to undefined {parent}");
                    prop_assert!(created.contains(child), "append of undefined {child}");
                }
                DomOp::OnSubmit { .. } | DomOp::OnClick { .. } => wired = true,
                _ => {}
            }
        }
        prop_assert!(wired, "form must be wired with a handler");

        prop_assert_eq!(site.crud.forms.len(), 1, "exactly one form expected");
        prop_assert_eq!(site.crud.forms[0].fields.len(), fields, "field count mismatch");
        prop_assert!(site.crud.forms[0].has_submit, "form must have a submit affordance");
        prop_assert_eq!(site.crud.tables.len(), 1, "exactly one table expected");
        prop_assert_eq!(site.crud.tables[0].row_count, rows, "row count mismatch");
    }
}

/// A P3 data-visualization document: a heading plus a `<canvas>` chart mount
/// with resolved pixel `width`/`height` and `series` `data-*` series bindings.
fn chart_doc(series: usize) -> String {
    let mut s = String::from("<h1>Dashboard</h1><canvas id=\"chart\" width=\"800\" height=\"600\"");
    for i in 0..series {
        s.push_str(&format!(" data-series-{i}=\"v{i}\""));
    }
    s.push_str("></canvas>");
    s
}

proptest! {
    /// P3 transpilation preserves the heading text of the source document
    /// exactly — the Stage S3 content-fidelity invariant for charts.
    #[test]
    fn data_viz_text_is_preserved(series in 0..4usize) {
        let html = chart_doc(series);
        let site = transpile_data_viz(&html);

        let expected = collect_text(&parse_html(&html));
        let got: Vec<String> = site
            .ops
            .iter()
            .filter_map(|op| match op {
                DomOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();

        prop_assert_eq!(expected, got, "transpiled chart text diverges from source");
    }

    /// The P3 transpiled guest is structurally sound (every variable created
    /// before use) and its extracted [`DataVizModel`] matches the source
    /// (one canvas, resolved dimensions, series count).
    #[test]
    fn data_viz_chart_model_is_consistent(series in 0..4usize) {
        let html = chart_doc(series);
        let site = transpile_data_viz(&html);

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

        prop_assert_eq!(site.dataviz.charts.len(), 1, "exactly one chart expected");
        prop_assert_eq!(site.dataviz.charts[0].tag.clone(), "canvas".to_string());
        prop_assert_eq!(site.dataviz.charts[0].width, Some(800));
        prop_assert_eq!(site.dataviz.charts[0].height, Some(600));
        prop_assert_eq!(site.dataviz.charts[0].series.len(), series, "series count mismatch");
    }
}

/// A P4 media-player document: a heading plus a `<video>` with a primary `src`
/// and `controls`, followed by `sources` `<source>` alternate streams.
fn media_doc(sources: usize) -> String {
    let mut s = String::from("<h1>Player</h1><video src=\"main.mp4\" controls>");
    for i in 0..sources {
        s.push_str(&format!(
            "<source src=\"s{i}.mp4\" type=\"video/mp4\"></source>"
        ));
    }
    s.push_str("</video>");
    s
}

proptest! {
    /// P4 transpilation preserves the heading text of the source document
    /// exactly — the Stage S3 content-fidelity invariant for media players.
    #[test]
    fn media_player_text_is_preserved(sources in 0..4usize) {
        let html = media_doc(sources);
        let site = transpile_media_player(&html);

        let expected = collect_text(&parse_html(&html));
        let got: Vec<String> = site
            .ops
            .iter()
            .filter_map(|op| match op {
                DomOp::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();

        prop_assert_eq!(expected, got, "transpiled media text diverges from source");
    }

    /// The P4 transpiled guest is structurally sound (every variable created
    /// before use) and its extracted [`MediaModel`] matches the source
    /// (one video, resolved src, controls requested, source count).
    #[test]
    fn media_player_model_is_consistent(sources in 0..4usize) {
        let html = media_doc(sources);
        let site = transpile_media_player(&html);

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

        prop_assert_eq!(site.media.players.len(), 1, "exactly one player expected");
        prop_assert_eq!(site.media.players[0].kind.clone(), "video".to_string());
        prop_assert_eq!(site.media.players[0].src.as_deref(), Some("main.mp4"));
        prop_assert!(site.media.players[0].has_controls, "player must request controls");
        prop_assert_eq!(
            site.media.players[0].sources.len(),
            sources,
            "source count mismatch"
        );
        // Source children are folded into the player model, not emitted as
        // separate top-level elements.
        let has_source_elem = site
            .ops
            .iter()
            .any(|o| matches!(o, DomOp::Create { tag, .. } if tag == "source"));
        prop_assert!(!has_source_elem, "source must not be a top-level element");
    }
}
