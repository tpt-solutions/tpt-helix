pub mod capability;
pub mod content;
pub mod css;
pub mod dash;
pub mod display_list;
pub mod fonts;
pub mod gpu;
pub mod html;
pub mod js;
pub mod js_bridge;
pub mod layout;
pub mod manifest;
pub mod media_decode;
pub mod p2p;
#[cfg(feature = "libp2p")]
pub mod p2p_libp2p;
pub mod profiler;
pub mod raster;
pub mod screenshot_diff;
pub mod software_raster;
pub mod stub;
pub mod text;
pub mod wasm;

/// Installs a `tracing` subscriber that writes to stderr, filtered by `RUST_LOG`
/// (e.g. `RUST_LOG=helix::capability=warn`). Defaults to `warn` when `RUST_LOG`
/// is unset. Idempotent: calling it again, or after another subscriber is
/// installed, is a no-op. Hosts call this once at startup.
pub fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
