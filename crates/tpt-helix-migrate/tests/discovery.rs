//! Parity + fidelity tests for the Stage S1 discovery extractors.
//!
//! [`tpt_helix_migrate::discover`] runs the tree-sitter backend
//! ([`tpt_helix_migrate::discover_ast`]); these tests pin the
//! [`tpt_helix_migrate::ApiSurfaceMap`] contract and the grammar choice.

use tpt_helix_migrate::{ApiSurfaceMap, SourceLang, discover, discover_js, discover_ts};

const SAMPLE: &str = r#"
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

#[test]
fn discover_delegates_to_the_ast_frontend() {
    // `discover` is the public S1 entry point and runs the AST frontend (TS
    // grammar). The JS and TS grammars must agree on plain JavaScript.
    let via_discover = discover(SAMPLE);
    assert_eq!(via_discover, discover_ts(SAMPLE));
    assert_eq!(
        via_discover,
        discover_js(SAMPLE),
        "JS and TS grammars diverged"
    );
}

#[test]
fn ast_extracts_imports_and_exports() {
    let map = discover_js(SAMPLE);

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
}

#[test]
fn ast_is_robust_to_strings_containing_import_keywords() {
    // The tokenizer would mis-scan the `import` word inside the string literal;
    // the AST parser must not treat it as a real import.
    let src = r#"
        const s = "import nothing from 'nowhere'";
        export function real() {}
    "#;
    let map = discover_js(src);
    assert!(
        map.imports.is_empty(),
        "string literal must not yield imports"
    );
    assert_eq!(map.exports.len(), 1);
    assert_eq!(map.exports[0].name, "real");
}

#[test]
fn typescript_grammar_extracts_type_reexports() {
    let src = r#"
        import { Foo, Bar } from "./types";
        export type Alias = Foo;
        export interface Service { id: number }
        export const cfg = 1;
    "#;
    let map: ApiSurfaceMap = discover_ts(src);
    assert_eq!(map.imports.len(), 1);
    assert_eq!(map.imports[0].names, vec!["Foo", "Bar"]);
    let names: Vec<&str> = map.exports.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"cfg"));
}

#[test]
fn tsx_grammar_parses_jsx() {
    let src = r#"
        import React from "react";
        export function App() {
            return <div className="x">{null}</div>;
        }
    "#;
    let map = tpt_helix_migrate::discover_ast(src, SourceLang::Tsx);
    assert_eq!(map.imports.len(), 1);
    assert!(map.imports[0].default);
    assert_eq!(map.exports[0].name, "App");
}
