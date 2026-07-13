//! Task: Embed QuickJS as the legacy JS fallback interpreter.
//!
//! `rquickjs` is used because it wraps QuickJS (the same reference engine
//! `spec.txt` §4.2 names as the fallback target) with a safe Rust API.
//! Nothing here is on the WASM hot path (per G1): this interpreter only runs
//! legacy JS that hasn't been migrated to WASM yet.

use rquickjs::{Context, Runtime};

/// A single legacy-JS execution environment. One [`Interpreter`] per app
/// instance keeps globals (and any bridged host functions, see
/// [`crate::js_bridge`]) isolated between apps.
pub struct Interpreter {
    context: Context,
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
        let context = Context::full(&runtime).map_err(|e| JsError(e.to_string()))?;
        Ok(Interpreter { context })
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
        assert_eq!(interpreter.eval_to_string("1 + 2").unwrap(), Some("3".to_string()));
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
        assert!(interpreter.eval_to_string("this is not valid js (((").is_err());
    }

    #[test]
    fn interpreters_do_not_share_global_state() {
        let a = Interpreter::new().unwrap();
        let b = Interpreter::new().unwrap();
        a.eval_to_string("globalThis.x = 42").unwrap();
        assert_eq!(a.eval_to_string("globalThis.x").unwrap(), Some("42".to_string()));
        assert_eq!(b.eval_to_string("globalThis.x").unwrap(), None);
    }
}
