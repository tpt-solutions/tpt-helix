//! Stage S2 (Transpile) — `jscodeshift`-style AST-to-source transform pipeline.
//!
//! `jscodeshift` works by parsing source into a syntax tree, walking it with a
//! collection of *rules* (each matching a node kind and emitting a rewrite),
//! and re-serializing the result. This module adapts that exact shape for the
//! migration agent:
//!
//! * [`Transformer`] collects `(byte_range, replacement_text)` edits and applies
//!   them back onto the original source in one forward pass. An edit that
//!   overlaps one already applied is skipped, never spliced into the middle of
//!   another edit.
//! * [`Rule`] is the per-node visitor trait (`matches` + `rewrite`).
//! * [`transpile_js_to_rust`] runs a starter rule set over a *subset* of JS/TS:
//!   `function` → `fn`, `const`/`var` → `let`, `console.log(..)` → `println!(..)`
//!   with a format string, and single-quoted strings → double-quoted.
//!
//! The output is a **draft**. Rewritten code is closer to Rust, but JavaScript
//! and TypeScript types do not map onto Rust by rewriting alone, so every
//! translated function is listed in [`JsTranslation::needs_rust_types`] for a
//! human to type. Syntax the rules do not cover passes through unchanged.

use tree_sitter::{Language, Node, Parser};

use crate::SourceLang;

/// One splice: replace `source[start..end]` with `replacement`.
#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

/// Accumulates source splices and applies them in one forward pass.
pub struct Transformer<'a> {
    source: &'a str,
    edits: Vec<Edit>,
}

impl<'a> Transformer<'a> {
    pub fn new(source: &'a str) -> Self {
        Transformer {
            source,
            edits: Vec::new(),
        }
    }

    /// Replace the exact source span covered by `node`.
    pub fn replace_node(&mut self, node: Node, replacement: &str) {
        self.edits.push(Edit {
            start: node.start_byte(),
            end: node.end_byte(),
            replacement: replacement.to_string(),
        });
    }

    /// Apply the collected edits and return the rewritten source together with
    /// the number of edits applied.
    ///
    /// Edits are ordered by start, and among edits that start together the
    /// outermost (largest) comes first. An edit that starts inside an earlier
    /// applied edit is skipped, so overlapping rewrites can never corrupt the
    /// output. The outer rewrite wins because it already contains the inner
    /// code.
    pub fn apply(mut self) -> (String, usize) {
        self.edits
            .sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let mut out = String::with_capacity(self.source.len());
        let mut cursor = 0usize;
        let mut applied = 0usize;
        for e in &self.edits {
            if e.start < cursor {
                continue; // overlaps an edit already applied
            }
            out.push_str(&self.source[cursor..e.start]);
            out.push_str(&e.replacement);
            cursor = e.end;
            applied += 1;
        }
        out.push_str(&self.source[cursor..]);
        (out, applied)
    }
}

/// A single AST-to-source rewrite pass, in the `jscodeshift` visitor style.
pub trait Rule {
    /// Whether this rule fires on `node`.
    fn matches(&self, node: Node) -> bool;
    /// Emit any splices for `node` (matched) via `t`.
    fn rewrite(&self, node: Node, source: &str, t: &mut Transformer);
}

/// Visits every node in the tree, invoking each rule that matches.
fn visit_all(node: Node, source: &str, t: &mut Transformer, rules: &[&dyn Rule]) {
    for rule in rules {
        if rule.matches(node) {
            rule.rewrite(node, source, t);
        }
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            visit_all(cursor.node(), source, t, rules);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn grammar(lang: SourceLang) -> Language {
    match lang {
        SourceLang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        SourceLang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceLang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        SourceLang::Auto => tree_sitter_javascript::LANGUAGE.into(),
    }
}

/// The source text of `node`.
fn text<'s>(node: Node, source: &'s str) -> &'s str {
    &source[node.start_byte()..node.end_byte()]
}

/// A single-quoted string whose body can be copied verbatim into a Rust
/// double-quoted literal or format string.
fn simple_single_quoted(raw: &str) -> Option<&str> {
    let inner = raw.strip_prefix('\'')?.strip_suffix('\'')?;
    if inner.contains(['"', '\\', '\'', '\n']) {
        None
    } else {
        Some(inner)
    }
}

/// Maps `function`/`function_expression` keywords onto Rust's `fn`.
struct FnKeywordRule;

impl Rule for FnKeywordRule {
    fn matches(&self, node: Node) -> bool {
        node.kind() == "function_declaration" || node.kind() == "function_expression"
    }

    fn rewrite(&self, node: Node, _source: &str, t: &mut Transformer) {
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "function" {
                    t.replace_node(child, "fn");
                    break;
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Maps `const`/`var` declarations onto Rust's `let`. `var` is mutable in
/// JavaScript, so it becomes `let mut`. `const` never changes, so it becomes
/// `let`.
///
/// In tree-sitter, `const`/`let` are `lexical_declaration` nodes and `var` is a
/// `variable_declaration` node; both lead with the keyword token.
struct VarToLetRule;

impl Rule for VarToLetRule {
    fn matches(&self, node: Node) -> bool {
        node.kind() == "lexical_declaration" || node.kind() == "variable_declaration"
    }

    fn rewrite(&self, node: Node, _source: &str, t: &mut Transformer) {
        let Some(first) = node.child(0) else {
            return;
        };
        match first.kind() {
            "const" => t.replace_node(first, "let"),
            "var" => t.replace_node(first, "let mut"),
            _ => {}
        }
    }
}

/// Maps `console.log(..)` calls onto a `println!` with a format string.
///
/// Each string-literal argument that can be copied verbatim is folded into the
/// format string, with braces escaped. Every other argument becomes a `{}`
/// placeholder. JavaScript separates logged values with spaces, so the format
/// string does too.
struct ConsoleLogRule;

impl Rule for ConsoleLogRule {
    fn matches(&self, node: Node) -> bool {
        node.kind() == "call_expression"
    }

    fn rewrite(&self, node: Node, source: &str, t: &mut Transformer) {
        let Some(callee) = node.child_by_field_name("function") else {
            return;
        };
        if callee.kind() != "member_expression" || text(callee, source) != "console.log" {
            return;
        }
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return;
        };

        let mut format = String::new();
        let mut values: Vec<&str> = Vec::new();
        let mut cursor = arguments.walk();
        let args: Vec<Node> = arguments.named_children(&mut cursor).collect();
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                format.push(' ');
            }
            let raw = text(*arg, source);
            match (arg.kind(), simple_single_quoted(raw)) {
                ("string", Some(inner)) => {
                    format.push_str(&inner.replace('{', "{{").replace('}', "}}"));
                }
                _ => {
                    format.push_str("{}");
                    values.push(raw);
                }
            }
        }

        let mut replacement = format!("println!(\"{format}\"");
        for v in values {
            replacement.push_str(", ");
            replacement.push_str(v);
        }
        replacement.push(')');
        if args.is_empty() {
            replacement = "println!()".to_string();
        }
        t.replace_node(node, &replacement);
    }
}

/// Rewrites a single-quoted string literal as a Rust double-quoted one, when the
/// body needs no escaping.
struct StringQuoteRule;

impl Rule for StringQuoteRule {
    fn matches(&self, node: Node) -> bool {
        node.kind() == "string"
    }

    fn rewrite(&self, node: Node, source: &str, t: &mut Transformer) {
        if let Some(inner) = simple_single_quoted(text(node, source)) {
            t.replace_node(node, &format!("\"{inner}\""));
        }
    }
}

/// Maps a TypeScript type to its Rust draft type. Only the types that map
/// directly are handled. Anything else returns `None`, so the function that uses
/// it stays on the review list.
///
/// `number` is `f64`, because a JavaScript number may be fractional. A literal
/// integer assigned to it therefore still needs a human to fix it.
fn rust_type(ts: &str) -> Option<String> {
    let ts = ts.trim();
    if let Some(element) = ts.strip_suffix("[]") {
        return rust_type(element).map(|t| format!("Vec<{t}>"));
    }
    match ts {
        "number" => Some("f64".into()),
        "string" => Some("&str".into()),
        "boolean" => Some("bool".into()),
        "void" => Some("()".into()),
        _ => None,
    }
}

/// The Rust type a `type_annotation` node maps to, if it maps.
fn annotation_rust_type(node: Node, source: &str) -> Option<String> {
    rust_type(text(node, source).trim_start_matches(':'))
}

/// Rewrites TypeScript `: T` annotations to Rust types. A return annotation
/// becomes `-> T`, which is Rust's form. Annotations that do not map are left
/// alone.
struct TypeAnnotationRule;

impl Rule for TypeAnnotationRule {
    fn matches(&self, node: Node) -> bool {
        node.kind() == "type_annotation"
    }

    fn rewrite(&self, node: Node, source: &str, t: &mut Transformer) {
        let Some(rust) = annotation_rust_type(node, source) else {
            return;
        };
        let is_return = node.parent().is_some_and(|parent| {
            parent
                .child_by_field_name("return_type")
                .is_some_and(|r| r.id() == node.id())
        });
        if is_return {
            t.replace_node(node, &format!(" -> {rust}"));
        } else {
            t.replace_node(node, &format!(": {rust}"));
        }
    }
}

/// Whether a function declaration has a Rust type for every parameter and for
/// its return. A function with no annotated return that returns a value is not
/// typed, because the return type cannot be read from the source.
fn function_is_typed(node: Node, source: &str) -> bool {
    let Some(params) = node.child_by_field_name("parameters") else {
        return false;
    };
    let mut cursor = params.walk();
    for param in params.named_children(&mut cursor) {
        let mut pc = param.walk();
        let typed = param
            .children(&mut pc)
            .any(|c| c.kind() == "type_annotation" && annotation_rust_type(c, source).is_some());
        if !typed {
            return false;
        }
    }
    match node.child_by_field_name("return_type") {
        Some(ret) => annotation_rust_type(ret, source).is_some(),
        None => node
            .child_by_field_name("body")
            .is_none_or(|body| !has_value_return(body)),
    }
}

/// Whether `node` contains a `return` that carries a value.
fn has_value_return(node: Node) -> bool {
    if node.kind() == "return_statement" && node.named_child_count() > 0 {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).any(has_value_return)
}

/// Result of a successful translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsTranslation {
    /// The rewritten source. A draft: see the module docs.
    pub rust: String,
    /// How many rewrites were applied.
    pub rewrites: usize,
    /// Declared functions whose parameters and return types still need Rust
    /// types. JavaScript and TypeScript types do not map onto Rust by rewriting.
    pub needs_rust_types: Vec<String>,
}

/// Why a JS/TS source could not be translated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsTranspileError {
    /// The source does not parse. `line` is the first line with an error
    /// (1-based).
    Syntax { line: usize },
    /// The grammar could not be loaded.
    Grammar,
}

impl std::fmt::Display for JsTranspileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsTranspileError::Syntax { line } => {
                write!(f, "JavaScript/TypeScript does not parse (first error on line {line})")
            }
            JsTranspileError::Grammar => f.write_str("could not load the JavaScript grammar"),
        }
    }
}

impl std::error::Error for JsTranspileError {}

/// Parses `source` and runs the starter JS→Rust rule set.
///
/// With [`SourceLang::Auto`], the JavaScript grammar is tried first and the
/// TypeScript grammar second, so TypeScript annotations parse too. A source that
/// fails every grammar is an error, never a silently mangled output.
pub fn transpile_js_to_rust(
    source: &str,
    lang: SourceLang,
) -> Result<JsTranslation, JsTranspileError> {
    let candidates = match lang {
        SourceLang::Auto => vec![SourceLang::JavaScript, SourceLang::TypeScript],
        other => vec![other],
    };
    let mut first_error = None;
    for candidate in candidates {
        let mut parser = Parser::new();
        parser
            .set_language(&grammar(candidate))
            .map_err(|_| JsTranspileError::Grammar)?;
        let tree = parser.parse(source, None).ok_or(JsTranspileError::Grammar)?;
        let root = tree.root_node();
        if !root.has_error() {
            return Ok(translate(source, root));
        }
        first_error.get_or_insert(JsTranspileError::Syntax {
            line: first_error_line(root),
        });
    }
    Err(first_error.unwrap_or(JsTranspileError::Grammar))
}

fn translate(source: &str, root: Node) -> JsTranslation {
    let rules: &[&dyn Rule] = &[
        &FnKeywordRule,
        &TypeAnnotationRule,
        &VarToLetRule,
        &ConsoleLogRule,
        &StringQuoteRule,
    ];
    let mut t = Transformer::new(source);
    visit_all(root, source, &mut t, rules);
    let (rust, rewrites) = t.apply();

    let mut needs_rust_types = Vec::new();
    collect_untyped_functions(root, source, &mut needs_rust_types);
    JsTranslation {
        rust,
        rewrites,
        needs_rust_types,
    }
}

fn collect_untyped_functions(node: Node, source: &str, out: &mut Vec<String>) {
    if node.kind() == "function_declaration"
        && !function_is_typed(node, source)
        && let Some(name) = node.child_by_field_name("name")
    {
        out.push(text(name, source).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_untyped_functions(child, source, out);
    }
}

/// Line (1-based) of the first parse error under `root`, or 1 when none is found.
fn first_error_line(root: Node) -> usize {
    fn find(node: Node) -> Option<Node> {
        if node.is_error() || node.is_missing() {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.has_error()
                && let Some(found) = find(child)
            {
                return Some(found);
            }
        }
        None
    }
    find(root).map(|n| n.start_position().row + 1).unwrap_or(1)
}

#[cfg(test)]
mod type_mapping_tests {
    use super::*;

    fn ts(src: &str) -> JsTranslation {
        transpile_js_to_rust(src, SourceLang::TypeScript).expect("parses")
    }

    #[test]
    fn annotated_function_becomes_typed_fn_and_needs_no_review() {
        let t = ts("function add(a: number, b: number): number { return a + b; }");
        assert_eq!(t.rust, "fn add(a: f64, b: f64) -> f64 { return a + b; }");
        assert!(t.needs_rust_types.is_empty(), "{:?}", t.needs_rust_types);
    }

    #[test]
    fn string_and_boolean_annotations_map_to_rust_types() {
        let t = ts("function hi(name: string, loud: boolean): string { return name; }");
        assert_eq!(t.rust, "fn hi(name: &str, loud: bool) -> &str { return name; }");
        assert!(t.needs_rust_types.is_empty());
    }

    #[test]
    fn array_annotations_map_to_vec() {
        let t = ts("function total(xs: number[]): number { return 0; }");
        assert_eq!(t.rust, "fn total(xs: Vec<f64>) -> f64 { return 0; }");
    }

    #[test]
    fn annotated_declaration_keeps_its_type() {
        assert_eq!(ts("const n: number = 1;").rust, "let n: f64 = 1;");
    }

    #[test]
    fn unmappable_annotation_still_needs_review() {
        let t = ts("function f(x: string | null): number { return 1; }");
        assert_eq!(t.needs_rust_types, vec!["f".to_string()]);
    }

    #[test]
    fn untyped_return_with_a_value_still_needs_review() {
        let t = ts("function f(a: number) { return a; }");
        assert_eq!(t.needs_rust_types, vec!["f".to_string()]);
    }

    #[test]
    fn function_without_return_value_needs_no_return_type() {
        let t = ts("function log(a: number) { console.log(a); }");
        assert!(t.needs_rust_types.is_empty(), "{:?}", t.needs_rust_types);
    }

    #[test]
    fn plain_javascript_parameters_still_need_review() {
        let t = transpile_js_to_rust("function add(a, b) { return a + b; }", SourceLang::JavaScript)
            .unwrap();
        assert_eq!(t.needs_rust_types, vec!["add".to_string()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust(src: &str, lang: SourceLang) -> String {
        transpile_js_to_rust(src, lang).expect("parses").rust
    }

    #[test]
    fn function_keyword_becomes_fn_and_is_flagged_for_types() {
        let t = transpile_js_to_rust("function add(a, b) { return a + b; }", SourceLang::JavaScript)
            .unwrap();
        assert_eq!(t.rust, "fn add(a, b) { return a + b; }");
        assert_eq!(t.needs_rust_types, vec!["add".to_string()]);
    }

    #[test]
    fn const_becomes_let() {
        assert_eq!(rust("const x = 1;", SourceLang::JavaScript), "let x = 1;");
    }

    #[test]
    fn var_becomes_mutable_let() {
        assert_eq!(rust("var y = 2;", SourceLang::JavaScript), "let mut y = 2;");
    }

    #[test]
    fn console_log_string_literal_becomes_println() {
        assert_eq!(
            rust("console.log('hi');", SourceLang::JavaScript),
            "println!(\"hi\");"
        );
    }

    #[test]
    fn console_log_values_become_placeholders() {
        assert_eq!(
            rust("console.log('x =', x);", SourceLang::JavaScript),
            "println!(\"x = {}\", x);"
        );
    }

    #[test]
    fn console_log_escapes_braces_in_literal_text() {
        assert_eq!(
            rust("console.log('{a}');", SourceLang::JavaScript),
            "println!(\"{{a}}\");"
        );
    }

    #[test]
    fn console_log_with_no_arguments_prints_a_blank_line() {
        assert_eq!(rust("console.log();", SourceLang::JavaScript), "println!();");
    }

    #[test]
    fn single_quoted_strings_become_double_quoted() {
        assert_eq!(
            rust("const s = 'abc';", SourceLang::JavaScript),
            "let s = \"abc\";"
        );
    }

    #[test]
    fn combined_transform_produces_valid_structure() {
        let src = "function greet(name) { const msg = 'hi'; console.log(msg, name); }";
        assert_eq!(
            rust(src, SourceLang::JavaScript),
            "fn greet(name) { let msg = \"hi\"; println!(\"{} {}\", msg, name); }"
        );
    }

    #[test]
    fn overlapping_edits_never_corrupt_the_output() {
        // The outer `console.log` rewrite already contains the inner string, so
        // the inner quote rewrite is skipped rather than spliced into the middle.
        assert_eq!(
            rust("console.log(f('a'));", SourceLang::JavaScript),
            "println!(\"{}\", f('a'));"
        );
    }

    #[test]
    fn syntax_errors_are_reported_not_rewritten() {
        let err = transpile_js_to_rust("function ( {\n", SourceLang::JavaScript).unwrap_err();
        assert!(matches!(err, JsTranspileError::Syntax { line: 1 }), "{err:?}");
    }

    #[test]
    fn auto_falls_back_to_typescript_for_annotations() {
        let t = transpile_js_to_rust("const n: number = 1;", SourceLang::Auto).unwrap();
        assert_eq!(t.rust, "let n: f64 = 1;");
        assert!(transpile_js_to_rust("const n: number = 1;", SourceLang::JavaScript).is_err());
    }

    #[test]
    fn leaves_unhandled_syntax_intact() {
        // Array literals are outside the starter rule set, so they pass through.
        let src = "let arr = [1, 2, 3];";
        assert_eq!(rust(src, SourceLang::JavaScript), src);
    }

    #[test]
    fn transformer_applies_edits_in_one_pass_and_skips_overlaps() {
        let mut t = Transformer::new("abcdef");
        t.edits.push(Edit {
            start: 3,
            end: 5,
            replacement: "X".into(),
        });
        t.edits.push(Edit {
            start: 0,
            end: 2,
            replacement: "Y".into(),
        });
        // An edit inside [0,2) overlaps the first and is skipped.
        t.edits.push(Edit {
            start: 1,
            end: 2,
            replacement: "Z".into(),
        });
        assert_eq!(t.apply(), ("YcXf".to_string(), 2));
    }
}
