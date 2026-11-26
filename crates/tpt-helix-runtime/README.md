# tpt-helix-runtime

The core engine of TPT Helix. It turns HTML and CSS into pixels, hosts
WebAssembly guests behind a capability broker, runs legacy JavaScript through a
sandboxed interpreter, and resolves content by hash instead of by URL.

This is the crate most of the other workspace members depend on. The CLI
(`tpt-helix-cli`) and the window host (`tpt-helix-appfront`) are thin shells
around it.

## What it provides

| Area | Modules | What it does |
| --- | --- | --- |
| Markup | `html` | HTML parsing with `html5ever`; charset sniffing (BOM, `<meta charset>`, Windows-1252 fallback); `inline_styles` collects `<style>` blocks. |
| Styling | `css` | Stylesheets via `lightningcss`; selectors via the Servo `selectors` crate. Hostile-input limits on block nesting and selector nesting. |
| Layout | `layout` | Builds a `taffy` layout tree from the DOM and computed rules. Rejects non-finite dimensions and caps tree depth. |
| Text | `text`, `fonts` | Shaping with `cosmic-text` and font loading with `fontdb`. |
| Images | `raster` | PNG/JPEG/WebP/GIF decode and SVG rasterization (`image`, `resvg`). |
| Paint | `display_list`, `software_raster`, `gpu` | Display-list construction, a CPU rasterizer for headless use, and a `wgpu` presenter. |
| Capabilities | `capability`, `stub`, `wasm` | The capability broker (grant, revoke, delegate, scope) enforces `network`, `storage`, `dom`, and `media` access for each app. |
| WASM | `wasm` | Hosts WebAssembly components with a per-call step budget and wall-clock deadline, and memory/table limits. Deny-by-default: an app with no grants can do nothing. |
| Legacy JS | `js`, `js_bridge` | A QuickJS interpreter (`rquickjs`) with a timeout, bridged to the same DOM, storage, and network interfaces. |
| Distribution | `content`, `p2p`, `p2p_libp2p` | Content-addressed storage (SHA-256 `ContentId`), an in-process DHT/bitswap model, and an optional real libp2p transport. |
| Media | `dash`, `media_decode` | DASH manifest parsing (built on `roxmltree`, DTDs rejected), segment planning, and ABR selection. The decoder is a trait; hardware backends are feature-gated. |
| Observability | `profiler`, `lib::init_logging` | Named timing spans, and `tracing` output filtered by `RUST_LOG`. Capability denials are logged with a remediation hint. |

## Quick start

```rust
use tpt_helix_runtime::css::parse_stylesheet;
use tpt_helix_runtime::display_list::build_display_list;
use tpt_helix_runtime::html::parse_html;
use tpt_helix_runtime::layout::{build_layout_tree, compute};
use tpt_helix_runtime::software_raster::{encode_png, rasterize_display_list};

fn render(html: &str, width: u32, height: u32) -> Vec<u8> {
    let dom = parse_html(html);
    let rules = parse_stylesheet("div { width: 40px; height: 40px; background-color: #3366ff; }");
    let mut layout = build_layout_tree(&dom, &rules).expect("layout");
    compute(&mut layout, width as f32, height as f32).expect("compute");
    let items = build_display_list(&layout.tree, layout.root, &rules);
    encode_png(&rasterize_display_list(&items, width, height))
}
```

### Running a guest under capabilities

```rust
use tpt_helix_runtime::capability::{Capability, DomScope};
use tpt_helix_runtime::wasm::{Host, Module, sandboxed_engine};

let engine = sandboxed_engine()?;
let module = Module::load(&engine, &wasm_bytes)?;
// Only the listed capabilities are granted. Anything else is denied.
let host = Host::with_grants("my-app", [Capability::Dom { scope: DomScope::Full }]);
let mut instance = module.instantiate(&engine, host)?;
instance.run()?;
```

`Host::unrestricted()` bypasses the broker entirely. It exists for trusted
in-process tests and should not be used for untrusted code.

## Cargo features

| Feature | Default | Effect |
| --- | --- | --- |
| `libp2p` | off | Replaces the in-process `PeerNetwork` with a real libp2p Kademlia + request-response transport. Pulls in `libp2p`, `tokio`, and `async-trait`. |
| `hardware-decode` | off | Enables the VA-API / Vulkan Video decode backend slot. Off so CI runs headless. |

## Security posture

- **Deny by default.** `Host::new()` and `Host::with_grants` deny every
  capability that was not granted. Scopes are enforced: a `Subtree` DOM grant
  cannot reach outside its root, and a `Namespace` storage grant cannot touch a
  key outside its prefix.
- **Bounded guests.** WASM execution has a per-call step budget and a wall-clock
  deadline; memory and table growth is capped by `ResourceLimits`. Per-app
  DOM, storage, and handler state is capped (`MAX_ELEMENTS`,
  `MAX_CLICK_HANDLERS`, `MAX_STORAGE_*`), and so is the JS batch buffer
  (`MAX_BATCH_OPS`).
- **Bounded input.** DASH manifests are capped on representations and segments
  (`MAX_REPRESENTATIONS`, `MAX_SEGMENTS_PER_REPRESENTATION`) and on XML node
  count (`MAX_MPD_XML_NODES`); DTDs are rejected. libp2p frames are capped
  (`MAX_REQUEST_BYTES`, `MAX_BLOCK_BYTES`) before allocation. Stylesheets are
  refused past 128 nested blocks, and selectors past 32 nested parentheses.
- **Verified content.** Every block fetched by `ContentSource::get` is checked
  against its SHA-256 id before it is returned.

## Status and known limits

- The WASM host runs on `tpt-wasm`, taken from its GitHub repository. Each guest
  call has a wall-clock deadline (`ResourceLimits::call_deadline`) as well as a
  step budget. The deadline is enforced by tpt-wasm's Micro interpreter, the only
  tier tpt-helix uses.
- The GPU presenter (`gpu`) needs a display or a software Vulkan driver. CI uses
  the software rasterizer.
- The JS-to-WIT bridge is a compatibility shim, not a stable API.

## Testing

```sh
cargo test -p tpt-helix-runtime                    # unit + integration suites
cargo test -p tpt-helix-runtime --features libp2p  # real libp2p transport
cargo bench -p tpt-helix-runtime --bench layout_bench
```

The render tests compare pixels against expected output (`screenshot_diff`),
so they run without a display.

## License

MIT OR Apache-2.0. See the workspace `LICENSE-MIT` and `LICENSE-APACHE`.
