//! QuickJS fallback interpreter profiling benchmark.
//!
//! Measures the memory and throughput overhead of the legacy JS path so we
//! can track regression against the §7 budget and inform the boa evaluation
//! (TODO.md: "Evaluate boa as an alternative/replacement path").
//!
//! Run with `cargo bench -p helix-runtime --bench js_bench`.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use helix_runtime::js::Interpreter;

fn bench_js(c: &mut Criterion) {
    let mut group = c.benchmark_group("quickjs");

    // --- Cold start: cost of spinning up a fresh QuickJS runtime ---
    group.bench_function("cold_start", |b| {
        b.iter(|| Interpreter::new().expect("interpreter creates"));
    });

    // --- Simple eval: arithmetic (tiny script, engine overhead dominated) ---
    group.bench_function("eval_arithmetic", |b| {
        let interp = Interpreter::new().unwrap();
        b.iter(|| interp.eval_to_string("1 + 2 * 3").unwrap());
    });

    // --- String concat: moderate work ---
    group.bench_function("eval_string_concat", |b| {
        let interp = Interpreter::new().unwrap();
        b.iter(|| {
            interp
                .eval_to_string("'hello' + ' ' + 'world' + ' ' + 42")
                .unwrap()
        });
    });

    // --- Array iteration: realistic lightweight work ---
    group.bench_function("eval_array_iteration", |b| {
        let interp = Interpreter::new().unwrap();
        b.iter(|| {
            interp
                .eval_to_string(
                    "var s = 0; for (var i = 0; i < 100; i++) { s += i; } s",
                )
                .unwrap()
        });
    });

    // --- Large function parse: measures compile overhead ---
    let large_script = make_large_script(500);
    group.bench_with_input(
        BenchmarkId::new("eval_large_function", large_script.len()),
        &large_script,
        |b, script| {
            let interp = Interpreter::new().unwrap();
            b.iter(|| interp.eval_to_string(script).unwrap());
        },
    );

    // --- Timeout path: overhead of the interrupt handler machinery ---
    group.bench_function("eval_with_timeout_overhead", |b| {
        use std::time::Duration;
        let interp = Interpreter::new().unwrap();
        b.iter(|| {
            interp
                .eval_with_timeout("1 + 2", Duration::from_secs(5))
                .unwrap()
        });
    });

    group.finish();
}

/// Build a JS function with `n` branches to exercise the parser/compiler on a
/// non-trivial but still single-parse workload.
fn make_large_script(n: usize) -> String {
    let mut lines = Vec::with_capacity(n + 2);
    lines.push("function compute(x) {".to_string());
    for i in 0..n {
        lines.push(format!(
            "  if (x === {i}) return {i} * 2;",
        ));
    }
    lines.push("  return 0;".to_string());
    lines.push("}".to_string());
    lines.push("compute(42)".to_string());
    lines.join("\n")
}

criterion_group!(benches, bench_js);
criterion_main!(benches);
