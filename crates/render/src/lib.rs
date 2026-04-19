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

pub mod renderer;

#[cfg(feature = "shader-hot-reload")]
pub mod hot_reload;

pub use renderer::{Renderer, SceneParams, ShadingParams};

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
use pollster as _;
