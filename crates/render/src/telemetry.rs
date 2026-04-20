//! Derived per-frame telemetry for every display surface.
//!
//! See [`DESIGN.md § Telemetry derivation (render side)`](../../../DESIGN.md#telemetry-derivation-render-side)
//! and [`FEATURES.md § Telemetry & Instrumentation Data Flow`](../../../FEATURES.md#telemetry--instrumentation-data-flow)
//! for the architectural boundary and consumer roadmap.
//!
//! `Telemetry` is a pure function of `SimSnapshot` (which already carries the
//! tick-boundary `FlightOutputs` since Phase 2.4.2c). It is computed **once per render
//! frame** and read by every display surface in parallel -- debug HUD, debug gizmo,
//! Phase 4 hand-rolled HUD, Phase 4+ cockpit instruments, autopilot readout, flight log.
//! No consumer should reach into `SimState` or recompute any of these quantities.
//!
//! Invariants (see DESIGN.md):
//! - Pure derivation: no hidden state, no per-frame accumulators.
//! - Render-rate, not sim-rate.
//! - SI units internally; display-unit conversions happen at the consumer format edge.
//! - One struct, many consumers: never duplicate the derivation.
//! - Determinism-safe: presentation-only smoothing is OK here; never in sim.

#![deny(unsafe_code)]

use flowplan_sim::{ControlsSnapshot, FlapPosition, MomentBreakdown, SimSnapshot, StallState};
use glam::{EulerRot, Quat, Vec3};

/// Derived per-frame telemetry. Every display surface reads from this.
///
/// Phase 2.4.2c baseline fields. Expands in later phases as instruments need more
/// readings (G-load, wind, engine cluster, etc.). The field list in [FEATURES.md §
/// Telemetry & Instrumentation Data Flow](../../../FEATURES.md#telemetry--instrumentation-data-flow)
/// tracks the phased expansion.
#[derive(Clone, Copy, Debug, Default)]
pub struct Telemetry {
    // --- Pose ---
    /// World-space position of the vehicle CG (m).
    pub position: Vec3,
    /// World-space orientation of the vehicle.
    pub orientation: Quat,

    // --- Linear motion ---
    /// World-space velocity (m/s).
    pub velocity: Vec3,
    /// Total airspeed magnitude |v| (m/s).
    pub airspeed_total: f32,
    /// Horizontal forward airspeed component (m/s). Feeds the current airspeed-based
    /// stall heuristic; replaced by α-based detection in 2.4.4.
    pub airspeed_horizontal_fwd: f32,
    /// Vertical component of velocity (m/s, +up). Identical to `velocity.y`; duplicated
    /// for call-site clarity and so a VSI reading has a canonical name.
    pub airspeed_vertical: f32,

    // --- Attitude (radians) ---
    /// Heading / yaw from +X axis (approx; derived from quat YXZ decomposition).
    pub heading_rad: f32,
    /// Pitch angle (nose-up positive).
    pub pitch_rad: f32,
    /// Bank / roll angle (right-wing-down positive).
    pub bank_rad: f32,

    // --- Aerodynamics ---
    /// Angle of attack (rad). Positive when nose is above relative wind.
    pub alpha_rad: f32,
    /// Sideslip angle (rad). Positive when relative wind comes from the right.
    pub beta_rad: f32,
    /// Lift coefficient (dimensionless).
    pub cl: f32,
    /// Drag coefficient (dimensionless).
    pub cd: f32,

    // --- Forces (world-frame accelerations, m/s²) ---
    pub thrust: Vec3,
    pub lift: Vec3,
    pub drag: Vec3,
    pub gravity: Vec3,
    /// Sum of the above: `thrust + lift + drag + gravity`. Useful for the "net force"
    /// gizmo arrow and for G-load derivation in 2.4.4.
    pub linear_accel: Vec3,

    // --- Angular (body-frame) ---
    /// Actual angular velocity (rad/s).
    pub actual_angular_vel: Vec3,
    /// Angular acceleration this tick (rad/s²), derived from aerodynamic moments.
    pub angular_accel: Vec3,
    /// Per-axis aerodynamic moment breakdowns (N·m). Physics convention:
    /// pitch > 0 nose up, roll > 0 right-wing-down, yaw > 0 nose right.
    pub pitch_moment: MomentBreakdown,
    pub roll_moment: MomentBreakdown,
    pub yaw_moment: MomentBreakdown,

    // --- Altitude / VSI / flight path ---
    /// Mean-sea-level altitude in meters. AGL arrives when terrain-sample is plumbed
    /// through `SimSnapshot`.
    pub altitude_msl: f32,
    /// Vertical speed in m/s. Alias of `airspeed_vertical` for instrument-reading
    /// clarity.
    pub vertical_speed_mps: f32,
    /// Flight path angle (γ) in radians. Positive when climbing; negative when
    /// descending. `atan2(vertical, horizontal)` of the velocity vector.
    pub flight_path_angle_rad: f32,
    /// Lift-to-drag ratio at the current instant, approximated as
    /// `horizontal_airspeed / |vertical_speed|` when descending. `f32::INFINITY`
    /// when the aircraft is level or climbing (not in a glide). The attitude-indicator
    /// widget shows this as "--" in that case.
    pub glide_ratio: f32,

    // --- Controls (input state) ---
    /// Throttle position in `[0, 1]`.
    pub throttle: f32,
    /// Pitch stick position in `[-1, 1]`. Positive = nose up.
    pub pitch_input: f32,
    /// Yaw pedal / rudder position in `[-1, 1]`. Positive = nose right.
    pub yaw_input: f32,
    /// Roll stick position in `[-1, 1]`. Positive = roll right.
    pub roll_input: f32,

    // --- Engine + fuel (Phase 2.4.3) ---
    /// Shaft power this tick, kilowatts. Zero when fuel is exhausted.
    pub engine_power_kw: f32,
    /// Propeller efficiency this tick (0..~0.85).
    pub prop_efficiency: f32,
    /// Fuel flow (gallons per hour) at the current engine state.
    pub fuel_flow_gph: f32,
    /// Remaining fuel (US gallons).
    pub fuel_gal: f32,
    /// Estimated endurance in seconds at the current fuel flow. `f32::INFINITY` when
    /// burn is zero (idle + out-of-fuel case).
    pub endurance_seconds: f32,

    // --- Stall state (Phase 2.4.4d) ---
    /// α-based stall classification. Drives the HUD stall alert and (future) audio
    /// warning. Replaces the pre-2.4.4d airspeed/Vs heuristic that false-positived
    /// in dives and missed accelerated stalls.
    pub stall_state: StallState,

    // --- Flaps (Phase 2.4.5) ---
    /// Current flap detent.
    pub flap_position: FlapPosition,

    // --- Density altitude (Phase 2.4.7) ---
    /// Air density at the aircraft's current altitude (kg/m³). Exponential ISA
    /// approximation; 1.225 at sea level, ~0.69 at 5 km.
    pub air_density_kg_m3: f32,
    /// Ratio `air_density / ρ_0`. Drives both aero-force scaling and engine-power
    /// derate. HUD shows this as a percentage for a quick sense of "thin air".
    pub density_ratio: f32,

    /// Sim tick this telemetry was derived from. `u64::MAX` reserved for defaults.
    pub tick: u64,
}

impl Telemetry {
    /// Derive telemetry from a single sim snapshot. Pure function.
    #[must_use]
    pub fn derive(snapshot: &SimSnapshot) -> Self {
        let veh = &snapshot.vehicle;
        let f = &snapshot.flight;
        // glam's `to_euler(YXZ)` returns angles in the enum axis order: (y, x, z).
        let (yaw, pitch, roll) = veh.orientation.to_euler(EulerRot::YXZ);
        Self {
            position: veh.position,
            orientation: veh.orientation,

            velocity: veh.velocity,
            airspeed_total: veh.velocity.length(),
            airspeed_horizontal_fwd: f.horizontal_airspeed,
            airspeed_vertical: veh.velocity.y,

            heading_rad: yaw,
            pitch_rad: pitch,
            bank_rad: roll,

            alpha_rad: f.alpha,
            beta_rad: f.beta,
            cl: f.cl,
            cd: f.cd,

            thrust: f.thrust,
            lift: f.lift,
            drag: f.drag,
            gravity: f.gravity,
            linear_accel: f.linear_accel,

            actual_angular_vel: veh.angular_velocity,
            angular_accel: f.angular_accel,
            pitch_moment: f.pitch_moment,
            roll_moment: f.roll_moment,
            yaw_moment: f.yaw_moment,

            altitude_msl: veh.position.y,
            vertical_speed_mps: veh.velocity.y,
            flight_path_angle_rad: {
                let horiz = veh.velocity.x.hypot(veh.velocity.z);
                if horiz < 1e-3 && veh.velocity.y.abs() < 1e-3 {
                    0.0
                } else {
                    veh.velocity.y.atan2(horiz)
                }
            },
            glide_ratio: {
                let horiz = veh.velocity.x.hypot(veh.velocity.z);
                // Only meaningful when descending; infinity indicates level/climbing.
                if veh.velocity.y < -0.1 {
                    horiz / -veh.velocity.y
                } else {
                    f32::INFINITY
                }
            },

            throttle: veh.controls.throttle,
            pitch_input: veh.controls.pitch,
            yaw_input: veh.controls.yaw,
            roll_input: veh.controls.roll,

            engine_power_kw: f.engine_power_kw,
            prop_efficiency: f.prop_efficiency,
            fuel_flow_gph: f.fuel_flow_gph,
            fuel_gal: veh.fuel_gal,
            endurance_seconds: if f.fuel_flow_gph > 1e-3 {
                veh.fuel_gal / f.fuel_flow_gph * 3600.0
            } else {
                f32::INFINITY
            },

            stall_state: f.stall_state,

            flap_position: ControlsSnapshot::flap_position_from_u32(veh.controls.flap_position),

            air_density_kg_m3: flowplan_sim::flight::air_density(veh.position.y),
            density_ratio: flowplan_sim::flight::density_ratio(veh.position.y),

            tick: snapshot.tick,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowplan_sim::{SimState, VehicleState};

    #[test]
    fn derive_of_default_is_default_shape() {
        let snap = SimSnapshot::default();
        let t = Telemetry::derive(&snap);
        assert_eq!(t.tick, 0);
        assert!((t.airspeed_total - 0.0).abs() < f32::EPSILON);
        assert_eq!(t.position, Vec3::ZERO);
    }

    #[test]
    fn derive_populates_velocity_and_airspeed() {
        let mut s = SimState::new(1);
        s.vehicle = VehicleState::at_origin();
        s.vehicle.velocity = Vec3::new(3.0, 0.0, -4.0); // |v| = 5
        let snap = SimSnapshot::from_state(&s);
        let t = Telemetry::derive(&snap);
        assert!((t.airspeed_total - 5.0).abs() < 1e-5);
        assert!((t.velocity - Vec3::new(3.0, 0.0, -4.0)).length() < 1e-5);
    }

    #[test]
    fn derive_carries_controls() {
        let mut s = SimState::new(1);
        s.vehicle.controls.throttle = 0.7;
        s.vehicle.controls.pitch = -0.25;
        let snap = SimSnapshot::from_state(&s);
        let t = Telemetry::derive(&snap);
        assert!((t.throttle - 0.7).abs() < f32::EPSILON);
        assert!((t.pitch_input - -0.25).abs() < f32::EPSILON);
    }
}
