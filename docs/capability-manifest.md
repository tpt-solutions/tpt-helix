# Capability manifest reference

Helix is deny-by-default. An app receives only the capabilities a user grants it,
and never the ones it merely asks for (`spec.txt` §5.1).

## The manifest

A guest's manifest is its `capabilities` file: one comma-separated line of grant
names, in the same syntax as `helix run --grant`:

```
dom,network
```

Every template ships one. `helix run` reads the grants from the command line, so
pass the file's contents:

```sh
helix run app.wasm --grant "$(cat capabilities)" --app my-app
```

The `--app` name identifies the app to the capability broker and to audit logs.

## Grant names

| Grant     | Interface | Scope granted by `helix run`                              |
|-----------|-----------|-----------------------------------------------------------|
| `dom`     | `dom`     | Full DOM: any element the app created, all operations      |
| `storage` | `storage` | Global storage namespace (keys are the app's own)          |
| `network` | `network` | Any host (`HostPattern::Any`)                              |
| `media`   | `media`   | Playback up to 1920 x 1080                                 |

An unknown name is an error (`unknown capability`), so a typo cannot silently
widen or narrow the grant. An empty list denies everything.

## What a denial looks like

A call outside the grant fails with a `string` error. The guest sees
`Err(...)` from `network::fetch` or `storage::set`, and the runtime logs a
capability-denial audit entry on the `helix::capability` target. The entry includes
a remediation hint. Set `RUST_LOG=helix::capability=warn` to see them.

## Trust levels

- `helix run --grant ...`: the broker enforces the grants above.
- `helix run --unrestricted`: the broker is bypassed. Local debugging only. Never
  use it for an app you did not write.
- The library equivalents are `Host::with_grants(app, caps)`,
  `Host::unrestricted()`, and `Host::new()`, which denies everything.

## Inferring a manifest

`helix manifest <component.wasm>` reads the component's imports and prints the grants
it needs (spec §10.1). A grant is inferred only when the component imports the
matching `helix:runtime` interface, and wit-bindgen imports only interfaces the guest
calls. So the output is the smallest set the guest can use. An import outside
`helix:runtime` is reported and never granted.

## Reviewing an update

`helix manifest <new.wasm> --diff <old.wasm>` lists the grants added and removed
(spec §10.2). It exits with status 2 when the update adds a grant, so a release
pipeline can require a review first.

## Not yet covered

- The user prompt for a new grant. The diff is computed, but the prompt is UI, which
  is outside the CLI.
- Per-resource scopes in the manifest. Today grants are coarse, so `storage` covers
  every key the app writes.
