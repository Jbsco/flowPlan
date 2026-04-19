//! flowPlan world rules: terrain, water, sky, weather, wind, time-of-day.
//!
//! Phase 1 scope: a procedural heightmap sampler (sum of sines, seeded) exposed to sim
//! for future buoyancy queries and to render/assets for mesh generation. Water, sky, and
//! weather join in later phases.
//!
//! See [`DESIGN.md § Subsystems`](../../../DESIGN.md#subsystems).

#![forbid(unsafe_code)]

pub mod terrain;

pub use terrain::{HeightSampler, Terrain};
