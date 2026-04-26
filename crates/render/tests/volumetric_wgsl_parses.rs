// Test-file relaxations per CODING_RULES § Testing Requirements.
#![allow(clippy::expect_used)]

//! Offline syntax + type-checking validation for `volumetric.wgsl`.
//!
//! Phase 3.6.3c.6 / 3.2x B2 deliverable. Validates the cloud-shader
//! skeleton parses correctly and exposes the entry points the renderer
//! pipeline expects. Doesn't need a GPU device — runs naga's WGSL
//! frontend + validator directly.

use flowplan_render::VOLUMETRIC_WGSL_SOURCE;

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
    let module = naga::front::wgsl::parse_str(VOLUMETRIC_WGSL_SOURCE)
        .expect("volumetric.wgsl failed to parse — fix syntax errors");

    let validator_caps = naga::valid::Capabilities::all();
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), validator_caps);
    if let Err(e) = validator.validate(&module) {
        panic!(
            "volumetric.wgsl failed naga validation: {e:?}\n\
             B3+ will only build on a parseable skeleton; fix syntax/type \
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
fn volumetric_wgsl_skeleton_returns_transparent() {
    // The B2 skeleton must return alpha = 0 (or near-zero) so it's a
    // visual no-op until B3 wires real density. Catch accidental
    // baked-in opacity that would cover the scene.
    //
    // Heuristic: the placeholder `sample_density_at` should return 0.0
    // and the early-out threshold should still be a hard alpha cap.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("return 0.0;"),
        "volumetric.wgsl placeholder density sampler must return 0.0 in B2"
    );
}
