//! Boa engine evaluation: API compatibility and functional correctness.
//!
//! Runs the same workload contract as QuickJS (`js.rs`) through Boa to
//! verify it can serve as a drop-in fallback replacement. The performance
//! comparison lives in `benches/js_engine_compare.rs`.

use boa_engine::Context;

#[test]
fn boa_evaluates_arithmetic() {
    let mut ctx = Context::default();
    let result = ctx.eval(boa_engine::Source::from_bytes("1 + 2"));
    assert!(result.is_ok());
}

#[test]
fn boa_evaluates_string_concatenation() {
    let mut ctx = Context::default();
    let result = ctx.eval(boa_engine::Source::from_bytes("'hello' + ' ' + 'world'"));
    assert!(result.is_ok());
}

#[test]
fn boa_undefined_expressions_do_not_panic() {
    let mut ctx = Context::default();
    let result = ctx.eval(boa_engine::Source::from_bytes("undefined"));
    assert!(result.is_ok());
}

#[test]
fn boa_syntax_errors_surface() {
    let mut ctx = Context::default();
    let result = ctx.eval(boa_engine::Source::from_bytes("this is not valid js ((("));
    assert!(result.is_err());
}

#[test]
fn boa_global_state_isolated_per_context() {
    let mut a = Context::default();
    let mut b = Context::default();
    a.eval(boa_engine::Source::from_bytes("var x = 42")).unwrap();
    // b must not see a's global
    let b_result = b.eval(boa_engine::Source::from_bytes("typeof x")).unwrap();
    assert_eq!(b_result.as_string().unwrap().to_std_string_escaped(), "undefined");
}

#[test]
fn boa_handles_dom_bridge_pattern() {
    let mut ctx = Context::default();
    // Register a host function (simulating __helix_create_element)
    ctx.register_global_builtin_callable(
        boa_engine::js_string!("__helix_create_element"),
        1,
        boa_engine::NativeFunction::from_fn_ptr(|_ctx, _args, _new_target| {
            Ok(boa_engine::JsValue::undefined())
        }),
    )
    .unwrap();
    let result = ctx.eval(boa_engine::Source::from_bytes(
        "typeof __helix_create_element",
    ));
    assert!(result.is_ok());
    assert_eq!(
        result.unwrap().as_string().unwrap().to_std_string_escaped(),
        "function"
    );
}

#[test]
fn boa_handles_array_iteration() {
    let mut ctx = Context::default();
    let result = ctx.eval(boa_engine::Source::from_bytes(
        "var s = 0; for (var i = 0; i < 100; i++) { s += i; } s",
    ));
    assert!(result.is_ok());
    // Sum of 0..100 = 4950
    let val = result.unwrap();
    assert_eq!(val.as_number().unwrap() as i64, 4950);
}

#[test]
fn boa_handles_large_function() {
    let mut ctx = Context::default();
    let mut script = String::from("function compute(x) {");
    for i in 0..500 {
        script.push_str(&format!("  if (x === {i}) return {i} * 2;"));
    }
    script.push_str("  return 0; } compute(42)");
    let result = ctx.eval(boa_engine::Source::from_bytes(&script));
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_number().unwrap() as i64, 84);
}
