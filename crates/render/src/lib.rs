//! flowPlan renderer.
//!
//! Phase 1 scope: a single flat-shaded pipeline that draws a terrain patch + a placeholder
//! cube driven by `SimSnapshot` interpolation. Depth buffer, camera uniform, per-draw
//! model uniform. Cel shading arrives in Phase 2.
//!
//! `unsafe` is permitted only at documented wgpu lifetime-management sites; none are
//! currently required -- the safe wgpu API covers Phase 1. We keep `deny(unsafe_code)` as
//! a guard and will relax to a documented `allow` if specific needs arise.

#![deny(unsafe_code)]
// Mesh index counts and window dimensions cast between usize/u32/f32 at boundaries; the
// values involved fit within safe ranges for our scene sizes.
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
// Some clippy::too_many_lines targets are unavoidable for wgpu init where the linearity
// of the descriptor chain is part of the readability.
#![allow(clippy::too_many_lines, clippy::too_many_arguments)]

pub mod debug_lines;
pub mod instances;
pub mod renderer;
pub mod telemetry;

#[cfg(feature = "shader-hot-reload")]
pub mod hot_reload;

pub use debug_lines::{DebugLineBatch, DebugLineVertex, DebugLinesRenderer};
pub use instances::{InstanceGpuData, InstanceRenderer};
pub use renderer::{
    Renderer, SceneParams, ShadingParams, CLOUD_NOISE_WGSL_SOURCE, VOLUMETRIC_WGSL_SOURCE,
    WATER_WGSL_SOURCE,
};
pub use telemetry::Telemetry;

// Dev-deps are consumed by `tests/visual_snapshot.rs` (Tier-3, hardware-gated). Mark
// them as intentional here so the lib-test target does not trip
// `-D unused_crate_dependencies` when the feature is disabled.
#[cfg(test)]
use flowplan_world as _;
#[cfg(test)]
use image as _;
#[cfg(test)]
use insta as _;
#[cfg(test)]
use naga as _;
#[cfg(test)]
use pollster as _;

/// Verify that the water shader source compiled into this binary contains the
/// expected wave-set + modulation features. Catches the stale-build failure mode.
#[cfg(test)]
mod shader_baked_in_tests {
    use super::WATER_WGSL_SOURCE;

    #[test]
    fn water_shader_has_seven_waves() {
        assert!(WATER_WGSL_SOURCE.contains("WAVE_COUNT: u32 = 7u"));
    }

    #[test]
    fn water_shader_has_amp_modulation() {
        assert!(WATER_WGSL_SOURCE.contains("fn amp_modulation"));
        assert!(WATER_WGSL_SOURCE.contains("amp_modulation(i, t, world_xz)"));
    }

    #[test]
    fn water_shader_uses_value_noise_for_modulation() {
        assert!(WATER_WGSL_SOURCE.contains("value_noise2(noise_pos)"));
    }

    #[test]
    fn water_shader_has_noise_displacement() {
        assert!(WATER_WGSL_SOURCE.contains("noise_displacement"));
    }

    #[test]
    fn water_shader_has_domain_warping() {
        // Domain warping is the critical technique for breaking the regular
        // Gerstner interference lattice — wave phases must sample at a
        // noise-perturbed position, not the raw vertex position.
        assert!(WATER_WGSL_SOURCE.contains("warped_x"));
        assert!(WATER_WGSL_SOURCE.contains("warped_z"));
        assert!(
            WATER_WGSL_SOURCE.contains("dir.x * warped_x + dir.y * warped_z"),
            "wave phase must use warped position, not world0 directly",
        );
    }

    #[test]
    fn water_shader_has_per_component_wave_t() {
        assert!(
            WATER_WGSL_SOURCE.contains("wave_t:      vec2<f32>")
                || WATER_WGSL_SOURCE.contains("wave_t: vec2<f32>"),
        );
    }
}
