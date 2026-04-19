//! flowPlan application entry point.
//!
//! Phase 1 scope: fixed-timestep sim loop, input queue, camera view switching, procedural
//! terrain + placeholder cube rendered with a flat-shaded pipeline, double-buffered sim
//! snapshot with interpolation into the render.
//!
//! See [`ROADMAP.md § Phase 1`](../../../ROADMAP.md#phase-1-core-loop).

#![deny(unsafe_code)]
// Binary crates do not expose APIs externally; `unreachable_pub` warnings are noise.
#![allow(unreachable_pub)]

mod debug_gizmo;
mod debug_hud;
mod game_app;
mod input_map;
mod snapshot_buf;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    install_tracing();
    tracing::info!("flowPlan starting");
    game_app::run()
}

fn install_tracing() {
    // `calloop=error` silences a known benign warning from winit 0.30's Wayland
    // backend ("Received an event for non-existence source") that fires when calloop
    // dispatches a queued event whose source was removed between queueing and
    // dispatch -- typically during resize, scale change, or focus transitions.
    // Upstream race in winit/calloop; no downstream fix. Drop the `calloop=error`
    // segment to re-surface genuine calloop errors if we ever hit one.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,calloop=error")
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
