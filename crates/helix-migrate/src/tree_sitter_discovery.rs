//! Stage S1 (Discovery) — tree-sitter AST-backed surface extraction.
//!
//! This is the reference implementation of [`crate::discover`]'s contract: it
//! produces the same [`crate::ApiSurfaceMap`] as the dependency-free tokenizer,
//! but from a real grammar parse so it is robust to comments, strings, and
//! nested syntax that a line scanner cannot see. The tokenizer ([`crate::discover`])
//! remains available as a zero-dependency fallback.
//!
//! Three grammars are supported: JavaScript, TypeScript, and TSX (TypeScript
//! with embedded JSX). All share the `import_statement` / `export_statement`
//! node shapes, so a single walker handles them.

use crate::{ApiSurfaceMap, Export, ExportKind, Import};
use tree_sitter::{Language, Node, Parser};

/// Which grammar to parse `source` with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLang {
    JavaScript,
    TypeScript,
    Tsx,
}

fn language_for(lang: SourceLang) -> Language {
    match lang {
        SourceLang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        SourceLang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceLang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Extract an [`ApiSurfaceMap`] from JS/TS `source` using a tree-sitter parse.
///
/// Falls back to an empty map if the grammar cannot parse `source` (tree-sitter
/// recovers from syntax errors rather than failing, so this is rare).
pub fn discover_ast(source: &str, lang: SourceLang) -> ApiSurfaceMap {
    let mut parser = Parser::new();
    let language = language_for(lang);
    if parser.set_language(&language).is_err() {
        return ApiSurfaceMap::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return ApiSurfaceMap::default();
    };
    let bytes = source.as_bytes();
    let mut map = ApiSurfaceMap::default();
    walk(tree.root_node(), bytes, &mut map);
    map
}

/// Convenience wrappers for each supported grammar.
pub fn discover_js(source: &str) -> ApiSurfaceMap {
    discover_ast(source, SourceLang::JavaScript)
}
pub fn discover_ts(source: &str) -> ApiSurfaceMap {
    discover_ast(source, SourceLang::TypeScript)
}
pub fn discover_tsx(source: &str) -> ApiSurfaceMap {
    discover_ast(source, SourceLang::Tsx)
}

fn walk(node: Node, src: &[u8], map: &mut ApiSurfaceMap) {
    match node.kind() {
        "import_statement" => {
            handle_import(node, src, map);
            return;
        }
        "export_statement" => {
            handle_export(node, src, map);
            return;
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(src).ok())
            {
                map.functions.push(name.to_string());
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, src, map);
    }
}

fn handle_import(node: Node, src: &[u8], map: &mut ApiSurfaceMap) {
    let mut imp = Import {
        module: String::new(),
        names: Vec::new(),
        namespace: false,
        default: false,
    };

    // `import_statement` children: an `import_clause` wrapper (which itself
    // holds the default identifier / `namespace_import` / `named_imports`)
    // plus the module `string`. Walk the named children, descending into the
    // clause.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_import_clause(child, src, &mut imp);
    }

    map.imports.push(imp);
}

fn collect_import_clause(child: Node, src: &[u8], imp: &mut Import) {
    match child.kind() {
        "import_clause" => {
            let mut cursor = child.walk();
            for g in child.named_children(&mut cursor) {
                collect_import_clause(g, src, imp);
            }
        }
        "string" => {
            if let Ok(s) = child.utf8_text(src) {
                imp.module = strip_quotes(s).to_string();
            }
        }
        "identifier" => imp.default = true,
        "namespace_import" => imp.namespace = true,
        "named_imports" => {
            let mut inner = child.walk();
            for spec in child.named_children(&mut inner) {
                if spec.kind() == "import_specifier"
                    && let Some(name_node) = spec.child_by_field_name("name")
                    && let Ok(name) = name_node.utf8_text(src)
                {
                    imp.names.push(name.to_string());
                }
            }
        }
        _ => {}
    }
}

fn handle_export(node: Node, src: &[u8], map: &mut ApiSurfaceMap) {
    // Re-export form (`export { a, b } from "mod"`) surfaces `mod` as a
    // dependency, mirroring the tokenizer's handling.
    if let Some(src_node) = node.child_by_field_name("source") {
        let module = src_node.utf8_text(src).ok().map(strip_quotes).unwrap_or("");
        if !module.is_empty() {
            map.imports.push(Import {
                module: module.to_string(),
                names: Vec::new(),
                namespace: false,
                default: false,
            });
        }
        return;
    }

    // `export default …` is recorded as a Default export, matching the
    // tokenizer (the declared symbol, if any, rides along inside the default).
    let mut has_default = false;
    {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "default" {
                has_default = true;
                break;
            }
        }
    }
    if has_default {
        map.exports.push(Export {
            kind: ExportKind::Default,
            name: "default".to_string(),
        });
        return;
    }

    if let Some(decl) = node.child_by_field_name("declaration") {
        match decl.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(name) = decl
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                {
                    map.exports.push(Export {
                        kind: ExportKind::Function,
                        name: name.to_string(),
                    });
                }
            }
            "class_declaration" => {
                if let Some(name) = decl
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                {
                    map.exports.push(Export {
                        kind: ExportKind::Class,
                        name: name.to_string(),
                    });
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = decl.walk();
                for declarator in decl.named_children(&mut cursor) {
                    if declarator.kind() == "variable_declarator" {
                        if let Some(name) = declarator
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(src).ok())
                        {
                            map.exports.push(Export {
                                kind: ExportKind::Const,
                                name: name.to_string(),
                            });
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiSurfaceMap, ExportKind, Import};

    fn import(module: &str, names: &[&str], namespace: bool, default: bool) -> Import {
        Import {
            module: module.to_string(),
            names: names.iter().map(|s| s.to_string()).collect(),
            namespace,
            default,
        }
    }

    fn contains_import(map: &ApiSurfaceMap, imp: &Import) -> bool {
        map.imports.iter().any(|i| {
            i.module == imp.module
                && i.names == imp.names
                && i.namespace == imp.namespace
                && i.default == imp.default
        })
    }

    #[test]
    fn js_imports_all_forms() {
        let src = r#"
            import React, { useState, useEffect } from "react";
            import * as utils from "./utils";
            import def from "mod";
            import only_named from "other";
        "#;
        let map = discover_js(src);

        assert!(contains_import(
            &map,
            &import("react", &["useState", "useEffect"], false, true)
        ));
        assert!(contains_import(&map, &import("./utils", &[], true, false)));
        assert!(contains_import(&map, &import("mod", &[], false, true)));
        assert!(contains_import(&map, &import("other", &[], false, true)));
        assert_eq!(map.imports.len(), 4);
    }

    #[test]
    fn js_exports_all_kinds() {
        let src = r#"
            export function start() {}
            export const VERSION = "1.0";
            export class Service {}
            export default function () {}
            export default MyClass2 {}
        "#;
        let map = discover_js(src);

        let names: Vec<(&str, ExportKind)> = map
            .exports
            .iter()
            .map(|e| (e.name.as_str(), e.kind.clone()))
            .collect();
        assert!(names.contains(&("start", ExportKind::Function)));
        assert!(names.contains(&("VERSION", ExportKind::Const)));
        assert!(names.contains(&("Service", ExportKind::Class)));
        assert!(names.contains(&("default", ExportKind::Default)));
        assert!(names.len() >= 4);
    }

    #[test]
    fn ts_reexport_surfaces_dependency() {
        let src = r#"export { a, b } from "./other";"#;
        let map = discover_ts(src);
        assert_eq!(map.imports.len(), 1);
        assert_eq!(map.imports[0].module, "./other");
        assert!(map.exports.is_empty());
    }

    #[test]
    fn top_level_functions_collected() {
        let src = r#"
            function helper() {}
            function another() {}
            export function exposed() {}
        "#;
        let map = discover_js(src);
        assert_eq!(map.functions, vec!["helper", "another"]);
        assert!(map.exports.iter().any(|e| e.name == "exposed"));
    }

    #[test]
    fn tsx_embedded_jsx_does_not_break_parse() {
        let src = r#"
            import React from "react";
            export function App() {
                return <div className="root"><span>hello</span></div>;
            }
        "#;
        let map = discover_tsx(src);
        assert!(contains_import(&map, &import("react", &[], false, true)));
        assert!(map.exports.iter().any(|e| e.name == "App"));
    }

    #[test]
    fn comments_and_strings_ignored_unlike_tokenizer() {
        // A string containing an import-like token must not be misread.
        let src = r#"
            const s = "import { fake } from 'nope'";
            export const label = s;
        "#;
        let map = discover_js(src);
        assert!(map.imports.is_empty());
        assert!(map.exports.iter().any(|e| e.name == "label"));
    }

    #[test]
    fn empty_source_yields_empty_map() {
        assert_eq!(discover_js("").symbol_count(), 0);
        assert_eq!(discover_ts("  \n  ").symbol_count(), 0);
        assert_eq!(discover_tsx("").symbol_count(), 0);
    }

    #[test]
    fn ast_matches_tokenizer_on_clean_source() {
        // On comment/string-free source the two extractors should agree on the
        // import module set (the reference AST impl is the source of truth).
        let src = r#"
            import a from "mod-a";
            import { b, c } from "mod-b";
            export function f() {}
            export const K = 1;
            function local() {}
        "#;
        let ast = discover_ast(src, SourceLang::JavaScript);
        let tok = crate::discover(src);
        let ast_mods: std::collections::BTreeSet<&str> =
            ast.imports.iter().map(|i| i.module.as_str()).collect();
        let tok_mods: std::collections::BTreeSet<&str> =
            tok.imports.iter().map(|i| i.module.as_str()).collect();
        assert_eq!(ast_mods, tok_mods);
    }
}
