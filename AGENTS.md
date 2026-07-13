# AGENTS.md

Compact guidance for working in the TPT Helix repo.

## What this repo is
- A Rust **cargo workspace** (resolver 2) building the Helix Runtime, a WASM-native web platform (successor to the browser).
- Five workspace members: `helix-runtime` (core: HTML/CSS/layout/text/raster/gpu/JS-fallback/WASM-host + capability broker + content-addressed store), `helix-wit` (generated WIT bindings), `helix-guest-example` (wasm32 guest component), `helix-migrate` (AI migration agent, tree-sitter-based), and `appfront` (egui + taffy native window host).

## Source of truth
- `spec.txt` is the **living design specification** (v0.1). If scope changes, update `spec.txt` *first*, then reflect it in `TODO.md`. (`TODO.md` is a derived execution checklist, not a design doc.)
- `TODO.md` tracks work grouped into Phases 0–4. Active phase is **Phase 0: Foundation**.
- The draft WIT interfaces for `network`, `storage`, `dom`, and `media` live in `spec.txt` §5.2 — treat them as the canonical shape for any `wit/` files. Do not invent interface signatures that contradict §5.2 without updating the spec.

## Commands
- Build: `cargo build` (or `cargo build -p helix-runtime`).
- Test: `cargo test` (or `cargo test -p helix-runtime`).
- Release/package (Linux x86_64): see the `linux-x86_64` job in `.github/workflows/ci.yml` —
  it builds the windowed AppFront host binary (`cargo build -p appfront --features window --release`),
  runs the headless smoke suite (`cargo test --release`), and uploads the binary as an artifact.
- The `appfront` windowed binary needs a GPU/display to present; run it with
  `cargo run -p appfront --features window`. The headless `cargo test` suite is the
  CI smoke test and exercises the static render pipeline without a GPU.
- No CI, lint, format, or pre-commit config exists yet — there is no `clippy`/`rustfmt`/fmt-check gate to satisfy. Don't assume one.
- No README or docs site yet.

## Planned structure (not yet present — do not assume these exist)
- WIT interface definitions are expected under a `wit/` directory, with `wit-bindgen` codegen for **host + guest** bindings (see `TODO.md` §"WIT interface definitions").
- Conformance tests are expected to run against a "runtime stub" implementing the generated `Host` traits (no wasmtime wired up yet).
- Guest modules target `wasm32-unknown-unknown` (already installed in this toolchain).

## Environment notes
- Not a git repository — there is no commit history or branch workflow. Don't run `git` expecting VCS state.
- Windows host (win32), toolchain cargo/rustc 1.96.1. Edition 2024 requires a recent toolchain.
- `helix-runtime/Cargo.toml` currently depends only on `html5ever`, `lightningcss`, `markup5ever_rcdom`, `selectors`. New crates (e.g. `wasmtime`, `wit-bindgen`) will be added as Phase 0 tasks proceed.
