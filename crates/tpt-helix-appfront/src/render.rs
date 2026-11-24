//! Render items produced from the `taffy` layout tree.
//!
//! These are plain, window-independent data structures so the bridge can be
//! tested headlessly. The `egui` shell converts them into `egui` draw calls.

/// A filled rectangle (optionally with a border and rounded corners).
#[derive(Debug, Clone, PartialEq)]
pub struct RectItem {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub fill: [u8; 4],
    pub border_color: Option<[u8; 4]>,
    pub border_width: f32,
    pub radius: f32,
}

/// A run of text positioned by its top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: [u8; 4],
    pub size: f32,
}

/// A single drawable produced by the Helix render surface.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderItem {
    Rect(RectItem),
    Text(TextItem),
}
