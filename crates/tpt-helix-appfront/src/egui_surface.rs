//! The AppFront native shell: an `egui` (via `eframe`) window that hosts the
//! Helix render surface.
//!
//! This module is only compiled with the `window` feature. It wires AppFront as
//! the native UI shell (window chrome in `egui`) and bridges its central panel
//! — the Helix render surface — to the `taffy` layout tree produced by
//! [`crate::HelixDocument`].
//!
//! The page is either a static HTML document, or the DOM of a running guest
//! ([`crate::GuestSurface`]). A click on a guest page dispatches the guest's
//! handlers, and the page is refreshed from the guest's DOM afterwards.

use eframe::egui;

use crate::guest::GuestSurface;
use crate::layout::{FrameCache, HelixDocument, HitBox};
use crate::render::RenderItem;

/// A primary-button click on the render surface.
///
/// `node` is the pre-order index of the DOM node under the pointer (the
/// innermost box), and `tag` its tag name. `element` is the guest element under the
/// pointer, which the guest surface uses to dispatch handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct ClickEvent {
    pub node: usize,
    pub tag: String,
    /// The guest element under the pointer (its `data-helix-id`).
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
    guest: Option<GuestSurface>,
}

impl AppFront {
    /// A window showing the static HTML `source`.
    pub fn new(_cc: &eframe::CreationContext<'_>, source: String) -> Self {
        AppFront {
            doc: HelixDocument::parse(&source),
            cache: FrameCache::new(),
            on_click: None,
            guest: None,
        }
    }

    /// A window showing a running guest's page. Clicks dispatch the guest's handlers.
    pub fn with_guest(guest: GuestSurface) -> Self {
        AppFront {
            doc: HelixDocument::parse(&guest.html()),
            cache: FrameCache::new(),
            on_click: None,
            guest: Some(guest),
        }
    }

    /// Installs the callback that receives clicks on the render surface.
    pub fn on_click(mut self, handler: impl FnMut(ClickEvent) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Handles one click: the host callback first, then the guest, whose handlers
    /// may change its DOM. The page is re-read when they do.
    fn handle_click(&mut self, click: ClickEvent) {
        if let Some(handler) = self.on_click.as_mut() {
            handler(click.clone());
        }
        let Some(guest) = self.guest.as_mut() else {
            return;
        };
        let Some(element) = click.element else {
            return;
        };
        match guest.click(element) {
            Ok(Some(html)) => {
                self.doc = HelixDocument::parse(&html);
                self.cache = FrameCache::new();
            }
            Ok(None) => {}
            Err(e) => eprintln!("guest handler failed: {e}"),
        }
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
        let mut clicked: Option<ClickEvent> = None;
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

            // Primary clicks inside the surface go to the innermost box under the
            // pointer. The click is handled after the panel, since handling it may
            // replace the page this frame was laid out from.
            if ui.input(|i| i.pointer.primary_clicked())
                && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
                && panel.contains(pos)
            {
                let local = pos - origin;
                if let Some(hit) = frame.hit_test(local.x, local.y) {
                    let element = frame.element_at(local.x, local.y);
                    clicked = Some(hit_to_click(hit, element, local.x, local.y));
                }
            }
        });

        if let Some(click) = clicked {
            self.handle_click(click);
        }
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

/// Opens the native AppFront window showing a running guest's page.
pub fn run_guest(guest: GuestSurface) -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "TPT AppFront",
        options,
        Box::new(move |_cc| Ok(Box::new(AppFront::with_guest(guest)))),
    )
}
