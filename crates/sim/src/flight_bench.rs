//! Flight test bench (Phase 2.4.8).
//!
//! Deterministic scenario runner for validating flight-model behavior against
//! per-aircraft POH figures. Scenarios are static data: initial state, aircraft spec,
//! scripted input trace, duration, and a list of assertions. The runner advances
//! `SimState::tick` exactly like production code, capturing a `BenchSample` each
//! tick for telemetry output and assertion checking.
//!
//! See [`TEST_PLAN.md § Flight Test Bench`](../../../TEST_PLAN.md#flight-test-bench)
//! for the full design intent.
//!
//! # Usage
//!
//! Headless (integration test):
//! ```ignore
//! for scenario in BEAVER_SCENARIOS {
//!     let spec = load_aircraft_spec(scenario.aircraft_spec_path).unwrap();
//!     let log = run_scenario_headless(scenario, &spec);
//!     let results = check_assertions(scenario, &log);
//!     for r in &results {
//!         assert!(r.passed, "{}: {}", scenario.name, r.message);
//!     }
//! }
//! ```
//!
//! Visual playback (via app's `--scenario <name>` CLI flag): the app looks up the
//! scenario by name, seeds `SimState` from `ScenarioInitial`, and uses a scripted
//! input source that emits events from `ScenarioInput` at their target tick. The
//! live renderer + HUD + widgets work unchanged; telemetry is optionally captured
//! to a CSV via the same path as headless runs.

use glam::{Quat, Vec3};

use crate::aircraft_spec::AircraftSpec;
use crate::controls::FlapPosition;
use crate::difficulty::DifficultyDials;
use crate::flight::{FlightOutputs, StallState};
use crate::input::{InputEvent, InputEvents};
use crate::state::{SimState, FIXED_DT};
use crate::vehicle::VehicleState;

/// Initial state for a scenario. Seeded into `SimState` before the first tick.
#[derive(Clone, Copy, Debug)]
pub struct ScenarioInitial {
    pub position: Vec3,
    pub velocity: Vec3,
    pub orientation: Quat,
    pub angular_velocity: Vec3,
    pub throttle: f32,
    pub flaps: FlapPosition,
    pub fuel_gal: f32,
}

impl ScenarioInitial {
    /// Level flight at the given altitude + airspeed (nose-aligned with velocity,
    /// full fuel). Convenience for scenarios that don't need a special attitude.
    #[must_use]
    pub fn level_cruise(altitude_m: f32, airspeed_mps: f32, throttle: f32, fuel_gal: f32) -> Self {
        Self {
            position: Vec3::new(0.0, altitude_m, 0.0),
            velocity: Vec3::new(0.0, 0.0, -airspeed_mps),
            orientation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            throttle,
            flaps: FlapPosition::Up,
            fuel_gal,
        }
    }
}

/// A scripted input event delivered at a specific tick during scenario playback.
#[derive(Clone, Copy, Debug)]
pub struct ScenarioInput {
    pub tick: u32,
    pub event: InputEvent,
}

/// What to verify about the scenario's telemetry log. Each variant has a check
/// function in `check_assertions()`.
#[derive(Clone, Copy, Debug)]
pub enum Assertion {
    /// Mean altitude over the last `last_secs` seconds is within `tol_m` of the
    /// initial altitude. Tests "level flight holds altitude".
    AltitudeStableAtEnd { last_secs: f32, tol_m: f32 },

    /// Mean vertical speed over the last `last_secs` seconds is within
    /// `tol_mps` of `target_mps`. Tests "plane climbs at expected ROC".
    ClimbRateAtEnd {
        last_secs: f32,
        target_mps: f32,
        tol_mps: f32,
    },

    /// Mean airspeed over the last `last_secs` seconds is within `tol_mps` of
    /// `target_mps`. Tests cruise speed achievement.
    AirspeedAtEnd {
        last_secs: f32,
        target_mps: f32,
        tol_mps: f32,
    },

    /// No tick in the log reports `StallState::Stall`.
    NeverStalls,

    /// At least one tick reports `StallState::Stall`. Used to verify scenarios that
    /// are supposed to induce stall do so.
    EventuallyStalls,

    /// Final airspeed within [`min_mps`, `max_mps`].
    FinalAirspeedWithin { min_mps: f32, max_mps: f32 },

    /// Minimum airspeed over the whole scenario is within `tol_mps` of `target_mps`.
    /// Used for stall-speed validation: drive the plane into stall and check the
    /// speed at which it stalled.
    MinAirspeedNearTarget { target_mps: f32, tol_mps: f32 },

    /// Final fuel remaining is within `[min_gal, max_gal]`. Used for
    /// endurance / burn-rate validation: run at a known power setting for a known
    /// duration and verify fuel consumed matches POH burn rate.
    FinalFuelWithin { min_gal: f32, max_gal: f32 },
}

/// A `FlightScenario` is the top-level description of a test case.
#[derive(Clone, Copy, Debug)]
pub struct FlightScenario {
    pub name: &'static str,
    pub description: &'static str,
    /// Workspace-relative path to the aircraft spec RON, e.g.
    /// `"data/aircraft/northlander.ron"`. The runner loads and verifies at startup.
    pub aircraft_spec_path: &'static str,
    pub initial: ScenarioInitial,
    pub inputs: &'static [ScenarioInput],
    pub duration_ticks: u32,
    pub assertions: &'static [Assertion],
}

/// Per-tick capture of the flight state during a scenario run.
///
/// Serializable to CSV for offline plotting. Fields mirror the subset of `Telemetry`
/// that matters for POH analysis; keeping this in `sim` (rather than re-using
/// `render::Telemetry`) avoids pulling the render crate into headless tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct BenchSample {
    pub tick: u64,
    pub time_s: f32,

    // Pose
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
    pub yaw_deg: f32,

    // Velocity / airspeed
    pub velocity_x: f32,
    pub velocity_y: f32,
    pub velocity_z: f32,
    pub airspeed_mps: f32,
    pub vertical_speed_mps: f32,
    pub flight_path_angle_deg: f32,

    // Aerodynamics
    pub alpha_deg: f32,
    pub beta_deg: f32,
    pub cl: f32,
    pub cd: f32,

    // Controls
    pub throttle: f32,
    pub pitch_input: f32,
    pub roll_input: f32,
    pub yaw_input: f32,
    pub flap_deg: f32,

    // Engine
    pub engine_power_kw: f32,
    pub prop_efficiency: f32,
    pub fuel_flow_gph: f32,
    pub fuel_remaining_gal: f32,

    // Moments (physics convention)
    pub pitch_moment_nm: f32,
    pub roll_moment_nm: f32,
    pub yaw_moment_nm: f32,

    // State flags (stored as ints for CSV compatibility: 0=Normal, 1=Approach, 2=Stall)
    pub stall_state: u32,
}

impl BenchSample {
    #[must_use]
    pub fn capture(tick: u64, sim: &SimState, flight: &FlightOutputs) -> Self {
        let (yaw, pitch, roll) = sim.vehicle.orientation.to_euler(glam::EulerRot::YXZ);
        let horiz = sim.vehicle.velocity.x.hypot(sim.vehicle.velocity.z);
        let fpa_rad = if horiz < 1e-3 && sim.vehicle.velocity.y.abs() < 1e-3 {
            0.0
        } else {
            sim.vehicle.velocity.y.atan2(horiz)
        };
        Self {
            tick,
            #[allow(clippy::cast_precision_loss)] // tick is a small counter; 32-bit precision is fine
            time_s: tick as f32 * FIXED_DT,
            position_x: sim.vehicle.position.x,
            position_y: sim.vehicle.position.y,
            position_z: sim.vehicle.position.z,
            pitch_deg: pitch.to_degrees(),
            roll_deg: roll.to_degrees(),
            yaw_deg: yaw.to_degrees(),
            velocity_x: sim.vehicle.velocity.x,
            velocity_y: sim.vehicle.velocity.y,
            velocity_z: sim.vehicle.velocity.z,
            airspeed_mps: sim.vehicle.velocity.length(),
            vertical_speed_mps: sim.vehicle.velocity.y,
            flight_path_angle_deg: fpa_rad.to_degrees(),
            alpha_deg: flight.alpha.to_degrees(),
            beta_deg: flight.beta.to_degrees(),
            cl: flight.cl,
            cd: flight.cd,
            throttle: sim.vehicle.controls.throttle,
            pitch_input: sim.vehicle.controls.pitch,
            roll_input: sim.vehicle.controls.roll,
            yaw_input: sim.vehicle.controls.yaw,
            flap_deg: sim.vehicle.controls.flaps.deflection_deg(),
            engine_power_kw: flight.engine_power_kw,
            prop_efficiency: flight.prop_efficiency,
            fuel_flow_gph: flight.fuel_flow_gph,
            fuel_remaining_gal: sim.vehicle.fuel_gal,
            pitch_moment_nm: flight.pitch_moment.total(),
            roll_moment_nm: flight.roll_moment.total(),
            yaw_moment_nm: flight.yaw_moment.total(),
            stall_state: match flight.stall_state {
                StallState::Normal => 0,
                StallState::Approach => 1,
                StallState::Stall => 2,
            },
        }
    }

    /// CSV header line matching the field order in `write_csv_row`.
    pub fn csv_header() -> &'static str {
        "tick,time_s,pos_x,pos_y,pos_z,pitch_deg,roll_deg,yaw_deg,\
         vel_x,vel_y,vel_z,airspeed_mps,vs_mps,fpa_deg,\
         alpha_deg,beta_deg,cl,cd,\
         throttle,pitch_in,roll_in,yaw_in,flap_deg,\
         engine_kw,prop_eff,fuel_gph,fuel_gal,\
         pitch_M,roll_L,yaw_N,stall_state"
    }

    pub fn write_csv_row(&self, out: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(
            out,
            "{},{:.5},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},\
             {:.3},{:.3},{:.3},{:.3},{:.3},{:.3},\
             {:.3},{:.3},{:.4},{:.4},\
             {:.3},{:.3},{:.3},{:.3},{:.1},\
             {:.2},{:.3},{:.2},{:.2},\
             {:.1},{:.1},{:.1},{}",
            self.tick,
            self.time_s,
            self.position_x,
            self.position_y,
            self.position_z,
            self.pitch_deg,
            self.roll_deg,
            self.yaw_deg,
            self.velocity_x,
            self.velocity_y,
            self.velocity_z,
            self.airspeed_mps,
            self.vertical_speed_mps,
            self.flight_path_angle_deg,
            self.alpha_deg,
            self.beta_deg,
            self.cl,
            self.cd,
            self.throttle,
            self.pitch_input,
            self.roll_input,
            self.yaw_input,
            self.flap_deg,
            self.engine_power_kw,
            self.prop_efficiency,
            self.fuel_flow_gph,
            self.fuel_remaining_gal,
            self.pitch_moment_nm,
            self.roll_moment_nm,
            self.yaw_moment_nm,
            self.stall_state,
        );
    }
}

/// Assertion outcome.
#[derive(Clone, Debug)]
pub struct AssertionResult {
    pub assertion: Assertion,
    pub passed: bool,
    pub message: String,
}

/// Seed a `SimState` from a scenario's initial conditions + aircraft spec.
#[must_use]
pub fn seed_state_from_scenario(
    scenario: &FlightScenario,
    spec: AircraftSpec,
    seed: u64,
) -> SimState {
    let mut sim = SimState::new(seed);
    sim.vehicle = VehicleState::at_origin();
    sim.vehicle.position = scenario.initial.position;
    sim.vehicle.velocity = scenario.initial.velocity;
    sim.vehicle.orientation = scenario.initial.orientation;
    sim.vehicle.angular_velocity = scenario.initial.angular_velocity;
    sim.vehicle.controls.throttle = scenario.initial.throttle;
    sim.vehicle.controls.flaps = scenario.initial.flaps;
    sim.vehicle.fuel_gal = scenario.initial.fuel_gal;
    sim.aircraft = Some(spec);
    sim
}

/// Run a scenario headlessly at fixed `min_altitude = 0.0` (sea level). Returns the
/// per-tick bench-sample log.
///
/// The scenario's `inputs` are injected into the tick's `InputEvents` at the
/// matching tick. Tests should then call `check_assertions()` to validate.
#[must_use]
pub fn run_scenario_headless(scenario: &FlightScenario, spec: AircraftSpec) -> Vec<BenchSample> {
    let mut sim = seed_state_from_scenario(scenario, spec, 0xBEEF);
    let dials = DifficultyDials::default();
    let mut log = Vec::with_capacity(scenario.duration_ticks as usize);

    for tick in 0..scenario.duration_ticks {
        let mut batch = InputEvents::default();
        for input in scenario.inputs {
            if input.tick == tick {
                batch.push(input.event);
            }
        }
        sim.tick(&batch, &dials, FIXED_DT, 0.0);
        log.push(BenchSample::capture(tick as u64, &sim, &sim.last_flight));
    }

    log
}

/// Check every assertion in the scenario against the captured log.
#[must_use]
pub fn check_assertions(scenario: &FlightScenario, log: &[BenchSample]) -> Vec<AssertionResult> {
    let mut out = Vec::with_capacity(scenario.assertions.len());
    for &a in scenario.assertions {
        out.push(check_one(a, log));
    }
    out
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]
fn check_one(assertion: Assertion, log: &[BenchSample]) -> AssertionResult {
    if log.is_empty() {
        return AssertionResult {
            assertion,
            passed: false,
            message: "empty log".to_string(),
        };
    }
    match assertion {
        Assertion::AltitudeStableAtEnd { last_secs, tol_m } => {
            let n = ((last_secs / FIXED_DT) as usize).max(1).min(log.len());
            let mean_y: f32 =
                log.iter().rev().take(n).map(|s| s.position_y).sum::<f32>() / n as f32;
            let initial_y = log[0].position_y;
            let drift = (mean_y - initial_y).abs();
            AssertionResult {
                assertion,
                passed: drift <= tol_m,
                message: format!(
                    "altitude drift {drift:.1} m (initial {initial_y:.1}, final-window mean {mean_y:.1}, tol {tol_m:.1})"
                ),
            }
        },
        Assertion::ClimbRateAtEnd {
            last_secs,
            target_mps,
            tol_mps,
        } => {
            let n = ((last_secs / FIXED_DT) as usize).max(1).min(log.len());
            let mean_vs: f32 = log
                .iter()
                .rev()
                .take(n)
                .map(|s| s.vertical_speed_mps)
                .sum::<f32>()
                / n as f32;
            AssertionResult {
                assertion,
                passed: (mean_vs - target_mps).abs() <= tol_mps,
                message: format!(
                    "climb rate {mean_vs:.2} m/s (target {target_mps:.2}, tol {tol_mps:.2})"
                ),
            }
        },
        Assertion::AirspeedAtEnd {
            last_secs,
            target_mps,
            tol_mps,
        } => {
            let n = ((last_secs / FIXED_DT) as usize).max(1).min(log.len());
            let mean_v: f32 = log
                .iter()
                .rev()
                .take(n)
                .map(|s| s.airspeed_mps)
                .sum::<f32>()
                / n as f32;
            AssertionResult {
                assertion,
                passed: (mean_v - target_mps).abs() <= tol_mps,
                message: format!(
                    "airspeed {mean_v:.2} m/s (target {target_mps:.2}, tol {tol_mps:.2})"
                ),
            }
        },
        Assertion::NeverStalls => {
            let stalled = log.iter().any(|s| s.stall_state == 2);
            AssertionResult {
                assertion,
                passed: !stalled,
                message: if stalled {
                    "stall state observed at some tick".to_string()
                } else {
                    "no stall in log".to_string()
                },
            }
        },
        Assertion::EventuallyStalls => {
            let stalled = log.iter().any(|s| s.stall_state == 2);
            AssertionResult {
                assertion,
                passed: stalled,
                message: if stalled {
                    "stall observed".to_string()
                } else {
                    "stall never triggered".to_string()
                },
            }
        },
        Assertion::FinalAirspeedWithin { min_mps, max_mps } => {
            let last = log.last().map_or(0.0, |s| s.airspeed_mps);
            AssertionResult {
                assertion,
                passed: last >= min_mps && last <= max_mps,
                message: format!(
                    "final airspeed {last:.2} m/s (range [{min_mps:.2}, {max_mps:.2}])"
                ),
            }
        },
        Assertion::MinAirspeedNearTarget {
            target_mps,
            tol_mps,
        } => {
            let min_v = log
                .iter()
                .map(|s| s.airspeed_mps)
                .fold(f32::INFINITY, f32::min);
            AssertionResult {
                assertion,
                passed: (min_v - target_mps).abs() <= tol_mps,
                message: format!(
                    "min airspeed {min_v:.2} m/s (target {target_mps:.2}, tol {tol_mps:.2})"
                ),
            }
        },
        Assertion::FinalFuelWithin { min_gal, max_gal } => {
            let last = log.last().map_or(0.0, |s| s.fuel_remaining_gal);
            AssertionResult {
                assertion,
                passed: last >= min_gal && last <= max_gal,
                message: format!("final fuel {last:.2} gal (range [{min_gal:.2}, {max_gal:.2}])"),
            }
        },
    }
}

/// Serialize a scenario log to CSV text (including header).
pub fn log_to_csv(log: &[BenchSample]) -> String {
    let mut out = String::with_capacity(log.len() * 200);
    out.push_str(BenchSample::csv_header());
    out.push('\n');
    for sample in log {
        sample.write_csv_row(&mut out);
    }
    out
}

// ================================================================================
// Beaver scenarios. Each entry validates a different aspect of the flight envelope
// against DHC-2 POH figures (converted to SI). Add more as feel-tuning uncovers
// new tolerances to lock.
// ================================================================================

/// Path to the Beaver spec, relative to workspace root.
pub const BEAVER_SPEC_PATH: &str = "data/aircraft/northlander.ron";

/// A small nose-up initial orientation aligning roughly with cruise α. Helps
/// scenarios start near trim rather than transient.
///
/// 3° nose-up around body +X.
const CRUISE_ATTITUDE: Quat = Quat::from_xyzw(0.026_177_19, 0.0, 0.0, 0.999_657_4);

pub const BEAVER_SCENARIOS: &[FlightScenario] = &[
    // beaver_cruise_sl
    //
    // Real-world maneuver:
    //   Level cruise, wings level, coordinated, steady airspeed. Trimmed hands-off
    //   flight stand-in (no trim system ships pre-Phase 3; scripted stick trims
    //   emulate it). Tests force balance at cruise α and cross-axis coupling
    //   cancellation (torque roll + P-factor + α-stability).
    //
    // POH target:
    //   Published cruise is 125 kt (64.3 m/s) at 65% power. Our current drag polar
    //   (Cd0 = 0.045) under-predicts parasite drag, so 65% power equilibrates well
    //   above 64 m/s and is not suitable for "level cruise" testing today. Instead
    //   we test at ~55 m/s with throttle reduced to thrust-drag equilibrium. When
    //   drag polar tunes to POH, throttle rises toward 0.65 and this assertion
    //   tightens to target 64.3 m/s.
    //
    // Isolation (held constant):
    //   - Sea level (500 m MSL to avoid ground-clamp transients).
    //   - Zero wind (Phase 2 sim has no wind anyway).
    //   - MTOW, flaps UP, no fuel-transfer CG effects.
    //   - Scripted pitch/roll/yaw trim inputs stand in for a trim system (no trim
    //     ships until Phase 3+). Pilot would normally use trim wheels to null
    //     steady-state stick forces.
    //   - Throttle set for thrust-drag equilibrium at ~55 m/s (currently 0.55 in
    //     our model; POH would say ~0.65). Tracks to POH as Cd0 tunes.
    //
    // Real-world fidelity:
    //   Faithful: pilot actually would hold this approximate trim state. Directions
    //     of all inputs match real-world technique (right rudder for torque/P-factor,
    //     slight nose-up trim for cruise α, trace right aileron for torque roll).
    //   Approximated: SetPitch/SetRoll/SetYaw constants stand in for trim-wheel
    //     positions that a real pilot would set and release. Net elevator/aileron/
    //     rudder deflections at equilibrium match what trim would produce.
    //   Magnitudes: rudder at 0.03 matches analytic P-factor balance (~1570 N·m
    //     nose-left at cruise α=3°, thrust ~3000 N; rudder authority at cruise q
    //     gives ~47100 N·m per unit input, so 0.033 nulls it — we set 0.03).
    //     Real Beaver pilots report "modest right rudder at cruise," consistent.
    //   Throttle 0.55 is lower than POH's 0.65 at cruise because our Cd0 is light;
    //     tracks toward POH as drag-polar tunes (documented above).
    //
    // Does NOT test:
    //   - Phugoid damping (separate scenario: decel disturbance + recovery).
    //   - Gust response (blocked: Phase 3.2 weather).
    //   - Turn coordination (separate: *_coordinated_turn_30deg).
    //   - Published cruise airspeed (blocked: drag-polar coefficient tune).
    //
    // Assertion rationale:
    //   AltitudeStableAtEnd pins the primary behavior (altitude hold). AirspeedAtEnd
    //   pins the secondary equilibrium (coupled to throttle/drag balance) so drift
    //   into a different part of the envelope is caught. NeverStalls is a free
    //   sanity check from the captured log.
    FlightScenario {
        name: "beaver_cruise_sl",
        description: "Level cruise at SL, trimmed hands-off wings-level coordinated flight.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 500.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -55.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 0.55,
            flaps: FlapPosition::Up,
            fuel_gal: 80.0,
        },
        inputs: &[
            // Trim stand-ins. Scripted constants until the trim system ships.
            // Values tuned empirically from bench feedback:
            //   pitch 0.05 nulls α-stability's restoring moment at cruise α.
            //   roll  0.005 nulls torque-roll reaction from the right-rotating prop.
            //   yaw   0.03 nulls P-factor (nose-left at +α with cruise thrust).
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.05),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetRoll(0.005),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetYaw(0.03),
            },
        ],
        duration_ticks: 3600, // 30 s
        assertions: &[
            Assertion::AltitudeStableAtEnd {
                last_secs: 5.0,
                tol_m: 10.0, // measured drift ~3 m; narrow to 5 m when Cm_α tune closes Gap 1
            },
            Assertion::AirspeedAtEnd {
                last_secs: 5.0,
                target_mps: 56.5, // empirical equilibrium at throttle 0.55; tracks toward 64.3 as Cd0 closes Gap 1
                tol_mps: 1.0,     // measured diff 0.14; ±1 catches meaningful drag-polar regression
            },
            Assertion::NeverStalls,
        ],
    },
    // beaver_cruise_climb
    //
    // Real-world maneuver:
    //   Full-throttle climb at cruise airspeed ("cruise climb" profile -- the
    //   climb regime most general-aviation pilots actually use, since holding Vy
    //   nose-up degrades visibility over the cowl). Tests steady climb rate
    //   produced by excess thrust over cruise drag.
    //
    // POH target:
    //   Beaver at full power at cruise speed (~55 m/s) produces climb on the order
    //   of 500-800 fpm (2.5-4 m/s) per POH composite figures; published Vy ROC is
    //   1000 fpm at 38 m/s, this profile trades some ROC for speed/visibility.
    //   DHC-2 POH § 5 Performance.
    //
    // Model note:
    //   Targets the model's current output (~6-8 m/s at cruise speed full power)
    //   rather than POH; tightens toward POH as drag polar tunes. Vy-specific
    //   climb (held at 38 m/s) is blocked on trim robustness at low V / high α,
    //   where P-factor and torque overwhelm the small-deflection trim stand-in;
    //   revisit with a proper trim system.
    //
    // Isolation (held constant):
    //   - Sea level (500 m MSL start; climbs ~200 m in 30 s, well clear of
    //     ground-clamp and density-altitude transitions).
    //   - Zero wind.
    //   - MTOW, flaps UP.
    //   - Full-throttle trim scaled from cruise-trim values by power ratio
    //     (1.0/0.55 ≈ 1.8× for roll torque and P-factor):
    //       pitch 0.05 (same as cruise -- climb at cruise speed doesn't need more
    //                   back-pressure since α stays near cruise; plane climbs via
    //                   excess thrust, not via changed attitude).
    //       roll  0.009 (~1.8× cruise 0.005).
    //       yaw   0.055 (~1.8× cruise 0.03).
    //
    // Real-world fidelity:
    //   Faithful: full throttle, right rudder, right aileron, slight back-pressure
    //     all match real cruise-climb technique. Directions all correct.
    //   Approximated: constant scripted inputs stand in for trim-wheel positions
    //     (as in cruise). Real pilot sets trim and releases stick; net control
    //     surface deflection at equilibrium is what our inputs produce.
    //   Magnitudes -- KNOWN FIDELITY GAP: real Beaver pilots at full power
    //     typically use 20-40% right rudder (especially near Vy); we set 5.5%.
    //     This reflects our model's P-factor magnitude at cruise α=3° being
    //     relatively mild (~2880 N·m at 5500 N thrust, 3° α). Real P-factor at
    //     higher α in a real climb would be stronger. Possible causes: (a) our
    //     P_FACTOR_ARM_M=10m may be light, (b) the coupling-airspeed-ramp
    //     (5-20 m/s) may still be damping at 55 m/s, (c) real aircraft need more
    //     rudder for slipstream-over-rudder effects we don't model. Flagged in
    //     symptom-map.md as a coefficient-tuning item.
    //   We do NOT inflate the rudder trim to "look right" -- the value matches
    //     what the model physically needs. Fixing this requires coefficient work
    //     or new physics (slipstream), not scenario fudging.
    //
    // Does NOT test:
    //   - Vy-specific ROC (blocked on trim robustness at Vy α).
    //   - Vx climb angle (separate scenario when trim system lands).
    //   - Service ceiling (separate: *_ceiling, requires ρ-derate).
    //   - Engine-out climb (N/A for single-engine).
    //
    // Assertion rationale:
    //   ClimbRateAtEnd over last 5 s -- by t=25 s the aircraft is in quasi-steady
    //   cruise climb. Target reflects current model; narrows as Cd tunes.
    //   AirspeedAtEnd pins the climb profile so this stays a "cruise climb," not
    //   a Vy climb; if speed drifts way off cruise, the test no longer isolates
    //   what it claims to.
    FlightScenario {
        name: "beaver_cruise_climb",
        description: "Full-throttle climb at cruise airspeed. Tests quasi-steady ROC.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 500.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -55.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 1.0,
            flaps: FlapPosition::Up,
            fuel_gal: 80.0,
        },
        inputs: &[
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.05),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetRoll(0.009),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetYaw(0.055),
            },
        ],
        duration_ticks: 3600, // 30 s
        assertions: &[
            Assertion::ClimbRateAtEnd {
                last_secs: 5.0,
                target_mps: 7.0, // measured 7.26; target tracks toward POH 5.08 as Cd_induced closes Gap 1
                tol_mps: 1.0,    // measured diff 0.26; ±1 catches ~15% ROC regression
            },
            Assertion::AirspeedAtEnd {
                last_secs: 5.0,
                target_mps: 56.0, // measured 56.7; equilibrium slightly above cruise
                tol_mps: 1.5,     // measured diff 0.69; ±1.5 catches meaningful regression
            },
            Assertion::NeverStalls,
        ],
    },
    // beaver_ldmax_glide
    //
    // Real-world maneuver:
    //   Power-off glide at best-glide airspeed (L/D_max). From cruise, pilot
    //   reduces throttle to idle and trims nose-down to hold best-glide speed.
    //   Aircraft descends at a steady flight-path angle equal to the glide
    //   angle γ = arctan(D/L) ≈ 1/(L/D_max).
    //
    // POH target:
    //   Best glide speed ≈ 70 mph = 31.3 m/s. L/D ≈ 10 for a clean Beaver.
    //   DHC-2 POH § 3 Emergency Procedures / § 5 Performance (Glide chart).
    //
    // Model analytic:
    //   L/D_max at Cl_opt = sqrt(Cd0 · π · AR · e) = sqrt(0.045 · π · 9.2 · 0.75)
    //   ≈ 0.988. Our model predicts V_best_glide = sqrt(2W/(ρS·Cl_opt)) ≈ 40 m/s,
    //   L/D ≈ 11, sink rate ≈ 3 m/s. The gap from POH's 31 m/s/10:1 traces to
    //   Gap 1 (Cd0 light) -- lower Cd0 pushes best-glide V upward because less
    //   drag at low Cl lets aircraft find equilibrium at higher speed. This
    //   scenario is a second data point that constrains the Cd0 tuning.
    //
    // Isolation (held constant):
    //   - Altitude 1500 m MSL (allows 30 s of ~3 m/s descent without ground clamp).
    //   - Zero wind, MTOW, flaps UP.
    //   - Throttle 0.0 (idle); small residual thrust from engine idle 17 kW.
    //   - Scripted pitch trim holds α near 7° (model's L/D_max α).
    //   - Roll/yaw trim near zero (idle = negligible torque and P-factor).
    //
    // Real-world fidelity:
    //   Faithful: idle throttle matches real power-off glide technique. Pitch
    //     input 0.12 derived analytically from δ = (|Cm_α|/Cm_δe)·α_rad at α = 7°.
    //     Matches what a real pilot would apply via elevator trim.
    //   Approximated: scripted constant pitch. Real pilot continuously adjusts to
    //     hold indicated airspeed; our constant value produces a stable glide at
    //     whatever airspeed the model's coefficients equilibrate at (~40 m/s).
    //   Known fidelity gap: model's best-glide V ≈ 40 m/s vs POH 31 m/s; traces
    //     to Gap 1 (Cd0 light) in PROGRESS.md. Closing Gap 1 brings V down toward
    //     POH. Scenario assertions target model's current equilibrium; tighten
    //     toward POH as Cd0 tunes.
    //
    // Does NOT test:
    //   - Post-flameout glide (engine damage) — no engine-failure model yet.
    //   - Gliding with flaps deployed (separate: *_landing_approach).
    //   - Glide-to-landing touchdown (blocked on 2.5 contact physics).
    //   - Slipping glide / forward slip (separate scenario with rudder input).
    //
    // Assertion rationale:
    //   AirspeedAtEnd pins best-glide speed (model's prediction). ClimbRateAtEnd
    //   with negative target asserts we're sinking at the expected rate.
    //   NeverStalls is free sanity. Narrow targets toward POH as Cd0 tunes.
    FlightScenario {
        name: "beaver_ldmax_glide",
        description: "Power-off glide at L/D_max. Tests drag polar minimum + steady glide.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 1500.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -40.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 0.0,
            flaps: FlapPosition::Up,
            fuel_gal: 80.0,
        },
        inputs: &[
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.12),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetRoll(0.001),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetYaw(0.005),
            },
        ],
        duration_ticks: 7200, // 60 s — needed because phugoid period is ~25 s in this config
        assertions: &[
            // Averaging over last 15 s spans more than half a phugoid cycle so the
            // mean is close to the oscillation center.
            Assertion::AirspeedAtEnd {
                last_secs: 15.0,
                target_mps: 44.0, // mean of phugoid; tracks toward POH 31 as Cd0 tunes
                tol_mps: 5.0,
            },
            Assertion::ClimbRateAtEnd {
                last_secs: 15.0,
                target_mps: -3.5, // mean sink across phugoid; analytic ~3.1 m/s at L/D_max
                tol_mps: 2.0,
            },
            Assertion::NeverStalls,
        ],
    },
    // beaver_vne_dive
    //
    // Real-world maneuver:
    //   Throttle-managed pushover dive. From cruise, pilot reduces throttle and
    //   commits nose-down attitude. Aircraft accelerates toward terminal dive
    //   speed where drag balances gravity component along velocity. Pilot
    //   responsibility is to not exceed Vne (structural airspeed limit).
    //
    // POH target:
    //   Vne = 160 mph = 71.5 m/s. Never exceed. A 10° idle-throttle dive should
    //   equilibrate near Vne without overshooting it structurally. DHC-2 POH
    //   § 2 Limitations (Vne), § 5 Performance (Dive recovery).
    //
    // Model analytic:
    //   Idle thrust ~300 N at dive speed; terminal airspeed V_dive where
    //   D(V_dive) = W·sin(γ). For γ = 10°: D_required ≈ 3948 N. With our model's
    //   Cd at cruise α ≈ 0.049, analytic V_dive ≈ 75 m/s. Slightly above Vne,
    //   reflects Gap 1 (Cd0 light). Scenario tests bounded behavior, not exact
    //   Vne match.
    //
    // Isolation (held constant):
    //   - Altitude 2500 m MSL (enough altitude for 30 s dive at V·sin(γ) ≈ 13 m/s
    //     descent rate — ~400 m loss, end at ~2100 m).
    //   - Zero wind, MTOW, flaps UP.
    //   - Throttle 0.0 (idle) — pilot cut power to avoid overspeed.
    //   - Scripted nose-down pitch input to commit to dive, then hold.
    //   - Roll/yaw trim near zero (idle power, low coupling).
    //
    // Real-world fidelity:
    //   Faithful: idle throttle matches cautious-pilot dive technique.
    //     Nose-down pitch input is the actual control action used to enter a dive.
    //   Approximated: scripted constant SetPitch(-0.08). Real pilot would vary
    //     pitch to maintain target dive angle and limit speed — our constant
    //     produces whatever equilibrium the aerodynamics allow.
    //   Known fidelity gap: model's terminal dive speed overshoots POH Vne
    //     slightly (Gap 1, Cd0 light). Scenario accommodates with assertion
    //     band [65, 85]; narrows toward [70, 73] as Cd0 tunes.
    //
    // Does NOT test:
    //   - Vne recovery (pulling out of dive before structural damage).
    //   - Full-throttle dive (the overspeed scenario — dangerous, tests hard limits).
    //   - Spin entry from accelerated-stall at dive speed (separate).
    //   - Wing flutter / aeroelastic effects (not modeled).
    //
    // Assertion rationale:
    //   FinalAirspeedWithin bounded range catches: (a) aircraft failing to
    //   accelerate past cruise (drag polar wrong); (b) aircraft over-accelerating
    //   to ridiculous speeds (Cd0 catastrophically missing high-speed term).
    //   NeverStalls ensures the dive doesn't weirdly stall (couldn't physically,
    //   but catches model bugs).
    FlightScenario {
        name: "beaver_vne_dive",
        description: "Idle-throttle nose-down dive. Tests high-speed drag envelope + Vne behavior.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 2500.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -55.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 0.0,
            flaps: FlapPosition::Up,
            fuel_gal: 80.0,
        },
        inputs: &[
            // Real pushover technique: brief forward push to establish dive
            // angle, then release stick. A constant nose-down input would keep
            // pitching — no steady-state exists for constant negative elevator
            // because q·Cm_δe grows quadratically with airspeed. We simulate
            // real pilot behavior: pulse the stick, then release to neutral,
            // letting α-stability settle the aircraft into whatever dive angle
            // matches neutral α.
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(-0.05),
            },
            ScenarioInput {
                tick: 120,                        // 1 s pushover
                event: InputEvent::SetPitch(0.0), // release to neutral
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetRoll(0.001),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetYaw(0.005),
            },
        ],
        duration_ticks: 1800, // 15 s — short enough aircraft doesn't overspeed unrealistically
        assertions: &[
            Assertion::FinalAirspeedWithin {
                min_mps: 60.0, // must have accelerated past cruise
                max_mps: 95.0, // must stay below catastrophic overspeed; narrows toward POH Vne 71.5 as Cd0 tunes and a proper dive-hold input shipsets
            },
            Assertion::NeverStalls,
        ],
    },
    // beaver_stall_clean
    //
    // Real-world maneuver:
    //   Power-off stall recognition: from cruise, pilot reduces power to idle and
    //   gradually applies back-pressure to maintain altitude as airspeed bleeds off.
    //   Aircraft decelerates at near-constant altitude until α passes the stall
    //   threshold; wing stalls at Vs. Pilot releases back-pressure to recover.
    //
    // POH target:
    //   Vs (clean) = 52 mph = 23.25 m/s, MTOW, SL, 1 g.
    //   DHC-2 POH § 3 Emergency Procedures / § 5 Performance (Stall Speed chart).
    //
    // Isolation (held constant):
    //   - Start at 1000 m MSL (enough altitude to complete the decel + stall +
    //     post-stall recovery without ground-clamp interference).
    //   - Zero wind.
    //   - MTOW, flaps UP ("clean" stall, not Vso).
    //   - Idle throttle (0.0) so thrust term is near-zero; decel is pure drag.
    //   - Modest scripted back-pressure: enough to keep α rising as V drops, not
    //     so much that aircraft zoom-climbs into stall.
    //   - Trim inputs for roll/yaw are reduced from cruise because at idle the
    //     P-factor and torque reaction are near-zero.
    //
    // Real-world fidelity:
    //   Faithful: throttle 0.0 (idle), near-zero roll/yaw trim (because at idle
    //     P-factor and torque-roll are both near zero), altitude 1000 m MSL for
    //     safe decel + recovery envelope. All match real-world technique.
    //     The staged pitch ramp (7 small steps, 0.05 → 0.30) produces an
    //     approach-to-stall trajectory that matches the real procedure:
    //       - First ~6 s: near-level decel (small ±1 m altitude band).
    //       - Next ~6 s: gradual pitch-up, mild zoom (~40 m altitude gain) as
    //         back-pressure rises faster than the aircraft re-trims.
    //       - Stall break at α past threshold, recovery via post-stall pitch-down.
    //     Total zoom is ~40 m vs the ~86 m zoom of the previous constant-input
    //     version -- meaningfully closer to the near-level decel of real technique.
    //   Approximated: real pilot applies CONTINUOUS back-pressure; we approximate
    //     with 7 discrete SetPitch events at 2-s intervals. Each jump produces a
    //     small pitch transient at the event boundary. Smaller/more-frequent
    //     steps would reduce these transients but with diminishing returns.
    //   Magnitudes: step values derive analytically from the α-stability /
    //     elevator-authority balance δ_pitch = (|Cm_α|/Cm_δe) · α_rad, where α
    //     for level flight is set by Cl = W/(½ρV²S). No inflation beyond what
    //     the physics requires; values progress with airspeed as a pilot would.
    //   Known coefficient gap (not a scenario fudge): the model stalls at
    //     V ≈ 29 m/s (consistent with Cl_max = 1.65 at MTOW by Vs² = 2W/(ρSCl_max)),
    //     while the POH reports Vs = 23.25 m/s. Resolving this requires tuning
    //     the Cl table peak upward or reviewing the POH measurement conditions
    //     (power-on vs power-off, actual flaps position during measurement).
    //     Flagged in symptom-map.md. Assertion tolerance (8 m/s) accommodates
    //     the gap for now; tightens when Cl table is reconciled to POH.
    //   Planned future refinement: a ramped-input event type (continuous change
    //     over N ticks) would replace the staged steps, eliminating transient
    //     boundary effects entirely.
    //
    // Does NOT test:
    //   - Accelerated (G-loaded) stall (separate: *_accelerated_stall).
    //   - Flap-down stall / Vso (separate: *_stall_landing_flaps).
    //   - Power-on stall (separate: *_stall_power_on).
    //   - Spin entry / autorotation (post-2.4 flight model work).
    //   - Stall recovery time (separate assertion variant).
    //
    // Assertion rationale:
    //   EventuallyStalls verifies the scenario did actually stall -- without this
    //   MinAirspeedNearTarget could pass trivially if the aircraft just slowed
    //   without reaching stall α. MinAirspeedNearTarget pins the speed at which
    //   stall occurred, which is the primary behavior under test (matches POH
    //   Vs). Tolerance is wide until α-based stall, Cl table, AND staged-input
    //   refinement all land.
    FlightScenario {
        name: "beaver_stall_clean",
        description: "Power-off decel-to-stall from cruise, clean config. Min speed near Vs.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 1000.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -55.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 0.0,
            flaps: FlapPosition::Up,
            fuel_gal: 80.0,
        },
        inputs: &[
            // Staged back-pressure mimicking real-pilot technique: smoothly pull
            // back to hold altitude as airspeed decays. Analytic basis:
            //   Level flight requires elevator to null α-stability:
            //     δ_pitch = (Cm_α / Cm_δe) · α_rad  ≈  1.04 · α_rad
            //   where α rises as V drops to maintain L = W. At our coefficients
            //   (Cm_α = -0.25, Cm_δe = 0.24) and Beaver mass/wing/Cl, the sequence
            //   V=55/α=3° → V=45/α=5° → V=40/α=9° → V=35/α=14° → V=30/α=16°(stall)
            //   maps to pitch input 0.05 → 0.09 → 0.16 → 0.25 → 0.29.
            // Scripted as 7 small steps at 2-s intervals. Smaller steps reduce
            // the pitch-up transient at each event (larger steps produce zoom
            // climbs as the pilot-analog "over-pulls" relative to instantaneous
            // airspeed). Matches a real pilot's continuous, gentle back-pressure.
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.05),
            },
            ScenarioInput {
                tick: 240, // 2 s
                event: InputEvent::SetPitch(0.08),
            },
            ScenarioInput {
                tick: 480, // 4 s
                event: InputEvent::SetPitch(0.11),
            },
            ScenarioInput {
                tick: 720, // 6 s
                event: InputEvent::SetPitch(0.14),
            },
            ScenarioInput {
                tick: 960, // 8 s
                event: InputEvent::SetPitch(0.18),
            },
            ScenarioInput {
                tick: 1200, // 10 s
                event: InputEvent::SetPitch(0.23),
            },
            ScenarioInput {
                tick: 1440, // 12 s
                event: InputEvent::SetPitch(0.30),
            },
            // Near-zero thrust means near-zero P-factor and torque-roll; small
            // trims are enough to keep wings level during the decel.
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetRoll(0.001),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetYaw(0.005),
            },
        ],
        duration_ticks: 2400, // 20 s
        assertions: &[
            Assertion::EventuallyStalls,
            Assertion::MinAirspeedNearTarget {
                target_mps: 23.25, // POH Vs_clean
                // Wide tolerance reflects PROGRESS.md § Bench-identified gap 3
                // (Cl_max too low for POH Vs). Measured min airspeed ~29 m/s
                // (diff ~5.7 from target). Do NOT narrow until Gap 3 resolves --
                // narrowing here would mask the coefficient gap.
                tol_mps: 8.0,
            },
        ],
    },
    // beaver_stall_flaps_landing
    //
    // Real-world maneuver:
    //   Power-off stall with full LANDING flaps. Approach / landing-regime stall
    //   recognition. Same technique as clean stall (idle throttle, progressive
    //   back-pressure) but in landing configuration; aircraft stalls at Vso
    //   (POH 47 mph = 21.01 m/s), lower than Vs_clean due to ΔCl_max from
    //   flap deployment.
    //
    // POH target:
    //   Vso = 47 mph = 21.01 m/s, MTOW, SL, 1 g, LANDING flap.
    //   DHC-2 POH § 5 Performance (Stall Speed chart, flaps-down column).
    //
    // Model analytic:
    //   Cl_max_flapped = Cl_max_clean + Δ Cl_landing = 1.65 + 0.37 = 2.02.
    //   Vs_flapped = sqrt(2W/(ρSCl_max_flapped)) ≈ 28.1 m/s. The gap to POH Vso
    //   is smaller than the clean-Vs gap (Gap 3) because the additive ΔCl brings
    //   our total closer to POH-implied Cl_max. This scenario constrains the
    //   flap ΔCl term separately from Cl_max_clean.
    //
    // Isolation (held constant):
    //   - Same setup as beaver_stall_clean: 1000 m MSL, 55 m/s cruise, idle
    //     throttle, 80 gal fuel. Only flaps change.
    //   - LANDING flap detent deployed from tick 0 (aircraft enters scenario
    //     already in approach configuration).
    //
    // Real-world fidelity:
    //   Faithful: idle throttle (real approach-stall technique), staged back-
    //     pressure mimicking continuous pilot pull, LANDING flaps (the approach
    //     config where this stall would really happen).
    //   Approximated: pitch input steps shifted upward by ~0.08 compared to
    //     clean stall because the flap Δ Cm = -0.030 nose-down pitching moment
    //     requires additional elevator to hold same α (Δδ = 0.030/Cm_δe = 0.125).
    //   Known fidelity gap: same Cl_max-class gap as clean stall (Gap 3); wide
    //     tolerance accommodates, narrows as Cl_max tunes.
    //
    // Does NOT test:
    //   - Accelerated / G-loaded stall (separate).
    //   - Clean stall (that's beaver_stall_clean).
    //   - Flap-over-speed (Vfe — never-exceed-flaps; separate scenario).
    //   - Power-on flaps stall (go-around scenario; separate).
    //
    // Assertion rationale:
    //   EventuallyStalls verifies α reaches threshold. MinAirspeedNearTarget
    //   pins Vso; wide tolerance while Gap 3 open.
    FlightScenario {
        name: "beaver_stall_flaps_landing",
        description: "Power-off stall with LANDING flaps. Min speed near Vso = 21.01 m/s.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 1000.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -45.0), // slower start since landing approach is slower
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 0.0,
            flaps: FlapPosition::Landing,
            fuel_gal: 80.0,
        },
        inputs: &[
            // Staged back-pressure, shifted up by ~0.08 vs clean stall to
            // counter flap Δ Cm = -0.030 nose-down moment.
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.13),
            },
            ScenarioInput {
                tick: 240, // 2 s
                event: InputEvent::SetPitch(0.16),
            },
            ScenarioInput {
                tick: 480, // 4 s
                event: InputEvent::SetPitch(0.19),
            },
            ScenarioInput {
                tick: 720, // 6 s
                event: InputEvent::SetPitch(0.22),
            },
            ScenarioInput {
                tick: 960, // 8 s
                event: InputEvent::SetPitch(0.26),
            },
            ScenarioInput {
                tick: 1200, // 10 s
                event: InputEvent::SetPitch(0.31),
            },
            ScenarioInput {
                tick: 1440, // 12 s
                event: InputEvent::SetPitch(0.38),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetRoll(0.001),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetYaw(0.005),
            },
        ],
        duration_ticks: 2400, // 20 s
        assertions: &[
            Assertion::EventuallyStalls,
            Assertion::MinAirspeedNearTarget {
                target_mps: 21.01, // POH Vso
                // Same Gap 3 concern as clean stall: Cl_max too low means Vs
                // predicted ~28 m/s not POH 21. Wide tolerance until Cl_max
                // tunes.
                tol_mps: 8.0,
            },
        ],
    },
    // Coordinated-turn scenarios (beaver_coordinated_turn_30, _60) are DEFERRED
    // post-2.4.8. Attempted in development and hit a structural barrier: our
    // flight model lacks Cl_β (dihedral roll stability), so a banked aircraft
    // has no restoring roll moment. With only scripted constant inputs and no
    // trim system, bank angle spirals rather than locking at a target. The
    // aircraft physically executes the turn (yaw rate, altitude loss, load
    // factor all compute correctly) but the bank angle at steady state can't
    // be pinned to 30° or 60° precisely.
    //
    // Unblocking options (pick when feel-tuning next returns to rotational):
    //   (a) Add Cl_β (dihedral) coefficient → aircraft self-stabilizes in bank,
    //       simple SetRoll inputs hold target bank. Cleanest physics fix.
    //   (b) Ship a trim system (Phase 3+) → pilot-trim-wheel inputs can hold
    //       bank without needing dihedral stability.
    //   (c) Add a `SetRoll_ramped` scripted input type that continuously adjusts
    //       input over time → can simulate a pilot actively holding bank.
    //
    // Not blocking Phase 2.5 start: coefficient tuning has 7 existing scenarios
    // for constraints (cruise, climb, glide, dive, 2 stalls, endurance) — plenty.
    // Turn scenarios add value but aren't structurally required. Documented as
    // deferred in PROGRESS.md § Path to Phase 2.5, behavior-taxonomy.md, and
    // ROADMAP 2.4.8 Step 1.
    //
    // beaver_endurance_cruise
    //
    // Real-world maneuver:
    //   Extended cruise segment. Validates fuel burn rate at cruise power and
    //   that trim remains stable over longer durations than the other scenarios.
    //
    // POH target:
    //   Cruise fuel burn = 27 gph at 65% power.
    //   Full-tank endurance = 95 gal / 27 gph = 3.5 hours = 12600 s.
    //   Range at cruise = 455 nm = 843 km.
    //   DHC-2 POH § 5 Performance (Cruise Performance chart).
    //
    // Model analytic:
    //   At throttle 0.55: engine_kw = 20 + 315·0.55 = 193 kW.
    //   Fuel flow = 193 · 0.124 gph/kW = 23.9 gph.
    //   POH 27 gph → model 23.9 gph = ~88% of POH rate. Gap is coupled to Gap 1
    //   (Cd0 light → less power needed for cruise → lower burn). Closing Gap 1
    //   raises throttle, raises power, raises burn toward POH.
    //
    // Isolation (held constant):
    //   - Identical trim + config to beaver_cruise_sl (throttle 0.55, pitch 0.05,
    //     roll 0.005, yaw 0.03). Only difference: smaller starting fuel + longer
    //     duration, to expose the burn rate as a finite-change signal.
    //   - 60 s duration with 10 gal starting fuel exposes ~0.4 gal burn window;
    //     large enough to catch rate errors > ~5%, small enough that scenario
    //     isn't wasteful in test time.
    //
    // Real-world fidelity:
    //   Faithful: identical to cruise_sl setup, just longer duration. A real
    //     pilot "just cruises" for this duration — no pilot input changes.
    //   Magnitudes: trim values analytically matched (see cruise_sl fidelity
    //     block). Fuel flow computed from the same engine model as a real
    //     cruise leg.
    //   Known fidelity gap: burn rate 23.9 gph vs POH 27 gph, traces to Gap 1
    //     (Cd0 light means less throttle holds cruise, less power, less burn).
    //     Target final-fuel band accommodates model's current burn rate; narrows
    //     as Cd0 tunes.
    //
    // Does NOT test:
    //   - Range (requires ground-track integration, not currently asserted).
    //   - High-altitude endurance (would need density altitude + higher altitude start).
    //   - Fuel exhaustion / flameout transition (separate scenario when needed).
    //   - Cruise at POH-correct 65% throttle (blocked on Gap 1 closure).
    //
    // Assertion rationale:
    //   AltitudeStableAtEnd + AirspeedAtEnd: confirms trim still holds at 60 s
    //   (transients fully decayed, long-term stability demonstrated).
    //   FinalFuelWithin: validates burn rate. At 23.9 gph model rate and 60 s
    //   duration, starting 10 gal → end ~9.60 gal. Band [9.35, 9.75] = ±0.2 gal
    //   tolerance catches > ~50% rate deviation; narrows as Gap 1 tunes.
    FlightScenario {
        name: "beaver_endurance_cruise",
        description: "Extended cruise segment. Tests fuel burn rate + long-term trim stability.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 500.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -55.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 0.55,
            flaps: FlapPosition::Up,
            fuel_gal: 10.0, // smaller than cruise_sl so burn rate shows as finite delta
        },
        inputs: &[
            // Identical trim to beaver_cruise_sl.
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.05),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetRoll(0.005),
            },
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetYaw(0.03),
            },
        ],
        duration_ticks: 7200, // 60 s
        assertions: &[
            Assertion::AltitudeStableAtEnd {
                last_secs: 5.0,
                tol_m: 30.0, // wider than cruise_sl because 60 s vs 30 s integrates more drift
            },
            Assertion::AirspeedAtEnd {
                last_secs: 5.0,
                target_mps: 56.5,
                tol_mps: 3.0, // slightly wider than cruise_sl for the longer window
            },
            Assertion::FinalFuelWithin {
                min_gal: 9.35, // 23.9 gph · 60s = 0.398 gal burned; band accommodates ± 0.2 gal
                max_gal: 9.75, // narrows toward model's exact rate when Gap 1 tunes
            },
            Assertion::NeverStalls,
        ],
    },
];
