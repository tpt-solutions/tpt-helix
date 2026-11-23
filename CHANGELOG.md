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

### Changed

- `gpu::GpuContext::render_to_rgba8` returns `Result` with a `GpuError`
  instead of panicking on readback failure.
- Removed the scaffold `add()` function and its test from `helix-runtime`.
