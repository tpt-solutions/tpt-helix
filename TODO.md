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
- [x] Set up `cargo` build profile and CI job targeting Linux x86_64
      (`Cargo.toml` `[profile.release]`; `.github/workflows/ci.yml`
      `linux-x86_64` job builds the release AppFront host binary + runs the
      headless smoke suite on `ubuntu-latest`)
- [ ] Verify GPU rasterization path (`wgpu`) works on target Linux GPU drivers (Vulkan)
      — BLOCKED: standard GitHub runners have no GPU. The `linux-x86_64` job
      proves the `window`/wgpu binary *links* on Linux, but presenting a frame
      still needs a GPU-enabled runner (e.g. a self-hosted Vulkan runner or
      `swiftshader`) to confirm the render path end-to-end.
- [x] Package a minimal runnable binary + smoke test app for Linux x86_64
      (`linux-x86_64` CI job packages `target/release/appfront` as
      `helix-appfront-x86_64-unknown-linux-gnu` and uploads it as an artifact;
      the smoke test is the release `cargo test` suite — static pipeline, WIT
      conformance, and the wasmtime "hello DOM" component all run headless)

## Phase 1: Core (Months 4–6)

### Capability broker implementation
- [x] Implement the Capability Broker core (grant registry, per-app/per-resource scoping)
- [x] Implement per-capability revocation at runtime
- [x] Implement capability delegation between modules (composability)
- [x] Implement trap/abort behavior when a module exceeds its granted capabilities
- [x] Build the user-facing grant/deny/modify prompt flow (declare → request → approve)
- [x] Wire `network`, `storage`, `dom`, and `media` capability handles through the broker

### Content-addressed distribution (libp2p integration)
- [x] Integrate `libp2p` for DHT + bitswap content resolution
       — `crates/tpt-helix-runtime/src/p2p_libp2p.rs` now implements a real
       `Libp2pContentSource` (Kademlia DHT `provide`/`get_providers` + a
       request-response bitswap protocol with SHA-256 integrity verification on
       fetch) behind the `libp2p` cargo feature; it satisfies the same
       `ContentSource` contract as the in-process `PeerNetwork` simulation, so
       the default (headless) build still uses the simulation.
- [x] Replace location-based asset URLs with content-addressed identifiers
      (`crates/tpt-helix-runtime/src/content.rs`: `AssetRegistry` maps a legacy
      asset URL → `ContentId`, so assets are thereafter addressed by hash,
      never by location; underlying `ContentStore` still integrity-verifies)
- [x] Implement local content cache/store for resolved immutable assets
- [x] Add integrity verification (hash-check) on all content-addressed fetches

### Media pipeline (hardware decode + DASH streaming)
- [x] Author `media` WIT interface implementation (`video-config`, `create-player`,
      `play`/`pause`/`seek`) — defined in `wit/helix.wit`, generated bindings,
      and wired through `stub` + wasmtime `Host` with capability (resolution-cap) checks
- [ ] Integrate hardware decode paths (VA-API / Vulkan video)
      — PARTIAL: `crates/tpt-helix-runtime/src/media_decode.rs` defines a
      `DecoderBackend` trait with a working `Software` backend, but
      `HardwareBackend::decode_segment` just estimates frame counts the same
      way software does (no VA-API/Vulkan calls), and
      `platform::vulkan_video_available()` is hardcoded to `false` pending
      real wiring
- [x] Implement DASH adaptive streaming client
      (`crates/tpt-helix-runtime/src/dash.rs`: real MPD XML parsing (`parse_mpd`),
      `SegmentTemplate`/`SegmentList`/`BaseURL` handling, ABR selection
      (`select_representation`, `AbrPolicy::MaximizeQuality/Conservative`),
      and `DashClient` segment planning, covered by unit tests)
- [x] Benchmark 720p video player against the ≤200MB memory target (§7.1)
       — `tests/video_memory.rs::video_player_720p_stays_within_200mb_target`
       models the CPU-side decoded-frame buffer (bounded in-flight queue +
       parsed manifest) for a 720p DASH plan and asserts it stays under the
       200 MB target; `benches/video_bench.rs` records the decode-throughput
       trend (run with `cargo bench -p tpt-helix-runtime`). Note: the ≤200 MB
       target assumes hardware decode (frames on the GPU); the software-only
       path on headless CI models the minimal CPU working set.

### Cross-platform builds (macOS, Windows)
- [x] Extend `cross`/CI to build and test macOS targets (`.github/workflows/ci.yml`)
- [x] Extend `cross`/CI to build and test Windows targets (incl. `patch` for QuickJS)
- [ ] Validate `wgpu` backend selection (Metal / DX12) on each platform
      — BLOCKED: backend selection only matters once the windowed binary is
      run on real GPU hardware per platform; CI only links it. Needs a
      GPU-capable runner to observe Metal/DX12 selection.
- [x] Validate QuickJS + wasmtime builds/runs on each platform
      (the cross-OS `test` matrix in `.github/workflows/ci.yml` runs `cargo
      test` on ubuntu/macos/windows-latest; the QuickJS eval tests in
      `src/js.rs` and the wasmtime "hello DOM" smoke test in
      `tests/wasm_smoke.rs` therefore build *and* execute on all three targets)

### AI migration agent v0.1: JS → TS → Rust for static sites
- [x] Integrate `tree-sitter` for JS/TS AST parsing (Stage S1: Discovery) —
      `crates/tpt-helix-migrate/src/tree_sitter_discovery.rs` is now the reference
      implementation backing `discover_ast` (JS / TS / TSX grammars) and
      satisfies the same `ApiSurfaceMap` contract as the dependency-free
      tokenizer (`discover`), which remains as a zero-cost fallback
- [x] Build dependency graph / API-surface-map extraction from a repo + package.json
      (`crates/tpt-helix-migrate`: `discover` → `ApiSurfaceMap` of imports/exports/functions)
- [x] Adapt `jscodeshift`-style AST-to-AST transform pipeline (Stage S2: Transpile)
      (`crates/tpt-helix-migrate/src/js_transform.rs`: a `jscodeshift`-style
      `Transformer`/`Rule` splice-rewrite driver over the tree-sitter CST, with a
      starter rule set mapping `function`→`fn`, `const`/`var`→`let`, and
      `console.log(..)`→`println!(..)`)
- [x] Wire TPT Eve for high-level migration planning orchestration
       — `crates/tpt-helix-migrate/src/eve.rs` is the deterministic offline planner:
       `detect_pattern` classifies HTML into P1–P4 via the transpiler's own
       `crud`/`dataviz`/`media` structure extraction, `plan`/`orchestrate` run
       the full Stage S1→S5 flow (S1 discovery, S2 transpile, S3 equivalence,
       S4 optimize suggestions, S5 deploy config + feature flags), and
       `lib.rs::migrate` wires Eve → Spark. Covered by `eve::tests`.
- [x] Wire TPT Spark for local on-device code generation
       — `crates/tpt-helix-migrate/src/spark.rs` is the generator:
       `generate_guest_crate` folds the transpiled `lib.rs`, Stage S4
       optimization notes, inferred `types` module (via `type_infer`), and
       Stage S5 `features` module into a componentizable `GuestCrate`
       (`Cargo.toml` + `src/lib.rs` + `helix-guest.wit`). Covered by
       `spark::tests`.
- [x] Implement P1-pattern (static content sites) transpilation to Rust/WIT
      (`crates/tpt-helix-migrate/src/transpile.rs`: `transpile_static_site` parses
      static HTML and emits an ordered `DomOp` list plus generated guest Rust
      source and a `helix-guest` WIT world that rebuild the DOM via the `dom`
      capability interface)
- [x] Build `proptest`/fuzzing-based equivalence validation (Stage S3: Validate)
      (`crates/tpt-helix-migrate/tests/equivalence.rs`: property tests over randomly
      generated HTML trees assert transpiled text content is preserved 1:1 and
      the generated guest source is structurally sound)
- [x] Build screenshot-diff visual regression checking (Stage S3: Validate)
      (`crates/tpt-helix-runtime/src/software_raster.rs` paints the static
      display-list into an in-memory RGBA buffer; `src/screenshot_diff.rs`
      compares frames, reports a changed-pixel ratio, and renders a red
      diff image — a headless stand-in for GPU frame comparison in CI)

## Phase 2: Migration (Months 7–12)

### AI migration agent v1.0: 80% pattern coverage
- [x] Implement P2 pattern support: form-based CRUD apps / dashboards / admin panels
       — `crates/tpt-helix-migrate/src/transpile.rs` adds `transpile_form_app` (shares
       the P1 pipeline) plus a `CrudModel` (`FormModel`/`FormField`/`TableModel`)
       that captures form fields (`input`/`select`/`textarea` `name`/`type`), submit
       affordances, `onsubmit`/`onclick`/`onchange`/`oninput` handler ops, and
       `tr`-row entity listings; `coverage.rs` now reports P2 as supported (the
       `supported()` probe requires a bound form + wired submit + entity table),
       and `tests/equivalence.rs` adds P2 fuzz properties (text fidelity + CRUD
       model consistency). Pipeline coverage is now 2/6 patterns.
- [x] Implement P3 pattern support: data visualization (charts, real-time metrics)
       — `crates/tpt-helix-migrate/src/transpile.rs` adds `transpile_data_viz` (shares
       the P1/P2 pipeline) plus a `DataVizModel` (`ChartModel` per `<canvas>`/`<svg>`
       mount, recording `id`, pixel `width`/`height`, and `data-*` series); `coverage.rs`
       now reports P3 as supported (the `supported()` probe requires a bound chart
       with resolved dimensions + a series), and `tests/equivalence.rs` adds P3 fuzz
       properties (text fidelity + chart-model consistency). Pipeline coverage is now
       4/6 patterns.
- [x] Implement P4 pattern support: media players (video streaming, audio)
       — `crates/tpt-helix-migrate/src/transpile.rs` adds `transpile_media_player` plus a
       `MediaModel` (`MediaPlayerModel` per `<video>`/`<audio>`, recording `src`,
       `controls`/`autoplay`/`loop` hints, and `<source>` alternate streams) and wires
       `onplay`/`onpause`/`onended` handler ops; `coverage.rs` now reports P4 as
       supported, and `tests/equivalence.rs` adds P4 fuzz properties (text fidelity +
       media-model consistency). Pipeline coverage is now 4/6 patterns.
- [x] Implement type inference (custom + `tsc` APIs) for JS → Rust type generation
       — `crates/tpt-helix-migrate/src/type_infer.rs`: `infer_types` is a dependency-free
       custom inferer (literal initializers, explicit TS annotations, function return
       literals → `InferredType`), `generate_rust_type_decls` emits a `pub mod inferred`
       type-alias block, and `infer_types_via_tsc` is the `tsc`-backed provider (shells
       out to `tsc --declaration`, best-effort — returns `Err` when `tsc` is absent) so
       CI never depends on a toolchain. Pulled into the guest by `spark::generate_guest_crate`.
- [x] Implement Stage S4 (Optimize): dead-code elimination, struct packing suggestions,
       loop parallelization
       — `crates/tpt-helix-migrate/src/optimize.rs`: `eliminate_dead_code` keeps only
       symbols reachable from exports + named roots via a call graph; `suggest_struct_packing`
       recommends largest-first field reordering to cut padding; `suggest_parallel_loops`
       flags `for`/`while` loops for `rayon` parallelization. Consumed by `eve::orchestrate`
       and rendered into the guest by `spark`.
- [x] Implement Stage S5 (Deploy): deployment config + feature flag generation
       — `crates/tpt-helix-migrate/src/deploy.rs`: `generate_deploy_config` emits a
       `helix-deploy.toml` (`[package]` + per-target `[[target]]` with cargo triples;
       empty when no targets) and `generate_feature_flags` emits a `features` module with
       `pub const *_ENABLED` flags + `is_enabled(name)`. Consumed by `eve::orchestrate` and
       embedded by `spark`.
- [x] Measure and report pattern coverage against the 80%-in-12-months target (G2)
       — `crates/tpt-helix-migrate/src/coverage.rs` `CoverageReport` probes each pattern for
       real transpiler support and computes the live Stage S3 equivalence pass-rate,
       with a `report_text()` summary for CI/dashboards; now 4/6 patterns (P1 + P2 + P3 +
       P4) at 100% pass-rate (66.7%), up from the 2/6 baseline — the planted P3/P4 work
       directly advances the G2 80%-in-12-months target.

### First production TPT app migration (internal dogfood)
- [ ] Select an internal TPT application as the dogfood migration target
- [ ] Run it through the full S1–S5 migration pipeline
- [ ] Track and resolve equivalence/coverage failures found in Stage S3
- [ ] Deploy the migrated app to the Helix Runtime and gather perf/memory results

### Legacy JS compatibility layer optimization
- [x] Profile QuickJS fallback path memory/perf overhead
       — `benches/js_bench.rs` benchmarks cold-start, eval (arithmetic,
       string-concat, array-loop), large-function parse (500-branch), and
       timeout-path overhead. `benches/js_engine_compare.rs` runs the same
       workloads through Boa for comparison. Run with
       `cargo bench -p tpt-helix-runtime --bench js_bench` or
       `--bench js_engine_compare`.
- [x] Optimize JS → WIT bridge (custom, currently "to be built" per §4.2)
       — `crates/tpt-helix-runtime/src/js_bridge.rs` installs `install_dom_bridge`,
       `install_storage_bridge`, and `install_network_bridge` (global-function
       bridge to `RuntimeStub`), plus a batched DOM path (`install_dom_batch_bridge`
       + `__helix_batch_*` helpers + `__helix_batch_commit`/`clear_dom_batch`):
       accumulated ops replay into `RuntimeStub` in a single pass, collapsing the
       per-op thread-local borrow + id-allocation overhead of building a large tree
       from legacy JS. Covered by `js_batched_dom_bridge_*` unit tests in
       `js_bridge.rs`. (Further serialization efficiency — e.g. zero-copy value
       passing — remains future work once a JS-side `document.*` shim exists.)
- [x] Evaluate `boa` (pure-Rust JS engine) as an alternative/replacement path
       — `tests/boa_evaluation.rs` verifies Boa v0.21.1 (94.1% ECMAScript
       conformance, pure Rust, no C deps) handles arithmetic, string concat,
       syntax errors, per-context isolation, host-function registration (DOM
       bridge pattern), array iteration, and large-function parse — all
       passing. `benches/js_engine_compare.rs` benchmarks both engines
       side-by-side. Boa is a viable drop-in replacement; the main tradeoff
       is cold-start cost (Boa's `Context::default()` is heavier than
       QuickJS's `Runtime::new()`) but Boa offers pure-Rust compilation
       (no C build toolchain) and higher ECMAScript conformance.
- [x] Sandbox `eval`/dynamic-code-generation cases per Q1's proposed resolution
      (`crates/tpt-helix-runtime/src/js.rs`: `Interpreter::eval_with_timeout` runs
      untrusted/dynamic legacy JS in a capability-free context, aborted via a
      QuickJS interrupt handler once a per-eval deadline passes)

### Developer tooling: IDE plugins, debugger, profiler
- [ ] Build IDE plugin(s) for TPT Lang / Rust / Zig → WASM authoring workflow
- [ ] Build a WASM module debugger integrated with the capability broker
- [x] Build a runtime profiler (memory, layout ops/sec, WASM execution overhead)
       — `crates/tpt-helix-runtime/src/profiler.rs`: dependency-free `Profiler` with
       timed spans / `time` / `count` per channel (ops, total, mean, ops/sec) and
       caller-reported memory gauges with peak tracking; disabled mode is a no-op.
       Memory values are reported by owning subsystems (no OS heap probe yet).
       Covered by `profiler::tests`.

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

## Testing & Quality Assurance (cross-phase)

Cuts across all phases above — each item traces back to a component already scoped
in Architecture (§3), Tech Stack (§4, CI/CD row), Migration Pipeline (§6, Stage S3),
or Security Model (§8). Add new items here as new components land; don't let feature
work outrun its test coverage.

### Unit test coverage per crate
- [x] `helix-runtime` `html.rs`: malformed HTML, encoding detection, edge-case parsing
       — covers nested elements, unclosed-tag recovery, attributes,
       doctype/comments, and void/self-closing elements; `parse_html_bytes`
       performs HTML-standard encoding sniffing (UTF-8/`UTF-16` BOM,
       `<meta charset>` override, windows-1252 default) via `encoding_rs` and
       is covered by dedicated encoding-detection tests
- [x] `helix-runtime` `css.rs`: selector matching, cascade/specificity, malformed CSS
       — covers class/id/attribute/descendant selector matching, malformed-CSS
       degradation (unbalanced braces, bad declarations), and a dedicated
       cascade/specificity-ordering test (`higher_specificity_overrides_source_order`):
       `#id` (1,0,0) beats `.class` (0,1,0) beats `tag` (0,0,1) even when the
       lower-specificity rule appears later in source — `resolve_style` now sorts
       matched rules by `selectors::Selector::specificity()` then source index.
- [x] `helix-runtime` `layout.rs` (`taffy`): flex/grid edge cases, intrinsic sizing
        — covers side-by-side flex children, percentage-width resolution
        against the viewport, cascade rule precedence, grid row-stacking, and a
        dedicated intrinsic-sizing test (`block_auto_width_fills_containing_block`:
        a `width: auto` block fills its containing block). The previously-RED
        `percentage_width_resolves_against_viewport` now passes: the root cause
        was `taffy`'s `Display::default() == Display::Flex`, which made every
        node a flex item whose `auto` width collapsed to 0 and broke percentage
        resolution through `html > body`. `resolve_style` now seeds the default
        display to `Block` (real-CSS model), so `auto` widths fill the
        containing block and nested percentages resolve (50% of an 800px
        viewport = 400.0).
- [x] `helix-runtime` `text.rs`: shaping/line-breaking (bidi, ligatures, CJK)
       — `shape_text` tests cover short/long runs, narrow-wrap multi-line
       breaking, CJK (space-less) shaping, accented/ligature text, and empty input
- [x] `helix-runtime` `raster.rs`: image/SVG decode error paths, malformed assets
       — `decode_raster`/`rasterize_svg` tests cover a 1x1 PNG round-trip, a
       solid-color SVG, garbage-byte rejection, truncated-PNG rejection, and
       malformed-SVG returning `None` (no panic)
- [x] `helix-runtime` `display_list.rs` + `gpu.rs`: command-buffer generation correctness
       — `display_list` tests cover CSS color parsing and one-item-per-colored-box
       emission (verified against the Block-default layout); `gpu.rs` has a
       `renders_a_solid_rect` command-buffer/present smoke test
- [x] `helix-runtime` `js.rs`: QuickJS eval correctness, timeout/interrupt behavior
      (arithmetic/string/undefined/syntax-error/global-isolation cases, plus
      `eval_with_timeout_aborts_infinite_loop` and
      `sandboxed_interpreter_has_no_bridged_host_functions` regression tests
      for the interrupt-driven abort path)
- [x] `helix-runtime` `js_bridge.rs`: stub delegation correctness for each WIT call
       — tests cover dom (`__helix_create_element`/`set_text`/`set_attribute`/
       `append_child`/`on_click`), storage (`set`/`get`/`delete` round-trip),
       and network (`__helix_fetch` against a registered route) all delegating
       to `RuntimeStub`
- [x] `helix-runtime` `content.rs`: `AssetRegistry` + `ContentStore` integrity-check paths
       — tests cover deterministic hex digest, put/get idempotency, tamper
       rejection (`verify`), missing-id `get_verified`, URL→ContentId rebinding,
       and integrity-checked `fetch`
- [x] `helix-migrate` `tree_sitter_discovery.rs`: AST parsing across JS/TS/TSX grammars
      (`src/tree_sitter_discovery.rs` now has a `#[cfg(test)]` suite covering
      default/named/namespace/default imports, function/class/const/default
      exports, re-exports, top-level functions, embedded JSX in TSX, and
      comment/string resilience, plus parity with the tokenizer `discover`)
- [x] `helix-migrate` `js_transform.rs`: rule-by-rule transform correctness
       — tests cover `function`→`fn`, `const`/`var`→`let`, `console.log`→`println!`,
       combined transforms, and that unhandled syntax (e.g. `let`) passes through
- [x] `helix-migrate` `transpile.rs`: static-site `DomOp` + generated WIT world correctness
       — tests cover nested elements/attributes, comment/doctype/whitespace
       skipping, ops-match-source-shape (text runs preserved 1:1, append refs
       defined nodes), and well-formed generated Rust + WIT world

### Integration test coverage
- [x] End-to-end HTML → CSS → layout → display-list pipeline golden-file fixtures
      (`tests/render_pipeline.rs` drives the same stages as the GPU presenter but
      paints into an in-memory RGBA buffer and asserts on concrete pixel output)
- [x] WIT interface conformance suite (`network`/`storage`/`dom`/`media`) run against
      both the wasmtime host and the QuickJS bridge, not just the runtime stub
      (`tests/conformance.rs` = stub path; `tests/bridge_conformance.rs` = QuickJS
      JS→WIT bridge path; `tests/wit_boundary.rs` = both stub and wasmtime `Host`
      under adversarial input)
- [x] Capability broker integration tests: grant/revoke/delegate/trap across
      multi-module scenarios (`tests/capability_broker.rs`)
- [x] `wasmtime` module lifecycle tests (load/instantiate/teardown) under adversarial
      or malformed WASM module inputs (`tests/wasm_lifecycle.rs`)

### Migration pipeline validation coverage
- [x] Extend `proptest`/fuzz equivalence suite (Stage S3) to cover P2–P4 patterns as
       they land
        — `tests/equivalence.rs` now fuzzes the P2 form-based CRUD pattern
        (`form_app_text_is_preserved`, `form_app_crud_model_is_consistent`) over
        randomly generated forms/tables, and the P3 data-visualization pattern
        (`data_viz_text_is_preserved`, `data_viz_chart_model_is_consistent`) and P4
        media-player pattern (`media_player_text_is_preserved`,
        `media_player_model_is_consistent`) over randomly generated charts/players;
        P5/P6 fuzzing to be added when those patterns land.
- [x] Extend screenshot-diff visual regression suite beyond the static-site pipeline
       — `screenshot_diff::changed_bounds` localizes a regression to the changed
       region; `tests/render_pipeline.rs::composed_layout_regression_localizes_to_changed_region`
       exercises a composed multi-component layout (header + two flex columns)
       and asserts the visual-regression gate confines the change to the
       affected column, not the whole frame.
- [x] Track and publish pattern-coverage + equivalence pass-rate metrics (feeds the
       G2 measurement item above)
       — `helix-migrate/src/coverage.rs` defines `Pattern` (P1–P6) and a
       `CoverageReport` that probes each pattern for real transpiler support and
       computes the live Stage S3 equivalence pass-rate, with a `report_text()`
         summary for CI/dashboards; now 4/6 patterns (P1 + P2 + P3 + P4) at 100% pass-rate.

### Cross-platform / CI coverage
- [x] Add `clippy` + `rustfmt` gates to CI
      (`.github/workflows/ci.yml`: `lint` job runs
      `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`.
      NOTE: as of 2026-07-14 the `lint` gate was actually red — pre-existing
      clippy lints (collapsible-if, redundant closures, needless casts/refs,
      explicit lifetimes, `sort_by_key`) and `cargo fmt --check` drift across
      `helix-runtime`/`helix-migrate`. These were fixed; the gate now passes.)
- [x] Add code coverage reporting (`cargo-llvm-cov` or `tarpaulin`) to CI and publish
      the coverage trend
      (`.github/workflows/ci.yml`: new `coverage` job runs `cargo llvm-cov
      --workspace --lcov`, uploads `lcov.info` as an artifact, and uploads to
      Codecov when a `CODECOV_TOKEN` secret is present)
- [ ] Stand up a GPU-capable CI runner (self-hosted or `swiftshader`) to unblock the
      wgpu rasterization / backend-selection validation items in Phase 0/1
- [x] Add performance regression benchmarks (layout ops/sec, memory footprint) gated
      against the §7 performance targets
      (`crates/tpt-helix-runtime/benches/layout_bench.rs`: criterion harness tracking
      layout throughput for a 1000-element tree; `crates/tpt-helix-runtime/tests/
      perf_regression.rs` enforces the §7.3 ≥60fps/1000-element budget as a
      failing CI test)

### Security test coverage
- [x] Fuzz the capability broker's grant/revoke/delegate state machine
      (`tests/capability_fuzz.rs`: deterministic pseudo-random op-sequence driver
      with a maintained model checked against real `check`/`revoke`/`delegate`)
- [x] Fuzz WIT interface boundary parsing (`network`/`storage`/`dom`/`media`) against
      malformed guest input (`tests/wit_boundary.rs`: adversarial bytes/long
      strings/empty keys/oversized payloads against both `RuntimeStub` and the
      wasmtime `Host`; asserts the boundary never panics)
- [x] Add regression tests locking in sandboxed `eval` timeout/abort behavior (Q1)
      (`crates/tpt-helix-runtime/src/js.rs`: `eval_with_timeout_aborts_infinite_loop`
      asserts an infinite loop is aborted by the interrupt handler and the
      interpreter recovers afterward; `sandboxed_interpreter_has_no_bridged_host_functions`
      asserts a bare interpreter exposes no `__helix_*` host hooks)

## Review Follow-ups (2026-11-24 platform review)

Findings from a read-only code review; none are execution-verified yet, so each
fix should start with a failing regression test. WIT/spec changes go in
`spec.txt` first (§5.2), then `helix.wit`, then the transpiler.

### Security hardening (spec §5 capability model, §8 security testing)
- [x] Make the capability broker mandatory: `Host::new()` (`wasm.rs`) currently has
      no broker and `check_cap` allows everything; default to deny
      (`wasm.rs`: `Host::new()` denies all until `Host::with_grants`; permissive path
      is the explicit `Host::unrestricted()`; tests `ungranted_guest_is_denied_all_dom_and_storage`)
- [x] Add wasmtime epoch/fuel interruption plus a `ResourceLimiter` (memory/table
      caps) so a guest cannot spin or grow without bound (`wasm.rs`: `sandboxed_engine`,
      `ResourceLimits`; test `fuel_exhaustion_traps_the_guest`)
- [x] Fix `pattern_covers(Exact, Suffix)` scope widening on delegation; an exact
      grant must cover only exact requests (`capability.rs`:
      `exact_grant_does_not_cover_suffix_request`)
- [x] Replace `host_of` with the `url` crate: handle ports, backslash, userinfo,
      case, IPv6; add edge-case tests (`capability.rs`:
      `host_of_handles_ports_userinfo_case_and_ipv6`)
- [x] Use OS entropy (`getrandom`) for the token secret instead of clock/counter/
      stack address (`capability.rs`: tokens drawn from `getrandom::fill`)
- [x] Intersect a prompter's `Modify(narrowed)` with the requested capability
      (`capability.rs`: `modify_cannot_widen_requested_scope`)
- [x] Enforce parent expiry/revocation on delegated children (walk parent chain in
      `check`) (`capability.rs`: `ensure_live`, `parent_expiry_invalidates_delegated_child`)
- [x] Reject empty host lists, `(Some, None)` media bounds, and non-delimited
      namespace prefixes (`app` must not cover `apple:x`) (`capability.rs`: tests
      `empty_host_list_is_never_granted_or_authorized`, `bounded_media_grant_rejects_unbounded_request`,
      `namespace_prefix_must_be_delimited`)
- [x] Implement `DomScope::Subtree` authorization and verify target ids lie inside
      the subtree (`stub.rs`: `check_dom_target`, parent index)
- [x] Cap libp2p frame/length allocations (`p2p_libp2p.rs`: `read_framed` rejects
      oversize length prefixes before allocating; `MAX_REQUEST_BYTES`/`MAX_BLOCK_BYTES`
      enforced in the codec, since libp2p's `RrConfig` has no per-message size knob;
      tests `oversized_request_frame_is_rejected_before_allocation`,
      `oversized_block_frame_is_rejected_before_allocation`)
- [x] Verify content hashes inside `ContentSource::get`, validate `ContentId` as
      64 lowercase hex, and replace the `"local-cache"` magic string with an enum
      (`p2p.rs`: `get` verifies every fetch; `content.rs`: `ContentId::parse`/
      `is_well_formed`; `ProviderKind::LocalCache`; already in place, re-verified)
- [x] Cap DASH segment counts and use checked arithmetic (`dash.rs`); swap the
      hand-rolled XML scan for `quick-xml`/`roxmltree`
      (`dash.rs`: segment/representation caps, saturating arithmetic, and `parse_mpd`
      on `roxmltree` with DTDs disallowed and `MAX_MPD_XML_NODES`; the scanner's
      subtree BaseURL leak and prefix matching are fixed, tests in
      `xml_hardening_tests`)
- [x] Reject non-finite CSS values (`NaN%`, `inf`) in layout and add a recursion
      depth limit to `layout.rs`, `html.rs`, `css.rs`
      (`layout.rs`: non-finite rejection and `MAX_LAYOUT_DEPTH`; `css.rs`:
      `MAX_CSS_BLOCK_DEPTH` pre-scan and `MAX_SELECTOR_NESTING`, which fixed stack
      overflows found by `hostile_depth_tests`; `html.rs`: walkers stop at the first
      match and 5,000-deep markup parses, covered by `hostile_depth_tests`)
- [x] Cap JS bridge batch ops and per-app guest state (elements, handlers, storage);
      isolate bridge state per app and route it through the broker
      (`MAX_BATCH_OPS` with atomic reject as a RangeError in `js_bridge.rs`;
      `MAX_ELEMENTS`/`MAX_CLICK_HANDLERS`/`MAX_STORAGE_*` in `stub.rs`. Isolation:
      `js_bridge::BridgeSession` owns its own `RuntimeState` and batch buffer, and the
      `install_*` functions take a session, so the JS path no longer touches the
      thread-local host. `BridgeSession::for_app` routes every storage, network, and DOM
      call through the capability broker. Tests: `sessions_do_not_share_dom_or_storage`,
      `brokered_session_denies_storage_until_granted`, `tests/bridge_conformance.rs`.
      The WIT `RuntimeStub` keeps its thread-local state for the conformance tests)
- [x] Add hostile-input tests: DASH segment explosion, deep nesting, oversized
      frames, and unit tests for `stub.rs`/`wasm.rs`
      (DASH explosion, HTML/CSS/layout deep nesting, oversized libp2p frames, `stub.rs`
      guest-state caps, `wasm.rs` limiter and default-deny tests)

### WIT / migration correctness (spec §5.2, §6)
- [x] Reconcile `spec.txt` §5.2 WIT blocks (duplicated/garbled, undefined types,
      world name `helix` vs `helix-guest`) with `helix.wit`
      (§5.2 now carries a verbatim copy of `helix.wit` between `[BEGIN helix.wit]`
      and `[END helix.wit]`. `crates/tpt-helix-wit/tests/spec_sync.rs` fails on any drift)
- [ ] Add the missing DOM events (`submit`, `change`, `input`, `play`, `pause`,
      `ended`) or a generic `on-event`, plus a host->guest `dispatch` export
      (done: `dom.on-event(el, event, handler-id)` in `helix.wit`, implemented on the stub
      and wasm hosts; the migration generator emits it for every event it uses, and
      `every_supported_pattern_generates_a_guest_that_compiles` proves the output compiles.
      Done too: the `helix-guest` world exports `dispatch(handler-id)`, the host calls it
      through `Instance::fire_event`/`dispatch_handler`, and `tests/event_dispatch.rs`
      runs the componentized guest end to end. Handler attributes that call a plain
      `name()` dispatch to that function when it is translated (parameterless and fully
      typed), and `guest_build.rs` compile-checks a migrated click handler. Open: handler
      arguments, untyped JavaScript functions, and inline handler bodies are not
      translated, and each one is reported as a warning)
      End-to-end: `tests/e2e.rs` migrates a page with a click handler, builds and
      componentizes the guest, runs it in the runtime, and checks the storage write the
      translated handler makes. Open: the test is `#[ignore]`d because it needs the wasm32
      target and `wasm-tools`. CI's `wasm-guest` job installs `wasm-tools` (pinned to
      1.263.0) and runs both ignored tests.
- [x] Generate the guest world from (or check it against) `helix.wit`; remove the
      hard-coded `../tpt-helix-wit/wit` path in generated and example crates
      (generated guest crates now carry their own `wit/helix.wit`, embedded from
      `tpt_helix_wit::HELIX_WIT`, and bind with `path: "wit"`. `spark.rs`, `transpile.rs`,
      and the CLI's `write_guest_crate` write it. `guest_build` compiles all 6 patterns
      from the crate-local WIT. Deliberately kept: `tpt-helix-guest-example` and
      `tpt-helix-runtime` still use `../tpt-helix-wit/wit`, the single canonical copy
      inside the repo)
- [ ] Make `coverage.rs` `supported()` compile the generated guest crate so the
      "6/6 patterns" figure is meaningful; add screenshot-diff equivalence (S3)
      (partly done: `tests/guest_build.rs` compiles each pattern's generated guest for
      wasm32, runs in CI, and passes 6/6. Open: `supported()` still uses the structural
      probe, and the S3 screenshot diff is not built. A compile check does not prove the
      component links or runs against the runtime)
- [x] Make `migrate()` return diagnostics/`Result`: surface `equivalence_ok`,
      stop on `Fit::None`, and stop `detect_pattern` defaulting to P1/Strong
      (`lib.rs`: `migrate()` returns `Result<_, MigrationError::Unsupported>`;
      `eve.rs`: `detect_pattern` is `Fit::None` for documents with no elements and
      `Fit::Partial` for elements with no text; `orchestrate` skips the transpile
      for `Fit::None`; `equivalence_ok` is reported; tests in `detection_hardening_tests`)
- [ ] Wire `transpile_js_to_rust` into the S2 pipeline and fix its output (valid
      `println!`, parameter types, overlapping edits, TS vs JS grammar errors)
      (done: wired into `orchestrate` as `OrchestratedMigration::js`, with errors in
      `js_error` and warnings from `warnings()`. Output fixes: edits apply in one forward
      pass and overlaps are skipped; `console.log` becomes `println!` with a format
      string; `var` becomes `let mut`; single-quoted strings become double-quoted; syntax
      errors are reported; `Auto` falls back to TypeScript. Tests in `js_transform`.
      TypeScript annotations map to Rust types (`number`→`f64`, `string`→`&str`,
      `boolean`→`bool`, `T[]`→`Vec<T>`); a function with an unmapped or missing type is
      listed in `needs_rust_types`. Emission: the guest crate now includes every
      translated function a handler reaches, transitively, in source order
      (`spark::closure_of_calls`). Before this, a handler that called a helper broke the
      guest build. Regression tests in `spark::tests`, and `migrate --check` builds the
      result for wasm32. Parameterless functions whose literal return is a number or a
      bool are typed from it (`type_infer`), so they are emitted too. Open: functions
      with parameters, and functions that return a string, are still not emitted
      (plain JavaScript gives no parameter types, and a string literal is `&str` in Rust
      but `String` to the inferencer). Each is reported as a warning)
- [ ] Fix HTML transpiler text/whitespace joining, `<script>`/`<style>` raw text,
      entity decoding; consider `html5ever` instead of `parse_html`
      (done: words no longer join, edge spaces kept, `Node::Raw` for script/style bodies,
      `decode_entities` for text and attributes; tests in `parser_hardening_tests`.
      Open: `parse_html` is still hand-written, not `html5ever`)
- [x] Replace id-substring pattern sniffing (`board`/`chat`/`doc`) with structural
      detection; make the `discover` line-based fallback AST-only
      (detection uses parsed structure only, and S1 runs the AST frontend over
      inline `<script>` bodies. `discover` (`lib.rs`) now delegates to the tree-sitter
      TypeScript frontend and the line scanner is deleted; tests in
      `tests/discovery.rs` and `tree_sitter_discovery.rs`)
- [ ] appfront: use `from_rgba_unmultiplied`, cache layout per size, and route egui
      input events to guest `on-click`
      (done: colours use `from_rgba_unmultiplied`; a `FrameCache` relays out only when
      the surface size changes; paint is offset by the panel origin, which the old code
      missed; `Frame::hit_test` finds the innermost box, including unpainted elements;
      primary clicks invoke `AppFront::on_click` with a `ClickEvent`. Note that
      `parse_color` has no `rgba()` support, so every CSS colour is opaque today and the
      premultiplied bug was latent. Click routing: `RuntimeState::document_html` renders
      the guest's DOM with each element's `data-helix-id`. A click's `ClickEvent::element`
      is that id (the deepest element under the pointer, so a text run reports its
      container), and the host dispatches `click_handler_ids(element)`. Tested headlessly
      in `tests/click.rs`. Open: the `appfront` binary does not load a guest, so no window
      dispatches yet. `cargo check -p tpt-helix-appfront --features window` passes; the
      window has not been run on a display)

### Render pipeline gaps (found by the migration screenshot check)
- [x] Paint text. `text::draw_text` paints glyph coverage with cosmic-text's
      `Buffer::draw`. `display_list::build_text_runs` collects the runs in paint order,
      each with its cascaded `color` and `font-size`, and `software_raster::rasterize_text`
      composites them over the boxes. Caveats: the output depends on the host's installed
      fonts, which the tests check for and skip without. Lines wrap to the width of the
      box they sit in (taffy measure callback). Tests in `tests/render_pipeline.rs`.
- [x] Give text runs a height in layout. Text nodes are layout leaves sized by
      `text::measure_text`, so a block that holds only text takes the text's height and
      paints its background. Non-rendered elements (`head`, `title`, `style`, `script`,
      and others) are excluded, so their contents never become page text. Tests in
      `tests/render_pipeline.rs`.

### Observability and hygiene (spec §7, §9 developer tooling)
- [x] Wire `profiler.rs` into `wasm.rs` and `layout.rs`; fix `mean()` u32 clamp and
      allow nested spans
- [x] Adopt `tracing` with `RUST_LOG` and a capability-denial audit log with
      remediation hints (`lib.rs`: `init_logging()` via `RUST_LOG`; `stub.rs`:
      `audit_denied` on target `helix::capability`; `capability.rs`: `remediation_hint`)
- [x] Replace `gpu.rs` readback `unwrap`/`expect` with `Result` (`GpuError`)
- [x] Remove the scaffold `add()`/`it_works` in `lib.rs` and "stub" wording in
      `js_bridge.rs`/`stub.rs` (no scaffold remained; header wording updated)

### Adoption and developer experience (spec §9 roadmap: developer tooling)
- [x] New `helix-cli` crate: `run`, `render` (headless PNG), `migrate --report`,
      `new`, `build`, `doctor` (all six subcommands present; `new --template` added)
- [x] `examples/`: hello-static (P1), todo-crud (P2), chart-dashboard (P3),
      video-player (P4), legacy-js-app before/after; each run as a CI test
      (`crates/tpt-helix-cli/tests/examples.rs`. `legacy-js-app/after` is generated by
      the CLI, and the test fails if the generator's output drifts from it)
- [x] `templates/`: guest-basic, guest-network, guest-storage with capability
      manifests, consumed by `helix new` (`helix new <dir> --template basic|network|storage`.
      Each manifest is a `capabilities` file in `--grant` syntax, tested with
      `parse_grants`. `tests/templates.rs` checks the scaffold, and the ignored test
      compiles all three for wasm32)
- [x] Docs: getting-started, guest authoring guide, capability manifest reference,
      migration guide with coverage/limits table (`docs/`)
- [x] Add `rust-toolchain.toml` + MSRV and document `wasm-tools` in the README
      (`rust-toolchain.toml` pins 1.96.1; `rust-version = "1.96.1"` in
      `Cargo.toml` `[workspace.package]`, inherited by every crate; README "Toolchain"
      section documents the `wasm-tools` 1.263.0 pin for the `#[ignore]`d tests)
- [x] Add `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md`, issue/PR templates,
      Dependabot (`CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md`;
      `.github/ISSUE_TEMPLATE/{bug_report,feature_request}.md`,
      `.github/PULL_REQUEST_TEMPLATE.md`, `.github/dependabot.yml`)
- [x] CI: tag-triggered release workflow (Linux/macOS/Windows binaries),
      `cargo-deny`/`cargo-audit`, criterion benchmark tracking
      (`.github/workflows/release.yml` builds the `helix` CLI for Linux x86_64,
      macOS arm64 and Windows x86_64 on `v*` tags and attaches archives and SHA-256
      sums. `deny.toml` and a `deny` job in `ci.yml` cover licenses, bans, sources, and
      RustSec advisories; the offline checks pass locally. A `bench` job in `ci.yml`
      runs the criterion layout benchmark on master pushes and uploads the results.
      None of these has run on GitHub yet: the release on a real tag, the advisories
      check, and the bench job)

### Innovation candidates (specified in `spec.txt` §10 before implementation)
- [x] Capability manifest inference (learn mode / static scan -> minimal manifest)
      (spec §10.1. `runtime/src/manifest.rs` maps a component's imported `helix:runtime`
      interfaces to grants. `helix manifest <wasm>` prints them. Tests in
      `manifest::tests` and `tpt-helix-cli/tests/manifest.rs`, against the componentized
      guest example. Learn mode from a live run is not built: the static scan is the
      recommended design)
- [x] Migration confidence report with compile check and screenshot diff
      (spec §10.3. `helix migrate --confidence` prints pattern, equivalence, warning
      count, compile status, and the screenshot check. `--check --out <dir>` builds the
      written crate for wasm32. `migrate::reconstruct` rebuilds HTML from the `DomOp`
      list, and the check compares it pixel by pixel with the original.
      Limits: text is compared using the host's installed fonts.
      Style bodies are not carried into the guest, so a lost stylesheet counts as a
      difference)
- [ ] Deterministic host-call record/replay for tests and bug reports
      (spec §10.5. Specified, not implemented. It needs a recorder at the host boundary
      that touches every `Guest` impl)
- [ ] `helix dev` hot-reload server preserving DOM/storage state
      (spec §10.6. Specified, not implemented. Durable per-app storage does not exist yet)
- [ ] Capability-diff prompt on app update
      (spec §10.2. Done: `helix manifest <new> --diff <old>` lists added and removed
      grants and exits 2 when an update adds one. Open: the user prompt. The runtime
      computes the diff, and the prompt is UI outside this crate)
- [x] Signed content-addressed app bundles (CID manifest, offline verify)
      (spec §10.4. `helix keygen`, `helix bundle <dir> --key <file>`, and
      `helix verify <dir> --trust <file>`. The manifest lists each file's SHA-256. Ed25519
      signs the canonical manifest, and the content id is its SHA-256. Verification needs
      the trusted key from the caller, and a key embedded in the bundle is never trusted.
      Tests in `bundle::tests` cover tampering, added files, a forged key, and a key
      mismatch)
- [ ] Web/WASM migration playground (JS on the left, generated guest on the right)
      (spec §10.7. Specified, deferred until the tree-sitter grammars build for wasm32,
      which is unverified)

## Open Questions to Resolve (Section 10)

- [x] **Q1** — How to handle `eval` and dynamic code generation in legacy apps?
       Resolution: sandboxed QuickJS interpreter for dynamic code only (implemented
       in `src/js.rs`: `Interpreter::eval_with_timeout` runs untrusted/dynamic
       legacy JS in a capability-free context, aborted via a QuickJS interrupt
       handler once a per-eval deadline passes).
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
