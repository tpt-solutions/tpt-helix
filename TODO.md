# TPT Helix — Task Checklist

Derived from `spec.txt` (v0.1, 2026-07-13). Roadmap structure follows Section 9; each
top-level item is expanded into concrete subtasks sourced from the Architecture (§3),
Tech Stack (§4), Capability Model (§5), Migration Pipeline (§6), Performance Targets (§7),
and Security Model (§8) sections. Update `spec.txt` first if scope changes — this file
tracks execution, not design decisions.

## Phase 0: Foundation (Months 1–3)

### Helix Runtime v0.1: Static renderer + QuickJS fallback
- [x] Stand up `cargo` workspace skeleton for the Helix Runtime
- [x] Integrate `html5ever` for HTML5 parsing (`src/html.rs`)
- [x] Integrate `lightningcss` + `selectors` (Servo) for CSS parsing and rule matching (`src/css.rs`)
- [x] Integrate `taffy` for flexbox/grid layout (`src/layout.rs`)
- [x] Integrate `cosmic-text` for font shaping and line breaking (`src/text.rs`)
- [x] Integrate `fontdb` for system font enumeration (`src/fonts.rs`)
- [x] Integrate `image` (PNG/JPEG/WebP/GIF) and `resvg` (SVG) decoding (`src/raster.rs`)
- [x] Build display-list → `wgpu` GPU command buffer → present pipeline
      (`src/display_list.rs`, `src/gpu.rs`)
- [x] Embed QuickJS as the legacy JS fallback interpreter (`src/js.rs`, via `rquickjs`)
- [x] Define the JS → WIT bridge stub (expose minimal host functions to QuickJS)
      (`src/js_bridge.rs`, delegates to `stub::RuntimeStub`)

### WIT interface definitions for DOM, network, storage
- [x] Author `network` WIT interface (`request`/`response` records, `fetch` func)
- [x] Author `storage` WIT interface (`get`/`set`/`delete` over `key`/`value(u8)`)
- [x] Author `dom` WIT interface (`element-id`, `create-element`, `set-text`,
      `set-attribute`, `append-child`, `on-click`)
- [x] Set up `wit-bindgen` codegen for host + guest bindings
- [x] Write conformance tests for each interface against the runtime stub

### AppFront integration (egui + taffy rendering)
- [x] Wire TPT AppFront as the native UI shell hosting the render surface
- [x] Bridge AppFront's `egui` widget tree with the Helix `taffy` layout tree
- [x] Validate a minimal AppFront-hosted window renders static HTML/CSS content

### Basic WASM module loading (wasmtime)
- [x] Integrate `wasmtime` for JIT-compiled WASM execution
- [x] Implement module load/instantiate/teardown lifecycle
- [x] Wire generated WIT bindings into the module host-import table
- [x] Add a smoke-test WASM module (e.g. "hello DOM") that exercises `dom` interface calls

### Single-platform build (Linux x86_64)
- [ ] Set up `cargo` build profile and CI job targeting Linux x86_64
- [ ] Verify GPU rasterization path (`wgpu`) works on target Linux GPU drivers (Vulkan)
- [ ] Package a minimal runnable binary + smoke test app for Linux x86_64

## Phase 1: Core (Months 4–6)

### Capability broker implementation
- [x] Implement the Capability Broker core (grant registry, per-app/per-resource scoping)
- [x] Implement per-capability revocation at runtime
- [x] Implement capability delegation between modules (composability)
- [x] Implement trap/abort behavior when a module exceeds its granted capabilities
- [x] Build the user-facing grant/deny/modify prompt flow (declare → request → approve)
- [x] Wire `network`, `storage`, `dom`, and `media` capability handles through the broker

### Content-addressed distribution (libp2p integration)
- [ ] Integrate `libp2p` for DHT + bitswap content resolution
- [x] Replace location-based asset URLs with content-addressed identifiers
      (`crates/helix-runtime/src/content.rs`: `AssetRegistry` maps a legacy
      asset URL → `ContentId`, so assets are thereafter addressed by hash,
      never by location; underlying `ContentStore` still integrity-verifies)
- [x] Implement local content cache/store for resolved immutable assets
- [x] Add integrity verification (hash-check) on all content-addressed fetches

### Media pipeline (hardware decode + DASH streaming)
- [x] Author `media` WIT interface implementation (`video-config`, `create-player`,
      `play`/`pause`/`seek`) — defined in `wit/helix.wit`, generated bindings,
      and wired through `stub` + wasmtime `Host` with capability (resolution-cap) checks
- [ ] Integrate hardware decode paths (VA-API / Vulkan video)
- [ ] Implement DASH adaptive streaming client
- [ ] Benchmark 720p video player against the ≤200MB memory target (§7.1)

### Cross-platform builds (macOS, Windows)
- [x] Extend `cross`/CI to build and test macOS targets (`.github/workflows/ci.yml`)
- [x] Extend `cross`/CI to build and test Windows targets (incl. `patch` for QuickJS)
- [ ] Validate `wgpu` backend selection (Metal / DX12) on each platform
- [ ] Validate QuickJS + wasmtime builds/runs on each platform

### AI migration agent v0.1: JS → TS → Rust for static sites
- [x] Integrate `tree-sitter` for JS/TS AST parsing (Stage S1: Discovery) —
      `crates/helix-migrate/src/tree_sitter_discovery.rs` is now the reference
      implementation backing `discover_ast` (JS / TS / TSX grammars) and
      satisfies the same `ApiSurfaceMap` contract as the dependency-free
      tokenizer (`discover`), which remains as a zero-cost fallback
- [x] Build dependency graph / API-surface-map extraction from a repo + package.json
      (`crates/helix-migrate`: `discover` → `ApiSurfaceMap` of imports/exports/functions)
- [ ] Adapt `jscodeshift`-style AST-to-AST transform pipeline (Stage S2: Transpile)
- [ ] Wire TPT Eve for high-level migration planning orchestration
- [ ] Wire TPT Spark for local on-device code generation
- [x] Implement P1-pattern (static content sites) transpilation to Rust/WIT
      (`crates/helix-migrate/src/transpile.rs`: `transpile_static_site` parses
      static HTML and emits an ordered `DomOp` list plus generated guest Rust
      source and a `helix-guest` WIT world that rebuild the DOM via the `dom`
      capability interface)
- [x] Build `proptest`/fuzzing-based equivalence validation (Stage S3: Validate)
      (`crates/helix-migrate/tests/equivalence.rs`: property tests over randomly
      generated HTML trees assert transpiled text content is preserved 1:1 and
      the generated guest source is structurally sound)
- [ ] Build screenshot-diff visual regression checking (Stage S3: Validate)

## Phase 2: Migration (Months 7–12)

### AI migration agent v1.0: 80% pattern coverage
- [ ] Implement P2 pattern support: form-based CRUD apps / dashboards / admin panels
- [ ] Implement P3 pattern support: data visualization (charts, real-time metrics)
- [ ] Implement P4 pattern support: media players (video streaming, audio)
- [ ] Implement type inference (custom + `tsc` APIs) for JS → Rust type generation
- [ ] Implement Stage S4 (Optimize): dead-code elimination, struct packing suggestions,
      loop parallelization
- [ ] Implement Stage S5 (Deploy): deployment config + feature flag generation
- [ ] Measure and report pattern coverage against the 80%-in-12-months target (G2)

### First production TPT app migration (internal dogfood)
- [ ] Select an internal TPT application as the dogfood migration target
- [ ] Run it through the full S1–S5 migration pipeline
- [ ] Track and resolve equivalence/coverage failures found in Stage S3
- [ ] Deploy the migrated app to the Helix Runtime and gather perf/memory results

### Legacy JS compatibility layer optimization
- [ ] Profile QuickJS fallback path memory/perf overhead
- [ ] Optimize JS → WIT bridge (custom, currently "to be built" per §4.2)
- [ ] Evaluate `boa` (pure-Rust JS engine) as an alternative/replacement path
- [ ] Sandbox `eval`/dynamic-code-generation cases per Q1's proposed resolution

### Developer tooling: IDE plugins, debugger, profiler
- [ ] Build IDE plugin(s) for TPT Lang / Rust / Zig → WASM authoring workflow
- [ ] Build a WASM module debugger integrated with the capability broker
- [ ] Build a runtime profiler (memory, layout ops/sec, WASM execution overhead)

### Embedded targets (Raspberry Pi, ARM64)
- [ ] Cross-compile runtime for ARM64 / Raspberry Pi via `cross`
- [ ] Validate GPU/software rasterization fallback on embedded hardware
- [ ] Assess ESP32-class feasibility (per G7) and document constraints

## Phase 3: Ecosystem (Months 13–24)

### Third-party app migrations (partners)
- [ ] Define partner onboarding process for the migration pipeline
- [ ] Run migration pipeline against at least one external partner codebase
- [ ] Collect and incorporate partner feedback into pipeline/runtime

### Component marketplace (reusable WASM modules)
- [ ] Design marketplace metadata format (capability declarations, WIT interface refs)
- [ ] Build publish/discovery/install flow for WASM components
- [ ] Define content-addressing + verification for marketplace packages

### Enterprise deployment (on-premise, air-gapped)
- [ ] Build on-premise deployment packaging (no external DHT dependency required)
- [ ] Validate air-gapped operation (offline-first per G9) end-to-end
- [ ] Document enterprise capability-grant administration model

### Mobile builds (Android, iOS)
- [ ] Cross-compile runtime for Android
- [ ] Cross-compile runtime for iOS
- [ ] Validate `wgpu` backend (Vulkan/Metal) and media hardware decode on mobile
- [ ] Adapt capability grant UI for mobile form factors

### Specification publication (WIT standards)
- [ ] Finalize WIT interface definitions (network, storage, dom, media, and any added)
- [ ] Publish specification under Apache 2.0 (per Q6's proposed resolution)
- [ ] Establish TPT-maintained reference implementation as the spec's anchor

## Phase 4: Displacement (Months 25–36)

### Consumer release
- [ ] Finalize consumer-facing capability grant UX
- [ ] Harden and performance-tune for general consumer hardware profiles
- [ ] Ship consumer release build/distribution channel

### Major site partnerships (demonstrate viability)
- [ ] Identify and secure candidate high-profile site partnerships
- [ ] Migrate and deploy partner site(s) on Helix as public case studies

### JS compatibility layer deprecation path
- [ ] Define deprecation timeline/criteria for the QuickJS/Boa fallback layer
- [ ] Communicate migration deadlines to remaining legacy-dependent apps
- [ ] Remove/retire fallback layer once deprecation criteria are met

### Hardware integration (smart TVs, automotive, IoT)
- [ ] Evaluate smart TV platform integration requirements
- [ ] Evaluate automotive platform integration requirements
- [ ] Evaluate IoT platform integration requirements
- [ ] Cross-compile and validate runtime for selected hardware targets

## Open Questions to Resolve (Section 10)

- [ ] **Q1** — How to handle `eval` and dynamic code generation in legacy apps?
      Resolution: sandboxed QuickJS interpreter for dynamic code only.
- [ ] **Q2** — What is the minimum viable CSS subset for 90% of sites?
      Resolution: survey top 10k sites, measure property usage.
- [ ] **Q3** — How to migrate React/Vue component ecosystems?
      Resolution: build AppFront equivalents for the top 50 components.
- [ ] **Q4** — What is the DRM story for premium video?
      Resolution: EME-compatible CDM interface, or native app fallback.
- [ ] **Q5** — How to handle browser extensions (ad blockers, password managers)?
      Resolution: WASM-native extension API with capability restrictions.
- [ ] **Q6** — What is the governance model for WIT standards?
      Resolution: open specification under Apache 2.0, TPT maintains reference
      implementation.
