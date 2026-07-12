# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

A Rust cargo workspace (resolver 2) building the **Helix Runtime**: a WASM-native
web platform pitched as a successor to the browser (see `spec.txt`). The repo is
early-stage — currently one workspace member, `crates/helix-runtime` (edition 2024),
with HTML5 parsing (`html5ever`) wired up and CSS parsing (`lightningcss` + `selectors`)
in progress. Most of the architecture described in `spec.txt` (WASM host, capability
broker, GPU renderer, AI migration pipeline) does not exist as code yet.

## Source of truth

- `spec.txt` is the living design specification (v0.1). If scope changes, update
  `spec.txt` *first*, then reflect the change in `TODO.md` — `TODO.md` is a derived
  execution checklist, not a design doc.
- `TODO.md` tracks work grouped into Phases 0–4 (Foundation → Core → Migration →
  Ecosystem → Displacement). Active phase is **Phase 0: Foundation**. Check off tasks
  there as they're completed; don't add new tasks that aren't traceable back to `spec.txt`.
- Draft WIT interfaces for `network`, `storage`, `dom`, and `media` live in `spec.txt`
  §5.2 — treat them as the canonical shape for any future `wit/` files. Don't invent
  interface signatures that contradict §5.2 without updating the spec first.
- `TODO 1260713.md` is a duplicate snapshot of `TODO.md` — keep both in sync if you
  edit one (or ask whether it should just be removed).

## Commands

- Build: `cargo build` (or `cargo build -p helix-runtime`)
- Test: `cargo test` (or `cargo test -p helix-runtime`)
- Single test: `cargo test -p helix-runtime <test_name>`
- No CI, lint, formatter, or pre-commit config exists yet — there's no `clippy`/`rustfmt`
  gate to satisfy, and no README or docs site.
- This directory is **not** a git repository — there is no commit history or branch
  workflow. Don't run `git` expecting VCS state, and don't `cargo init` new crates with
  a nested `.git` (use `cargo init --vcs none` or strip the generated `.git` afterward).

## Architecture / big picture

- **Workspace layout**: root `Cargo.toml` just lists workspace members under
  `crates/`. Each roadmap subsystem in `TODO.md` (Phase 0's "Helix Runtime v0.1",
  "WIT interface definitions", "AppFront integration", "WASM module loading", etc.)
  is expected to land as either a module inside `crates/helix-runtime` or a new
  workspace member, added incrementally as its checklist item is worked.
- **Rendering pipeline** (per `spec.txt` §3–4): HTML (`html5ever`) → CSS
  (`lightningcss` + `selectors`) → layout tree (`taffy`) → text shaping
  (`cosmic-text` + `fontdb`) → decoded images/SVG (`image` + `resvg`) → display list
  → `wgpu` command buffer → present. Each stage is a separate integration task in
  `TODO.md` and should be buildable/testable independently before wiring the next
  stage on top.
- **Execution model**: WASM modules (via `wasmtime`) are the primary app runtime;
  legacy JS runs through an embedded QuickJS fallback bridged to the same WIT host
  interfaces (`dom`, `network`, `storage`, `media`) rather than having its own ambient
  APIs — the JS→WIT bridge is a deliberate compatibility shim, not a long-term API surface.
- **Capability security** (§5, later phase): apps declare required capabilities
  up front; a capability broker grants/revokes/delegates handles to `network`,
  `storage`, `dom`, `media` per app/resource. There is no origin-based ambient
  authority anywhere in the design — don't introduce APIs that assume it.
- **AI migration pipeline** (§6, later phase): a staged S1–S5 pipeline (Discovery →
  Transpile → Validate → Optimize → Deploy) intended to port existing JS/TS apps to
  Rust/WASM. Not yet implemented; don't build ad hoc migration tooling that bypasses
  this staged model.

## Environment notes

- Windows host (win32), toolchain cargo/rustc 1.96.1. Edition 2024 requires this
  recent a toolchain — don't downgrade crate editions to 2021 to "fix" build issues
  without checking whether the real fix is elsewhere.
