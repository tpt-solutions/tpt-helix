//! Entry point for the AppFront native window.
//!
//! Run with: `cargo run -p appfront --features window`

#![cfg(feature = "window")]

fn main() {
    let html = r#"<!doctype html>
<html>
  <head>
    <style>
      body { display: block; background: #1e1e2e; color: #cdd6f4; }
      .card { display: block; background: #313244; border: 2px solid #89b4fa;
              border-radius: 8px; padding: 16px; margin: 12px; }
      h1 { display: block; color: #f9e2af; }
    </style>
  </head>
  <body>
    <div class="card">
      <h1>Hello from Helix</h1>
      <p>This content is laid out by taffy and painted by egui.</p>
    </div>
  </body>
</html>"#;

    appfront::egui_surface::run(html.to_string()).expect("AppFront window failed to run");
}
