//! flowPlan application entry point.
//!
//! Phase 0 scope: open a window, initialize wgpu, clear each frame to a fixed color, exit
//! cleanly on close. Does not yet host the sim/render/audio layers; those integrate in
//! Phase 1+.
//!
//! See [`ROADMAP.md § Phase 0`](../../../ROADMAP.md#phase-0-skeleton).

// winit surface creation requires owned wgpu operations against a raw handle; wgpu's
// safe API covers everything we need in Phase 0, so no unsafe code yet.
#![deny(unsafe_code)]
// Binary crates do not expose APIs externally; `unreachable_pub` warnings are noise here.
// The `pub(crate)` / `pub(super)` alternatives collide with clippy's `redundant_pub_crate`,
// so plain `pub` + this allow is the consistent choice for the binary.
#![allow(unreachable_pub)]

mod clear_app;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    install_tracing();
    tracing::info!("flowPlan starting (Phase 0 skeleton)");
    clear_app::run()
}

fn install_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
