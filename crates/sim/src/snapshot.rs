//! POD snapshot types for cross-layer reads.
//!
//! `SimSnapshot` is the render-facing immutable view of sim state, constructed at the end
//! of each tick. Render interpolates between two consecutive snapshots by alpha.
//!
//! Per [`DESIGN.md § Data Flow`](../../../DESIGN.md#data-flow), snapshots are `repr(C)` POD
//! where practical so GPU uniform uploads can cast without copying.

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};

use crate::camera::{Camera, ViewKind};
use crate::state::SimState;
use crate::vehicle::VehicleState;

/// Vehicle snapshot: the fields render + audio care about.
///
/// `#[repr(C)]` and explicit pad scalars keep the layout consistent for GPU uniform
/// uploads. Pod/Zeroable come from bytemuck derives so we remain `forbid(unsafe_code)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct VehicleSnapshot {
    pub position: Vec3,
    pad0: f32,
    pub orientation: Quat,
    pub velocity: Vec3,
    pad1: f32,
}

impl VehicleSnapshot {
    #[must_use]
    pub fn from_state(v: &VehicleState) -> Self {
        Self {
            position: v.position,
            pad0: 0.0,
            orientation: v.orientation,
            velocity: v.velocity,
            pad1: 0.0,
        }
    }
}

/// Camera snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct CameraSnapshot {
    pub position: Vec3,
    pad0: f32,
    pub orientation: Quat,
    pub fov_y_radians: f32,
    pub view_kind: u32, // cast of ViewKind
    pad1: [f32; 2],
}

impl CameraSnapshot {
    #[must_use]
    pub fn from_state(c: &Camera) -> Self {
        let view_kind = match c.view_kind {
            ViewKind::FirstPerson => 0,
            ViewKind::ChaseThirdPerson => 1,
            ViewKind::ExteriorOrbit => 2,
        };
        Self {
            position: c.position,
            pad0: 0.0,
            orientation: c.orientation,
            fov_y_radians: c.fov_y_radians,
            view_kind,
            pad1: [0.0; 2],
        }
    }
}

/// Top-level simulation snapshot produced each tick for render + audio consumption.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimSnapshot {
    pub tick: u64,
    pub vehicle: VehicleSnapshot,
    pub camera: CameraSnapshot,
}

impl SimSnapshot {
    #[must_use]
    pub fn from_state(s: &SimState) -> Self {
        Self {
            tick: s.tick.0,
            vehicle: VehicleSnapshot::from_state(&s.vehicle),
            camera: CameraSnapshot::from_state(&s.camera),
        }
    }
}

impl Default for SimSnapshot {
    fn default() -> Self {
        Self {
            tick: 0,
            vehicle: VehicleSnapshot {
                position: Vec3::ZERO,
                pad0: 0.0,
                orientation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                pad1: 0.0,
            },
            camera: CameraSnapshot {
                position: Vec3::new(0.0, 5.0, 15.0),
                pad0: 0.0,
                orientation: Quat::IDENTITY,
                fov_y_radians: 60_f32.to_radians(),
                view_kind: 1,
                pad1: [0.0; 2],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SimState;

    #[test]
    fn snapshot_carries_tick() {
        let s = SimState::new(0xDEAD_BEEF);
        let snap = SimSnapshot::from_state(&s);
        assert_eq!(snap.tick, 0);
    }

    #[test]
    fn snapshot_vehicle_matches_state() {
        let s = SimState::new(0xFEED_FACE);
        let snap = SimSnapshot::from_state(&s);
        assert!((snap.vehicle.position - s.vehicle.position).length() < f32::EPSILON);
    }

    #[test]
    fn default_snapshot_has_identity_orientations() {
        let snap = SimSnapshot::default();
        assert_eq!(snap.vehicle.orientation, Quat::IDENTITY);
        assert_eq!(snap.camera.orientation, Quat::IDENTITY);
    }
}
