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
    // Cruise at ~55 m/s with moderate throttle. Pilot holds ~5% back-stick (typical
    // elevator trim for a hands-on cruise). Should hold altitude within a
    // reasonable band and not stall. 30-second run.
    FlightScenario {
        name: "beaver_cruise_sl",
        description: "Level cruise at sea level, moderate throttle, light back-stick trim.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 500.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -55.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 0.65,
            flaps: FlapPosition::Up,
            fuel_gal: 80.0,
        },
        inputs: &[
            // 5% back-stick to hold cruise α against α-stability. Scenario-level
            // "trim" until a trim system ships.
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.05),
            },
        ],
        duration_ticks: 3600, // 30 s
        assertions: &[
            Assertion::AltitudeStableAtEnd {
                last_secs: 5.0,
                tol_m: 100.0, // wide until 2.4.8 tune
            },
            Assertion::NeverStalls,
        ],
    },
    // Climb at Vy (~38 m/s) with full throttle. Pilot holds moderate back-stick.
    // Beaver POH lists ~5 m/s (1000 fpm) ROC at MTOW; wide tolerance until
    // coefficient tuning dials it in.
    FlightScenario {
        name: "beaver_vy_climb",
        description: "Full-throttle climb at Vy. Verify positive ROC matches POH band.",
        aircraft_spec_path: BEAVER_SPEC_PATH,
        initial: ScenarioInitial {
            position: Vec3::new(0.0, 500.0, 0.0),
            velocity: Vec3::new(0.0, 0.0, -38.0),
            orientation: CRUISE_ATTITUDE,
            angular_velocity: Vec3::ZERO,
            throttle: 1.0,
            flaps: FlapPosition::Up,
            fuel_gal: 80.0,
        },
        inputs: &[ScenarioInput {
            tick: 0,
            event: InputEvent::SetPitch(0.10),
        }],
        duration_ticks: 3600, // 30 s
        assertions: &[Assertion::ClimbRateAtEnd {
            last_secs: 5.0,
            target_mps: 5.0,
            tol_mps: 5.0, // very wide until 2.4.8 tune
        }],
    },
    // Slow decel to stall with idle throttle. Record minimum airspeed -- should be
    // near Vs_clean = 23.25 m/s. Wide tolerance.
    FlightScenario {
        name: "beaver_stall_clean",
        description: "Decel to stall from cruise, clean config. Min airspeed near Vs_clean.",
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
            // Hold back-stick to keep α high as speed drops.
            ScenarioInput {
                tick: 0,
                event: InputEvent::SetPitch(0.30),
            },
        ],
        duration_ticks: 2400, // 20 s
        assertions: &[
            Assertion::EventuallyStalls,
            Assertion::MinAirspeedNearTarget {
                target_mps: 23.25,
                tol_mps: 8.0, // very wide -- refine during tune
            },
        ],
    },
];
