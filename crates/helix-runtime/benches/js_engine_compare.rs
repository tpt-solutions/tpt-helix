//! Comparative benchmark: QuickJS (rquickjs) vs Boa (boa_engine).
//!
//! Runs identical workloads through both JS engines to measure relative
//! throughput and startup cost. This informs the TODO.md evaluation:
//! "Evaluate boa as an alternative/replacement path".
//!
//! Run with `cargo bench -p helix-runtime --bench js_engine_compare`.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use helix_runtime::js::Interpreter;

fn bench_engine_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("js_engine_comparison");

    // --- Cold start ---
    group.bench_function("quickjs_cold_start", |b| {
        b.iter(|| Interpreter::new().expect("interpreter creates"));
    });
    group.bench_function("boa_cold_start", |b| {
        b.iter(|| boa_engine::Context::default());
    });

    // --- Simple eval ---
    group.bench_function("quickjs_eval_arithmetic", |b| {
        let interp = Interpreter::new().unwrap();
        b.iter(|| interp.eval_to_string("1 + 2 * 3").unwrap());
    });
    group.bench_function("boa_eval_arithmetic", |b| {
        let mut ctx = boa_engine::Context::default();
        b.iter(|| {
            ctx.eval(boa_engine::Source::from_bytes("1 + 2 * 3"))
                .unwrap()
        });
    });

    // --- String concat ---
    group.bench_function("quickjs_eval_string_concat", |b| {
        let interp = Interpreter::new().unwrap();
        b.iter(|| {
            interp
                .eval_to_string("'hello' + ' ' + 'world' + ' ' + 42")
                .unwrap()
        });
    });
    group.bench_function("boa_eval_string_concat", |b| {
        let mut ctx = boa_engine::Context::default();
        b.iter(|| {
            ctx.eval(boa_engine::Source::from_bytes(
                "'hello' + ' ' + 'world' + ' ' + 42",
            ))
            .unwrap()
        });
    });

    // --- Array iteration ---
    group.bench_function("quickjs_eval_array_loop", |b| {
        let interp = Interpreter::new().unwrap();
        b.iter(|| {
            interp
                .eval_to_string("var s = 0; for (var i = 0; i < 100; i++) { s += i; } s")
                .unwrap()
        });
    });
    group.bench_function("boa_eval_array_loop", |b| {
        let mut ctx = boa_engine::Context::default();
        b.iter(|| {
            ctx.eval(boa_engine::Source::from_bytes(
                "var s = 0; for (var i = 0; i < 100; i++) { s += i; } s",
            ))
            .unwrap()
        });
    });

    // --- Large function parse ---
    let large_script = make_large_script(500);
    group.bench_with_input(
        BenchmarkId::new("quickjs_eval_large_function", large_script.len()),
        &large_script,
        |b, script| {
            let interp = Interpreter::new().unwrap();
            b.iter(|| interp.eval_to_string(script).unwrap());
        },
    );
    group.bench_with_input(
        BenchmarkId::new("boa_eval_large_function", large_script.len()),
        &large_script,
        |b, script| {
            let mut ctx = boa_engine::Context::default();
            b.iter(|| ctx.eval(boa_engine::Source::from_bytes(script.as_str())).unwrap());
        },
    );

    group.finish();
}

fn make_large_script(n: usize) -> String {
    let mut lines = Vec::with_capacity(n + 2);
    lines.push("function compute(x) {".to_string());
    for i in 0..n {
        lines.push(format!("  if (x === {i}) return {i} * 2;",));
    }
    lines.push("  return 0;".to_string());
    lines.push("}".to_string());
    lines.push("compute(42)".to_string());
    lines.join("\n")
}

criterion_group!(benches, bench_engine_comparison);
criterion_main!(benches);
