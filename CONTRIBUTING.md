# Contributing to TPT Helix

Thanks for working on Helix. The design lives in `spec.txt`; the execution
checklist lives in `TODO.md`.

## Ground rules

- **Spec first.** If a change alters scope or an interface, update `spec.txt`
  before the code, then reflect it in `TODO.md`. WIT interfaces in
  `crates/tpt-helix-wit/wit/helix.wit` must agree with `spec.txt` §5.2.
- **Security fixes start with a failing test.** Capability and boundary
  changes need a regression test that fails before the fix.
- **No ambient authority.** New host APIs must go through the capability
  broker. Do not add APIs that assume origin-based access.

## Building and testing

```sh
cargo build                              # default members
cargo test                               # default members
cargo test -p tpt-helix-runtime <name>       # single test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

`tpt-helix-appfront` and `tpt-helix-guest-example` are not default members. Build them
explicitly (see `CLAUDE.md`). Run the same gates CI runs before opening a PR.

## Pull requests

- Keep PRs focused; one logical change per PR.
- Describe what changed and why, and link the `TODO.md` item if one applies.
- Tick the matching `TODO.md` box in the same PR when it is genuinely done.
- Mention any test you could not run locally.

## Licensing

By contributing you agree that your work is dual-licensed under MIT OR
Apache-2.0, matching the repository (`LICENSE-MIT`, `LICENSE-APACHE`).
