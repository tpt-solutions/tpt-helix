# TPT Helix

TPT Helix is a next-generation, WASM-native web platform — an attempt at a
successor to the browser rather than another browser. It is:

- A WASM-native application platform that renders web content without a
  JavaScript engine in the hot path.
- An AI-accelerated migration system that ports existing web applications to
  native WASM.
- A capability-secure runtime where applications declare what they need, not
  what they can take.
- A memory-efficient engine targeting significantly less RAM than Chromium
  for equivalent workloads.

See [`spec.txt`](spec.txt) for the full design document (v0.1) and
[`TODO.md`](TODO.md) for the phased execution checklist.

> **Status:** early-stage / active development. Phase 0 (Foundation) and much
> of Phase 1 (Core) from the roadmap are implemented; see `TODO.md` for what's
> done vs. outstanding in any given subsystem.

## Workspace layout

This is a Rust cargo workspace (resolver 2, edition 2024) with five members
under `crates/`:

| Crate | Purpose |
| --- | --- |
| [`tpt-helix-runtime`](crates/tpt-helix-runtime) | Core engine: HTML/CSS/layout/text/raster/GPU rendering pipeline, QuickJS/Boa legacy-JS fallback, capability broker, `wasmtime` WASM hosting, content-addressed storage, libp2p/DASH networking. |
| [`tpt-helix-wit`](crates/tpt-helix-wit) | Generated `wit-bindgen` host + guest bindings for the `network`, `storage`, `dom`, and `media` capability interfaces. |
| [`tpt-helix-guest-example`](crates/tpt-helix-guest-example) | A `wasm32-unknown-unknown` guest component exercising the generated guest bindings. |
| [`tpt-helix-migrate`](crates/tpt-helix-migrate) | AI migration pipeline: tree-sitter discovery, pattern transpilers, type inference, optimize/deploy stages, and Eve/Spark orchestration to port JS/TS apps to Rust/WASM. |
| [`tpt-helix-appfront`](crates/tpt-helix-appfront) | `egui` + `taffy` native window host for the render surface. |

## Building and testing

```sh
# Build the default workspace members (tpt-helix-runtime, tpt-helix-wit, tpt-helix-migrate)
cargo build

# Run all tests
cargo test

# Build/test a single crate
cargo build -p tpt-helix-runtime
cargo test -p tpt-helix-runtime <test_name>

# Benchmarks
cargo bench -p tpt-helix-runtime --bench layout_bench
cargo bench -p tpt-helix-runtime --bench js_bench
cargo bench -p tpt-helix-runtime --bench js_engine_compare
cargo bench -p tpt-helix-runtime --bench video_bench

# Lint / format check
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

`tpt-helix-appfront` and `tpt-helix-guest-example` are excluded from the default workspace
members because `tpt-helix-appfront` needs a GPU/display and `tpt-helix-guest-example` only
targets `wasm32-unknown-unknown`:

```sh
# Native windowed host (needs a GPU/display)
cargo run -p tpt-helix-appfront --features window

# WASM guest component
cargo build -p tpt-helix-guest-example --target wasm32-unknown-unknown
```

The `libp2p` cargo feature on `tpt-helix-runtime` is off by default (the default
build uses an in-process peer-network simulation of the same DHT/bitswap
contract); enable it to pull in the real `libp2p` Kademlia/bitswap transport:

```sh
cargo build -p tpt-helix-runtime --features libp2p
```

On Windows, the QuickJS fallback's build script (`rquickjs`) needs `patch` on
`PATH` (Git for Windows ships it under `usr/bin`).

### Toolchain

- `rust-toolchain.toml` pins Rust `1.96.1` with `rustfmt`, `clippy`, and the
  `wasm32-unknown-unknown` target. `rustup` installs these automatically.
- The minimum supported Rust version (`rust-version`) is `1.96.1`, the pinned
  toolchain. Older compilers are not tested.
- The end-to-end guest tests (`tpt-helix-cli/tests/e2e.rs`,
  `tpt-helix-migrate/tests/guest_build.rs`, marked `#[ignore]`) need the wasm32
  target and `wasm-tools`. CI pins `wasm-tools` to `1.263.0`:

  ```sh
  cargo install wasm-tools --version 1.263.0 --locked
  cargo test -p tpt-helix-cli -- --ignored
  cargo test -p tpt-helix-migrate --test guest_build -- --ignored
  ```

### Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds the
`helix` CLI for Linux x86_64, macOS arm64, and Windows x86_64 and attaches the
archives and SHA-256 checksums to a GitHub release.

CI (`.github/workflows/ci.yml`) builds and tests on Linux/macOS/Windows,
lints with clippy + rustfmt, builds the wasm32 guest, reports coverage via
`cargo-llvm-cov`, and packages a Linux x86_64 release binary of `appfront`.

## License

Dual-licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
