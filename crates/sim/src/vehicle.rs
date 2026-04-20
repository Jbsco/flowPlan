//! Vehicle state.
//!
//! Phase 1: simple rigid body ("the cube") with linear and angular velocity. Thrust is
//! applied along the vehicle's local forward axis. Torques come from control inputs.
//! Phase 2 replaces this with per-aircraft aerodynamics from the DHC-2 Beaver spec sheet.

use glam::{Quat, Vec3};

use crate::controls::Controls;

/// Minimal rigid-body state for the Phase 1 placeholder vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleState {
    /// World-space position.
    pub position: Vec3,
    /// World-space linear velocity.
    pub velocity: Vec3,
    /// Orientation as a unit quaternion.
    pub orientation: Quat,
    /// Angular velocity in body frame, radians per second.
    pub angular_velocity: Vec3,
    /// Currently commanded control-surface positions.
    pub controls: Controls,
    /// Fuel quantity (US gallons). Set from `AircraftSpec.fuel_capacity_gal` at startup
    /// by the app; decremented each tick by the engine model (`flight.rs`). Zero fuel
    /// cuts engine power to zero regardless of throttle position.
    pub fuel_gal: f32,
}

impl VehicleState {
    /// A fresh vehicle sitting above the origin with controls neutral.
    ///
    /// Fuel starts at zero; the app is expected to refill from the loaded
    /// `AircraftSpec.fuel_capacity_gal` after wiring the spec. Pure-sim tests that
    /// don't load a spec use the no-spec thrust fallback, which bypasses fuel.
    #[must_use]
    pub fn at_origin() -> Self {
        Self {
            position: Vec3::new(0.0, 20.0, 0.0),
            velocity: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            controls: Controls::default(),
            fuel_gal: 0.0,
        }
    }

    /// Local forward vector (where the vehicle is pointing).
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        self.orientation * Vec3::NEG_Z
    }

    /// Local right vector.
    #[must_use]
    pub fn right(&self) -> Vec3 {
        self.orientation * Vec3::X
    }

    /// Local up vector.
    #[must_use]
    pub fn up(&self) -> Vec3 {
        self.orientation * Vec3::Y
    }
}

impl Default for VehicleState {
    fn default() -> Self {
        Self::at_origin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_origin_has_neutral_controls() {
        let v = VehicleState::at_origin();
        assert!((v.controls.throttle - 0.0).abs() < f32::EPSILON);
        assert_eq!(v.velocity, Vec3::ZERO);
    }

    #[test]
    fn identity_orientation_faces_minus_z() {
        let v = VehicleState::at_origin();
        let f = v.forward();
        assert!((f - Vec3::NEG_Z).length() < 1e-6);
        assert!((v.right() - Vec3::X).length() < 1e-6);
        assert!((v.up() - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn rotated_forward_changes() {
        let mut v = VehicleState::at_origin();
        v.orientation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        // After yawing 90 deg, forward should be -X (for a +Y up, -Z forward convention).
        let f = v.forward();
        assert!((f - Vec3::NEG_X).length() < 1e-5);
    }
}
