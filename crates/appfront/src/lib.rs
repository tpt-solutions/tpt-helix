//! TPT AppFront — the native UI shell for the Helix Runtime.
//!
//! AppFront hosts the Helix render surface inside an `egui` widget tree. Web
//! content (parsed HTML + CSS) is laid out with `taffy` and painted as a flat
//! list of render items, bridging the two trees:
//!
//! ```text
//! egui widget tree (AppFront chrome: window, header, panels)
//!        └── CentralPanel = Helix render surface
//!                 └── taffy layout tree (HTML/CSS content)
//!                         └── Vec<RenderItem>  →  painted via egui::Painter
//! ```
//!
//! The core bridge (`build` + `render`) is window-independent so it can be
//! unit-tested headlessly. The `window` feature adds the `eframe` shell that
//! opens a real native window and paints the surface with `egui`.

pub mod css;
pub mod layout;
pub mod render;

#[cfg(feature = "window")]
pub mod egui_surface;

pub use layout::HelixDocument;
pub use render::{RenderItem, RectItem, TextItem};
