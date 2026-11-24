# Security Policy

## Reporting a vulnerability

Please do not open a public issue for security problems. Report them privately
to the maintainers (contact details in the repository profile) with:

- the affected crate and file(s), and the commit you tested against;
- a minimal reproduction (a WASM guest, URL, or input bytes);
- the observed impact, e.g. a capability check that allows an operation it
  should deny, or a guest that escapes its fuel or memory budget.

We aim to acknowledge reports within a few business days and to agree a
disclosure timeline with the reporter before anything is published.

## Scope

In scope: the capability broker (`crates/tpt-helix-runtime/src/capability.rs`),
the wasmtime host and its limits (`wasm.rs`), the DOM/storage/network stub
(`stub.rs`), the JS bridge (`js_bridge.rs`), and the content and P2P layers.

Helix has no ambient authority by design (spec §5). A report that an
operation succeeds without a grant is treated as a high-severity issue.

## Current hardening status

Tracked in `TODO.md` under "Review Follow-ups". Items already in place include
deny-by-default `wasm::Host::new()`, fuel and memory/table limits, and
OS-entropy capability tokens. Items still open are listed there and should be
treated as known gaps.
