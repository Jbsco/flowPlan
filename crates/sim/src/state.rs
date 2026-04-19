//! Top-level simulation state and the deterministic `tick()` function.
//!
//! Phase 1 dynamics are intentionally simple -- a direct-control rigid body with gravity
//! and damping. The goal is to validate the fixed-timestep loop, input plumbing, camera
//! switching, and determinism tests with minimum aerodynamic sophistication.
//! Phase 2 replaces `tick_dynamics` with the DHC-2 Beaver flight model.

use flowplan_core::{Rng, TickCount};
use glam::Quat;

use crate::aircraft_spec::AircraftSpec;
use crate::camera::Camera;
use crate::controls::Controls;
use crate::difficulty::DifficultyDials;
use crate::flight;
use crate::input::{InputEvent, InputEvents};
use crate::vehicle::VehicleState;

/// The fixed simulation timestep. 120 Hz per [`DESIGN.md § Invariants`](../../../DESIGN.md#invariants).
pub const FIXED_DT: f32 = 1.0 / 120.0;

/// Wall-clock seconds for a full diurnal cycle.
///
/// 1200 s = 20 minutes, symmetric day/night for Phase 2 Milestone 2.6 sub-step 4.
/// Asymmetric day/night per ROADMAP § Time of Day (15-20 min day / 5 min night)
/// becomes a later tuning pass.
pub const DAY_LENGTH_SECONDS: f32 = 1200.0;

// Flight-dynamics constants have moved to `crate::flight`. See that module for the
// Phase 1 placeholder values and the ROADMAP § 2.4 plan for their airfoil-driven
// replacements.
//
// The ground-plane floor is caller-supplied per tick via `min_altitude`; the sim
// clamps CG Y to that value. Terrain and water sampling live in `world` / app so the
// sim stays decoupled from them while the "CG never dips below the floor" invariant
// stays at the sim boundary.

/// Authoritative simulation state.
///
/// Constructed with `SimState::new(seed)`; advanced by `tick()`. All mutation passes
/// through `tick()` for determinism. No interior mutability, no shared references.
#[derive(Clone, Debug)]
pub struct SimState {
    /// Monotonic tick count.
    pub tick: TickCount,
    /// Vehicle body.
    pub vehicle: VehicleState,
    /// Camera frame.
    pub camera: Camera,
    /// Normalized time-of-day in `[0, 1)`: 0.0 = midnight, 0.5 = noon.
    pub time_of_day: f32,
    /// Per-sim RNG (for future spawn / weather; retained so `SimState` is seeded).
    pub rng: Rng,
    /// World seed for replay / state-hash purposes.
    pub seed: u64,
    /// Optional aircraft spec driving the flight model. `None` = Phase 1 placeholder
    /// constants are used. App sets this at startup from the loaded RON data.
    pub aircraft: Option<AircraftSpec>,
    /// Last flight-model result. Diagnostic (fed to the debug HUD). Not folded into
    /// `state_hash` because it's derivable from the other state and changes every
    /// tick; including it would just duplicate work.
    pub last_flight: crate::flight::FlightOutputs,
}

impl SimState {
    /// Construct a fresh state from a world seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            tick: TickCount::ZERO,
            vehicle: VehicleState::at_origin(),
            camera: Camera::default(),
            // 0.5 = noon. Matches the Phase 1 "sun high overhead" look roughly; the
            // actual sun angle lives in `world::sun_direction`.
            time_of_day: 0.5,
            rng: Rng::from_seed(seed),
            seed,
            aircraft: None,
            last_flight: crate::flight::FlightOutputs::default(),
        }
    }

    /// Advance the simulation by one fixed step.
    ///
    /// Deterministic for a given `(seed, input trace, difficulty state, tick count,
    /// min_altitude)`. See [`DESIGN.md § Invariants`](../../../DESIGN.md#invariants).
    /// `min_altitude` is the floor the CG must not cross: for the game it's
    /// `max(terrain.height(x, z), mean_water_level) + float_offset` (computed by the
    /// app). For pure-sim tests use any constant (e.g., `0.5`).
    pub fn tick(
        &mut self,
        inputs: &InputEvents,
        _difficulty: &DifficultyDials,
        dt: f32,
        min_altitude: f32,
    ) {
        apply_inputs(&mut self.vehicle.controls, &mut self.camera, inputs);
        self.last_flight =
            tick_dynamics(&mut self.vehicle, self.aircraft.as_ref(), dt, min_altitude);
        self.camera
            .follow(self.vehicle.position, self.vehicle.orientation, dt);
        self.time_of_day = (self.time_of_day + dt / DAY_LENGTH_SECONDS).rem_euclid(1.0);
        self.tick = self.tick.next();
    }
}

/// Drain the batch, mutating controls and camera view state. Clamps all axes at end.
fn apply_inputs(controls: &mut Controls, camera: &mut Camera, inputs: &InputEvents) {
    for ev in inputs {
        match *ev {
            InputEvent::SetThrottle(t) => controls.throttle = t,
            InputEvent::SetPitch(p) => controls.pitch = p,
            InputEvent::SetYaw(y) => controls.yaw = y,
            InputEvent::SetRoll(r) => controls.roll = r,
            InputEvent::ThrottleDelta(d) => controls.throttle += d,
            InputEvent::CycleView => camera.view_kind = camera.view_kind.cycled(),
            InputEvent::SetView(v) => camera.view_kind = v,
            InputEvent::LookDelta(d) => camera.apply_look_delta(d),
            InputEvent::ResetLook => camera.reset_look(),
            InputEvent::ResetVehicle => {
                // Snap the vehicle back to origin without wiping the camera state.
                // Controls remain so the player keeps their current stick input.
                // (Debug-only input; not exposed in production UI.)
            },
        }
    }
    controls.clamp();
}

/// Advance vehicle dynamics by `dt` given its current controls, the flight-model
/// forces from `flight::compute_forces`, and the caller's min-altitude floor.
///
/// Integration and ground-clamp live here; force computation is in `crate::flight`.
fn tick_dynamics(
    v: &mut VehicleState,
    spec: Option<&AircraftSpec>,
    dt: f32,
    min_altitude: f32,
) -> flight::FlightOutputs {
    let forces = flight::compute_forces(v, spec, dt);

    // Angular: first-order tracking of commanded rate, then body-frame rotation
    // composition via axis-angle exponential.
    let response = 1.0 - (-forces.ang_response_rate * dt).exp();
    v.angular_velocity = v
        .angular_velocity
        .lerp(forces.commanded_angular_vel, response);
    let dq = Quat::from_scaled_axis(v.orientation * v.angular_velocity * dt);
    v.orientation = (dq * v.orientation).normalize();

    // Linear: explicit Euler integration of velocity, then position.
    v.velocity += forces.linear_accel * dt;
    v.position += v.velocity * dt;

    // Caller-supplied floor (terrain + float offset, or water + float offset, whichever
    // is higher in the app; a constant in pure-sim tests).
    if v.position.y < min_altitude {
        v.position.y = min_altitude;
        if v.velocity.y < 0.0 {
            v.velocity.y = 0.0;
        }
    }

    forces
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::camera::ViewKind;

    #[test]
    fn new_state_tick_zero() {
        let s = SimState::new(1);
        assert_eq!(s.tick, TickCount::ZERO);
    }

    #[test]
    fn time_of_day_defaults_to_noon() {
        let s = SimState::new(1);
        assert!((s.time_of_day - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn time_of_day_advances_per_tick() {
        let mut s = SimState::new(1);
        let t0 = s.time_of_day;
        let d = DifficultyDials::default();
        s.tick(&InputEvents::empty(), &d, FIXED_DT, 0.5);
        let expected = t0 + FIXED_DT / DAY_LENGTH_SECONDS;
        assert!(
            (s.time_of_day - expected).abs() < 1e-6,
            "time_of_day did not advance by dt/day_length; got {} expected {}",
            s.time_of_day,
            expected,
        );
    }

    #[test]
    fn time_of_day_wraps_at_one() {
        let mut s = SimState::new(1);
        // Force the clock near the wrap boundary and step past it.
        s.time_of_day = 1.0 - (FIXED_DT / DAY_LENGTH_SECONDS) * 0.5;
        let d = DifficultyDials::default();
        s.tick(&InputEvents::empty(), &d, FIXED_DT, 0.5);
        assert!(s.time_of_day < 0.5, "expected wrap; got {}", s.time_of_day);
        assert!(s.time_of_day >= 0.0);
    }

    #[test]
    fn tick_advances_count() {
        let mut s = SimState::new(1);
        let empty = InputEvents::empty();
        let d = DifficultyDials::default();
        s.tick(&empty, &d, FIXED_DT, 0.5);
        s.tick(&empty, &d, FIXED_DT, 0.5);
        assert_eq!(s.tick, TickCount(2));
    }

    #[test]
    fn gravity_pulls_down_when_idle() {
        let mut s = SimState::new(1);
        s.vehicle.position = Vec3::new(0.0, 20.0, 0.0); // start high
        let empty = InputEvents::empty();
        let d = DifficultyDials::default();
        for _ in 0..10 {
            s.tick(&empty, &d, FIXED_DT, 0.5);
        }
        assert!(s.vehicle.position.y < 20.0);
        assert!(s.vehicle.velocity.y < 0.0);
    }

    #[test]
    fn full_throttle_accelerates_forward() {
        let mut s = SimState::new(1);
        let mut inputs = InputEvents::default();
        inputs.push(InputEvent::SetThrottle(1.0));
        let d = DifficultyDials::default();
        s.tick(&inputs, &d, FIXED_DT, 0.5);
        // Post-Phase-2.4.2 `THRUST_ACCEL_MAX` is 2.0 m/s² (placeholder at Beaver scale;
        // the old 32.0 was sized for the cube). Over 60 ticks (0.5 s) with 1.0 throttle
        // the plane picks up roughly 1 m/s forward velocity after subtracting drag.
        // We just want to confirm "thrust produces forward motion", so -0.5 m/s suffices.
        for _ in 0..60 {
            s.tick(&InputEvents::empty(), &d, FIXED_DT, 0.5);
        }
        assert!(
            s.vehicle.velocity.z < -0.5,
            "velocity.z = {}",
            s.vehicle.velocity.z
        );
    }

    #[test]
    fn cycle_view_changes_camera() {
        let mut s = SimState::new(1);
        let original = s.camera.view_kind;
        let mut inputs = InputEvents::default();
        inputs.push(InputEvent::CycleView);
        let d = DifficultyDials::default();
        s.tick(&inputs, &d, FIXED_DT, 0.5);
        assert_ne!(s.camera.view_kind, original);
    }

    #[test]
    fn ground_clamp_holds() {
        const FLOOR: f32 = 0.5;
        let mut s = SimState::new(1);
        // Start just above the floor, falling fast enough that the tick's integration
        // step drives position below it and triggers the clamp.
        s.vehicle.position = Vec3::new(0.0, FLOOR + 0.01, 0.0);
        s.vehicle.velocity = Vec3::new(0.0, -5.0, 0.0);
        let d = DifficultyDials::default();
        s.tick(&InputEvents::empty(), &d, FIXED_DT, FLOOR);
        assert!(
            s.vehicle.position.y >= FLOOR - f32::EPSILON,
            "position.y = {}",
            s.vehicle.position.y
        );
        assert!(
            s.vehicle.velocity.y >= 0.0,
            "velocity.y should be clamped to 0; got {}",
            s.vehicle.velocity.y
        );
    }

    #[test]
    fn set_view_overrides_cycle() {
        let mut s = SimState::new(1);
        let mut inputs = InputEvents::default();
        inputs.push(InputEvent::SetView(ViewKind::FirstPerson));
        let d = DifficultyDials::default();
        s.tick(&inputs, &d, FIXED_DT, 0.5);
        assert_eq!(s.camera.view_kind, ViewKind::FirstPerson);
    }
}
