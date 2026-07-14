//! Stage S1 (Discovery) of the AI migration pipeline (spec §6.1).
//!
//! `discover` extracts a structural [`ApiSurfaceMap`] from JS/TS source:
//! the modules a file depends on, what it exports, and its top-level
//! functions. The orchestration layer (TPT Eve) consumes this map to plan a
//! migration before any transpilation (Stage S2) happens.
//!
//! Two extractors satisfy the same stable contract ([`ApiSurfaceMap`]):
//! * [`discover`] — a dependency-free, token-based scanner. Correct and
//!   testable, but blind to strings/comments; kept as a zero-cost fallback.
//! * [`tree_sitter_discovery::discover_ast`] — the reference implementation,
//!   backed by `tree-sitter` grammars (JavaScript / TypeScript / TSX) for full
//!   AST fidelity.

pub mod coverage;
pub mod js_transform;
pub mod transpile;
pub mod tree_sitter_discovery;

pub use tree_sitter_discovery::{SourceLang, discover_ast, discover_js, discover_ts, discover_tsx};

/// A module a source file imports from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The module specifier, e.g. `"./util"` or `"react"`.
    pub module: String,
    /// The named bindings imported (empty for `import x from` / namespace imports).
    pub names: Vec<String>,
    /// `true` for `import * as ns from "…"`.
    pub namespace: bool,
    /// `true` for `import def from "…"` (default import).
    pub default: bool,
}

/// The kind of an exported symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportKind {
    Function,
    Const,
    Class,
    Default,
}

/// A symbol a source file exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub kind: ExportKind,
    pub name: String,
}

/// A structural summary of a JS/TS module — the Stage S1 discovery output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApiSurfaceMap {
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    /// Names of top-level (non-exported) function declarations.
    pub functions: Vec<String>,
}

impl ApiSurfaceMap {
    /// Count of declared symbols (imports + exports + functions), used as a
    /// crude "complexity" signal for migration planning.
    pub fn symbol_count(&self) -> usize {
        self.imports.len() + self.exports.len() + self.functions.len()
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn between(source: &str, open: char, close: char) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    let mut started = false;
    let mut start = 0;
    for (i, c) in source.char_indices() {
        if c == open || c == close {
            if c == close && depth > 0 {
                depth -= 1;
                if started && depth == 0 {
                    return Some((start, i));
                }
            } else {
                if !started {
                    started = true;
                    start = i + c.len_utf8();
                }
                depth += 1;
            }
        }
    }
    None
}

fn parse_import_names(spec: &str) -> Vec<String> {
    // spec is the text inside `{ … }` of `import { a, b as c } from "m"`.
    spec.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Extract an [`ApiSurfaceMap`] from JS/TS `source`.
///
/// This is a token-based approximation: it handles the common declaration
/// forms (`import`/`export function`/`export const`/`export class`/`export
/// default`/`function`) without a full grammar. It ignores comments and string
/// contents at a best-effort level (strings containing `;` would confuse it,
/// which a tree-sitter pass would not).
pub fn discover(source: &str) -> ApiSurfaceMap {
    let mut map = ApiSurfaceMap::default();

    for raw_line in source.lines() {
        // Strip trailing line comments (best-effort; not inside strings).
        let line = match raw_line.split("//").next() {
            Some(l) => l.trim(),
            None => raw_line.trim(),
        };
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("import ") {
            let module = between(rest, '"', '"')
                .or_else(|| between(rest, '\'', '\''))
                .map(|(s, e)| unquote(&rest[s..e]))
                .unwrap_or_default();
            let mut imp = Import {
                module,
                names: vec![],
                namespace: false,
                default: false,
            };
            if let Some((s, e)) = between(rest, '{', '}') {
                imp.names = parse_import_names(&rest[s..e]);
            }
            if rest.contains("* as") {
                imp.namespace = true;
            } else {
                // A default import identifier precedes the `{` (or `from`):
                // `import React, { … } from "m"` / `import React from "m"`.
                let head = rest
                    .split('{')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches(',')
                    .trim();
                if !head.is_empty() && !head.starts_with('"') && !head.starts_with('\'') {
                    imp.default = true;
                }
            }
            map.imports.push(imp);
            continue;
        }

        if let Some(rest) = line.strip_prefix("export ") {
            // Re-export form (`export { a, b } from "mod"`) surfaces `mod` as a
            // dependency even though nothing is declared locally.
            if let Some(from_idx) = rest.find(" from ") {
                let after = &rest[from_idx + " from ".len()..];
                if let Some((s, e)) =
                    between(after, '"', '"').or_else(|| between(after, '\'', '\''))
                {
                    let module = unquote(&after[s..e]);
                    if !module.is_empty() {
                        map.imports.push(Import {
                            module,
                            names: vec![],
                            namespace: false,
                            default: false,
                        });
                    }
                }
            }
            if let Some(name) = rest.strip_prefix("function ") {
                if let Some(n) = name
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    && !n.is_empty()
                {
                    map.exports.push(Export {
                        kind: ExportKind::Function,
                        name: n.to_string(),
                    });
                }
            } else if let Some(name) = rest.strip_prefix("class ") {
                if let Some(n) = name
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    && !n.is_empty()
                {
                    map.exports.push(Export {
                        kind: ExportKind::Class,
                        name: n.to_string(),
                    });
                }
            } else if rest.starts_with("default ") {
                map.exports.push(Export {
                    kind: ExportKind::Default,
                    name: "default".to_string(),
                });
            } else if let Some(kw) = rest
                .split_whitespace()
                .next()
                .filter(|k| *k == "const" || *k == "let" || *k == "var")
            {
                let _ = kw;
                if let Some(name) = rest[kw.len()..]
                    .trim_start()
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    && !name.is_empty()
                {
                    map.exports.push(Export {
                        kind: ExportKind::Const,
                        name: name.to_string(),
                    });
                }
            }
            continue;
        }

        if let Some(name) = line.strip_prefix("function ")
            && let Some(n) = name
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
            && !n.is_empty()
        {
            map.functions.push(n.to_string());
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_imports_and_exports() {
        let src = r#"
            // a comment with import inside should be ignored
            import React, { useState, useEffect } from "react";
            import * as utils from "./utils";
            import def from "mod";

            export function start() {}
            export const VERSION = "1.0";
            export class Service {}
            export default function () {}

            function helper() {}
        "#;

        let map = discover(src);

        assert_eq!(map.imports.len(), 3);
        assert_eq!(map.imports[0].module, "react");
        assert_eq!(map.imports[0].names, vec!["useState", "useEffect"]);
        assert!(map.imports[0].default);
        assert_eq!(map.imports[1].module, "./utils");
        assert!(map.imports[1].namespace);
        assert_eq!(map.imports[2].module, "mod");
        assert!(map.imports[2].default);

        let names: Vec<&str> = map.exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"start"));
        assert!(names.contains(&"VERSION"));
        assert!(names.contains(&"Service"));
        assert!(names.contains(&"default"));

        assert_eq!(map.functions, vec!["helper"]);
        assert!(map.symbol_count() >= 7);
    }

    #[test]
    fn handles_reexport_and_empty() {
        let map = discover("");
        assert_eq!(map.symbol_count(), 0);

        let src = r#"export { a, b } from "./other";"#;
        let map = discover(src);
        // Re-export form: module is captured, no named functions declared.
        assert_eq!(map.imports.len(), 1);
        assert_eq!(map.imports[0].module, "./other");
    }
}
