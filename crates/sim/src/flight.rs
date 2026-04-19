//! Flight model: aerodynamic forces and commanded body rates.
//!
//! Phase 2.4.1 sub-step: architectural skeleton. Captures the Phase 1 placeholder
//! dynamics (constant-thrust + horizontal-lift + linear+quadratic drag) as a standalone
//! pure function. Later sub-steps (2.4.2+) replace the hard-coded constants with
//! airfoil Cl(α) / drag polar tables and engine-specific thrust curves keyed to the
//! provided `AircraftSpec`.
//!
//! The function is pure: it takes the current vehicle state (read-only), an optional
//! aircraft spec, and dt, and returns the forces / commanded rates to apply this tick.
//! Integration and ground-clamp stay in `state::tick_dynamics` so the sim boundary is
//! simple: forces in, state-delta out.
//!
//! The function does not allocate and is safe to call per tick.

use glam::Vec3;

use crate::aircraft_spec::AircraftSpec;
use crate::vehicle::VehicleState;

/// Gravitational acceleration, world-space m/s².
pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

// Phase 1 placeholder constants. Replaced by `AircraftSpec`-driven values in 2.4.2+.

/// Thrust acceleration at full throttle along body forward, m/s².
const THRUST_ACCEL_MAX: f32 = 32.0;

/// Peak commanded angular rate per body axis (rad/s).
const ROLL_RATE_MAX: f32 = 1.6;
const PITCH_RATE_MAX: f32 = 1.0;
const YAW_RATE_MAX: f32 = 0.6;

/// First-order tracking rate toward commanded angular velocity (1/s).
const ANG_RESPONSE: f32 = 10.0;

/// Linear drag coefficient (1/s), low-speed stability.
const LIN_DAMPING: f32 = 0.05;

/// Quadratic drag coefficient (1/m), bounds terminal velocity.
const QUAD_DRAG: f32 = 0.014;

/// Placeholder wing-lift coefficient: m/s² of upward acceleration per m/s of
/// *horizontal* forward airspeed. Projection onto the world XZ plane keeps dives
/// accelerating because lift falls with horizontal airspeed rather than total airspeed.
/// Airfoil-based lift replaces this in 2.4.2.
const LIFT_COEFF: f32 = 0.344;

/// Result of one flight-model evaluation. The first three fields drive integration;
/// the remaining fields are diagnostics for the debug HUD.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlightOutputs {
    /// World-space linear acceleration applied to the velocity this tick.
    pub linear_accel: Vec3,
    /// Body-frame commanded angular velocity (rad/s) that integration tracks toward.
    pub commanded_angular_vel: Vec3,
    /// Smoothing rate used to blend toward `commanded_angular_vel` (1/s).
    pub ang_response_rate: f32,

    // Diagnostic breakdown of the force terms that compose `linear_accel`. All in m/s².
    pub thrust: Vec3,
    pub lift: Vec3,
    pub drag: Vec3,
    pub gravity: Vec3,
    /// Total airspeed magnitude (m/s).
    pub airspeed: f32,
    /// Horizontal component of forward airspeed (m/s); feeds lift in the placeholder
    /// model.
    pub horizontal_airspeed: f32,
}

/// Compute flight forces for the current vehicle state.
///
/// `spec` carries aircraft-specific data (mass, airfoil, engine). Phase 2.4.1 ignores
/// it; later sub-steps read lift / drag / thrust tables from it.
#[must_use]
pub fn compute_forces(
    state: &VehicleState,
    _spec: Option<&AircraftSpec>,
    _dt: f32,
) -> FlightOutputs {
    // Commanded angular rates per body axis.
    // Sign conventions (right-handed, Y-up, -Z forward):
    //   pitch positive = nose up      -> rotate +X (right)
    //   yaw positive   = nose right   -> rotate -Y (positive rot around +Y yaws LEFT)
    //   roll positive  = roll right   -> rotate -Z (positive rot around +Z rolls LEFT)
    let commanded = Vec3::new(
        state.controls.pitch * PITCH_RATE_MAX,
        -state.controls.yaw * YAW_RATE_MAX,
        -state.controls.roll * ROLL_RATE_MAX,
    );

    // Thrust along body forward.
    let forward = state.orientation * Vec3::NEG_Z;
    let thrust = forward * (state.controls.throttle * THRUST_ACCEL_MAX);

    // Placeholder wing lift: proportional to horizontal forward airspeed, along world +Y.
    let horiz_vel = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
    let horiz_fwd = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let horiz_airspeed = horiz_vel.dot(horiz_fwd).max(0.0);
    let lift = Vec3::Y * (horiz_airspeed * LIFT_COEFF);

    // Drag: linear + quadratic on speed, opposing velocity. Zero below 1e-3 m/s to
    // avoid a normalize of a vanishing vector.
    let speed = state.velocity.length();
    let drag = if speed > 1e-3 {
        -state.velocity.normalize_or_zero() * (speed * (LIN_DAMPING + QUAD_DRAG * speed))
    } else {
        Vec3::ZERO
    };

    FlightOutputs {
        linear_accel: thrust + lift + drag + GRAVITY,
        commanded_angular_vel: commanded,
        ang_response_rate: ANG_RESPONSE,
        thrust,
        lift,
        drag,
        gravity: GRAVITY,
        airspeed: speed,
        horizontal_airspeed: horiz_airspeed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn zero_state() -> VehicleState {
        VehicleState {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            controls: crate::controls::Controls::default(),
        }
    }

    #[test]
    fn at_rest_idle_only_gravity() {
        let out = compute_forces(&zero_state(), None, 1.0 / 120.0);
        assert!((out.linear_accel - GRAVITY).length() < 1e-5);
        assert!(out.commanded_angular_vel.length() < 1e-6);
    }

    #[test]
    fn full_throttle_pushes_forward() {
        let mut s = zero_state();
        s.controls.throttle = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0);
        // Forward is -Z at identity orientation, so horizontal component is negative Z.
        assert!(out.linear_accel.z < -1.0);
    }

    #[test]
    fn lift_rises_with_horizontal_airspeed() {
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -20.0); // forward at identity orientation
        let out = compute_forces(&s, None, 1.0 / 120.0);
        // Lift contribution in +Y exceeds gravity at this airspeed (20 * 0.344 = 6.88 > 9.81? no),
        // but should be meaningfully positive.
        assert!(
            out.linear_accel.y > GRAVITY.y + 3.0,
            "y accel {}",
            out.linear_accel.y
        );
    }

    #[test]
    fn commanded_rate_follows_stick() {
        let mut s = zero_state();
        s.controls.pitch = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0);
        assert!((out.commanded_angular_vel.x - PITCH_RATE_MAX).abs() < 1e-5);
    }
}
