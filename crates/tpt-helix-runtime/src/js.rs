//! Task: Embed QuickJS as the legacy JS fallback interpreter.
//!
//! `rquickjs` is used because it wraps QuickJS (the same reference engine
//! `spec.txt` §4.2 names as the fallback target) with a safe Rust API.
//! Nothing here is on the WASM hot path (per G1): this interpreter only runs
//! legacy JS that hasn't been migrated to WASM yet.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rquickjs::{Context, Runtime};

/// A single legacy-JS execution environment. One [`Interpreter`] per app
/// instance keeps globals (and any bridged host functions, see
/// [`crate::js_bridge`]) isolated between apps.
///
/// The underlying [`Runtime`] is owned here so the [`Context`] (which borrows
/// from it) stays valid for the interpreter's lifetime, and so a per-eval
/// timeout can be installed via an interrupt handler ([`Self::eval_with_timeout`]).
pub struct Interpreter {
    context: Context,
    #[allow(dead_code)]
    runtime: Runtime,
    /// Wall-clock deadline (ms since epoch) after which evaluation is aborted,
    /// or `0` for "no deadline". Shared with the interrupt handler closure.
    deadline: Arc<AtomicU64>,
}

/// A JS evaluation error, carrying the engine's message rather than the full
/// `rquickjs::Error` (which borrows interpreter-internal state).
#[derive(Debug, Clone, PartialEq)]
pub struct JsError(pub String);

impl std::fmt::Display for JsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for JsError {}

impl Interpreter {
    /// Creates a fresh interpreter with a new QuickJS runtime and global
    /// context (no host functions bridged in yet).
    pub fn new() -> Result<Self, JsError> {
        let runtime = Runtime::new().map_err(|e| JsError(e.to_string()))?;
        // Install an interrupt handler that aborts evaluation once a per-eval
        // deadline (set by `eval_with_timeout`) has passed. An unset deadline
        // (`0`) never interrupts.
        let deadline = Arc::new(AtomicU64::new(0));
        let handler_deadline = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let d = handler_deadline.load(Ordering::Relaxed);
            if d == 0 {
                return false;
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            now >= d
        })));
        let context = Context::full(&runtime).map_err(|e| JsError(e.to_string()))?;
        Ok(Interpreter {
            context,
            runtime,
            deadline,
        })
    }

    /// Evaluate `source` with an execution budget of `timeout`.
    ///
    /// The interpreter exposes no host capabilities (host functions are bridged
    /// separately in [`crate::js_bridge`]), so evaluating here is the sandbox
    /// for untrusted/dynamic legacy JS: a fresh, capability-free context that is
    /// interrupted if it runs past `timeout`. Returns an error if the budget is
    /// exceeded; the deadline is always cleared afterwards so later evals are
    /// unaffected.
    pub fn eval_with_timeout(
        &self,
        source: &str,
        timeout: Duration,
    ) -> Result<Option<String>, JsError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.deadline
            .store(now + timeout.as_millis() as u64, Ordering::Relaxed);
        let result = self.eval_to_string(source);
        self.deadline.store(0, Ordering::Relaxed);
        result
    }

    /// Evaluates `source` and returns its result coerced to a JS string via
    /// `String(value)`, or `None` for `undefined`/`null`. A minimal
    /// evaluation surface is enough to prove the engine is embedded end to
    /// end; typed results should go through [`Self::with`] instead.
    pub fn eval_to_string(&self, source: &str) -> Result<Option<String>, JsError> {
        self.context.with(|ctx| {
            let value: rquickjs::Value = ctx.eval(source).map_err(|e| JsError(e.to_string()))?;
            if value.is_undefined() || value.is_null() {
                return Ok(None);
            }
            let s: String = value
                .get::<rquickjs::Coerced<String>>()
                .map_err(|e| JsError(e.to_string()))?
                .0;
            Ok(Some(s))
        })
    }

    /// Runs `f` with the interpreter's [`rquickjs::Ctx`], for callers (like
    /// [`crate::js_bridge`]) that need to register host functions or work
    /// with typed JS values directly.
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(rquickjs::Ctx<'_>) -> R,
    {
        self.context.with(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_arithmetic() {
        let interpreter = Interpreter::new().unwrap();
        assert_eq!(
            interpreter.eval_to_string("1 + 2").unwrap(),
            Some("3".to_string())
        );
    }

    #[test]
    fn evaluates_string_concatenation() {
        let interpreter = Interpreter::new().unwrap();
        let result = interpreter.eval_to_string("'hello, ' + 'world'").unwrap();
        assert_eq!(result, Some("hello, world".to_string()));
    }

    #[test]
    fn undefined_expressions_yield_none() {
        let interpreter = Interpreter::new().unwrap();
        assert_eq!(interpreter.eval_to_string("undefined").unwrap(), None);
    }

    #[test]
    fn syntax_errors_surface_as_js_error() {
        let interpreter = Interpreter::new().unwrap();
        assert!(
            interpreter
                .eval_to_string("this is not valid js (((")
                .is_err()
        );
    }

    #[test]
    fn interpreters_do_not_share_global_state() {
        let a = Interpreter::new().unwrap();
        let b = Interpreter::new().unwrap();
        a.eval_to_string("globalThis.x = 42").unwrap();
        assert_eq!(
            a.eval_to_string("globalThis.x").unwrap(),
            Some("42".to_string())
        );
        assert_eq!(b.eval_to_string("globalThis.x").unwrap(), None);
    }

    #[test]
    fn eval_with_timeout_returns_results_and_clears_budget() {
        let interp = Interpreter::new().unwrap();
        // Within budget the result is unaffected by the timeout machinery.
        assert_eq!(
            interp
                .eval_with_timeout("6 * 7", Duration::from_secs(1))
                .unwrap(),
            Some("42".to_string())
        );
        // The deadline is cleared afterwards, so a subsequent eval still works.
        assert_eq!(
            interp
                .eval_with_timeout("1 + 1", Duration::from_secs(1))
                .unwrap(),
            Some("2".to_string())
        );
    }

    /// Regression test for Q1: untrusted/dynamic legacy JS that never terminates
    /// must be *aborted* by the per-eval deadline, and the interpreter must remain
    /// usable afterwards (the deadline is cleared on abort).
    ///
    /// The eval runs on a worker thread so a non-interrupting engine (or a future
    /// regression that disables the interrupt handler) fails the test via a
    /// `recv_timeout` rather than hanging the whole suite.
    #[test]
    fn eval_with_timeout_aborts_infinite_loop() {
        use std::sync::mpsc;
        use std::time::Duration;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let interp = Interpreter::new().unwrap();
            // An infinite loop must be aborted once the deadline passes.
            let aborted = interp.eval_with_timeout("while(true){}", Duration::from_millis(200));
            // The deadline is cleared after the abort, so later evals still work.
            let recovered = interp.eval_with_timeout("21 * 2", Duration::from_secs(1));
            let _ = tx.send((aborted, recovered));
        });

        let (aborted, recovered) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("eval was not aborted within its deadline — timeout/abort behavior is broken");

        assert!(
            aborted.is_err(),
            "an infinite loop was not aborted by the timeout"
        );
        assert_eq!(
            recovered.expect("interpreter must recover after abort"),
            Some("42".to_string())
        );
    }

    /// The sandbox exposes no host functions, so a fresh interpreter must not
    /// be able to reach host/IO machinery (regression guard for the Q1 sandbox).
    #[test]
    fn sandboxed_interpreter_has_no_bridged_host_functions() {
        let interp = Interpreter::new().unwrap();
        // `__helix_*` host hooks are only installed by `js_bridge`; a bare
        // interpreter must report them as undefined rather than silently
        // exposing host authority.
        let err = interp
            .eval_to_string("typeof __helix_create_element")
            .unwrap();
        assert_eq!(err, Some("undefined".to_string()));
    }
}
