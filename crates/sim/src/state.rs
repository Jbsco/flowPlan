//! Top-level simulation state and the deterministic `tick()` function.
//!
//! Phase 1 dynamics are intentionally simple -- a direct-control rigid body with gravity
//! and damping. The goal is to validate the fixed-timestep loop, input plumbing, camera
//! switching, and determinism tests with minimum aerodynamic sophistication.
//! Phase 2 replaces `tick_dynamics` with the DHC-2 Beaver flight model.

use flowplan_core::{Rng, TickCount};
use glam::{Quat, Vec3};

use crate::aircraft_spec::AircraftSpec;
use crate::buoyancy;
use crate::camera::Camera;
use crate::difficulty::DifficultyDials;
use crate::flight;
use crate::ground_contact;
use crate::input::{InputEvent, InputEvents};
use crate::vehicle::VehicleState;

/// Per-tick weather inputs.
///
/// The sim's view of the airmass at the aircraft's current position.
/// Populated by the app from `world::weather` samplers; left at
/// `Default::default()` for headless / bench scenarios that run in still air.
///
/// Semantics:
/// - `wind` is the airmass velocity in world frame (m/s). The aircraft's
///   aero forces see `vehicle.velocity - wind` as relative airflow, so
///   tailwinds reduce airspeed, headwinds boost it, crosswinds produce
///   sideslip.
/// - `turbulence_intensity` is a non-dimensional scalar matching the
///   `world::weather::turbulence_at_with_weather` output (0 = calm,
///   1 = moderate, 3+ = severe). Drives stochastic angular-velocity jitter
///   proportional to the magnitude.
#[derive(Copy, Clone, Debug, Default)]
pub struct WeatherInputs {
    pub wind: Vec3,
    pub turbulence_intensity: f32,
}

/// Gain for converting `turbulence_intensity` into per-tick angular-velocity
/// jitter (rad/s per unit intensity per second, roughly). Tuned so moderate
/// turbulence (intensity ≈ 1.0) produces noticeable but controllable buffet
/// and severe (≥ 3) is aggressively destabilizing.
const TURBULENCE_ANGULAR_JITTER_GAIN: f32 = 0.15;

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
    /// Deterministic wave clock (seconds). Advances `dt` each tick, drives the
    /// `world::water_height` sampler sim-side so buoyancy is bit-identical across
    /// runs regardless of wall-clock. Render continues to use its own wall-clock
    /// `SceneUniform.time.x` for visual waves; both consume the same `WAVES`
    /// constants in `crates/world/src/water.rs` so the surface stays visually
    /// aligned with the physics. Folded into `state_hash` so determinism covers
    /// wave phase.
    pub wave_time: f32,
    /// Chop intensity scalar — multiplies wave amplitudes sampled by buoyancy.
    /// Default 1.0 (nominal chop per `WAVES` constants). Phase 3.2 weather
    /// wires this from `ChopState`: 0.0 = glassy water (no waves), 1.0 = current
    /// baseline, 2.0+ = heavy chop. Folded into `state_hash` so changing chop
    /// state is part of the deterministic trace.
    ///
    /// Visual-only: render shader samples its own wall-clock wave sum without
    /// this scalar. Phase 3.2 will wire `SceneParams.chop_intensity` to keep
    /// visual and physics chop aligned; until then, render shows nominal chop
    /// regardless of this value.
    pub chop_intensity: f32,
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
    /// Last ground-contact strike flags (Phase 2.5b.6). Per-kind booleans for
    /// prop / tail / wingtip contact. Re-evaluated each tick; zeroed for
    /// floats-configured aircraft. Exposed so bench CSV + debug HUD + future
    /// Phase 4 damage system can observe strikes without re-running the
    /// contact check. Not folded into `state_hash` (derivable from position
    /// + orientation).
    pub last_strikes: crate::ground_contact::StrikeFlags,
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
            // 0.0 is the canonical reference phase for Gerstner waves: at (0, 0, 0)
            // the sampled surface height is zero. Scenarios start from this clean
            // state so bench CSVs are reproducible across runs.
            wave_time: 0.0,
            // Nominal chop baseline. Phase 3.2 weather drives this from ChopState.
            chop_intensity: 1.0,
            rng: Rng::from_seed(seed),
            seed,
            aircraft: None,
            last_flight: crate::flight::FlightOutputs::default(),
            last_strikes: crate::ground_contact::StrikeFlags::default(),
        }
    }

    /// Advance the simulation by one fixed step under still-air conditions.
    ///
    /// Convenience wrapper around [`SimState::tick_with_weather`] that passes
    /// `WeatherInputs::default()` (no wind, no turbulence). Preserves the
    /// pre-3.2.3 signature so existing tests / scenarios that don't model
    /// weather continue to work unchanged.
    pub fn tick(
        &mut self,
        inputs: &InputEvents,
        difficulty: &DifficultyDials,
        dt: f32,
        min_altitude: f32,
    ) {
        self.tick_with_weather(
            inputs,
            difficulty,
            dt,
            min_altitude,
            WeatherInputs::default(),
        );
    }

    /// Advance the simulation by one fixed step with explicit weather inputs.
    ///
    /// Deterministic for a given `(seed, input trace, difficulty state, tick
    /// count, min_altitude, weather trace)`. See [`DESIGN.md § Invariants`](../../../DESIGN.md#invariants).
    /// Weather inputs affect aero (wind on relative airflow) and attitude
    /// (turbulence buffet); they do not affect the integration path directly
    /// (which remains world-frame).
    pub fn tick_with_weather(
        &mut self,
        inputs: &InputEvents,
        _difficulty: &DifficultyDials,
        dt: f32,
        min_altitude: f32,
        weather: WeatherInputs,
    ) {
        apply_inputs(&mut self.vehicle, &mut self.camera, inputs);
        let (flight_out, strikes) = tick_dynamics(
            &mut self.vehicle,
            self.aircraft.as_ref(),
            dt,
            min_altitude,
            self.wave_time,
            self.chop_intensity,
            weather,
            &mut self.rng,
        );
        self.last_flight = flight_out;
        self.last_strikes = strikes;
        self.camera
            .follow(self.vehicle.position, self.vehicle.orientation, dt);
        self.time_of_day = (self.time_of_day + dt / DAY_LENGTH_SECONDS).rem_euclid(1.0);
        // wave_time accumulates deterministically; no wrap for now. Precision-preserving
        // wrap lands when buoyancy scenarios run long enough for f32 drift to matter.
        self.wave_time += dt;
        self.tick = self.tick.next();
    }
}

/// Drain the batch, mutating vehicle and camera view state. Clamps controls at end.
///
/// `ResetVehicle` is the in-game recovery action (bound to `R`): restores a level
/// attitude, zeros linear + angular velocity, and bumps altitude up by `RESET_DROP_M`
/// so buoyancy or aerodynamics can settle the aircraft from a "slight height"
/// instead of resetting in place. Throttle, stick inputs, flap state, fuel, and
/// horizontal position are preserved. Lets the player recover from an
/// unrecoverable attitude (water-flip, mid-air upset, sunken floats) without
/// losing their place in the world.
fn apply_inputs(v: &mut VehicleState, camera: &mut Camera, inputs: &InputEvents) {
    /// Altitude bump applied on reset. Sized to clear typical sunken-float depth
    /// (~2 m below water if the aircraft has flipped) plus a couple of meters of
    /// drop room so buoyancy / aero damps the impact rather than instantly
    /// settling. Repeated resets stack — pressing R several times gains altitude
    /// if a single bump isn't enough.
    const RESET_DROP_M: f32 = 5.0;

    for ev in inputs {
        match *ev {
            InputEvent::SetThrottle(t) => v.controls.throttle = t,
            InputEvent::SetPitch(p) => v.controls.pitch = p,
            InputEvent::SetYaw(y) => v.controls.yaw = y,
            InputEvent::SetRoll(r) => v.controls.roll = r,
            InputEvent::ThrottleDelta(d) => v.controls.throttle += d,
            InputEvent::CycleView => camera.view_kind = camera.view_kind.cycled(),
            InputEvent::SetView(nv) => camera.view_kind = nv,
            InputEvent::LookDelta(d) => camera.apply_look_delta(d),
            InputEvent::ResetLook => camera.reset_look(),
            InputEvent::ResetVehicle => {
                v.orientation = Quat::IDENTITY;
                v.velocity = glam::Vec3::ZERO;
                v.angular_velocity = glam::Vec3::ZERO;
                v.position.y += RESET_DROP_M;
            },
            InputEvent::ExtendFlaps => v.controls.flaps = v.controls.flaps.extended(),
            InputEvent::RetractFlaps => v.controls.flaps = v.controls.flaps.retracted(),
            InputEvent::SetWaterRudder(deployed) => v.controls.water_rudder = deployed,
            InputEvent::SetBrakes(on) => v.controls.brakes = on,
        }
    }
    v.controls.clamp();
}

/// Advance vehicle dynamics by `dt` given its current controls, the flight-model
/// forces from `flight::compute_forces`, the buoyancy contribution from
/// `buoyancy::compute_buoyancy`, and the caller's min-altitude floor.
///
/// Integration and ground-clamp live here; force computation is in `crate::flight`;
/// hydrostatic force/moment from floats is in `crate::buoyancy`.
fn tick_dynamics(
    v: &mut VehicleState,
    spec: Option<&AircraftSpec>,
    dt: f32,
    min_altitude: f32,
    wave_time: f32,
    chop_intensity: f32,
    weather: WeatherInputs,
    rng: &mut Rng,
) -> (flight::FlightOutputs, ground_contact::StrikeFlags) {
    // AGL approximation for ground effect (2.4.6). `min_altitude` is the app-supplied
    // surface clamp that includes the float offset + a small margin; for ground
    // effect we want the wing's height above surface, which tracks CG height above
    // surface closely enough. The float-offset + margin error is ~1-2 m out of a
    // 14.6 m wingspan, so the induced-drag reduction is slightly overstated near
    // the surface but converges to the right value above a few meters.
    let agl = (v.position.y - min_altitude).max(0.0);

    // Weather wind: aero sees `vehicle_velocity - wind` as relative airflow.
    // Temporarily swap the vehicle's velocity for airmass-relative before
    // calling the flight model, then restore. Forces computed at airflow
    // velocity are applied to world-frame velocity — a tailwind reduces
    // airspeed + lift, a headwind boosts both.
    let ground_velocity = v.velocity;
    v.velocity = ground_velocity - weather.wind;
    let forces = flight::compute_forces(v, spec, dt, agl);
    v.velocity = ground_velocity;

    // Per-pontoon buoyancy (Phase 2.5a.1). Short-circuits to zero above water,
    // when no aircraft spec is loaded, or when the aircraft is configured with
    // wheels instead of floats.
    let buoy = buoyancy::compute_buoyancy(v, spec, wave_time, chop_intensity);

    // Per-gear ground contact (Phase 2.5b.1). Short-circuits symmetrically:
    // no spec / floats config / all gears above ground → zero. The two
    // contact models are mutually exclusive by `VehicleConfig`, so in any given
    // tick at most one of `buoy` / `gc` contributes force; summing both is
    // equivalent to selecting the active one.
    let gc = ground_contact::compute_ground_contact(v, spec, min_altitude);

    // Angular: Euler-integrate angular velocity from the summed aero-moment-derived
    // and contact-derived angular acceleration, then compose the resulting rotation
    // onto orientation.
    //
    // Per 2.4.4, aero `angular_accel` already has flowplan's sign convention baked
    // in (physics-convention moments flipped on Y and Z inside `compute_forces`);
    // buoyancy and ground_contact both compute r × F directly in body frame and so
    // naturally match the same storage convention — no further flips needed when
    // summing.
    v.angular_velocity += (forces.angular_accel + buoy.angular_accel + gc.angular_accel) * dt;

    // Turbulence buffet: stochastic angular-velocity jitter scaled by
    // intensity. Deterministic via the sim RNG (folded into state_hash
    // automatically since we mutate the shared `rng`). Zero when
    // intensity = 0, so still-air scenarios are unaffected.
    if weather.turbulence_intensity > 0.0 {
        let scale = weather.turbulence_intensity * TURBULENCE_ANGULAR_JITTER_GAIN * dt;
        let jx = (rng.next_f32() * 2.0 - 1.0) * scale;
        let jy = (rng.next_f32() * 2.0 - 1.0) * scale;
        let jz = (rng.next_f32() * 2.0 - 1.0) * scale;
        v.angular_velocity += Vec3::new(jx, jy, jz);
    }

    // Residual rotational damping: a small airspeed-independent decay so angular
    // velocity bleeds off even at V=0 (where all q-based damping vanishes). Stands
    // in for gyroscopic stability / wheel friction / air viscosity not modeled in
    // the aero moments. Time constant ~3 s (`0.3 / s`), so a 1 rad/s spin at V=0
    // decays to 0.1 rad/s in ~8 s. Real ground contact physics in 2.5 will make
    // this moot for the on-ground case.
    let decay = (1.0 - 0.3 * dt).clamp(0.0, 1.0);
    v.angular_velocity *= decay;

    let dq = Quat::from_scaled_axis(v.orientation * v.angular_velocity * dt);
    v.orientation = (dq * v.orientation).normalize();

    // Linear: explicit Euler integration of velocity, then position.
    v.velocity += (forces.linear_accel + buoy.linear_accel + gc.linear_accel) * dt;
    v.position += v.velocity * dt;

    // Caller-supplied floor (terrain + float offset, or water + float offset, whichever
    // is higher in the app; a constant in pure-sim tests).
    if v.position.y < min_altitude {
        v.position.y = min_altitude;
        if v.velocity.y < 0.0 {
            v.velocity.y = 0.0;
        }
    }

    // Fuel burn (Phase 2.4.3). gph × dt / 3600 = gallons this tick. Clamped at 0 so
    // a running engine past fuel-exhaustion doesn't produce negative fuel; flameout
    // is handled in `compute_forces` via `fuel_gal > 0.0`.
    if v.fuel_gal > 0.0 && forces.fuel_flow_gph > 0.0 {
        let used = forces.fuel_flow_gph * dt / 3600.0;
        v.fuel_gal = (v.fuel_gal - used).max(0.0);
    }

    (forces, gc.strikes)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Deterministic float compares via `.to_array()` or to_bits-equivalent.
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

    #[test]
    fn wave_time_advances_per_tick() {
        let mut s = SimState::new(1);
        assert!(s.wave_time.abs() < f32::EPSILON, "fresh state starts at 0");
        let d = DifficultyDials::default();
        s.tick(&InputEvents::empty(), &d, FIXED_DT, 0.5);
        assert!(
            (s.wave_time - FIXED_DT).abs() < f32::EPSILON,
            "wave_time should advance by dt; got {}",
            s.wave_time
        );
        // Second tick.
        s.tick(&InputEvents::empty(), &d, FIXED_DT, 0.5);
        assert!(
            (s.wave_time - 2.0 * FIXED_DT).abs() < 1e-6,
            "wave_time should accumulate; got {}",
            s.wave_time
        );
    }

    #[test]
    fn reset_vehicle_restores_level_attitude_and_zeros_velocity() {
        // Player bound to `R` as the in-game recovery action: from any attitude /
        // velocity / angular velocity, reset to level + stationary while preserving
        // position, throttle, controls, flaps, fuel.
        let mut s = SimState::new(1);
        s.vehicle.position = Vec3::new(100.0, 50.0, -200.0);
        s.vehicle.velocity = Vec3::new(5.0, -10.0, -30.0);
        s.vehicle.orientation = Quat::from_rotation_z(std::f32::consts::PI); // upside-down
        s.vehicle.angular_velocity = Vec3::new(1.5, 0.7, -2.1);
        s.vehicle.controls.throttle = 0.72;
        s.vehicle.controls.pitch = 0.4;
        s.vehicle.controls.flaps = crate::controls::FlapPosition::Takeoff;
        s.vehicle.fuel_gal = 45.0;

        let mut inputs = InputEvents::default();
        inputs.push(InputEvent::ResetVehicle);
        let d = DifficultyDials::default();
        s.tick(&inputs, &d, FIXED_DT, 0.5);

        // Horizontal position preserved (a one-tick integration step happens after
        // the reset, so x/z move by essentially zero since velocity was zeroed first).
        // Vertical position bumps up by RESET_DROP_M so the aircraft drops from a
        // slight height — covers the sunken-float case where reset-in-place wouldn't
        // help.
        assert!(
            (s.vehicle.position.x - 100.0).abs() < 0.01
                && (s.vehicle.position.z - (-200.0)).abs() < 0.01,
            "horizontal position should be preserved, got {:?}",
            s.vehicle.position,
        );
        assert!(
            s.vehicle.position.y > 50.0 + 4.5 && s.vehicle.position.y < 50.0 + 5.5,
            "vertical position should bump up by ~5 m (gravity acts during the post-reset tick), got y={}",
            s.vehicle.position.y,
        );
        // Orientation reset to identity, velocity + angular velocity zeroed (modulo
        // the one tick of flight-model effects post-reset).
        assert!(
            s.vehicle.orientation.angle_between(Quat::IDENTITY) < 0.1,
            "orientation should be near-identity, got {:?}",
            s.vehicle.orientation,
        );
        assert!(
            s.vehicle.velocity.length() < 1.0,
            "velocity should be near-zero after reset, got {:?}",
            s.vehicle.velocity,
        );
        // Pilot settings preserved.
        assert!((s.vehicle.controls.throttle - 0.72).abs() < f32::EPSILON);
        assert!((s.vehicle.controls.pitch - 0.4).abs() < f32::EPSILON);
        assert_eq!(
            s.vehicle.controls.flaps,
            crate::controls::FlapPosition::Takeoff
        );
        assert!((s.vehicle.fuel_gal - 45.0).abs() < 0.01);
    }

    #[test]
    #[allow(clippy::expect_used, clippy::cast_precision_loss)]
    fn float_at_rest_settles_to_equilibrium() {
        // Phase 2.5a.2 acceptance (follows 2.5a.1 buoyancy + 2.5a.2 vertical damping):
        // aircraft dropped at water surface with zero velocity should settle to
        // equilibrium draft within a few simulated seconds. Analytic equilibrium for
        // the Beaver (MTOW 2313 kg, Edo 4930 floats per buoyancy::BEAVER_PONTOONS):
        // CG_y ≈ 0.66 m above water (see buoyancy.rs docs + unit tests).
        let spec_ron = r#"(
            name: "Northlander",
            inspiration: "DHC-2 Beaver",
            empty_mass_kg: 1315.5,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.8,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: "NACA 64A416",
            airfoil_tip: "NACA 64A412",
            engine: (name: "R-985", power_hp: 450.0, cruise_fuel_gph: 27.0),
            fuel_capacity_gal: 95.0,
            speeds: (
                vne_mps: 71.53, cruise_mps: 64.31,
                vs_clean_mps: 23.25, vso_mps: 21.01,
                vx_mps: 29.06, vy_mps: 37.99,
            ),
            variants: [
                (id: "floats", display_name: "Floats", config: Floats, mesh_path: "x.glb"),
            ],
            default_variant: "floats",
        )"#;
        let spec = AircraftSpec::from_ron(spec_ron).expect("parse spec");

        let mut s = SimState::new(1);
        s.aircraft = Some(spec);
        // Start above analytic equilibrium (CG≈2.40m post-2.5a.3 mesh-aligned float
        // geometry) with zero velocity; integration should settle downward under
        // damped buoyancy-gravity dynamics.
        s.vehicle.position = Vec3::new(0.0, 4.0, 0.0);
        s.vehicle.velocity = Vec3::ZERO;
        s.vehicle.orientation = Quat::IDENTITY;
        s.vehicle.angular_velocity = Vec3::ZERO;
        s.vehicle.controls.throttle = 0.0;
        // Fuel = 0 cuts engine power to zero (flameout behavior), isolating the test
        // from thrust-driven forward motion and the pitch coupling that it produces
        // through below-CG float drag. Forward-motion cases get their own scenario
        // (water-taxi) that asserts steady-state taxi speed under idle thrust.
        s.vehicle.fuel_gal = 0.0;

        let d = DifficultyDials::default();
        // Low min_altitude so the clamp doesn't preempt buoyancy. Buoyancy + damping
        // handle the water case; terrain clamp bounds against unexpected bugs.
        let min_alt = -20.0_f32;

        // Run 30 simulated seconds. With damping supercritical at full wet and
        // subcritical at equilibrium wet, oscillation amplitude decays exponentially
        // toward the analytic equilibrium. Sample position at multiple late-time
        // points so the assertion isn't sensitive to the exact phase of the residual
        // oscillation when sampled.
        let mut late_y_samples = Vec::new();
        for i in 0..3600 {
            s.tick(&InputEvents::empty(), &d, FIXED_DT, min_alt);
            if i >= 2400 && i % 120 == 0 {
                late_y_samples.push(s.vehicle.position.y);
            }
        }

        // Mean of the last ~10 seconds of samples averages out residual oscillation
        // and wave-driven surface noise; the analytic equilibrium is the center of
        // the oscillation envelope even if a single-tick sample happens to fall
        // at a phase extreme.
        let mean_y: f32 = late_y_samples.iter().sum::<f32>() / late_y_samples.len() as f32;

        assert!(
            (mean_y - 2.40).abs() < 0.20,
            "expected mean-over-last-10s settle near 2.40 m (analytic equilibrium for mesh-aligned floats), got {mean_y:.3} m (samples: {late_y_samples:?})",
        );
        // Vertical velocity envelope should be bounded (residual oscillation only).
        assert!(
            s.vehicle.velocity.y.abs() < 1.0,
            "expected bounded vertical velocity at 30 s, got {} m/s",
            s.vehicle.velocity.y
        );
    }

    // ---- Weather wiring (3.2.3b) ------------------------------------------

    #[test]
    fn tick_with_weather_default_equals_tick() {
        // Default WeatherInputs (zero wind, zero turbulence) must produce
        // byte-identical state to plain `tick()` — the new API doesn't
        // perturb still-air behavior.
        let mut a = SimState::new(42);
        let mut b = SimState::new(42);
        let d = DifficultyDials::default();
        for _ in 0..30 {
            a.tick(&InputEvents::empty(), &d, FIXED_DT, 0.5);
            b.tick_with_weather(
                &InputEvents::empty(),
                &d,
                FIXED_DT,
                0.5,
                WeatherInputs::default(),
            );
        }
        assert_eq!(a.vehicle.position.to_array(), b.vehicle.position.to_array());
        assert_eq!(a.vehicle.velocity.to_array(), b.vehicle.velocity.to_array());
    }

    #[test]
    fn headwind_reduces_horizontal_ground_track() {
        // Two simulations identical except for a 10 m/s headwind. After a
        // short cruise with throttle, the aircraft in the headwind should
        // travel LESS distance over the ground than the still-air case.
        let mut calm = SimState::new(1);
        let mut headwind = SimState::new(1);
        let d = DifficultyDials::default();
        // Give both a forward velocity in the air (nose aligned with -Z per
        // flowplan's forward convention).
        calm.vehicle.velocity = Vec3::new(0.0, 0.0, -30.0);
        headwind.vehicle.velocity = Vec3::new(0.0, 0.0, -30.0);
        // Headwind blows from -Z toward +Z at 10 m/s (opposing the aircraft).
        let wind = Vec3::new(0.0, 0.0, 10.0);
        let inputs = InputEvents::default();
        // Run for a second.
        for _ in 0..120 {
            calm.tick(&inputs, &d, FIXED_DT, -1000.0);
            headwind.tick_with_weather(
                &inputs,
                &d,
                FIXED_DT,
                -1000.0,
                WeatherInputs {
                    wind,
                    turbulence_intensity: 0.0,
                },
            );
        }
        // Headwind case: airspeed is higher (30 + 10 = 40 m/s relative
        // airflow), which means more drag; ground-frame velocity decays
        // faster. Over-ground distance traveled is less.
        let calm_dz = calm.vehicle.position.z;
        let headwind_dz = headwind.vehicle.position.z;
        // Both moved in -Z; magnitude comparison.
        assert!(
            headwind_dz.abs() < calm_dz.abs(),
            "headwind dz {headwind_dz} should have smaller magnitude than calm dz {calm_dz}",
        );
    }

    #[test]
    fn turbulence_induces_angular_velocity_jitter() {
        // Calm-air sim should keep angular velocity at zero (idle).
        // Turbulent sim should accumulate angular velocity jitter.
        let mut calm = SimState::new(1);
        let mut turb = SimState::new(1);
        let d = DifficultyDials::default();
        let inputs = InputEvents::default();
        let weather_turb = WeatherInputs {
            wind: Vec3::ZERO,
            turbulence_intensity: 2.0,
        };
        for _ in 0..30 {
            calm.tick(&inputs, &d, FIXED_DT, -1000.0);
            turb.tick_with_weather(&inputs, &d, FIXED_DT, -1000.0, weather_turb);
        }
        // Calm air: angular velocity stays near zero (damping + no injection).
        assert!(
            calm.vehicle.angular_velocity.length() < 0.01,
            "calm angular velocity should stay near zero; got {}",
            calm.vehicle.angular_velocity.length()
        );
        // Turbulence: some angular velocity should be present.
        assert!(
            turb.vehicle.angular_velocity.length() > 0.001,
            "turbulence should produce angular velocity jitter; got {}",
            turb.vehicle.angular_velocity.length()
        );
    }

    #[test]
    fn weather_tick_deterministic() {
        // Same seed + same weather trace must produce bit-identical state.
        let mut a = SimState::new(12345);
        let mut b = SimState::new(12345);
        let d = DifficultyDials::default();
        let weather = WeatherInputs {
            wind: Vec3::new(3.0, 0.0, -7.0),
            turbulence_intensity: 1.5,
        };
        let inputs = InputEvents::default();
        for _ in 0..60 {
            a.tick_with_weather(&inputs, &d, FIXED_DT, -1000.0, weather);
            b.tick_with_weather(&inputs, &d, FIXED_DT, -1000.0, weather);
        }
        assert_eq!(a.vehicle.position.to_array(), b.vehicle.position.to_array());
        assert_eq!(a.vehicle.velocity.to_array(), b.vehicle.velocity.to_array());
        assert_eq!(
            a.vehicle.angular_velocity.to_array(),
            b.vehicle.angular_velocity.to_array()
        );
    }
}
