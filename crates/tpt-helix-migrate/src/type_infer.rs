//! Stage "type inference" — JS/TS → Rust type generation (spec §6.2, the
//! *type inference* migration task).
//!
//! The pipeline needs to turn loosely-typed JS into statically-typed Rust. This
//! module provides two backends behind a common [`TypeBinding`] contract:
//!
//! * [`infer_types`] — a **custom**, dependency-free heuristic inferer built on
//!   the same token-scan discipline as [`crate::discover`]. It reads `let`/`const`
//!   literal initializers and `function` return literals, plus any explicit TS
//!   type annotations, and maps them to [`InferredType`].
//! * [`infer_types_via_tsc`] — a `tsc`-backed provider (the "tsc APIs" path). It
//!   shells out to the TypeScript compiler to emit a `.d.ts` and parses the
//!   resulting type annotations. It is best-effort: it returns `Err` when `tsc`
//!   is not on `PATH`, so CI never depends on it.
//!
//! The custom backend is the default and is what the test suite exercises; the
//! `tsc` backend is the higher-fidelity option when a toolchain is available.

use std::fmt::Write as _;
use std::process::Command;

/// A Rust-side type inferred from a JS/TS binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferredType {
    Str,
    Int,
    Float,
    Bool,
    Null,
    Array,
    Object,
    Unknown,
}

impl InferredType {
    /// The Rust type name this inference maps to.
    pub fn to_rust(self) -> &'static str {
        match self {
            InferredType::Str => "String",
            InferredType::Int => "i64",
            InferredType::Float => "f64",
            InferredType::Bool => "bool",
            InferredType::Null => "()",
            InferredType::Array => "Vec<serde_json::Value>",
            InferredType::Object => "serde_json::Value",
            InferredType::Unknown => "_",
        }
    }
}

/// Where a [`TypeBinding`] was derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    Let,
    Const,
    Var,
    Param,
    Return,
    Field,
}

/// An inferred (name → type) binding extracted from JS/TS source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeBinding {
    pub name: String,
    pub ty: InferredType,
    pub kind: BindingKind,
}

fn strip_line_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

fn literal_type(lit: &str) -> InferredType {
    let t = lit.trim();
    if t.is_empty() {
        return InferredType::Unknown;
    }
    // Template / double / single quoted string.
    if (t.starts_with('"') && t.ends_with('"'))
        || (t.starts_with('\'') && t.ends_with('\''))
        || (t.starts_with('`') && t.ends_with('`'))
    {
        return InferredType::Str;
    }
    if t == "true" || t == "false" {
        return InferredType::Bool;
    }
    if t == "null" || t == "undefined" {
        return InferredType::Null;
    }
    if t.starts_with('[') {
        return InferredType::Array;
    }
    if t.starts_with('{') {
        return InferredType::Object;
    }
    // Numeric literal.
    if t.chars()
        .all(|c| c.is_ascii_digit() || c == '_' || c == '+' || c == '-')
    {
        return InferredType::Int;
    }
    if t.chars().all(|c| {
        c.is_ascii_digit() || c == '.' || c == '_' || c == 'e' || c == 'E' || c == '+' || c == '-'
    }) && (t.contains('.') || t.contains('e') || t.contains('E'))
    {
        return InferredType::Float;
    }
    if t.chars().all(|c| {
        c.is_ascii_digit() || c == '.' || c == '_' || c == 'e' || c == 'E' || c == '+' || c == '-'
    }) {
        return InferredType::Int;
    }
    InferredType::Unknown
}

/// Map a TS-style type annotation to an [`InferredType`].
fn ts_annotation_type(ann: &str) -> InferredType {
    match ann.trim() {
        "string" => InferredType::Str,
        "number" => InferredType::Float,
        "boolean" | "bool" => InferredType::Bool,
        "any" | "unknown" | "object" => InferredType::Unknown,
        "null" | "undefined" => InferredType::Null,
        s if s.ends_with("[]") => InferredType::Array,
        s if s.starts_with('{') || s.starts_with("Record<") || s.starts_with("Partial<") => {
            InferredType::Object
        }
        _ => InferredType::Unknown,
    }
}

/// Custom (dependency-free) type inference over JS/TS `source`.
///
/// Scans each line for `let`/`const`/`var` declarations with a literal
/// initializer, explicit TS `: type` annotations, and `function … { return … }`
/// return literals, producing one [`TypeBinding`] per inferred symbol.
pub fn infer_types(source: &str) -> Vec<TypeBinding> {
    let mut bindings: Vec<TypeBinding> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();

    // First pass: declarations with optional TS annotation or literal initializer.
    // Split each line on `;` so multiple declarations on one line each resolve.
    for raw in source.lines() {
        let line = strip_line_comment(raw);
        for stmt in line.split(';') {
            let stmt = stmt.trim();
            for kw in ["const ", "let ", "var "] {
                if let Some(rest) = stmt.strip_prefix(kw) {
                    let name = rest
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        continue;
                    }
                    // Explicit TS annotation `name: Type = ...`.
                    let mut ty = InferredType::Unknown;
                    if let Some(colon) = rest.find(':') {
                        let ann = rest[colon + 1..].split('=').next().unwrap_or("").trim();
                        if !ann.is_empty() {
                            ty = ts_annotation_type(ann);
                        }
                    }
                    // Literal initializer `= <lit>` (only when no annotation was found).
                    if let Some(eq) = rest.find('=') {
                        let lit = rest[eq + 1..].trim();
                        if ty == InferredType::Unknown && !lit.is_empty() {
                            ty = literal_type(lit);
                        }
                    }
                    if seen.insert(name.clone()) {
                        let kind = match kw {
                            "const " => BindingKind::Const,
                            "let " => BindingKind::Let,
                            _ => BindingKind::Var,
                        };
                        bindings.push(TypeBinding { name, ty, kind });
                    }
                }
            }
        }
    }

    // Second pass: function return literals — `function name(...) { return <lit>; }`
    for (i, raw) in source.lines().enumerate() {
        let line = strip_line_comment(raw).trim();
        if let Some(rest) = line.strip_prefix("function ") {
            let name = rest
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            // Prefer an explicit `: Type` return annotation if present.
            if let Some(colon) = rest.find(':') {
                let ann = rest[colon + 1..]
                    .split(['{', '('])
                    .next()
                    .unwrap_or("")
                    .trim();
                if !ann.is_empty() {
                    if let Some(b) = bindings.iter_mut().find(|b| b.name == name) {
                        b.ty = ts_annotation_type(ann);
                    } else if seen.insert(name.clone()) {
                        bindings.push(TypeBinding {
                            name,
                            ty: ts_annotation_type(ann),
                            kind: BindingKind::Return,
                        });
                    }
                    continue;
                }
            }
            // Otherwise scan forward for the first `return <lit>;`.
            if let Some(ret) = source.lines().skip(i).find_map(|l| {
                let l = strip_line_comment(l);
                l.find("return ").map(|p| &l[p + "return ".len()..])
            }) {
                let lit = ret.trim_end_matches(';').trim();
                let ty = literal_type(lit);
                if ty != InferredType::Unknown {
                    if let Some(b) = bindings.iter_mut().find(|b| b.name == name) {
                        b.ty = ty;
                    } else if seen.insert(name.clone()) {
                        bindings.push(TypeBinding {
                            name,
                            ty,
                            kind: BindingKind::Return,
                        });
                    }
                }
            }
        }
    }

    bindings
}

/// Emit inferred bindings as Rust type aliases inside an `inferred` module.
///
/// Each binding becomes `pub type <name>_t = <rust>;`, giving the transpiler a
/// ready-to-paste set of types to reference from generated guest source.
pub fn generate_rust_type_decls(bindings: &[TypeBinding]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "// Inferred types (custom JS → Rust inference)");
    let _ = writeln!(s, "pub mod inferred {{");
    for b in bindings {
        let _ = writeln!(s, "    pub type {}_t = {};", b.name, b.ty.to_rust());
    }
    let _ = writeln!(s, "}}");
    s
}

/// Whether the `tsc` binary is resolvable on `PATH`.
pub fn tsc_available() -> bool {
    Command::new("tsc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `tsc`-backed type inference (the "tsc APIs" path).
///
/// Writes `source` to a temporary `.ts` file, asks `tsc` to emit a `.d.ts`
/// declaration, and parses the resulting `: Type` annotations, mapping them
/// through [`ts_annotation_type`]. Returns `Err` if `tsc` is unavailable or the
/// compile step fails. Falls back to [`infer_types`] when you only need a
/// guaranteed result.
pub fn infer_types_via_tsc(source: &str) -> std::io::Result<Vec<TypeBinding>> {
    if !tsc_available() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "tsc not found on PATH",
        ));
    }
    let dir = std::env::temp_dir().join("helix-migrate-tsc");
    std::fs::create_dir_all(&dir)?;
    let src = dir.join("infer.ts");
    let out = dir.join("infer.d.ts");
    std::fs::write(&src, source)?;
    let status = Command::new("tsc")
        .arg(&src)
        .arg("--declaration")
        .arg("--emitDeclarationOnly")
        .arg("--outFile")
        .arg(&out)
        .output()?;
    if !status.status.success() {
        return Err(std::io::Error::other("tsc failed to emit declarations"));
    }
    let decls = std::fs::read_to_string(&out)?;
    // Parse `name: Type` annotations out of the declaration file.
    let mut bindings = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();
    for line in decls.lines() {
        let line = strip_line_comment(line).trim();
        if let Some(colon) = line.rfind(':') {
            let name = line[..colon]
                .trim_end_matches("export")
                .trim_end_matches("declare")
                .split_whitespace()
                .last()
                .unwrap_or("")
                .trim_end_matches('?')
                .to_string();
            let ann = line[colon + 1..].trim();
            if name.is_empty() || ann.is_empty() {
                continue;
            }
            if seen.insert(name.clone()) {
                bindings.push(TypeBinding {
                    name,
                    ty: ts_annotation_type(ann),
                    kind: BindingKind::Let,
                });
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_literal_initializers() {
        let src = r#"
            const name = "helix";
            let count = 42;
            var ratio = 3.14;
            let active = true;
            const items = [1, 2, 3];
            let cfg = { a: 1 };
        "#;
        let types = infer_types(src);
        let get = |n: &str| types.iter().find(|b| b.name == n).map(|b| b.ty);
        assert_eq!(get("name"), Some(InferredType::Str));
        assert_eq!(get("count"), Some(InferredType::Int));
        assert_eq!(get("ratio"), Some(InferredType::Float));
        assert_eq!(get("active"), Some(InferredType::Bool));
        assert_eq!(get("items"), Some(InferredType::Array));
        assert_eq!(get("cfg"), Some(InferredType::Object));
    }

    #[test]
    fn honors_ts_annotations_over_literals() {
        let src = r#"
            let explicit: number = "not really a number";
            const flag: boolean = 1;
        "#;
        let types = infer_types(src);
        let get = |n: &str| types.iter().find(|b| b.name == n).map(|b| b.ty);
        assert_eq!(get("explicit"), Some(InferredType::Float));
        assert_eq!(get("flag"), Some(InferredType::Bool));
    }

    #[test]
    fn infers_function_return_literals() {
        let src = r#"
            function greeting() {
                return "hi";
            }
            function answer() {
                return 42;
            }
        "#;
        let types = infer_types(src);
        let get = |n: &str| types.iter().find(|b| b.name == n).map(|b| b.ty);
        assert_eq!(get("greeting"), Some(InferredType::Str));
        assert_eq!(get("answer"), Some(InferredType::Int));
    }

    #[test]
    fn generates_rust_type_decls() {
        let src = r#"const name = "x"; let n = 1;"#;
        let decls = generate_rust_type_decls(&infer_types(src));
        assert!(decls.contains("pub mod inferred {"));
        assert!(decls.contains("pub type name_t = String;"));
        assert!(decls.contains("pub type n_t = i64;"));
    }

    #[test]
    fn tsc_backend_reports_unavailable_without_toolchain() {
        // When tsc is absent this must fail gracefully (never panic).
        if !tsc_available() {
            assert!(infer_types_via_tsc("const a = 1;").is_err());
        }
    }
}
