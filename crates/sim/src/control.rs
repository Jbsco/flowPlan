//! Shared control laws: PID-based feedback controllers.
//!
//! Usable by both the flight-test-bench (scenario closed-loop testing) and the
//! in-game autopilot (Phase 4.7 — hold altitude, hold bank, hold heading,
//! hold-all, full-auto).
//!
//! The same `ControlLaw` enum drives both contexts, so an autopilot shipped in
//! Phase 4 inherits validation from the scenario bench by construction: if a
//! `BankHold { target: 30 }` controller holds a 30° bank in the bench, the
//! in-game autopilot's "hold bank" mode uses the same law and behaves identically.
//!
//! # Architecture
//!
//! - `ControlLaw` is the law: what to track, what to output, what gains.
//! - `ControlLawState` is the per-instance state: PID integral, previous error.
//!   Kept separate so scenarios (bench) can own state in the runner without
//!   polluting the scenario definition; the game's autopilot owns state in
//!   `SimState` for determinism.
//! - `step()` is pure: `(law, state, current_observation, dt) → output event`.
//!   Deterministic, no hidden state, no wall-clock.
//!
//! Sign convention: user-facing targets use intuitive signs (positive bank =
//! right bank, positive heading = clockwise from north). Internal conversion
//! to sim-frame signs happens at the observation-read boundary. This matters
//! because the sim's Euler decomposition gives negative roll for right-wing-down
//! attitude.
//!
//! # Determinism
//!
//! `ControlLawState` is pure POD (two f32s). For bench use it lives in the
//! scenario runner as a Vec<ControlLawState>. For autopilot use it will live
//! in `SimState` so it's part of the determinism hash. The tick ordering is:
//! scripted inputs first, then controllers (which may override).

#![forbid(unsafe_code)]

use crate::input::InputEvent;
use crate::state::SimState;

/// A closed-loop control law. Read observations from sim state, output an input
/// event to drive the controlled variable toward target.
///
/// Currently implements a simple PID for each law. Per-axis coupling (e.g.,
/// bank angle affects altitude) is NOT modeled inside the law — scenarios /
/// autopilot modes compose multiple laws (e.g., `BankHold` + `AltitudeHold` +
/// `AirspeedHold` for a coordinated turn at cruise speed).
#[derive(Clone, Copy, Debug)]
pub enum ControlLaw {
    /// Hold bank angle. Output drives `SetRoll` input. Target is in the user-
    /// intuitive convention: **positive = right bank, negative = left bank**.
    /// Internal sign-flip handles the sim's Euler decomposition convention.
    BankHold {
        target_bank_deg: f32,
        kp: f32,
        ki: f32,
        kd: f32,
    },

    /// Hold altitude. Output drives `SetPitch` input. Target in meters MSL.
    AltitudeHold {
        target_alt_m: f32,
        kp: f32,
        ki: f32,
        kd: f32,
    },

    /// Hold airspeed. Output drives `SetThrottle` input. Target in m/s.
    AirspeedHold {
        target_mps: f32,
        kp: f32,
        ki: f32,
        kd: f32,
    },

    /// Hold vertical speed. Output drives `SetPitch`. Target in m/s (+up).
    VerticalSpeedHold {
        target_vs_mps: f32,
        kp: f32,
        ki: f32,
        kd: f32,
    },

    /// Hold heading. Output drives `SetRoll` via a bank target computed from
    /// heading error. Target in degrees from north (+clockwise = east).
    /// Reserved for a future "hold heading" scenario / autopilot mode;
    /// implementation left blank in 2.4.9c — not needed for the initial
    /// turn scenarios.
    HeadingHold {
        target_heading_deg: f32,
        kp: f32,
        ki: f32,
        kd: f32,
    },

    /// Hold coordinated flight (β = 0). Output drives `SetYaw` (rudder) to
    /// zero sideslip. Essential for coordinated turns — without this, the
    /// aircraft slips/skids through the turn and generates extra drag.
    /// Also acts as the auto-rudder-coordination feature that will be a
    /// Phase 4.9 accessibility assist.
    CoordinationHold { kp: f32, ki: f32, kd: f32 },

    /// Altitude-aware glide-to-touchdown controller.
    ///
    /// Tracks a vertical-speed target that varies with altitude: high vs
    /// during approach, low vs at flare. Output drives `SetPitch` via a PID
    /// on vs-error. Used by water + ground landing scenarios, and will
    /// later back Phase 4.7 "approach / land" autopilot modes and NPC
    /// pilot landing behaviors.
    ///
    /// Phase logic (altitude-based):
    /// - `alt > flare_begin_alt_m`: `target_vs` = `approach_vs_mps`
    /// - `flare_peak_alt_m < alt < flare_begin_alt_m`: `target_vs` lerps linearly between `approach_vs_mps` and `touchdown_vs_mps`
    /// - `alt < flare_peak_alt_m`: `target_vs` = `touchdown_vs_mps`
    ///
    /// Tuning: `approach_vs` typically -5 to -3 m/s; `touchdown_vs`
    /// typically -0.5 to -0.2 m/s; `flare_begin` 12–18 m AGL (POH Vref
    /// approach path); `flare_peak` 1.5–3 m AGL (float top at touchdown).
    /// PID gains scale pitch input response; start with `kp = 0.08,
    /// ki = 0.01, kd = 0.03` and adjust per airframe.
    ///
    /// Future hooks for autopilot / NPC use:
    /// - Reference values are configurable — NPC skill levels can vary
    ///   (novice = higher flare altitude + sloppier gains, veteran =
    ///   tighter profile).
    /// - Controller state (mode) is implicit in altitude; external code
    ///   reads `sim.vehicle.position.y` to know "am I in flare yet?".
    /// - Composes with `AirspeedHold` (throttle) + `CoordinationHold`
    ///   (yaw) for a three-law autopilot stack: speed + coordination +
    ///   descent/flare.
    GlideToTouchdown {
        approach_vs_mps: f32,
        touchdown_vs_mps: f32,
        flare_begin_alt_m: f32,
        flare_peak_alt_m: f32,
        kp: f32,
        ki: f32,
        kd: f32,
    },
}

/// Per-instance state for a single `ControlLaw`. PID needs to remember the
/// integral of error and the previous error (for the derivative term).
#[derive(Clone, Copy, Debug, Default)]
pub struct ControlLawState {
    pub integral: f32,
    pub prev_error: f32,
}

impl ControlLaw {
    /// Advance the control law one tick. Reads the current sim state, updates
    /// `state`, returns an `InputEvent` that the tick loop should feed into
    /// the sim's input queue.
    ///
    /// The `dt` argument is the sim tick duration (`FIXED_DT`), NOT wall-clock
    /// time. Keeps the loop deterministic.
    #[must_use]
    pub fn step(self, state: &mut ControlLawState, sim: &SimState, dt: f32) -> InputEvent {
        match self {
            Self::BankHold {
                target_bank_deg,
                kp,
                ki,
                kd,
            } => {
                // Sim's Euler YXZ roll is negative when aircraft is right-banked.
                // Flip the sign so "target_bank_deg positive = right bank" is
                // intuitive for the scenario author.
                let (_, _, sim_roll_rad) = sim.vehicle.orientation.to_euler(glam::EulerRot::YXZ);
                let intuitive_bank_deg = -sim_roll_rad.to_degrees();
                let error = target_bank_deg - intuitive_bank_deg;
                let output = pid_step(state, error, dt, kp, ki, kd).clamp(-1.0, 1.0);
                InputEvent::SetRoll(output)
            },
            Self::AltitudeHold {
                target_alt_m,
                kp,
                ki,
                kd,
            } => {
                let error = target_alt_m - sim.vehicle.position.y;
                let output = pid_step(state, error, dt, kp, ki, kd).clamp(-1.0, 1.0);
                InputEvent::SetPitch(output)
            },
            Self::AirspeedHold {
                target_mps,
                kp,
                ki,
                kd,
            } => {
                let v = sim.vehicle.velocity.length();
                let error = target_mps - v;
                let output = pid_step(state, error, dt, kp, ki, kd).clamp(0.0, 1.0);
                InputEvent::SetThrottle(output)
            },
            Self::VerticalSpeedHold {
                target_vs_mps,
                kp,
                ki,
                kd,
            } => {
                let error = target_vs_mps - sim.vehicle.velocity.y;
                let output = pid_step(state, error, dt, kp, ki, kd).clamp(-1.0, 1.0);
                InputEvent::SetPitch(output)
            },
            Self::CoordinationHold { kp, ki, kd } => {
                // β (sideslip) is already computed by the flight model each tick.
                // Read it from the last flight outputs. Positive β = wind from
                // right of aircraft → want nose-right yaw → positive SetYaw output.
                let beta_rad = sim.last_flight.beta;
                let error = beta_rad.to_degrees(); // target β = 0, so error = β
                let output = pid_step(state, error, dt, kp, ki, kd).clamp(-1.0, 1.0);
                InputEvent::SetYaw(output)
            },
            Self::GlideToTouchdown {
                approach_vs_mps,
                touchdown_vs_mps,
                flare_begin_alt_m,
                flare_peak_alt_m,
                kp,
                ki,
                kd,
            } => {
                // Altitude-driven vs target: flat approach_vs above
                // flare_begin, flat touchdown_vs below flare_peak, linear
                // ramp in between. Captures real pilot flare technique —
                // pilot reads altitude (visual + instruments) and adjusts
                // target descent rate accordingly.
                let alt = sim.vehicle.position.y;
                let target_vs = if alt >= flare_begin_alt_m {
                    approach_vs_mps
                } else if alt <= flare_peak_alt_m {
                    touchdown_vs_mps
                } else {
                    let t = (alt - flare_peak_alt_m) / (flare_begin_alt_m - flare_peak_alt_m);
                    touchdown_vs_mps + (approach_vs_mps - touchdown_vs_mps) * t
                };
                let error = target_vs - sim.vehicle.velocity.y;
                let output = pid_step(state, error, dt, kp, ki, kd).clamp(-1.0, 1.0);
                InputEvent::SetPitch(output)
            },
            Self::HeadingHold {
                target_heading_deg,
                kp,
                ki,
                kd,
            } => {
                // Heading is +clockwise from -Z (nose forward). sim YXZ yaw is
                // the Y-rotation component. Currently unused by 2.4.9c scenarios;
                // implementation kept minimal + marked for future expansion into
                // bank-target cascaded control (outer heading → inner bank).
                let (sim_yaw_rad, _, _) = sim.vehicle.orientation.to_euler(glam::EulerRot::YXZ);
                let current_heading_deg = (-sim_yaw_rad).to_degrees();
                let raw = target_heading_deg - current_heading_deg;
                // Normalize to [-180, 180] without a float-loop (clippy::while_float).
                let error = ((raw + 180.0).rem_euclid(360.0)) - 180.0;
                // Direct bank command (simplification; proper cascaded control
                // would go error → bank_target → BankHold's inner loop).
                let output = pid_step(state, error, dt, kp, ki, kd).clamp(-1.0, 1.0);
                InputEvent::SetRoll(output)
            },
        }
    }
}

/// Advance a PID controller one tick. Pure function; only mutates `state`.
///
/// Standard textbook form: `output = kp·e + ki·∫e + kd·de/dt`.
fn pid_step(state: &mut ControlLawState, error: f32, dt: f32, kp: f32, ki: f32, kd: f32) -> f32 {
    state.integral += error * dt;
    // Clamp integral to prevent wind-up during long error periods.
    state.integral = state.integral.clamp(-100.0, 100.0);
    let derivative = if dt > 0.0 {
        (error - state.prev_error) / dt
    } else {
        0.0
    };
    state.prev_error = error;
    kp * error + ki * state.integral + kd * derivative
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_zero_error_zero_output() {
        let mut state = ControlLawState::default();
        let out = pid_step(&mut state, 0.0, 0.0083, 0.5, 0.1, 0.05);
        assert!(out.abs() < 1e-6);
    }

    #[test]
    fn pid_positive_error_positive_output() {
        let mut state = ControlLawState::default();
        let out = pid_step(&mut state, 10.0, 0.0083, 0.5, 0.0, 0.0);
        assert!((out - 5.0).abs() < 1e-3);
    }

    #[test]
    fn pid_integral_accumulates() {
        let mut state = ControlLawState::default();
        // Apply constant error of 1.0 for 100 ticks.
        for _ in 0..100 {
            pid_step(&mut state, 1.0, 0.0083, 0.0, 1.0, 0.0);
        }
        // Integral ≈ 100 · 0.0083 · 1.0 = 0.83
        assert!((state.integral - 0.83).abs() < 0.05);
    }

    #[test]
    fn pid_integral_clamped_against_windup() {
        let mut state = ControlLawState::default();
        // Large sustained error — integral should clamp at ±100.
        for _ in 0..100_000 {
            pid_step(&mut state, 10.0, 0.0083, 0.0, 1.0, 0.0);
        }
        assert!(state.integral.abs() <= 100.0);
    }
}
