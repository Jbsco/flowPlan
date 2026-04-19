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
//!
//! # Current limitations (Phase 2.4.1 placeholder)
//!
//! This is a toy model. TODOs below enumerate every physical aspect that needs work
//! before the flight model is "correct" in the ROADMAP § 2.4 sense. Each TODO names the
//! milestone that should close it. A careful reader should be able to cross-reference
//! any dynamics concern against this list.
//!
//! TODO(2.4.2): Lift should come from `0.5 ρ v² S Cl(α)`, not `horizontal_airspeed * k`.
//!   The horizontal-airspeed shortcut produces false-positive stalls in a dive (no
//!   horizontal component -> no lift -> "stall") even though true angle of attack
//!   may be fine. Also collapses into zero lift when flying backward, which is
//!   technically correct but for the wrong reason (Cl(α) at α > 90° is just the
//!   post-stall region of the table).
//! TODO(2.4.2): Angle of attack is not computed or exposed. Correct `α` is the angle
//!   between the chord line (body-frame approximation: -Z) and the relative wind
//!   (negated body-frame velocity projected onto the lift plane).
//! TODO(2.4.2): Sideslip angle (β) is not computed. Affects Cl / Cd coupling,
//!   rudder authority, and asymmetric wing loading.
//! TODO(2.4.3): Thrust must come from the R-985 power curve `T = P(RPM, MAP) / V`
//!   rather than a constant acceleration. Fuel burn follows.
//! TODO(2.4.3): Propeller model (fixed-pitch CS for 2.4, feathering post-MVP). Affects
//!   thrust at low airspeed (blade stall at high `AoA`) and drag of a feathered prop.
//! TODO(2.4.4): Real stall: Cl(α) falls past `α_stall`, typically ~16-18° for NACA
//!   64A416 section. Lift-center shifts aft of CP on stall, inducing a nose-down
//!   pitching moment -- the wing effectively tries to un-stall itself. Current
//!   placeholder has no such moment.
//! TODO(2.4.4): Accelerated / G-loaded stall: same `Cl_max` but at a higher speed
//!   when pulling positive G in a turn. `Vs_accelerated = Vs × sqrt(load_factor)`.
//! TODO(2.4.4): Wing-drop on stall. Asymmetric stall (one wing stalls first) rolls
//!   the aircraft -- direction keyed to current roll, slip, and P-factor. Recovery
//!   is pilot-input dependent; spins can develop from mis-handled stalls.
//! TODO(2.4.5): Flaps: 4 slotted positions matching POH lift-boost and drag-penalty.
//!   Flaps down lowers Vs (via higher `Cl_max`) and raises Cd; both must track the
//!   deployed notch.
//! TODO(2.4.6): Ground effect -- within ~1 wingspan of the surface, induced drag
//!   drops ~30%, giving a cushion on landing and helping heavy takeoffs.
//! TODO(2.4.6): P-factor -- right-rotating prop's descending blade is on the right,
//!   yawing the nose left under power + high α. Right rudder pressure required on
//!   takeoff and at low airspeed / high power.
//! TODO(2.4.6): Adverse yaw -- aileron deflection creates asymmetric induced drag,
//!   yawing the nose opposite to the roll direction. Rudder coordination fixes it.
//! TODO(2.4.6): Torque roll -- engine torque rolls the airframe opposite to prop
//!   rotation. Small for a single-engine Beaver but visible on power changes.
//! TODO(2.4.7): Density altitude -- lift and thrust fall with thinner air. `ρ` varies
//!   with temperature and pressure altitude; feeds both lift and engine power curves.
//! TODO(post-2.4): Spin dynamics (autorotation after a stall). Recovery procedure is
//!   PARE: Power idle, Ailerons neutral, Rudder opposite, Elevator forward through
//!   stall.
//! TODO(post-2.4): Gyroscopic precession -- prop acts as a gyroscope; pitching input
//!   produces a yaw response 90° later in the rotation direction. Small but real.
//! TODO(post-2.4): Slipstream / prop wash -- high-power, low-airspeed flight sees
//!   accelerated air over the rudder and elevator, increasing their authority well
//!   beyond what freestream airspeed suggests.
//! TODO(post-2.4): Cross-controls / slip / skid. Intentional slip for short-field
//!   landings, unintentional skid in uncoordinated turns (a classic stall-spin
//!   setup).
//! TODO(post-2.4): Icing effect on Cl -- rime on the leading edge raises Vs and
//!   degrades max-lift. Coupled to weather state.

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
    // TODO(2.4.4): angular response should depend on airspeed (slow-flight has weak
    //   control authority; Vne-plus has heavy authority, approaching Vne/Vd limits).
    //   Current fixed-rate tracking is a gross simplification.
    // TODO(2.4.4): post-stall, elevator authority is normal but ailerons lose authority
    //   (differentially: the retreating-inside-roll blade can be deep in stall). Rudder
    //   retains authority (hence PARE recovery).
    // TODO(2.4.6): aileron input should couple into a yaw contribution (adverse yaw).
    // TODO(2.4.6): throttle + nose-high attitude should couple into a yaw contribution
    //   (P-factor, right-rudder-required on takeoff).
    let commanded = Vec3::new(
        state.controls.pitch * PITCH_RATE_MAX,
        -state.controls.yaw * YAW_RATE_MAX,
        -state.controls.roll * ROLL_RATE_MAX,
    );

    // Thrust along body forward.
    // TODO(2.4.3): replace constant `THRUST_ACCEL_MAX` with
    //   `AircraftSpec.engine.power_hp` curve: thrust = P(rpm, map, alt) / airspeed,
    //   clamped at static thrust when airspeed is low.
    let forward = state.orientation * Vec3::NEG_Z;
    let thrust = forward * (state.controls.throttle * THRUST_ACCEL_MAX);

    // Placeholder wing lift: proportional to horizontal forward airspeed, along world +Y.
    // TODO(2.4.2): replace with airfoil Cl(α) lookup -- `L = 0.5 ρ V² S Cl(α)`.
    //   Using horizontal-airspeed here is why the HUD's stall alert false-positives in
    //   a dive: vertical velocity doesn't count as "airspeed," so lift reads zero and
    //   airspeed < Vs looks like a stall. Real α-based lift would note that the wing
    //   still has clean freestream over it.
    // TODO(2.4.2): compute angle of attack α = asin((forward x velocity) · right) or
    //   equivalent chord-to-relative-wind angle in body frame.
    let horiz_vel = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
    let horiz_fwd = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let horiz_airspeed = horiz_vel.dot(horiz_fwd).max(0.0);
    let lift = Vec3::Y * (horiz_airspeed * LIFT_COEFF);

    // Drag: linear + quadratic on speed, opposing velocity. Zero below 1e-3 m/s to
    // avoid a normalize of a vanishing vector.
    // TODO(2.4.2): drag polar: `Cd(α) = Cd0 + Cl² / (π AR e)`. Induced drag dominates
    //   at high α, parasite drag at high speed. Quadratic-on-speed approximates the
    //   parasite term only.
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
