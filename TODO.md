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
       — `crates/helix-runtime/src/p2p_libp2p.rs` now implements a real
       `Libp2pContentSource` (Kademlia DHT `provide`/`get_providers` + a
       request-response bitswap protocol with SHA-256 integrity verification on
       fetch) behind the `libp2p` cargo feature; it satisfies the same
       `ContentSource` contract as the in-process `PeerNetwork` simulation, so
       the default (headless) build still uses the simulation.
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
      — PARTIAL: `crates/helix-runtime/src/media_decode.rs` defines a
      `DecoderBackend` trait with a working `Software` backend, but
      `HardwareBackend::decode_segment` just estimates frame counts the same
      way software does (no VA-API/Vulkan calls), and
      `platform::vulkan_video_available()` is hardcoded to `false` pending
      real wiring
- [x] Implement DASH adaptive streaming client
      (`crates/helix-runtime/src/dash.rs`: real MPD XML parsing (`parse_mpd`),
      `SegmentTemplate`/`SegmentList`/`BaseURL` handling, ABR selection
      (`select_representation`, `AbrPolicy::MaximizeQuality/Conservative`),
      and `DashClient` segment planning, covered by unit tests)
- [x] Benchmark 720p video player against the ≤200MB memory target (§7.1)
       — `tests/video_memory.rs::video_player_720p_stays_within_200mb_target`
       models the CPU-side decoded-frame buffer (bounded in-flight queue +
       parsed manifest) for a 720p DASH plan and asserts it stays under the
       200 MB target; `benches/video_bench.rs` records the decode-throughput
       trend (run with `cargo bench -p helix-runtime`). Note: the ≤200 MB
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
      `crates/helix-migrate/src/tree_sitter_discovery.rs` is now the reference
      implementation backing `discover_ast` (JS / TS / TSX grammars) and
      satisfies the same `ApiSurfaceMap` contract as the dependency-free
      tokenizer (`discover`), which remains as a zero-cost fallback
- [x] Build dependency graph / API-surface-map extraction from a repo + package.json
      (`crates/helix-migrate`: `discover` → `ApiSurfaceMap` of imports/exports/functions)
- [x] Adapt `jscodeshift`-style AST-to-AST transform pipeline (Stage S2: Transpile)
      (`crates/helix-migrate/src/js_transform.rs`: a `jscodeshift`-style
      `Transformer`/`Rule` splice-rewrite driver over the tree-sitter CST, with a
      starter rule set mapping `function`→`fn`, `const`/`var`→`let`, and
      `console.log(..)`→`println!(..)`)
- [x] Wire TPT Eve for high-level migration planning orchestration
       — `crates/helix-migrate/src/eve.rs` is the deterministic offline planner:
       `detect_pattern` classifies HTML into P1–P4 via the transpiler's own
       `crud`/`dataviz`/`media` structure extraction, `plan`/`orchestrate` run
       the full Stage S1→S5 flow (S1 discovery, S2 transpile, S3 equivalence,
       S4 optimize suggestions, S5 deploy config + feature flags), and
       `lib.rs::migrate` wires Eve → Spark. Covered by `eve::tests`.
- [x] Wire TPT Spark for local on-device code generation
       — `crates/helix-migrate/src/spark.rs` is the generator:
       `generate_guest_crate` folds the transpiled `lib.rs`, Stage S4
       optimization notes, inferred `types` module (via `type_infer`), and
       Stage S5 `features` module into a componentizable `GuestCrate`
       (`Cargo.toml` + `src/lib.rs` + `helix-guest.wit`). Covered by
       `spark::tests`.
- [x] Implement P1-pattern (static content sites) transpilation to Rust/WIT
      (`crates/helix-migrate/src/transpile.rs`: `transpile_static_site` parses
      static HTML and emits an ordered `DomOp` list plus generated guest Rust
      source and a `helix-guest` WIT world that rebuild the DOM via the `dom`
      capability interface)
- [x] Build `proptest`/fuzzing-based equivalence validation (Stage S3: Validate)
      (`crates/helix-migrate/tests/equivalence.rs`: property tests over randomly
      generated HTML trees assert transpiled text content is preserved 1:1 and
      the generated guest source is structurally sound)
- [x] Build screenshot-diff visual regression checking (Stage S3: Validate)
      (`crates/helix-runtime/src/software_raster.rs` paints the static
      display-list into an in-memory RGBA buffer; `src/screenshot_diff.rs`
      compares frames, reports a changed-pixel ratio, and renders a red
      diff image — a headless stand-in for GPU frame comparison in CI)

## Phase 2: Migration (Months 7–12)

### AI migration agent v1.0: 80% pattern coverage
- [x] Implement P2 pattern support: form-based CRUD apps / dashboards / admin panels
       — `crates/helix-migrate/src/transpile.rs` adds `transpile_form_app` (shares
       the P1 pipeline) plus a `CrudModel` (`FormModel`/`FormField`/`TableModel`)
       that captures form fields (`input`/`select`/`textarea` `name`/`type`), submit
       affordances, `onsubmit`/`onclick`/`onchange`/`oninput` handler ops, and
       `tr`-row entity listings; `coverage.rs` now reports P2 as supported (the
       `supported()` probe requires a bound form + wired submit + entity table),
       and `tests/equivalence.rs` adds P2 fuzz properties (text fidelity + CRUD
       model consistency). Pipeline coverage is now 2/6 patterns.
- [x] Implement P3 pattern support: data visualization (charts, real-time metrics)
       — `crates/helix-migrate/src/transpile.rs` adds `transpile_data_viz` (shares
       the P1/P2 pipeline) plus a `DataVizModel` (`ChartModel` per `<canvas>`/`<svg>`
       mount, recording `id`, pixel `width`/`height`, and `data-*` series); `coverage.rs`
       now reports P3 as supported (the `supported()` probe requires a bound chart
       with resolved dimensions + a series), and `tests/equivalence.rs` adds P3 fuzz
       properties (text fidelity + chart-model consistency). Pipeline coverage is now
       4/6 patterns.
- [x] Implement P4 pattern support: media players (video streaming, audio)
       — `crates/helix-migrate/src/transpile.rs` adds `transpile_media_player` plus a
       `MediaModel` (`MediaPlayerModel` per `<video>`/`<audio>`, recording `src`,
       `controls`/`autoplay`/`loop` hints, and `<source>` alternate streams) and wires
       `onplay`/`onpause`/`onended` handler ops; `coverage.rs` now reports P4 as
       supported, and `tests/equivalence.rs` adds P4 fuzz properties (text fidelity +
       media-model consistency). Pipeline coverage is now 4/6 patterns.
- [x] Implement type inference (custom + `tsc` APIs) for JS → Rust type generation
       — `crates/helix-migrate/src/type_infer.rs`: `infer_types` is a dependency-free
       custom inferer (literal initializers, explicit TS annotations, function return
       literals → `InferredType`), `generate_rust_type_decls` emits a `pub mod inferred`
       type-alias block, and `infer_types_via_tsc` is the `tsc`-backed provider (shells
       out to `tsc --declaration`, best-effort — returns `Err` when `tsc` is absent) so
       CI never depends on a toolchain. Pulled into the guest by `spark::generate_guest_crate`.
- [x] Implement Stage S4 (Optimize): dead-code elimination, struct packing suggestions,
       loop parallelization
       — `crates/helix-migrate/src/optimize.rs`: `eliminate_dead_code` keeps only
       symbols reachable from exports + named roots via a call graph; `suggest_struct_packing`
       recommends largest-first field reordering to cut padding; `suggest_parallel_loops`
       flags `for`/`while` loops for `rayon` parallelization. Consumed by `eve::orchestrate`
       and rendered into the guest by `spark`.
- [x] Implement Stage S5 (Deploy): deployment config + feature flag generation
       — `crates/helix-migrate/src/deploy.rs`: `generate_deploy_config` emits a
       `helix-deploy.toml` (`[package]` + per-target `[[target]]` with cargo triples;
       empty when no targets) and `generate_feature_flags` emits a `features` module with
       `pub const *_ENABLED` flags + `is_enabled(name)`. Consumed by `eve::orchestrate` and
       embedded by `spark`.
- [x] Measure and report pattern coverage against the 80%-in-12-months target (G2)
       — `crates/helix-migrate/src/coverage.rs` `CoverageReport` probes each pattern for
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
- [ ] Profile QuickJS fallback path memory/perf overhead
- [x] Optimize JS → WIT bridge (custom, currently "to be built" per §4.2)
       — `crates/helix-runtime/src/js_bridge.rs` installs `install_dom_bridge`,
       `install_storage_bridge`, and `install_network_bridge` (global-function
       bridge to `RuntimeStub`), plus a batched DOM path (`install_dom_batch_bridge`
       + `__helix_batch_*` helpers + `__helix_batch_commit`/`clear_dom_batch`):
       accumulated ops replay into `RuntimeStub` in a single pass, collapsing the
       per-op thread-local borrow + id-allocation overhead of building a large tree
       from legacy JS. Covered by `js_batched_dom_bridge_*` unit tests in
       `js_bridge.rs`. (Further serialization efficiency — e.g. zero-copy value
       passing — remains future work once a JS-side `document.*` shim exists.)
- [ ] Evaluate `boa` (pure-Rust JS engine) as an alternative/replacement path
- [x] Sandbox `eval`/dynamic-code-generation cases per Q1's proposed resolution
      (`crates/helix-runtime/src/js.rs`: `Interpreter::eval_with_timeout` runs
      untrusted/dynamic legacy JS in a capability-free context, aborted via a
      QuickJS interrupt handler once a per-eval deadline passes)

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
      (`crates/helix-runtime/benches/layout_bench.rs`: criterion harness tracking
      layout throughput for a 1000-element tree; `crates/helix-runtime/tests/
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
      (`crates/helix-runtime/src/js.rs`: `eval_with_timeout_aborts_infinite_loop`
      asserts an infinite loop is aborted by the interrupt handler and the
      interpreter recovers afterward; `sandboxed_interpreter_has_no_bridged_host_functions`
      asserts a bare interpreter exposes no `__helix_*` host hooks)

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
