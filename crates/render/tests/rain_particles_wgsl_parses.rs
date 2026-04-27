// Test-file relaxations per CODING_RULES § Testing Requirements.
#![allow(clippy::expect_used)]

//! Offline syntax + validation for `rain_particles.wgsl`.
//!
//! Phase E.4 rain particle billboard shader. Catches WGSL syntax /
//! type errors at test time so they don't blow up at GPU-pipeline
//! creation in the running game (where they panic the device with
//! validation errors).

use flowplan_render::RAIN_PARTICLES_WGSL_SOURCE;

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
fn rain_particles_wgsl_parses_cleanly() {
    let module = naga::front::wgsl::parse_str(RAIN_PARTICLES_WGSL_SOURCE)
        .expect("rain_particles.wgsl failed to parse — fix syntax errors");

    let validator_caps = naga::valid::Capabilities::all();
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), validator_caps);
    if let Err(e) = validator.validate(&module) {
        panic!("rain_particles.wgsl failed naga validation: {e:?}");
    }
}

#[test]
fn rain_particles_wgsl_exports_entrypoints() {
    assert!(
        RAIN_PARTICLES_WGSL_SOURCE.contains("fn vs_main("),
        "rain_particles.wgsl missing vertex entry point `vs_main`"
    );
    assert!(
        RAIN_PARTICLES_WGSL_SOURCE.contains("fn fs_main("),
        "rain_particles.wgsl missing fragment entry point `fs_main`"
    );
}
