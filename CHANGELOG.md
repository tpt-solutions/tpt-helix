# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Security

- `wasm::Host::new()` now denies every capability-checked operation until a
  grant is installed. Use `Host::with_grants` for scoped access, or
  `Host::unrestricted` for trusted in-process use.
- Exact host grants no longer authorize suffix requests
  (`Exact("example.com")` does not cover `.example.com`).
- Empty host lists are never granted or authorized; a bounded media grant no
  longer authorizes an unbounded request.
- Storage namespaces must be delimited: `app` covers `app:x` but not `apple:x`.
- A prompter's `Modify` decision can only narrow the requested capability.
- Delegated children are invalidated when any ancestor expires, not only when
  it is revoked.
- Capability tokens are drawn from OS entropy rather than clock/counter state.
- URL host extraction uses the `url` crate and handles ports, userinfo, IPv6,
  and backslashes; hostless input fails closed.
- DOM operations on existing elements honor `DomScope::Subtree` grants.

### Added

- `wasm::sandboxed_engine()`: a wasmtime engine with fuel metering and epoch
  interruption enabled. Instantiation on a plain `Engine` now fails.
- `wasm::ResourceLimits`: memory, table, fuel, and epoch budgets per guest.
- `.github/workflows/release.yml`: pushing a `v*` tag builds the `helix` CLI for
  Linux x86_64, macOS arm64 and Windows x86_64 and publishes archives with SHA-256
  sums to a GitHub release.
- Repository templates: bug report and feature request issue forms, a pull
  request template, and Dependabot configuration for Cargo and GitHub Actions.
- `helix new <dir> --template basic|network|storage`: guest crate templates, each with
  a `capabilities` grant manifest and its own `wit/helix.wit`.
- `examples/`: hello-static, todo-crud, chart-dashboard, video-player, and a
  legacy-js-app before/after pair. Each one runs as a test.
- `docs/`: getting started, guest authoring, capability manifest reference, and
  migration guide with a coverage and limits table.
- `deny.toml` and a `deny` CI job: license, ban, source, and advisory checks.
- `helix manifest <wasm> [--diff <old>]`: infers a component's grants from its
  imports, and reports grants added by an update (spec §10.1, §10.2).
- `helix keygen`, `helix bundle`, and `helix verify`: Ed25519-signed,
  content-addressed app bundles that verify offline against a trusted key (spec §10.4).
- `helix migrate --confidence` and `--check`: a confidence block, and a wasm32 build of
  the written crate (spec §10.3).
- `migrate::reconstruct`: rebuilds the HTML a guest produces from its DOM ops. The
  confidence block's screenshot check compares it pixel by pixel with the original.
- `spec.txt` §10 specifies the seven innovation candidates. Each entry records the
  recommended decision and its status.

### Changed

- `gpu::GpuContext::render_to_rgba8` returns `Result` with a `GpuError`
  instead of panicking on readback failure.
- Removed the scaffold `add()` function and its test from `helix-runtime`.
- `tpt-helix-migrate::discover` now runs the tree-sitter AST frontend (TypeScript
  grammar) instead of a line-based token scanner. Imports, exports, and functions
  are parsed structurally, so comments and string contents no longer confuse it.
- `js_bridge::BridgeSession`: each legacy-JS app gets its own host state and batch
  buffer. The `install_*` functions take a session, and `BridgeSession::for_app`
  routes calls through the capability broker. Breaking change for direct callers.
- `spec.txt` §5.2 carries a verbatim copy of `helix.wit`, checked by a test.
- AppFront clicks carry the guest element under the pointer. `RuntimeState::document_html`
  renders the guest's DOM with `data-helix-id` on each element, so the host can dispatch
  that element's handlers.
- The static renderer wraps text to its box, and lays out and paints text from the host's
  fonts.
- A parameterless function returning a number or a bool is typed from its inferred return
  and dispatched by a handler.
- Generated guest crates include every translated function a handler reaches,
  transitively. A handler that called a helper used to produce a guest that did not
  compile.
- The static renderer paints `background:` shorthands and inline `style` attributes.
  Before, only `background-color` in a stylesheet was painted.
- The static renderer paints text, with the cascaded `color` and `font-size`, and sizes
  text boxes from the measured text. Before, text was never painted, and a block
  holding only text had no height. Lines wrap to their box width. Head, title, style, and
  script contents are no longer laid out as page text.
- Generated guest crates embed `wit/helix.wit` and no longer reference the
  repository layout. Their `Cargo.toml` names the license explicitly, so they build
  outside a workspace. This was a defect: `migrate --check` found it.
- AppFront paints relative to the render panel, colours are straight-alpha, and
  layout is cached per surface size. Clicks are hit-tested to the innermost element.
- Minimum supported Rust version is declared as `rust-version = "1.96.1"`, the
  pinned toolchain, in every workspace crate.
