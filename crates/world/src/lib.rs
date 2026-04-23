//! flowPlan world rules: terrain, water, sky, weather, wind, time-of-day.
//!
//! Phase 1 scope: a procedural heightmap sampler (sum of sines, seeded) exposed to sim
//! for future buoyancy queries and to render/assets for mesh generation. Water, sky, and
//! weather join in later phases.
//!
//! See [`DESIGN.md § Subsystems`](../../../DESIGN.md#subsystems).

#![forbid(unsafe_code)]

pub mod sky;
pub mod terrain;
pub mod water;
pub mod weather;

pub use sky::{local_time_of_day, sun_direction};
pub use terrain::{HeightSampler, Terrain};
pub use water::{chop_intensity_from_wind, max_amplitude as water_max_amplitude, water_height};
pub use weather::{
    cell_at as weather_cell_at, cell_at_drifted as weather_cell_at_drifted, frontal_segments,
    humidity_at, pressure_at, temperature_at, turbulence_at, wind_at, CellId as WeatherCellId,
    CellLookup as WeatherCellLookup, FrontalSegment,
};

// Dev-dep used only by `tests/weather_visual_bench.rs`; marker silences the
// unused-crate-dependencies lint on other test targets.
#[cfg(test)]
use image as _;
