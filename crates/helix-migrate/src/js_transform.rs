//! Stage S2 (Transpile) — `jscodeshift`-style AST-to-source transform pipeline.
//!
//! `jscodeshift` works by parsing source into a syntax tree, walking it with a
//! collection of *rules* (each matching a node kind and emitting a rewrite),
//! and re-serializing the result. This module adapts that exact shape for the
//! migration agent:
//!
//! * [`Transformer`] collects `(byte_range, replacement_text)` edits and applies
//!   them back onto the original source — the splice model jscodeshift uses
//!   instead of a full pretty-printer.
//! * [`Rule`] is the per-node visitor trait (`matches` + `rewrite`).
//! * [`transpile_js_to_rust`] runs a starter rule set that maps a *subset* of JS
//!   to idiomatic Rust: `function` → `fn`, `const`/`var` → `let`, and
//!   `console.log(..)` → `println!(..)`. It is deliberately a thin, auditable
//!   subset — the seam for additional patterns (arrow functions, classes, ...)
//!   is the [`Rule`] trait, not a rewrite of this driver.

use tree_sitter::{Language, Node, Parser};

use crate::SourceLang;

/// One splice: replace `source[start..end]` with `replacement`.
#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

/// Accumulates source splices and applies them in reverse byte order so earlier
/// offsets stay valid after each replacement.
pub struct Transformer<'a> {
    source: &'a str,
    edits: Vec<Edit>,
}

impl<'a> Transformer<'a> {
    pub fn new(source: &'a str) -> Self {
        Transformer { source, edits: Vec::new() }
    }

    /// Replace the exact source span covered by `node`.
    pub fn replace_node(&mut self, node: Node, replacement: &str) {
        self.edits.push(Edit {
            start: node.start_byte(),
            end: node.end_byte(),
            replacement: replacement.to_string(),
        });
    }

    /// Apply all collected edits and return the rewritten source.
    pub fn apply(mut self) -> String {
        // Largest start first so earlier spans are unaffected by later splicing.
        self.edits.sort_by(|a, b| b.start.cmp(&a.start));
        let mut out = self.source.to_string();
        for e in &self.edits {
            out.replace_range(e.start..e.end, &e.replacement);
        }
        out
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

/// Maps `const`/`let`/`var` declarations onto Rust's `let`.
///
/// In tree-sitter 0.23, `const`/`let` are `lexical_declaration` nodes and `var`
/// is a `variable_declaration` node; both lead with the keyword token.
struct VarToLetRule;

impl Rule for VarToLetRule {
    fn matches(&self, node: Node) -> bool {
        node.kind() == "lexical_declaration" || node.kind() == "variable_declaration"
    }

    fn rewrite(&self, node: Node, _source: &str, t: &mut Transformer) {
        // The leading keyword (`const`/`let`/`var`) is the first child and is an
        // anonymous node whose kind *is* the keyword text.
        if let Some(first) = node.child(0) {
            if matches!(first.kind(), "const" | "var") {
                t.replace_node(first, "let");
            }
        }
    }
}

/// Maps `console.log(..)` calls onto `println!(..)`.
struct ConsoleLogRule;

impl Rule for ConsoleLogRule {
    fn matches(&self, node: Node) -> bool {
        node.kind() == "call_expression"
    }

    fn rewrite(&self, node: Node, source: &str, t: &mut Transformer) {
        let Some(callee) = node.child_by_field_name("function") else {
            return;
        };
        if callee.kind() != "member_expression" {
            return;
        }
        let text = &source[callee.start_byte()..callee.end_byte()];
        if text == "console.log" {
            t.replace_node(callee, "println!");
        }
    }
}

/// Parse `source` with the given grammar and run the starter JS→Rust rule set.
///
/// Returns the original source unchanged if the grammar cannot be set or the
/// document cannot be parsed (tree-sitter recovers from syntax errors rather
/// than failing, so this is rare).
pub fn transpile_js_to_rust(source: &str, lang: SourceLang) -> String {
    let mut parser = Parser::new();
    if parser.set_language(&grammar(lang)).is_err() {
        return source.to_string();
    }
    let Some(tree) = parser.parse(source, None) else {
        return source.to_string();
    };

    let rules: &[&dyn Rule] = &[&FnKeywordRule, &VarToLetRule, &ConsoleLogRule];
    let mut t = Transformer::new(source);
    visit_all(tree.root_node(), source, &mut t, rules);
    t.apply()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_keyword_becomes_fn() {
        let src = "function add(a, b) { return a + b; }";
        assert_eq!(transpile_js_to_rust(src, SourceLang::JavaScript), "fn add(a, b) { return a + b; }");
    }

    #[test]
    fn const_becomes_let() {
        let src = "const x = 1;";
        assert_eq!(transpile_js_to_rust(src, SourceLang::JavaScript), "let x = 1;");
    }

    #[test]
    fn var_becomes_let() {
        let src = "var y = 2;";
        assert_eq!(transpile_js_to_rust(src, SourceLang::JavaScript), "let y = 2;");
    }

    #[test]
    fn console_log_becomes_println() {
        let src = "console.log(x);";
        assert_eq!(transpile_js_to_rust(src, SourceLang::JavaScript), "println!(x);");
    }

    #[test]
    fn combined_transform() {
        let src = "function greet(name) { const msg = 'hi ' + name; console.log(msg); }";
        assert_eq!(
            transpile_js_to_rust(src, SourceLang::JavaScript),
            "fn greet(name) { let msg = 'hi ' + name; println!(msg); }"
        );
    }

    #[test]
    fn leaves_unhandled_syntax_intact() {
        // `let` is already the Rust keyword (no rewrite), and array/template
        // literals are not in the starter set, so the source passes through
        // unchanged rather than being mis-transformed.
        let src = "let arr = [1, 2, 3];";
        assert_eq!(transpile_js_to_rust(src, SourceLang::JavaScript), src);
    }
}
