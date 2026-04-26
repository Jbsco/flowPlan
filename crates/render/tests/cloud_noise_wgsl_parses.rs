// Test-file relaxations per CODING_RULES § Testing Requirements.
// `expect` in tests is idiomatic and surfaces useful failure context.
#![allow(clippy::expect_used)]

//! Offline syntax + type-checking validation for `cloud_noise.wgsl`.
//!
//! Phase 3.6.3c.6 / 3.2x B1 deliverable. Doesn't need a GPU device —
//! runs naga's WGSL frontend + validator directly. Catches:
//!
//! - Syntax errors (typos, missing semicolons, etc.)
//! - Type mismatches (e.g. `i32` vs `u32` confusion)
//! - Undefined symbols
//! - Invalid module structure
//!
//! What it does NOT catch:
//!
//! - Numerical divergence between Rust and WGSL implementations
//!   (that's the B4 GPU readback bit-identity test)
//! - Driver-specific quirks (Phase 6.1 cross-platform hardening)
//!
//! When this test fails, fix the WGSL syntax. When the B4 readback
//! test fails, fix the divergence between Rust and WGSL math.

use flowplan_render::CLOUD_NOISE_WGSL_SOURCE;

// Other workspace + dev-deps belong to the lib + visual-snapshot test;
// pin as unused here so this test crate doesn't generate dead-extern
// warnings.
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
fn cloud_noise_wgsl_parses_cleanly() {
    let module = naga::front::wgsl::parse_str(CLOUD_NOISE_WGSL_SOURCE)
        .expect("cloud_noise.wgsl failed to parse — fix syntax errors");

    // Validate produces an Info struct or a list of errors. We want
    // zero errors; any non-fatal warnings are surfaced via panic with
    // the full diagnostic.
    let validator_caps = naga::valid::Capabilities::all();
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), validator_caps);
    if let Err(e) = validator.validate(&module) {
        panic!(
            "cloud_noise.wgsl failed naga validation: {e:?}\n\
             Errors above must be fixed before B4 GPU readback test can run."
        );
    }
}

#[test]
fn cloud_noise_wgsl_exports_expected_functions() {
    // Spot-check that the public functions intended for cloud-shader
    // consumption exist by name. Naga doesn't expose a clean function-
    // listing API; do a string search on the source so this test
    // catches accidental rename / removal during refactors.
    let expected_fns = [
        "worley_2d",
        "worley_3d",
        "feature_jitter_2d",
        "feature_jitter_3d",
        "pcg_2d_hash",
        "pcg_3d_hash",
        "smooth_blend",
        "wrap_coord",
        "wrap_cell_index_u32",
        "seed_to_u32",
        "rotl_u32",
        // B3c Perlin-Worley blend
        "cloud_perlin_3d",
        "cloud_perlin_gradient_3d",
        "cloud_perlin_fade",
        "cloud_perlin_lerp",
    ];
    for f in expected_fns {
        let signature = format!("fn {f}(");
        assert!(
            CLOUD_NOISE_WGSL_SOURCE.contains(&signature),
            "cloud_noise.wgsl missing expected function `{f}`. \
             If renamed, update this test."
        );
    }
}

#[test]
fn cloud_noise_wgsl_exports_worley_result_struct() {
    assert!(
        CLOUD_NOISE_WGSL_SOURCE.contains("struct WorleyResult {"),
        "cloud_noise.wgsl missing `WorleyResult` struct"
    );
    // Field names referenced by the cloud raymarcher (B3+).
    assert!(CLOUD_NOISE_WGSL_SOURCE.contains("f1: f32"));
    assert!(CLOUD_NOISE_WGSL_SOURCE.contains("f2: f32"));
    assert!(CLOUD_NOISE_WGSL_SOURCE.contains("f_blended: f32"));
}

#[test]
fn cloud_noise_wgsl_uses_pcg_constants_matching_rust() {
    // Bit-identity invariant: WGSL constants MUST match Rust
    // `pcg_2d_hash` / `pcg_3d_hash`. If you change one, change both.
    // See `crates/world/src/noise.rs::pcg_2d_hash`.
    assert!(
        CLOUD_NOISE_WGSL_SOURCE.contains("PCG_MULT: u32 = 1664525u"),
        "PCG_MULT constant changed — must match Rust 1_664_525 \
         in pcg_2d_hash/pcg_3d_hash"
    );
    assert!(
        CLOUD_NOISE_WGSL_SOURCE.contains("PCG_INC: u32 = 1013904223u"),
        "PCG_INC constant changed — must match Rust 1_013_904_223 \
         in pcg_2d_hash/pcg_3d_hash"
    );
}
