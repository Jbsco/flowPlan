// Test-file relaxations per CODING_RULES § Testing Requirements.
#![allow(clippy::expect_used)]

//! Offline syntax + validation for `wingtip_vortex.wgsl`.
//!
//! Catches WGSL syntax / type errors at test time so they don't blow
//! up at GPU-pipeline creation in the running game.

use flowplan_render::WINGTIP_VORTEX_WGSL_SOURCE;

// Other workspace + dev-deps belong to the lib + visual-snapshot test.
use bytemuck as _;
use flowplan_assets as _;
use flowplan_sim as _;
use flowplan_world as _;
use glam as _;
use image as _;
use insta as _;
use pollster as _;
use wgpu as _;

#[test]
fn wingtip_vortex_wgsl_parses_cleanly() {
    let module = naga::front::wgsl::parse_str(WINGTIP_VORTEX_WGSL_SOURCE)
        .expect("wingtip_vortex.wgsl failed to parse — fix syntax errors");

    let validator_caps = naga::valid::Capabilities::all();
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), validator_caps);
    if let Err(e) = validator.validate(&module) {
        panic!("wingtip_vortex.wgsl failed naga validation: {e:?}");
    }
}

#[test]
fn wingtip_vortex_wgsl_exports_entrypoints() {
    assert!(
        WINGTIP_VORTEX_WGSL_SOURCE.contains("fn vs_main("),
        "wingtip_vortex.wgsl missing vertex entry point `vs_main`"
    );
    assert!(
        WINGTIP_VORTEX_WGSL_SOURCE.contains("fn fs_main("),
        "wingtip_vortex.wgsl missing fragment entry point `fs_main`"
    );
}
