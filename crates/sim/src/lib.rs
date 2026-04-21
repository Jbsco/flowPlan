//! flowPlan simulation: fixed-timestep aircraft state and tick.
//!
//! Phase 1 scope: a placeholder vehicle ("the cube") with simple rigid-body dynamics,
//! an input queue drained at tick start, a camera with first-person / third-person /
//! orbit views, and a deterministic `tick()` that advances state by a fixed `dt`.
//! The Beaver-specific flight model replaces this in Phase 2.
//!
//! See [`DESIGN.md § Simulation Architecture`](../../../DESIGN.md#simulation-architecture).

#![forbid(unsafe_code)]

pub mod aircraft_spec;
pub mod buoyancy;
pub mod camera;
pub mod control;
pub mod controls;
pub mod difficulty;
pub mod flight;
pub mod flight_bench;
pub mod hash;
pub mod input;
pub mod snapshot;
pub mod state;
pub mod vehicle;

pub use aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
pub use camera::{Camera, ChaseConfig, ViewKind};
pub use control::{ControlLaw, ControlLawState};
pub use controls::{Controls, FlapPosition};
pub use difficulty::DifficultyDials;
pub use flight::{FlightOutputs, MomentBreakdown, StallState};
pub use hash::state_hash;
pub use input::{InputEvent, InputEvents};
pub use snapshot::{ControlsSnapshot, SimSnapshot, VehicleSnapshot};
pub use state::SimState;
pub use vehicle::VehicleState;
