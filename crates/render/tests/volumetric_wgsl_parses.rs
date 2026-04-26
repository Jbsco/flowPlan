// Test-file relaxations per CODING_RULES § Testing Requirements.
#![allow(clippy::expect_used)]

//! Offline syntax + type-checking validation for `volumetric.wgsl`.
//!
//! Phase 3.6.3c.6 / 3.2x B2 deliverable. Validates the cloud-shader
//! skeleton parses correctly and exposes the entry points the renderer
//! pipeline expects. Doesn't need a GPU device — runs naga's WGSL
//! frontend + validator directly.

use flowplan_render::{CLOUD_NOISE_WGSL_SOURCE, VOLUMETRIC_WGSL_SOURCE};

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
fn volumetric_wgsl_parses_cleanly() {
    // Cloud raymarcher composes cloud_noise.wgsl (noise primitives)
    // before volumetric.wgsl (cloud-density model + raymarch). The
    // renderer composes them the same way at shader-module creation.
    // Volumetric depends on Worley symbols from cloud_noise; standalone
    // it would fail to parse, which is intentional.
    let composed = format!("{CLOUD_NOISE_WGSL_SOURCE}\n{VOLUMETRIC_WGSL_SOURCE}");
    let module = naga::front::wgsl::parse_str(&composed)
        .expect("composed cloud shader failed to parse — fix syntax errors");

    let validator_caps = naga::valid::Capabilities::all();
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), validator_caps);
    if let Err(e) = validator.validate(&module) {
        panic!(
            "composed cloud shader failed naga validation: {e:?}\n\
             B3+ will only build on a parseable composition; fix syntax/type \
             errors here first."
        );
    }
}

#[test]
fn volumetric_wgsl_exports_renderer_entrypoints() {
    // Renderer's `build_volumetric_pipeline` uses these exact names.
    // If they're renamed in the WGSL, the renderer fails to build the
    // pipeline at runtime — catch the rename at test time instead.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("fn vs_volumetric("),
        "volumetric.wgsl missing vertex entry point `vs_volumetric`"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("fn fs_volumetric("),
        "volumetric.wgsl missing fragment entry point `fs_volumetric`"
    );
}

#[test]
fn volumetric_wgsl_uses_expected_bind_groups() {
    // @group(0) = camera, @group(1) = scene per
    // `docs/WEATHER.md` § Cloud rendering technique → Render pass
    // placement. B3 adds @group(2) for depth read.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("@group(0) @binding(0) var<uniform> camera"),
        "volumetric.wgsl missing camera bind group at @group(0)"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("@group(1) @binding(0) var<uniform> scene"),
        "volumetric.wgsl missing scene bind group at @group(1)"
    );
}

#[test]
fn volumetric_wgsl_b3_density_model_present() {
    // B3 ships altitude_profile + footprint_falloff + sample_cloud_density
    // + a hardcoded test-cell so clouds become visible. Catches
    // accidental rollback to the B2 transparent stub.
    for name in [
        "fn altitude_profile(",
        "fn footprint_falloff(",
        "fn sample_cloud_density(",
        "TEST_CELL_TYPE",
        "TEST_CELL_BASE_M",
        "TEST_CELL_TOP_M",
    ] {
        assert!(
            VOLUMETRIC_WGSL_SOURCE.contains(name),
            "volumetric.wgsl missing B3 density-model symbol `{name}`"
        );
    }
    // B3 calls Worley directly — make sure the function name reference
    // matches what cloud_noise.wgsl exports.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("worley_3d("),
        "volumetric.wgsl B3 must call worley_3d"
    );
}
