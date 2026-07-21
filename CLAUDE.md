# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

A Rust cargo workspace (resolver 2, edition 2024) building **TPT Helix**: a
WASM-native web platform pitched as a successor to the browser (see
`spec.txt`, the living design spec). Five workspace members:

- `helix-runtime` — the core engine: HTML/CSS/layout/text/raster/GPU
  rendering pipeline, the QuickJS/Boa legacy-JS fallback, the capability
  broker, `wasmtime` WASM hosting, content-addressed storage, and libp2p/DASH
  networking.
- `helix-wit` — generated `wit-bindgen` host + guest bindings from
  `crates/helix-wit/wit/helix.wit`.
- `helix-guest-example` — a `wasm32-unknown-unknown` guest component
  exercising the generated guest bindings.
- `helix-migrate` — the AI migration pipeline (`spec.txt` §6): tree-sitter
  discovery, pattern transpilers, type inference, optimize/deploy stages, and
  the Eve/Spark orchestration layer that ports JS/TS apps to Rust/WASM.
- `appfront` — an `egui` + `taffy` native window host for the render surface.

Most of Phase 0/1 (§9 roadmap) is implemented; check `TODO.md` for the
current state of any given subsystem rather than assuming spec.txt describes
what's built vs. planned.

## Source of truth

- `spec.txt` is the living design specification (v0.1). If scope changes,
  update `spec.txt` *first*, then reflect the change in `TODO.md` — `TODO.md`
  is a derived execution checklist, not a design doc.
- `TODO.md` tracks work grouped into Phases 0–4 (Foundation → Core →
  Migration → Ecosystem → Displacement), each item citing the file(s) that
  implement it. Check off tasks there as they're completed; don't add new
  tasks that aren't traceable back to `spec.txt`. `TODO 1260713.md` is a
  duplicate snapshot — keep both in sync if you edit one (or ask whether it
  should just be removed).
- WIT interfaces for `network`, `storage`, `dom`, and `media` are drafted in
  `spec.txt` §5.2 and implemented in `crates/helix-wit/wit/helix.wit` — keep
  the two in sync; don't invent interface signatures that contradict §5.2
  without updating the spec first.

## Commands

- Build everything (default members): `cargo build`
- Build one crate: `cargo build -p helix-runtime`
- Test everything: `cargo test`
- Single test: `cargo test -p helix-runtime <test_name>` (or `-p helix-migrate`, etc.)
- Benchmarks: `cargo bench -p helix-runtime --bench <layout_bench|js_bench|js_engine_compare|video_bench>`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format check: `cargo fmt --all -- --check`
- `appfront` and `helix-guest-example` are **not** in `default-members` (see
  root `Cargo.toml`) — they're excluded from a bare `cargo build`/`cargo test`
  because `appfront` needs a GPU/display (`eframe`/`wgpu`) and
  `helix-guest-example` only builds for `wasm32-unknown-unknown`. Build/run
  them explicitly:
  - `cargo build -p appfront --features window` / `cargo run -p appfront --features window`
  - `cargo build -p helix-guest-example --target wasm32-unknown-unknown`
- The `libp2p` cargo feature on `helix-runtime` is off by default (the
  default build uses an in-process `PeerNetwork` simulation of the same
  DHT/bitswap contract); enable with `--features libp2p` to pull in the real
  `libp2p` Kademlia/bitswap stack.
- CI (`.github/workflows/ci.yml`) runs build+test on Linux/macOS/Windows,
  clippy+rustfmt lint, a wasm32 guest build, `cargo-llvm-cov` coverage, and a
  Linux x86_64 release package job — mirror these locally before pushing.

## Architecture / big picture

- **Rendering pipeline** (`helix-runtime`, per `spec.txt` §3–4): HTML
  (`html5ever`, `src/html.rs`) → CSS (`lightningcss` + `selectors`,
  `src/css.rs`) → layout tree (`taffy`, `src/layout.rs`) → text shaping
  (`cosmic-text` + `fontdb`, `src/text.rs`, `src/fonts.rs`) → decoded
  images/SVG (`image` + `resvg`, `src/raster.rs`) → display list
  (`src/display_list.rs`) → `wgpu` command buffer → present (`src/gpu.rs`,
  with `src/software_raster.rs` as the headless/no-GPU path used by CI and
  `screenshot_diff.rs` visual-regression tests).
- **Execution model**: WASM modules (`src/wasm.rs`, via `wasmtime`) are the
  primary app runtime. Legacy JS runs through an embedded interpreter
  (QuickJS via `rquickjs` in `src/js.rs`; Boa evaluated as an alternative,
  see `tests/boa_evaluation.rs` and `benches/js_engine_compare.rs`) bridged to
  the same WIT host interfaces via `src/js_bridge.rs` (`install_dom_bridge`,
  `install_storage_bridge`, `install_network_bridge`) rather than having its
  own ambient APIs — the JS→WIT bridge is a deliberate compatibility shim,
  not a long-term API surface.
- **Capability security** (`src/capability.rs`, spec §5): apps declare
  required capabilities up front; a capability broker grants/revokes/delegates
  handles to `network`, `storage`, `dom`, `media` per app/resource. There is
  no origin-based ambient authority anywhere in the design — don't introduce
  APIs that assume it.
- **Content-addressed distribution** (`src/content.rs`, `src/p2p.rs`,
  `src/p2p_libp2p.rs`, spec §G5): `AssetRegistry` maps legacy asset URLs to
  `ContentId` hashes; a `ContentSource` trait abstracts DHT/bitswap
  resolution, backed by default by an in-process `PeerNetwork` simulation or
  (behind the `libp2p` feature) a real Kademlia + request-response transport.
- **Media** (`src/dash.rs`, `src/media_decode.rs`): DASH manifest
  parsing/ABR selection and a decode-backend trait with a `hardware-decode`
  feature flag (off by default so CI runs headless).
- **AI migration pipeline** (`helix-migrate`, spec §6): a staged S1–S5
  pipeline —
  - S1 Discovery: `tree_sitter_discovery.rs` (primary, full AST fidelity) and
    a dependency-free tokenizer fallback, both producing an `ApiSurfaceMap`.
  - S2 Transpile: `transpile.rs` implements pattern transpilers (P1 static
    site, P2 form/CRUD, P3 data-viz, P4 media player, ...; `js_transform.rs`
    handles rule-by-rule JS→Rust-shaped rewrites) — see `coverage.rs`'s
    `Pattern`/`CoverageReport` for which patterns are currently supported and
    their live equivalence pass-rate.
  - S3 Validate: fuzz/property equivalence tests in `tests/equivalence.rs`.
  - S4 Optimize: `optimize.rs` (dead-code elimination, struct-packing,
    loop-parallelization suggestions).
  - S5 Deploy: `deploy.rs` (deployment config + feature-flag generation).
  - Orchestration: `eve.rs` (`orchestrate`) plans which pattern/stages apply
    per app; `spark.rs` (`generate_guest_crate`) folds the plan's artifacts
    into a componentizable Helix guest crate; `lib.rs::migrate()` is the
    top-level S1→S5 entry point wiring Eve to Spark.
  - `type_infer.rs` does JS→Rust type inference feeding the transpilers.
  Don't build ad hoc migration tooling that bypasses this staged model.
- **AppFront** (`appfront`): bridges `egui`'s widget tree with the Helix
  `taffy` layout tree (`src/layout.rs`, `src/css.rs`, `src/egui_surface.rs`,
  `src/render.rs`) to host a native window (`src/bin/appfront.rs`, gated
  behind the `window` feature).

## Environment notes

- Windows host (win32), toolchain cargo/rustc 1.96.1. Edition 2024 requires
  this recent a toolchain — don't downgrade crate editions to 2021 to "fix"
  build issues without checking whether the real fix is elsewhere.
- On Windows, the QuickJS fallback (`rquickjs`) build script needs `patch` on
  PATH — Git for Windows ships it under `usr/bin` (see CI's Windows step).
- Dual-licensed under MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`);
  `license.workspace = true` in each crate's `Cargo.toml` inherits from the
  root `[workspace.package]`.
