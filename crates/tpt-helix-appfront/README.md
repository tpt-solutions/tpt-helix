# tpt-helix-appfront

AppFront is the native window host for TPT Helix. It lays out HTML and CSS with
`taffy`, turns the result into a flat list of render items, and paints those
items inside an `egui` window.

The layout and render bridge has no window dependency, so it builds and tests
headlessly. The window is behind the `window` feature.

## How the pieces fit

```text
egui widget tree (AppFront chrome: window, header, panels)
  └── central panel = Helix render surface
        └── taffy layout tree (HTML + CSS content)
              └── Vec<RenderItem>  →  painted with egui::Painter
```

| Module | Role |
| --- | --- |
| `layout` | `HelixDocument` parses HTML and computes the `taffy` tree. |
| `css` | Parses stylesheets and inline styles, resolves properties per element, and extracts `<style>` blocks. |
| `render` | `RectItem` and `TextItem`, the flat paint list the surface draws. |
| `egui_surface` | (`window` feature) the `eframe` application that opens a native window. |

## Headless use

```rust
use tpt_helix_appfront::HelixDocument;

let doc = HelixDocument::parse(
    "<style>div { width: 200px; height: 80px; background-color: #3366ff; }</style>
     <div></div>",
);
// `doc.build(width, height)` lays the document out. The `render` module then
// turns that tree into rectangles and text, in paint order.
```

Use this path for tests, snapshot checks, and anything that runs without a
display.

## Opening a window

The window needs a display and a GPU driver (or a software renderer). Build and
run it with the feature enabled:

```sh
cargo run -p tpt-helix-appfront --features window
```

`egui_surface::run(html)` opens the window with the given document. The release
binary is named `appfront`.

## Notes

- The `window` feature pulls in `eframe`, which brings `winit`, `wgpu`, and
  `glow`. It is heavy, so the workspace excludes this crate from default builds.
- The CI release job builds the `appfront` binary for Linux x86_64.

## License

MIT OR Apache-2.0. See the workspace `LICENSE-MIT` and `LICENSE-APACHE`.
