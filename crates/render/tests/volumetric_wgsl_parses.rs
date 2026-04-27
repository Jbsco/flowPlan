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

/// Structural perf-budget: asserts the cost-controlling constants in
/// `volumetric.wgsl` stay within the budget envelope. This is a cheap
/// regression guard — catches accidental bumps to `RAYMARCH_STEPS` or
/// `CLOUD_FEATURES_PER_CELL` that would tank frame rate without anyone
/// noticing until in-game playtest.
///
/// **NOTE (followup)**: real GPU wall-clock perf gate is queued as a
/// separate piece of infrastructure (hardware-gated test using wgpu
/// timestamp queries). This structural test cannot detect performance
/// regressions caused by changes to the math itself (e.g. unrolling
/// the `worley_3d` inner loop, adding more taps), only constant-bump
/// regressions. See `docs/WEATHER.md` § Performance budget for the
/// proper gate.
#[test]
fn volumetric_wgsl_cost_constants_within_budget() {
    // Helper: parse a WGSL `const NAME: type = value;` declaration and
    // return the literal numeric value. Returns None if not found.
    fn extract_const_u32(src: &str, name: &str) -> Option<u32> {
        let needle = format!("const {name}: ");
        let idx = src.find(&needle)?;
        let line_end = src[idx..].find(';')? + idx;
        let line = &src[idx..line_end];
        let eq = line.rfind('=')?;
        let value_str = line[eq + 1..].trim().trim_end_matches('u');
        value_str.parse().ok()
    }
    fn extract_const_i32(src: &str, name: &str) -> Option<i32> {
        let needle = format!("const {name}: ");
        let idx = src.find(&needle)?;
        let line_end = src[idx..].find(';')? + idx;
        let line = &src[idx..line_end];
        let eq = line.rfind('=')?;
        line[eq + 1..].trim().parse().ok()
    }
    fn extract_const_f32(src: &str, name: &str) -> Option<f32> {
        let needle = format!("const {name}: ");
        let idx = src.find(&needle)?;
        let line_end = src[idx..].find(';')? + idx;
        let line = &src[idx..line_end];
        let eq = line.rfind('=')?;
        line[eq + 1..].trim().parse().ok()
    }

    // Per docs/WEATHER.md § Performance budget — cloud-pass target is
    // ≤ 0.6 ms at 240 Hz reference. The cost-bounding constants below
    // are the dominant per-pixel cost factors. Bumping any past these
    // limits has shown FPS to fall under 60 Hz in playtest.

    let raymarch_steps = extract_const_i32(VOLUMETRIC_WGSL_SOURCE, "RAYMARCH_STEPS_MAX")
        .expect("RAYMARCH_STEPS_MAX const not found in volumetric.wgsl");
    assert!(
        raymarch_steps <= 64,
        "RAYMARCH_STEPS_MAX = {raymarch_steps} exceeds budget cap of 64. \
         Bumping past 32 was shown to drop FPS below 60 in playtest \
         without ray-cylinder clamping. If a cap raise is needed, also \
         tighten the ray-cylinder + altitude-band logic to keep \
         per-pixel cost bounded."
    );

    let target_step = extract_const_f32(VOLUMETRIC_WGSL_SOURCE, "RAYMARCH_TARGET_STEP_M")
        .expect("RAYMARCH_TARGET_STEP_M const not found in volumetric.wgsl");
    assert!(
        target_step >= 200.0,
        "RAYMARCH_TARGET_STEP_M = {target_step} is below 200 m floor. \
         Smaller target steps mean more samples per ray = higher per-pixel \
         cost. Adaptive sampler picks ceil(band_length / target_step); \
         dropping below 200 m fast-tracks fps regression."
    );

    let features_per_cell = extract_const_u32(VOLUMETRIC_WGSL_SOURCE, "CLOUD_FEATURES_PER_CELL")
        .expect("CLOUD_FEATURES_PER_CELL const not found in volumetric.wgsl");
    assert!(
        features_per_cell <= 2,
        "CLOUD_FEATURES_PER_CELL = {features_per_cell} exceeds budget cap of 2. \
         3+ features doubles+ per-Worley distance-compute cost, fast path to \
         FPS regression. Reserved for future Ultra-preset opt-in."
    );

    // Tessellation: shader uses nearest-cell lookup (not per-cell
    // cylinder iteration). C2-polish 2026-04-26 — circular footprints
    // were a wrong-feeling shortcut; tessellation is the locked design.
    // CM-6 (2026-04-26) replaced the single-cell `find_nearest_active_cell`
    // with `find_nearest_two_active_cells` for boundary blending.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("find_nearest_two_active_cells("),
        "volumetric.wgsl missing tessellation lookup — must use \
         find_nearest_two_active_cells (CM-6 boundary blending), not \
         per-cell circular footprint and not single-nearest"
    );
}

#[test]
fn volumetric_wgsl_uses_expected_bind_groups() {
    // @group(0) = camera, @group(1) = scene, @group(2) = scene depth
    // (B3b — for ray termination at terrain), @group(3) = weather cells
    // storage buffer (C2 — multi-cell streamed weather).
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("@group(0) @binding(0) var<uniform> camera"),
        "volumetric.wgsl missing camera bind group at @group(0)"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("@group(1) @binding(0) var<uniform> scene"),
        "volumetric.wgsl missing scene bind group at @group(1)"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("@group(2) @binding(0) var scene_depth"),
        "volumetric.wgsl missing depth bind group at @group(2) (B3b)"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("@group(3) @binding(0) var<storage, read> weather_cells"),
        "volumetric.wgsl missing weather-cells bind group at @group(3) (C2)"
    );
}

#[test]
fn volumetric_wgsl_b3b_depth_termination_present() {
    // B3b ships the depth-sampling logic that clamps t_exit to the
    // terrain-pixel distance. Catches accidental rollback to B3 (no
    // depth termination) which would render clouds through mountains.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("textureLoad(scene_depth"),
        "volumetric.wgsl missing scene_depth textureLoad — B3b regression"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("t_terrain"),
        "volumetric.wgsl missing t_terrain clamp — B3b regression"
    );
}

#[test]
fn volumetric_wgsl_density_model_present() {
    // C2: density model functions are present + storage-buffer iteration
    // replaced the hardcoded test cell.
    for name in [
        "fn altitude_profile(",
        "fn altitude_perturbation_amplitude(",
        "fn find_nearest_two_active_cells(",
        "fn perlin_worley_weights_for_type(",
        "fn sample_cloud_density(",
        "fn cell_density_contribution(",
        "fn ray_altitude_band(",
        "fn henyey_greenstein(",
        "fn cloud_density_cheap_in_cell(",
        "fn sun_transmittance_through_cell(",
        "struct GpuWeatherCell",
        "struct NearestTwo",
        "struct CloudSample",
        "weather_cells",
        "GLOBAL_CLOUD_BASE_M",
        "GLOBAL_CLOUD_TOP_M",
        "ALT_PERTURB_SPACING_M",
        "BOUNDARY_BLEND_DISTANCE_M",
        "VERTICAL_NOISE_STRETCH",
        "SUN_RAY_STEPS",
        "SUN_EXTINCTION_SCALE",
        "SUN_INTENSITY",
        "MULTISCATTER_INTENSITY",
        "AMBIENT_INTENSITY",
        "PHASE_G",
        "coverage_strength",
    ] {
        assert!(
            VOLUMETRIC_WGSL_SOURCE.contains(name),
            "volumetric.wgsl missing density-model symbol `{name}`"
        );
    }
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("worley_3d("),
        "volumetric.wgsl must call worley_3d"
    );
    // Catch accidental rollback to single hardcoded cell.
    assert!(
        !VOLUMETRIC_WGSL_SOURCE.contains("TEST_CELL_TYPE"),
        "volumetric.wgsl still references the B3 hardcoded TEST_CELL — \
         C2 should iterate the storage-buffer cell array"
    );
}

#[test]
fn volumetric_wgsl_b3c_perlin_worley_blend_present() {
    // B3c ships Perlin-Worley blend per WEATHER.md § Sub-cell detail.
    // Catches accidental rollback to Worley-only.
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("cloud_perlin_3d("),
        "volumetric.wgsl missing cloud_perlin_3d call — B3c regression"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("CUMULUS_WORLEY_WEIGHT"),
        "volumetric.wgsl missing CUMULUS_WORLEY_WEIGHT — B3c regression"
    );
    assert!(
        VOLUMETRIC_WGSL_SOURCE.contains("CUMULUS_PERLIN_WEIGHT"),
        "volumetric.wgsl missing CUMULUS_PERLIN_WEIGHT — B3c regression"
    );
}
