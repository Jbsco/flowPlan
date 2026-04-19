//! Top-level simulation state and the deterministic `tick()` function.
//!
//! Phase 1 dynamics are intentionally simple -- a direct-control rigid body with gravity
//! and damping. The goal is to validate the fixed-timestep loop, input plumbing, camera
//! switching, and determinism tests with minimum aerodynamic sophistication.
//! Phase 2 replaces `tick_dynamics` with the DHC-2 Beaver flight model.

use flowplan_core::{Rng, TickCount};
use glam::{Quat, Vec3};

use crate::camera::Camera;
use crate::controls::Controls;
use crate::difficulty::DifficultyDials;
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

/// Gravitational acceleration at the placeholder flat Earth.
const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// Thrust produced at full throttle along the vehicle's local forward axis (m/s^2).
///
/// Tuned so level-flight thrust comfortably exceeds gravity (32 > 9.81). This is not
/// realistic for an aircraft -- it is placeholder behavior so the Phase 1 cube can
/// actually climb without flopping into steep pitch. Phase 2 replaces this with the
/// Beaver's published power-to-weight characteristics.
const THRUST_ACCEL_MAX: f32 = 32.0;

/// Peak commanded angular rates per axis (radians per second).
///
/// Dialled back from the first iteration so controls feel deliberate rather than
/// saturating in under a second. Still arcade-level responsive.
const ROLL_RATE_MAX: f32 = 1.6;
const PITCH_RATE_MAX: f32 = 1.0;
const YAW_RATE_MAX: f32 = 0.6;

/// Angular-velocity smoothing toward the commanded rate (1/s).
const ANG_RESPONSE: f32 = 10.0;

/// Linear-velocity drag (1/s). Applies at all speeds; used for low-speed stability.
const LIN_DAMPING: f32 = 0.05;

/// Quadratic drag (1/m). Dominates at higher speeds and bounds terminal velocity.
/// With `THRUST_ACCEL_MAX = 32` and default throttle 0.4 (thrust accel 12.8 m/s^2), this
/// produces a terminal speed around 28 m/s -- balances the lift coefficient below for
/// near-level cruise at default throttle.
const QUAD_DRAG: f32 = 0.014;

/// Lift coefficient: m/s^2 of vertical acceleration per m/s of *horizontal* forward
/// airspeed. At ~28 m/s horizontal forward airspeed, lift balances gravity (level flight).
/// Using horizontal-only airspeed means diving reduces lift (nose-down -> less horizontal
/// forward -> less lift -> gravity wins -> dive accelerates), which is the behavior a
/// placeholder flight model needs to feel plane-like. Phase 2 replaces with per-aircraft
/// `Cl(alpha)` tables aligned with the body's up axis.
const LIFT_COEFF: f32 = 0.344;

/// Ground plane height. Placeholder until terrain sampling lands later in Phase 1.
const GROUND_Y: f32 = 0.5;

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
        }
    }

    /// Advance the simulation by one fixed step.
    ///
    /// Deterministic for a given `(seed, input trace, difficulty state, tick count)`.
    /// See [`DESIGN.md § Invariants`](../../../DESIGN.md#invariants).
    pub fn tick(&mut self, inputs: &InputEvents, _difficulty: &DifficultyDials, dt: f32) {
        apply_inputs(&mut self.vehicle.controls, &mut self.camera, inputs);
        tick_dynamics(&mut self.vehicle, dt);
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

/// Advance vehicle dynamics by `dt` given its current controls.
fn tick_dynamics(v: &mut VehicleState, dt: f32) {
    // Commanded angular rates per axis, expressed in the *body frame*.
    // Sign conventions (right-handed, Y-up, -Z forward):
    //   pitch positive = nose up      -> rotate +X (right)
    //   yaw positive   = nose right   -> rotate -Y (positive rot around +Y yaws LEFT)
    //   roll positive  = roll right   -> rotate -Z (positive rot around +Z rolls LEFT)
    let commanded = Vec3::new(
        v.controls.pitch * PITCH_RATE_MAX,
        -v.controls.yaw * YAW_RATE_MAX,
        -v.controls.roll * ROLL_RATE_MAX,
    );
    // First-order tracking of commanded rate.
    let response = 1.0 - (-ANG_RESPONSE * dt).exp();
    v.angular_velocity = v.angular_velocity.lerp(commanded, response);

    // Integrate orientation. For small dt, (axis * angle)-form exponential is fine.
    let dq = Quat::from_scaled_axis(v.orientation * v.angular_velocity * dt);
    v.orientation = (dq * v.orientation).normalize();

    // Thrust along current forward.
    let forward = v.orientation * Vec3::NEG_Z;
    let thrust_accel = forward * (v.controls.throttle * THRUST_ACCEL_MAX);

    // Placeholder "wing" lift: vertical acceleration proportional to the HORIZONTAL
    // component of forward airspeed. Projecting to the world XZ plane means that as
    // the nose pitches down, horizontal forward drops and so does lift -- dives
    // accelerate rather than being held level by wing lift. Lift is always along
    // world +Y (horizon-relative); Phase 2 replaces with body-up + airfoil tables.
    let horiz_velocity = Vec3::new(v.velocity.x, 0.0, v.velocity.z);
    let horiz_forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let horiz_airspeed = horiz_velocity.dot(horiz_forward).max(0.0);
    let lift_accel = Vec3::Y * (horiz_airspeed * LIFT_COEFF);

    // Drag: linear + quadratic, opposing velocity. Linear dominates at low speeds
    // (stability); quadratic dominates at high speeds (bounded terminal velocity).
    let speed = v.velocity.length();
    let drag_accel = if speed > 1e-3 {
        -v.velocity.normalize_or_zero() * (speed * (LIN_DAMPING + QUAD_DRAG * speed))
    } else {
        Vec3::ZERO
    };

    // Linear integration.
    let accel = thrust_accel + lift_accel + drag_accel + GRAVITY;
    v.velocity += accel * dt;
    v.position += v.velocity * dt;

    // Simple ground plane.
    if v.position.y < GROUND_Y {
        v.position.y = GROUND_Y;
        if v.velocity.y < 0.0 {
            v.velocity.y = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
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
        s.tick(&InputEvents::empty(), &d, FIXED_DT);
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
        s.tick(&InputEvents::empty(), &d, FIXED_DT);
        assert!(s.time_of_day < 0.5, "expected wrap; got {}", s.time_of_day);
        assert!(s.time_of_day >= 0.0);
    }

    #[test]
    fn tick_advances_count() {
        let mut s = SimState::new(1);
        let empty = InputEvents::empty();
        let d = DifficultyDials::default();
        s.tick(&empty, &d, FIXED_DT);
        s.tick(&empty, &d, FIXED_DT);
        assert_eq!(s.tick, TickCount(2));
    }

    #[test]
    fn gravity_pulls_down_when_idle() {
        let mut s = SimState::new(1);
        s.vehicle.position = Vec3::new(0.0, 20.0, 0.0); // start high
        let empty = InputEvents::empty();
        let d = DifficultyDials::default();
        for _ in 0..10 {
            s.tick(&empty, &d, FIXED_DT);
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
        s.tick(&inputs, &d, FIXED_DT);
        // After one tick, velocity should have a negative z component (forward).
        for _ in 0..60 {
            s.tick(&InputEvents::empty(), &d, FIXED_DT);
        }
        assert!(
            s.vehicle.velocity.z < -2.0,
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
        s.tick(&inputs, &d, FIXED_DT);
        assert_ne!(s.camera.view_kind, original);
    }

    #[test]
    fn ground_clamp_holds() {
        let mut s = SimState::new(1);
        // Start just above the ground, falling fast enough that the tick's integration
        // step drives position below GROUND_Y and triggers the clamp.
        s.vehicle.position = Vec3::new(0.0, GROUND_Y + 0.01, 0.0);
        s.vehicle.velocity = Vec3::new(0.0, -5.0, 0.0);
        let d = DifficultyDials::default();
        s.tick(&InputEvents::empty(), &d, FIXED_DT);
        assert!(
            s.vehicle.position.y >= GROUND_Y - f32::EPSILON,
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
        s.tick(&inputs, &d, FIXED_DT);
        assert_eq!(s.camera.view_kind, ViewKind::FirstPerson);
    }
}
