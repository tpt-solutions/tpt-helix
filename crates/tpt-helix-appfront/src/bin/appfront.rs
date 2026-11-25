//! Entry point for the AppFront native window.
//!
//! Without arguments, the window shows a built-in static page:
//!
//! ```text
//! cargo run -p tpt-helix-appfront --features window
//! ```
//!
//! With a component, the window shows that guest's page, and a click dispatches the
//! guest's handlers:
//!
//! ```text
//! appfront guest.component.wasm --grant dom,storage --app my-app
//! ```
//!
//! Grants are deny-by-default, as with `helix run`: only the names given are
//! authorized.

#![cfg(feature = "window")]

use tpt_helix_appfront::GuestSurface;
use tpt_helix_runtime::capability::{Capability, DomScope, HostPattern, StorageScope};

const DEMO_PAGE: &str = r#"<!doctype html>
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

/// The capabilities named in a comma-separated `--grant` list. An unknown name is
/// an error, so a typo cannot silently narrow or widen the grant.
fn parse_grants(spec: &str) -> Result<Vec<Capability>, String> {
    let mut caps = Vec::new();
    for name in spec.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        caps.push(match name {
            "dom" => Capability::Dom {
                scope: DomScope::Full,
            },
            "storage" => Capability::Storage {
                scope: StorageScope::Global,
            },
            "network" => Capability::Network {
                hosts: vec![HostPattern::Any],
            },
            "media" => Capability::Media {
                max_resolution: Some((1920, 1080)),
            },
            other => {
                return Err(format!(
                    "unknown capability `{other}` (expected dom, storage, network, media)"
                ));
            }
        });
    }
    Ok(caps)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        tpt_helix_appfront::egui_surface::run(DEMO_PAGE.to_string())
            .expect("AppFront window failed to run");
        return;
    }

    let mut path = None;
    let mut grant = String::new();
    let mut app = "app".to_string();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--grant" => grant = it.next().unwrap_or_default(),
            "--app" => app = it.next().unwrap_or_default(),
            _ if path.is_none() => path = Some(arg),
            other => {
                eprintln!("unexpected argument `{other}`");
                std::process::exit(2);
            }
        }
    }
    let path = path.unwrap_or_default();

    let caps = parse_grants(&grant).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(2);
    });
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {path}: {e}");
        std::process::exit(2);
    });
    let guest = GuestSurface::load(&bytes, &app, caps).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    tpt_helix_appfront::egui_surface::run_guest(guest).expect("AppFront window failed to run");
}
