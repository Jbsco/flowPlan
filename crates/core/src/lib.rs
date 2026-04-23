//! flowPlan core: math, arenas, handles, SPSC rings, RNG, time types.
//!
//! This crate is the bottom of the layer DAG per [`DESIGN.md`](../../../DESIGN.md#dependency-rules).
//! It depends only on `std`, `glam`, `bytemuck`, and minimal numeric/util crates. It never
//! references wgpu, winit, cpal, glicol, gltf, or any other flowPlan crate.

#![forbid(unsafe_code)]

pub mod handle;
pub mod rng;
pub mod time;
pub mod world_topology;

pub use handle::Handle;
pub use rng::{splitmix64, Rng};
pub use time::{Seconds, TickCount};
pub use world_topology::{
    quantize_angular_frequency, quantize_wavelength, wrap_cell_index, wrap_coord,
    WORLD_EXTENT_M, WORLD_EXTENT_M_INV,
};
