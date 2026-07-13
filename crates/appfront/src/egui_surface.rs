//! The AppFront native shell: an `egui` (via `eframe`) window that hosts the
//! Helix render surface.
//!
//! This module is only compiled with the `window` feature. It wires AppFront as
//! the native UI shell (window chrome in `egui`) and bridges its central panel
//! — the Helix render surface — to the `taffy` layout tree produced by
//! [`crate::HelixDocument`].

use eframe::egui;

use crate::layout::HelixDocument;
use crate::render::RenderItem;

/// The AppFront application: an `egui` window hosting a single Helix document.
pub struct AppFront {
    doc: HelixDocument,
}

impl AppFront {
    pub fn new(_cc: &eframe::CreationContext<'_>, source: String) -> Self {
        AppFront {
            doc: HelixDocument::parse(&source),
        }
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
            let size = ui.available_size();
            let items = self.doc.render(size.x, size.y);
            let painter = ui.painter();

            for item in items {
                match item {
                    RenderItem::Rect(r) => {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(r.x, r.y),
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
                            egui::pos2(t.x, t.y),
                            egui::Align2::LEFT_TOP,
                            &t.text,
                            egui::FontId::proportional(t.size),
                            rgba(t.color),
                        );
                    }
                }
            }
        });
    }
}

fn rgba(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3])
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
