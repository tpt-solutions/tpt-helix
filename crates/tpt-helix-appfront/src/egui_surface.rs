//! The AppFront native shell: an `egui` (via `eframe`) window that hosts the
//! Helix render surface.
//!
//! This module is only compiled with the `window` feature. It wires AppFront as
//! the native UI shell (window chrome in `egui`) and bridges its central panel
//! — the Helix render surface — to the `taffy` layout tree produced by
//! [`crate::HelixDocument`].

use eframe::egui;

use crate::layout::{FrameCache, HelixDocument, HitBox};
use crate::render::RenderItem;

/// A primary-button click on the render surface.
///
/// `node` is the pre-order index of the DOM node under the pointer (the
/// innermost box), and `tag` its tag name. The host maps this to whatever it
/// dispatches to; AppFront itself has no guest handler table.
#[derive(Debug, Clone, PartialEq)]
pub struct ClickEvent {
    pub node: usize,
    pub tag: String,
    /// The guest element under the pointer (its `data-helix-id`). The host looks up
    /// that element's handlers in the runtime and dispatches them.
    pub element: Option<u64>,
    /// Pointer position in surface coordinates (origin at the render surface's top-left).
    pub x: f32,
    pub y: f32,
}

/// The AppFront application: an `egui` window hosting a single Helix document.
pub struct AppFront {
    doc: HelixDocument,
    cache: FrameCache,
    on_click: Option<Box<dyn FnMut(ClickEvent)>>,
}

impl AppFront {
    pub fn new(_cc: &eframe::CreationContext<'_>, source: String) -> Self {
        AppFront {
            doc: HelixDocument::parse(&source),
            cache: FrameCache::new(),
            on_click: None,
        }
    }

    /// Installs the callback that receives clicks on the render surface.
    pub fn on_click(mut self, handler: impl FnMut(ClickEvent) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

fn hit_to_click(hit: &HitBox, element: Option<u64>, x: f32, y: f32) -> ClickEvent {
    ClickEvent {
        node: hit.node,
        tag: hit.tag.clone(),
        element,
        x,
        y,
    }
}

impl eframe::App for AppFront {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // AppFront chrome — the `egui` widget tree that surrounds the surface.
        egui::TopBottomPanel::top("appfront_header").show(ctx, |ui| {
            ui.heading("TPT AppFront — Helix render surface");
            ui.label("egui widget tree  ⟶  hosts taffy layout tree");
        });

        // The central panel is the Helix render surface. Its available size
        // drives the `taffy` layout, and the result is painted with the `egui`
        // painter — the actual bridge between the two trees.
        egui::CentralPanel::default().show(ctx, |ui| {
            // Layout is in surface coordinates; the panel sits below the header,
            // so every item is painted relative to the panel's top-left corner.
            let panel = ui.max_rect();
            let origin = panel.min;
            let size = ui.available_size();
            // Re-laid out only when the surface size changes.
            let frame = self.cache.frame(&self.doc, size.x, size.y);
            let painter = ui.painter();

            for item in &frame.items {
                match item {
                    RenderItem::Rect(r) => {
                        let rect = egui::Rect::from_min_size(
                            origin + egui::vec2(r.x, r.y),
                            egui::vec2(r.w, r.h),
                        );
                        let fill = rgba(r.fill);
                        if let Some(bc) = r.border_color {
                            painter.rect(rect, r.radius, fill, (r.border_width, rgba(bc)));
                        } else {
                            painter.rect_filled(rect, r.radius, fill);
                        }
                    }
                    RenderItem::Text(t) => {
                        painter.text(
                            origin + egui::vec2(t.x, t.y),
                            egui::Align2::LEFT_TOP,
                            &t.text,
                            egui::FontId::proportional(t.size),
                            rgba(t.color),
                        );
                    }
                }
            }

            // Primary clicks inside the surface go to the innermost box under
            // the pointer.
            if let Some(handler) = self.on_click.as_mut()
                && ui.input(|i| i.pointer.primary_clicked())
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
                && panel.contains(pos)
            {
                let local = pos - origin;
                if let Some(hit) = frame.hit_test(local.x, local.y) {
                    let element = frame.element_at(local.x, local.y);
                    handler(hit_to_click(hit, element, local.x, local.y));
                }
            }
        });
    }
}

/// Converts a Helix colour to egui. Helix colours are straight (non-premultiplied)
/// RGBA, so they must use `from_rgba_unmultiplied`; `from_rgba_premultiplied`
/// would darken any partially transparent fill.
fn rgba(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Opens the native AppFront window and renders `source`.
pub fn run(source: String) -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "TPT AppFront",
        options,
        Box::new(|cc| Ok(Box::new(AppFront::new(cc, source)))),
    )
}
