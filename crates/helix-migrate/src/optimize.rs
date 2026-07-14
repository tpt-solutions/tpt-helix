//! Stage S4 (Optimize) — dead-code elimination, struct packing suggestions,
//! and loop parallelization (spec §6.2, the *Stage S4* migration task).
//!
//! These are *suggestion* passes: each consumes a discovery/analysis result
//! and emits structured [`Suggestion`]s the migration planner can present to a
//! developer or fold into the generated Rust. They are pure and deterministic
//! so they can be unit-tested without a compiler.

use crate::ApiSurfaceMap;
use std::collections::{HashMap, HashSet};

/// Category of an optimization [`Suggestion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    DeadCode,
    StructPacking,
    LoopParallel,
}

/// A single optimization suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub category: SuggestionKind,
    pub message: String,
    /// Optional source location (e.g. a line excerpt) for the suggestion.
    pub location: Option<String>,
}

/// Count `name(` call sites that are *not* the `function name(` / `async
/// function name(` declaration, so a definition is not mistaken for a call.
fn net_call_count(source: &str, name: &str) -> usize {
    let total = source.matches(&format!("{name}(")).count();
    let defs = source.matches(&format!("function {name}(")).count()
        + source.matches(&format!("async function {name}(")).count();
    total.saturating_sub(defs)
}

/// Build a flat call graph: for each declared function, the list of other
/// declared functions it references via `name(` call sites (definitions
/// excluded).
pub fn public_call_graph(source: &str, functions: &[String]) -> HashMap<String, Vec<String>> {
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for f in functions {
        let callees: Vec<String> = functions
            .iter()
            .filter(|c| *c != f && net_call_count(source, c) > 0)
            .cloned()
            .collect();
        graph.insert(f.clone(), callees);
    }
    graph
}

/// Eliminate functions that are never reachable from the given `roots`.
///
/// Reachability starts from exported symbols plus any explicitly named entry
/// points in `roots`, then follows the call graph (built from `source`) to a
/// fixpoint. Exported symbols and roots are always retained; only unreferenced,
/// non-exported functions are dropped.
pub fn eliminate_dead_code(source: &str, roots: &[&str]) -> ApiSurfaceMap {
    let map = crate::discover(source);
    let functions = map.functions.clone();
    let mut syms = functions.clone();
    for e in &map.exports {
        if !syms.contains(&e.name) {
            syms.push(e.name.clone());
        }
    }
    let graph = public_call_graph(source, &syms);

    let mut reachable: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = Vec::new();
    for e in &map.exports {
        stack.push(e.name.clone());
    }
    for r in roots {
        stack.push((*r).to_string());
    }
    while let Some(node) = stack.pop() {
        if !reachable.insert(node.clone()) {
            continue;
        }
        if let Some(callees) = graph.get(&node) {
            for c in callees {
                if !reachable.contains(c) {
                    stack.push(c.clone());
                }
            }
        }
    }

    ApiSurfaceMap {
        imports: map.imports.clone(),
        exports: map.exports.clone(),
        functions: functions
            .into_iter()
            .filter(|f| reachable.contains(f))
            .collect(),
    }
}

/// Suggest struct field reordering to minimize padding.
///
/// Fields are supplied as `(name, size_in_bytes)`; the suggestion recommends
/// ordering them largest-first (the typical rule to minimize struct padding on
/// Rust/C-ABI layouts) and reports the estimated padded size.
pub fn suggest_struct_packing(fields: &[(String, usize)]) -> Vec<Suggestion> {
    if fields.len() < 2 {
        return Vec::new();
    }
    let mut ordered: Vec<(String, usize)> = fields.to_vec();
    ordered.sort_by_key(|x| std::cmp::Reverse(x.1));
    let reordered: Vec<String> = ordered.iter().map(|(n, _)| n.clone()).collect();

    let padded_size = |order: &[(String, usize)]| -> usize {
        let mut offset = 0usize;
        for (_, size) in order {
            let align = (*size).max(1);
            offset = offset.div_ceil(align) * align;
            offset += *size;
        }
        offset.div_ceil(8) * 8
    };

    let optimized = padded_size(&ordered);
    let original = padded_size(fields);

    let mut out = Vec::new();
    if reordered
        .iter()
        .map(|s| s.as_str())
        .ne(fields.iter().map(|(n, _)| n.as_str()))
        || optimized < original
    {
        out.push(Suggestion {
            category: SuggestionKind::StructPacking,
            message: format!(
                "Reorder fields largest-first to reduce padding: {} (est. {} bytes vs {} bytes)",
                reordered.join(", "),
                optimized,
                original
            ),
            location: None,
        });
    } else {
        out.push(Suggestion {
            category: SuggestionKind::StructPacking,
            message: "Field order is already packing-efficient".to_string(),
            location: None,
        });
    }
    out
}

/// Detect `for`/`while` loops in `source` and suggest parallelizing them with
/// `rayon` (`.par_iter()`).
pub fn suggest_parallel_loops(source: &str) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or(raw).trim();
        let is_loop = line.starts_with("for (")
            || line.starts_with("for(")
            || line.starts_with("while (")
            || line.starts_with("while(")
            || line.contains("for (let")
            || line.contains("for (const")
            || line.contains("for (var");
        if is_loop {
            let excerpt = line.chars().take(48).collect::<String>();
            out.push(Suggestion {
                category: SuggestionKind::LoopParallel,
                message: format!(
                    "Consider parallelizing with rayon (e.g. `.par_iter()`): {excerpt}"
                ),
                location: Some(format!("line {}", i + 1)),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_unreferenced_non_exported_functions() {
        // `used` is called by `main` (a root); `orphan` is never referenced.
        let src = r#"
            export function main() { used(); }
            function used() {}
            function orphan() {}
        "#;
        let kept = eliminate_dead_code(src, &["main"]);
        assert!(kept.functions.contains(&"used".to_string()));
        assert!(!kept.functions.contains(&"orphan".to_string()));
        // Exports are always retained even if unreferenced.
        assert!(kept.exports.iter().any(|e| e.name == "main"));
    }

    #[test]
    fn keeps_roots_even_when_not_exported_and_follows_calls() {
        let src = r#"
            function entry() { mid(); }
            function mid() { leaf(); }
            function leaf() {}
            function unused() {}
        "#;
        let kept = eliminate_dead_code(src, &["entry"]);
        for f in ["entry", "mid", "leaf"] {
            assert!(kept.functions.contains(&f.to_string()), "expected {f} kept");
        }
        assert!(!kept.functions.contains(&"unused".to_string()));
    }

    #[test]
    fn suggests_reorder_for_padding() {
        // bool(1) then u64(8) → padded; reordering helps.
        let sug = suggest_struct_packing(&[("flag".into(), 1), ("count".into(), 8)]);
        assert_eq!(sug.len(), 1);
        assert_eq!(sug[0].category, SuggestionKind::StructPacking);
        assert!(sug[0].message.contains("count, flag"));
    }

    #[test]
    fn detects_loops_for_parallelization() {
        let src = "for (let i = 0; i < n; i++) { sum += a[i]; }\nconst x = 1;\nwhile (running) { step(); }";
        let sug = suggest_parallel_loops(src);
        assert_eq!(sug.len(), 2);
        assert!(
            sug.iter()
                .all(|s| s.category == SuggestionKind::LoopParallel)
        );
        assert!(sug[0].location.is_some());
    }
}
