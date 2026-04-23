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
//! # Milestone ledger
//!
//! CLOSED items list the function / constant that carries the work. OPEN items name
//! the milestone that should close them. A careful reader should be able to
//! cross-reference any dynamics concern against this list.
//!
//! Closed (Phase 2.4.x shipped):
//!
//! CLOSED(2.4.2): Lift = `0.5 ρ V² S Cl(α)` via `lift_coefficient()`; `CL_TABLE`
//!   is a NACA 64A416-shaped piecewise-linear curve with whole-wing `Cl_max ≈ 1.65`.
//! CLOSED(2.4.2): Angle of attack + sideslip computed in
//!   `angle_of_attack_and_sideslip()` from body-frame velocity.
//! CLOSED(2.4.2b): Lift vector perpendicular to velocity, in the plane containing
//!   body-up (not body +Y). Closed out the high-α stall-cascade symptom from the
//!   body-+Y placeholder.
//! CLOSED(2.4.3): Thrust from R-985 power curve via `engine_power_kw()` +
//!   `prop_efficiency()` + `thrust_force_newtons()`. Fuel flow tracks with
//!   `fuel_flow_gph()`; `VehicleState.fuel_gal` depletes each tick.
//! CLOSED(2.4.4a): Keyboard input slew (`AXIS_SLEW_RATE` in `input_map.rs`).
//! CLOSED(2.4.4b): Moment-based rotational dynamics -- pitch / roll / yaw moments
//!   drive `angular_accel`, Euler-integrated in `tick_dynamics`.
//! CLOSED(2.4.4d): Real stall nose-down moment (`CM_STALL_PITCH_SLOPE`), wing drop
//!   (`wing_drop_roll_moment()`), α-based stall alert (`StallState`). Accelerated
//!   stall emerges from the model (fixed `Cl_max` + α-driven test).
//! CLOSED(2.4.5): Flaps -- `flap_aero()` returns per-position Cl / Cd0 / Cm deltas;
//!   `FlapPosition` on `Controls`, wired to F / C keys.
//! CLOSED(2.4.6): Ground effect -- `ground_effect_induced_drag_factor()` multiplies
//!   induced drag by Phillips factor inside 1 wingspan AGL.
//! CLOSED(2.4.6): Adverse yaw -- `CN_DELTA_A_ADVERSE` couples aileron input into the
//!   yaw-moment coupling slot; aileron stick → opposite-direction yaw.
//! CLOSED(2.4.6): P-factor -- `P_FACTOR_ARM_M` applied to thrust × sin(α) as a
//!   nose-left yaw moment (right-rotating prop). Strongest at high α + high power.
//! CLOSED(2.4.6): Torque roll -- `TORQUE_ROLL_NM_PER_KW` × shaft power → left-roll
//!   moment reaction in `roll_moment.coupling`.
//! CLOSED(2.4.7): Density altitude -- `air_density(h)` exponential ISA approximation
//!   (`ρ₀ · exp(-h/8500)`) feeds both `q = 0.5 ρ V²` and a linear engine-power derate
//!   by `ρ/ρ_0`. At 3 km `ρ/ρ_0` ≈ 0.71; at 5 km ≈ 0.56.
//!
//! Open (still to ship in Phase 2.4):
//!
//! TODO(2.4.8): Feel-tuning pass against published DHC-2 POH performance figures
//!   (cruise speed at published power, climb rate at Vy, stall speeds clean / flaps,
//!   takeoff distance, max range / endurance). Now that every effect is modeled,
//!   coefficients get a final reconciliation pass.
//!
//! Open (post-2.4):
//!
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
use crate::controls::FlapPosition;
use crate::vehicle::VehicleState;

/// Gravitational acceleration, world-space m/s².
pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

// Shared placeholder constants. Some stay (angular rates) until 2.4.4 replaces them
// with airspeed-scaled / stall-aware authority; others are replaced in 2.4.2 below
// by airfoil-driven force computation when an `AircraftSpec` is provided.

/// Legacy constant-thrust fallback (m/s²). Used on the no-spec path only, so the
/// pure-sim tests that don't load an `AircraftSpec` still produce sane numbers. The
/// airfoil path (2.4.3+) computes thrust from the R-985 power curve + prop efficiency
/// -- see `engine_power_kw` / `prop_efficiency` / `thrust_force_newtons` below.
const THRUST_ACCEL_MAX_FALLBACK: f32 = 3.5;

// --- Engine + prop model (Phase 2.4.3) ----------------------------------------
//
// Shaft power from the R-985 Wasp Junior radial. Sea-level static rating is 450 hp
// (~335 kW) at 2300 rpm; idle draw is ~6% of max. A detailed RPM + MAP simulation is
// out of scope for MVP -- we map throttle position linearly to available power
// between idle and max. Altitude density effects land with 2.4.7.
//
// Thrust is computed from power via `T = η_prop × P / V`. Prop efficiency is a
// piecewise-linear curve vs airspeed (0.3 static, peaking at 0.82 around cruise,
// falling to 0.55 past Vne). Below `V_MIN_FOR_THRUST` we clamp the denominator so
// static thrust doesn't diverge -- the clamp value is tuned so full-throttle static
// accel comes out near 3.6 m/s² (~9 kN / 2313 kg), matching the real Beaver's
// "P(hp) × 4.5 lbf" rule of thumb for static thrust.
//
// Fuel: burn is proportional to power via a constant BSFC (gph / kW). Cruise burn of
// 27 gph at 65% power gives `0.124 gph/kW`, which produces ~2.5 gph idle and ~42 gph
// full-throttle. Fuel state lives in `VehicleState.fuel_gal`; the tick decrements it
// each step. When fuel reaches zero, power drops to zero regardless of throttle
// position (flameout -- matches how a carb engine actually behaves).

/// Maximum sea-level shaft power (kW). 450 hp × 0.7457 kW/hp ≈ 335 kW.
const ENGINE_POWER_MAX_KW: f32 = 335.0;
/// Idle shaft power (kW). ~3% of max — calibrated against real R-985 idle behavior
/// (~8 hp / 6 kW shaft at 600-700 RPM). At idle the prop is mostly producing drag,
/// not thrust; the small residual thrust is enough for water-taxi creep but should
/// not produce meaningful in-flight acceleration. Previously 20 kW (6% of max),
/// which produced ~700 N idle thrust at low V — enough to slowly accelerate the
/// aircraft at low cruise speeds, more than real Beaver behavior.
const ENGINE_POWER_IDLE_KW: f32 = 10.0;

/// Minimum airspeed used as the denominator in `T = η·P/V`, to prevent thrust from
/// diverging as V → 0. Tuned so static thrust at max power ≈ 9 kN (Beaver rule-of-
/// thumb 2025 lbf for 450 hp at static).
const V_MIN_FOR_THRUST: f32 = 12.0;

/// Prop efficiency piecewise-linear keypoints `(airspeed_m_s, efficiency)`. Peaks
/// around cruise (55–70 m/s), falls off at both ends. Matches typical fixed-pitch
/// prop curves for a general-aviation installation.
#[allow(clippy::approx_constant)]
const PROP_EFF_TABLE: &[(f32, f32)] = &[
    (0.0, 0.30),
    (10.0, 0.40),
    (30.0, 0.70),
    (55.0, 0.82),
    (70.0, 0.82),
    (85.0, 0.65),
    (120.0, 0.45),
];

/// Brake specific fuel consumption as gallons-per-hour per kilowatt of shaft power.
/// Calibrated from the Beaver's published cruise burn (27 gph at 65% of 450 hp).
const BSFC_GPH_PER_KW: f32 = 0.124;

// --- Rotational dynamics (Phase 2.4.4): moment-based, not fiat rate.
//
// Each control surface produces an aerodynamic moment keyed to dynamic pressure `q`,
// the reference length for that axis, and a coefficient. Static stability terms pull
// α / β back toward trim (this is what recovers the "plane weathervanes toward the
// relative wind" behavior the fiat rate model was missing). Damping terms oppose
// angular velocity proportional to `ω * L / (2V)`, so the aircraft self-stabilizes at
// altitude and feels mushy at low speed. Each moment is divided by the aircraft's
// moment of inertia about that axis to give angular acceleration, then Euler-integrated
// in `state::tick_dynamics`.
//
// Physics-convention signs used internally:
//   M (pitch)  > 0  -> nose up
//   L (roll)   > 0  -> right wing down
//   N (yaw)    > 0  -> nose right
//
// Flowplan's body-frame storage negates Y (yaw) and Z (roll), so the moment-to-accel
// conversion negates on those axes. See the conversion site in `compute_forces`.

/// Moments of inertia about the three body axes (Beaver placeholder, kg·m²).
/// `pub(crate)` so sibling modules (`buoyancy`) can use the same values without
/// duplicating; migrates to `AircraftSpec` alongside the other per-aircraft
/// coefficients in the planned 2.4.x follow-up.
/// TODO(2.4.x): move to `AircraftSpec` so other aircraft tune independently.
pub(crate) const I_XX: f32 = 8000.0; // pitch (rotation about right-wing axis)
pub(crate) const I_YY: f32 = 12000.0; // yaw (rotation about up axis)
pub(crate) const I_ZZ: f32 = 5000.0; // roll (rotation about nose axis)

/// Reference lengths used in moment coefficients (Beaver placeholder, meters).
/// TODO(2.4.x): pull from `AircraftSpec` alongside wing area.
const MEAN_CHORD_M: f32 = 1.52;
const WING_SPAN_M: f32 = 14.63;

// TODO(2.4.3): velocity-scaled control authority will shift once `THRUST_ACCEL_MAX`
// is replaced with the R-985 power curve. Current coefficient tuning is against the
// constant-thrust placeholder; final values should be re-checked against the stable
// cruise / slow-flight / climb regimes the engine model actually holds.
// TODO(2.4.x): migrate these per-aircraft data to `AircraftSpec`.

/// Aerodynamic moment coefficients. Signs are stated for our physics convention above.
/// Values are textbook small-aircraft ballpark tuned by flight-test feel on the
/// Beaver-placeholder airframe.
///
/// Pitch: static stability (`Cm_α < 0`) pulls α back toward trim; elevator command
/// (`Cm_δe > 0`) pitches nose up for positive stick; damping (`Cm_q < 0`) opposes
/// pitch rate.
///
/// `Cm_δe` tuning history:
///   0.05 → "almost nonexistent" feel
///   0.12 → "half the size it should be"
///   0.24 → current; full stick at cruise hits ~40°/s steady pitch rate, initial
///          accel ~2 rad/s². Subjectively crisp without being unrecoverable.
const CM_ALPHA: f32 = -0.25;
// Task #178 + playtest follow-up (2026-04-22): tuned 0.24 → 0.22 (8%
// reduction). Iterated toward playability: 0.15 gave realistic pitch rate
// (18°/s) but required 22s of saturated stick for water takeoff —
// unreachable by human pilot. 0.20 still needed 15s saturated. 0.22
// leaves enough elevator authority for realistic water-takeoff rotation.
// Pitch rate 35-40°/s — above real ~20 but within the permissive band
// the user OK'd for playability. Further reduction blocks on water-physics
// pitch-restoring moment being too strong (buoyancy asymmetry at small
// pitch angle creates ~3× elevator moment).
const CM_DELTA_E: f32 = 0.22;
const CM_Q: f32 = -25.0;

/// Tailplane lift coefficient (wing-area-referenced, per unit elevator).
///
/// Real tailplane surfaces contribute a small vertical force when the elevator
/// is deflected — distinct from the pitching moment already modeled via
/// `CM_DELTA_E`. Missing this physics contributed to restrictive on-step
/// rotation failures (task #174): elevator commanded pitch-up but couldn't
/// kickstart rotation from level attitude because no force component existed
/// to help raise the aircraft; rotation only began once buoyancy asymmetry
/// developed, and buoyancy asymmetry only develops AFTER some pitch exists.
///
/// Coefficient derivation: moment from tail lift × `tail_arm` = existing
/// `CM_DELTA_E` moment; so `TAILPLANE_LIFT_COEFF = CM_DELTA_E × c / tail_arm`.
/// For Beaver: 0.24 × 1.52 / 5.5 ≈ 0.066 (theoretical value).
///
/// At 0.066, the tail lift is sufficient to kickstart rotation from level
/// attitude in the restrictive glassy-water takeoff scenario (task #174 key
/// success criterion — real Beaver CAN take off from glassy water, and our
/// model now matches). Lower values (0.033) don't unblock glassy rotation.
const TAILPLANE_LIFT_COEFF: f32 = 0.066;

/// Roll: aileron command (`Cl_δa > 0`) rolls right wing down for positive stick;
/// damping (`Cl_p < 0`) opposes roll rate. Dihedral effect (`Cl_β < 0`) provides
/// static roll stability: when aircraft sideslips (β ≠ 0), the lower wing gets
/// more effective span-wise airflow, generating a rolling moment that opposes
/// the bank causing the sideslip. This makes a banked aircraft self-correct
/// toward wings level unless actively held — canonical GA handling.
///
/// `Cl_δa` tuning: 0.08 (too lively) → 0.07 (slightly weak) → 0.075 (smidge up).
/// `Cl_β` typical Beaver-class GA single: -0.05 to -0.10 per rad. -0.08 gives
/// modest but noticeable roll stability; enough to let banked aircraft slowly
/// return to level under uncorrected sideslip, without the over-damped feel of
/// training-aircraft dihedral (-0.10+).
// Task #177 (2026-04-22): reduced from 0.075 → 0.040 after roll-step-response
// scenarios measured peak roll rate 75.7°/s at full aileron — 2.5× above real
// Beaver POH / pilot-reported ~30°/s. User feedback: "yaw/roll in air feels
// very extreme". First attempt at 0.030 measured 22.5°/s (under target 25-40);
// 0.040 is middle-of-range, measured ~30°/s. Turn / bank scenarios compensate
// via PID commanding more aileron input (within ±1.0 clamp).
const CL_DELTA_A: f32 = 0.040;
const CL_P: f32 = -0.5;
/// Dihedral effect: sideslip β produces rolling moment. Negative = stable
/// (positive β / wind-from-right yields roll-left, restoring toward
/// wings-level). Retuned 2026-04-22 post-playtest: -0.05 → -0.025 after
/// air-abusive-yaw scenario revealed ~10× too-fast spiral mode. User report:
/// sustained rudder causes aircraft to roll into pronounced bank within
/// seconds; real Beaver's spiral mode has ~30-45s time constant. Halving
/// `CL_BETA` slows equilibrium spiral roll rate from 22°/s to 11°/s (from
/// formula `ω = 2V·Cl_β·β / (b·Cl_p)`), which matches real-world spiral
/// divergence and removes the "pervasive left yaw" playtest complaint.
/// Other scenarios compensate via PIDs (`BankHold` commands more aileron
/// in turns). If dihedral-dependent tests drift, this is the knob to touch.
const CL_BETA: f32 = -0.025;

/// Yaw: weathervane stability (`Cn_β > 0`) yaws nose toward velocity when sideslip
/// develops; rudder command (`Cn_δr > 0`) yaws nose right for positive pedal; damping
/// (`Cn_r < 0`) opposes yaw rate.
///
/// `Cn_δr` tuning: 0.05 (wobble due to spring/damper imbalance, plus too lively) →
/// 0.03 (lively fixed via `Cn_r` increase below, but felt a little small) →
/// 0.04 (small bump back toward responsive while keeping the damping margin).
/// `Cn_r` lifted 0.15 → 0.6 (4×) to kill the visible Dutch-roll-like oscillation.
const CN_BETA: f32 = 0.08;
const CN_DELTA_R: f32 = 0.04;
const CN_R: f32 = -0.6;

/// Minimum airspeed used as the denominator in the nondimensional damping factor
/// `ω·L/(2V)`. Below this, the plane is essentially parked and damping should saturate
/// rather than blow up; clamping to 1 m/s gives sensible behavior without a divide.
const MIN_DAMPING_AIRSPEED: f32 = 1.0;

// --- Secondary aerodynamic effects (Phase 2.4.6) ------------------------------
//
// Adverse yaw: rolling aileron → more lift on one wing → more induced drag on that
// wing → nose yaws OPPOSITE to the roll direction. Coefficient applied to aileron
// input, scaled by q·S·b like other yaw moments.
const CN_DELTA_A_ADVERSE: f32 = -0.012;

// P-factor: descending blade of a right-rotating prop is on the right; at high α +
// high power, that blade generates asymmetric thrust → nose yaws LEFT. Modeled as an
// effective moment arm applied to the current thrust force × sin(α). Real prop-disc
// math is complex; this arm is a tunable lumped parameter.
const P_FACTOR_ARM_M: f32 = 10.0;

// Spiraling slipstream: a right-rotating propeller imparts rotational swirl to its
// wash. The swirl hits the vertical tail (typically on its left side for a top-mounted
// prop), pushing the tail right and yawing the nose LEFT. Effect is strongest at
// low V + high power (swirl velocity dominates over freestream); weakest at cruise.
// This is DIFFERENT from P-factor and torque roll — it's the third classic "left-
// turning tendency" that explains why Beaver pilots hold substantial right rudder
// at takeoff / initial climb even after P-factor alone is accounted for.
// Modeled as: moment = -arm · thrust · V_ref/(V_ref + V), so at V=0 full strength,
// decays hyperbolically with airspeed.
const SPIRAL_SLIPSTREAM_ARM_M: f32 = 2.0;
const SPIRAL_SLIPSTREAM_V_REF: f32 = 20.0;

// Slipstream-augmented dynamic pressure at rudder/elevator. The prop wash blasts
// air over the empennage, multiplying control-surface authority on top of free-
// stream q. Required for the model to be physically consistent: spiral slipstream
// produces a strong nose-left yaw at low V + high power, but rudder authority
// grows only with freestream q² → at takeoff regime the rudder cannot counter
// slipstream yaw. Real Beaver pilots use 50-80% right rudder during takeoff;
// the slipstream-augmented q-boost makes that input range produce realistic
// counter-yaw authority.
//
// Magnitude tuned so that at V=20 m/s, full thrust (~10 kN), full rudder produces
// enough moment to slightly exceed the spiral-slipstream yaw — giving the pilot
// authority to hold heading at takeoff with realistic rudder input (~80%).
// At cruise the boost is small (~7-12%) so existing scenario yaw trims see a
// modest but real change.
//
// Same factor reused in the spiral-slipstream-moment formula
// (V_REF/(V_REF+V) × coupling_airspeed_factor) so rudder authority and slipstream
// yaw scale together — pilot input has correct relative effect at all speeds.
//
// The cruise / cruise-climb / endurance scenarios were tuned without the q-boost
// (pre-2.5a-yaw-fix); their yaw trim values may need slight downward adjustment
// once this constant is fully tuned. Marked as triaged behavior during 2.5a
// completion work.
const RUDDER_SLIPSTREAM_Q_PER_N: f32 = 0.15;

// Torque roll: engine torque reacts against the airframe. Right-rotating prop rolls
// the airframe LEFT. Magnitude scales with shaft power. At 335 kW max: ~500 N·m
// rolling-left moment → ~0.1 rad/s² roll accel on I_zz=5000. Noticeable on power
// changes, easy to counter with slight right aileron.
//
// Real-world dependencies (status):
// - Throttle: YES — `engine_power_kw_out` already scales linearly with throttle
//   (idle 20 kW at 0.0 → max 335 kW at 1.0). Torque scales 1:1 with this.
// - Density altitude: YES — `engine_power_kw_out = engine_power_sl × rho_ratio`
//   so torque automatically derates at altitude (2.4.7 density model).
// - Prop pitch / feathering: NOT YET. Current model uses a fixed-pitch prop
//   efficiency curve. When the constant-speed prop and feathering controls land
//   (post-2.4 / Phase 4 cockpit milestone), torque will need a feather check:
//   `if feathered { 0.0 } else { -K · power }`. Feathered prop blades go to ~90°
//   pitch, dramatically unloading the engine — torque on the airframe drops to
//   near zero (engine RPM races up, airframe stops feeling the reaction).
// - Coupling airspeed gating: torque-roll is multiplied by `coupling_airspeed_factor`
//   (ramp 5→20 m/s) which is non-physical (torque is engine reaction, not aero)
//   but practical: at V=0 with full power, ungated torque produces a 0.1 rad/s²
//   roll spin with no aero damping. Buoyancy fights it on water; ground physics
//   (Phase 2.5b) will provide wheel-friction roll restoring on land. Once 2.5b
//   lands the gating can be relaxed or removed.
const TORQUE_ROLL_NM_PER_KW: f32 = 1.5;

/// Airspeed ramp for rotational-coupling effects (P-factor + torque roll). Below
/// `COUPLING_AIRSPEED_RAMP_MIN`, these effects are zero; above
/// `COUPLING_AIRSPEED_RAMP_MAX`, full strength; linear in between. This is a
/// physics-simplification to prevent the moments from accumulating rotation without
/// bound on the ground or at hover-like low airspeeds. Real wheels provide the
/// resistance we don't model until 2.5 contact physics lands; this ramp is the
/// interim proxy.
const COUPLING_AIRSPEED_RAMP_MIN: f32 = 5.0;
const COUPLING_AIRSPEED_RAMP_MAX: f32 = 20.0;

/// Ground-effect induced-drag factor (Phillips approximation).
///
/// Induced drag scales by `33·(h/b)^1.5 / (1 + 33·(h/b)^1.5)`. At h/b=0.1 ~49%
/// reduction, at h/b=0.5 ~8%, above h/b=1 essentially no effect. `h` is altitude
/// above surface, `b` is wingspan. Returns 1.0 when `agl_m >= wingspan_m` (outside
/// effective range).
#[must_use]
pub fn ground_effect_induced_drag_factor(agl_m: f32, wingspan_m: f32) -> f32 {
    if agl_m >= wingspan_m || wingspan_m < 1e-3 {
        return 1.0;
    }
    let h_over_b = (agl_m / wingspan_m).max(0.0);
    let k = 33.0 * libm::powf(h_over_b, 1.5);
    k / (1.0 + k)
}

/// Stall α (rad). Matches the peak of the `CL_TABLE` (`Cl_max` at 16°). Past this α
/// the airfoil loses Cl and the center of lift shifts aft.
const ALPHA_STALL_RAD: f32 = 0.2793;

/// Approach-to-stall α (rad). 80% of stall; HUD flags this as "APPROACH".
const ALPHA_APPROACH_RAD: f32 = 0.2234;

/// Pitch-moment coefficient per radian of α past stall. Produces a nose-down
/// moment when α > `α_stall`, modeling the aft shift of the center of lift at stall.
/// Real aircraft self-recover from stall partly through this mechanism; small
/// deviations past stall generate restoring moment proportional to how far past.
///
/// Tuning: started at -2.0 in 2.4.4d, reduced to -0.75 in 2.4.6 after flight-test
/// feedback that stall pitch-down was too severe (combining with `Cm_α × α` it
/// produced ~107°/s² nose-whip at α=20°). Current value gives ~25°/s² nose-down at
/// 4° past stall -- clearly a stall, but recoverable.
const CM_STALL_PITCH_SLOPE: f32 = -0.75;

/// Quarter-span fraction used to sample per-wing α for the wing-drop calculation.
/// Each wing half's representative α = `α_aircraft` ± `roll_rate` × (`SPAN` × 0.25) / V.
/// Smaller fractions under-represent tip effects; larger ones over-represent.
const WING_DROP_SAMPLE_FRAC: f32 = 0.25;

/// Stall state for the HUD indicator and any autopilot / audio hook that keys off it.
/// Derived from |α| vs `ALPHA_STALL_RAD` / `ALPHA_APPROACH_RAD` in `compute_forces`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StallState {
    /// Normal flight, well clear of stall.
    #[default]
    Normal,
    /// α is within 80%-100% of stall; stick further back would stall.
    Approach,
    /// α is at or past the stall angle. Cl is on the back side of the curve.
    Stall,
}

impl StallState {
    /// Classify based on the absolute value of α (rad).
    #[must_use]
    pub fn from_alpha(alpha_abs: f32) -> Self {
        if alpha_abs >= ALPHA_STALL_RAD {
            Self::Stall
        } else if alpha_abs >= ALPHA_APPROACH_RAD {
            Self::Approach
        } else {
            Self::Normal
        }
    }
}

/// Per-flap-position aerodynamic deltas applied on top of the clean-configuration
/// Cl curve, Cd0, and pitching moment. Beaver-calibrated values:
///
/// | Position | Δ Cl | Δ Cd0 | Δ Cm   | Notes                                |
/// |----------|------|-------|--------|--------------------------------------|
/// | Up       | 0.00 | 0.000 |  0.00  | Clean                                |
/// | Climb    | 0.12 | 0.005 | -0.02  | ~20° deflection; takeoff climb out   |
/// | Takeoff  | 0.25 | 0.015 | -0.06  | ~35°; soft-field / short-field       |
/// | Landing  | 0.37 | 0.040 | -0.10  | ~58° slotted; Vso ~21 m/s target     |
///
/// Landing `Cl` offset puts `Cl_max ≈ 1.65 + 0.37 = 2.02`, which at MTOW drops the
/// stall airspeed from `Vs_clean = 23 m/s → Vso ≈ 20.8 m/s`, matching the Beaver POH
/// `Vso = 21 m/s` (47 mph). Drag penalty at Landing (+89% zero-lift drag) is what
/// makes the approach descend aggressively when flaps are hung out.
#[derive(Clone, Copy, Debug)]
pub struct FlapAeroDeltas {
    pub delta_cl: f32,
    pub delta_cd0: f32,
    pub delta_cm: f32,
}

/// Per-position flap-aero lookup.
///
/// `Δ Cm` tuning history: initial 2.4.5 values (Climb -0.02, Takeoff -0.06, Landing
/// -0.10) pitched the plane down ~0.82 rad/s² at Landing in cruise q -- much stronger
/// than the pilot-manageable trim change a real Beaver shows. Reduced by ~3× at
/// Landing so the moment is clearly noticeable but easily countered with back-stick.
#[must_use]
pub fn flap_aero(position: FlapPosition) -> FlapAeroDeltas {
    match position {
        FlapPosition::Up => FlapAeroDeltas {
            delta_cl: 0.00,
            delta_cd0: 0.000,
            delta_cm: 0.000,
        },
        FlapPosition::Climb => FlapAeroDeltas {
            delta_cl: 0.12,
            delta_cd0: 0.005,
            delta_cm: -0.010,
        },
        FlapPosition::Takeoff => FlapAeroDeltas {
            delta_cl: 0.25,
            delta_cd0: 0.015,
            delta_cm: -0.020,
        },
        FlapPosition::Landing => FlapAeroDeltas {
            delta_cl: 0.37,
            delta_cd0: 0.040,
            delta_cm: -0.030,
        },
    }
}

// --- Placeholder (no-spec) drag constants ---

/// Linear drag coefficient (1/s), low-speed stability -- only used when no
/// `AircraftSpec` is present (airfoil drag polar takes over when one is).
const LIN_DAMPING: f32 = 0.05;

/// Quadratic drag coefficient (1/m), bounds terminal velocity -- only used without
/// an `AircraftSpec`.
const QUAD_DRAG: f32 = 0.014;

/// Placeholder wing-lift coefficient: m/s² of upward acceleration per m/s of
/// *horizontal* forward airspeed. Only used when no `AircraftSpec` is present.
const LIFT_COEFF: f32 = 0.344;

// --- Airfoil / atmosphere constants (Phase 2.4.2) ---

/// International Standard Atmosphere sea-level density, kg/m³.
const AIR_DENSITY_SEA_LEVEL: f32 = 1.225;

/// Scale height (m) for the exponential atmosphere approximation. `ρ(h) = ρ₀ ·
/// exp(-h / H)` matches ISA to within a few percent up to ~5 km, which covers the
/// Beaver's service ceiling. Full barometric formula with temperature lapse is a
/// post-MVP refinement (would let the player fly into non-ISA-standard days).
const ATMOSPHERE_SCALE_HEIGHT_M: f32 = 8500.0;

/// Air density (kg/m³) at altitude `h_msl_m` above sea level, using the exponential
/// atmosphere approximation. Negative altitudes (e.g., below-horizon terrain) are
/// clamped to sea level.
#[must_use]
pub fn air_density(h_msl_m: f32) -> f32 {
    let h = h_msl_m.max(0.0);
    AIR_DENSITY_SEA_LEVEL * libm::expf(-h / ATMOSPHERE_SCALE_HEIGHT_M)
}

/// Ratio of air density at altitude to sea-level density.
///
/// Feeds both the aerodynamic `q = 0.5 ρ V²` scaling and the naturally-aspirated
/// engine's linear power derate. At 3000 m the ratio is ~0.71; at 5000 m ~0.56.
#[must_use]
pub fn density_ratio(h_msl_m: f32) -> f32 {
    air_density(h_msl_m) / AIR_DENSITY_SEA_LEVEL
}

/// Zero-lift drag coefficient (clean airframe, no flaps).
///
/// Tuning history:
///   0.028 → "Cessna-retractable slippery"; `L/D_max` ~14 at cruise, plane barely slows
///           with throttle off (flight-test feedback: "feels like it glides forever").
///   0.045 → pre-2.4.8-tune; `L/D_max` ~11. Too low: bench revealed cruise equilibrium
///           at throttle 0.55 vs POH 0.65, and best-glide V 45 m/s vs POH 31.
///   0.050 → 2.4.8 Gap 1 tune step 1; measured ripples smaller than analytic due to
///           self-regulating equilibrium (lower V → lower induced drag).
///   0.055 → 2.4.8 Gap 1 tune step 2; closes more of cruise-throttle gap, brings
///           best-glide V further down toward POH 31 m/s. See PROGRESS.md § gaps.
///
/// TODO(2.4.5): flap-deployment bumps this by ~0.02 per notch.
const CD0: f32 = 0.055;

/// Oswald efficiency factor. Typical for small single-engine aircraft: 0.7-0.85.
/// Set slightly toward the low end (0.75) because strut-bracing and fixed gear
/// disturb the lift distribution away from ideal elliptical.
const OSWALD_E: f32 = 0.75;

/// NACA 64A416-shaped `Cl(α)` table adjusted for finite-span whole-wing effects.
/// `α` in radians; `Cl` dimensionless. Piecewise linearly interpolated. Zero-lift at
/// ~-2°, linear slope ~0.10/deg through the clean range, peak `Cl_max` ≈ 2.0 at 16°,
/// post-stall drop -0.25 per 2° past peak, deep-stall plateau near 0.55 out to 60°.
/// Mirror-extended to negative α for inverted flight.
///
/// Peak raised from 1.65 → 2.0 as Phase 2.4.8 Gap 3 tuning pass (see PROGRESS.md
/// § Bench-identified model gaps). Analytic Vs at `Cl_max = 2.0` ≈ 26.3 m/s (was
/// 29; POH = 23.25; residual 3 m/s gap attributed to missing slipstream physics
/// and POH measurement conditions). Entries below 14° unchanged to preserve
/// cruise / climb / glide behavior (those regimes never reach the peak region).
///
/// TODO(2.4.2): per-aircraft tables should live in `AircraftSpec` rather than being
/// hard-coded here. For the MVP (Beaver) this matches published NACA TN-1945-ish
/// figures closely enough, with whole-wing uplift approximating fuselage + vortex
/// contributions that a 2D airfoil table wouldn't capture.
#[allow(clippy::approx_constant)]
const CL_TABLE: &[(f32, f32)] = &[
    (-1.5708, 0.0),   // -90° (flying backward / full sideways)
    (-0.5236, -0.85), // -30°
    (-0.3491, -1.45), // -20° (mirrored absolute drop from new peak)
    (-0.2793, -1.85), // -16° (inverted Cl_max mirrored; slightly below positive peak for asymmetry)
    (-0.1745, -1.10), // -10°
    (-0.0873, -0.65), // -5°
    (-0.0349, 0.00),  // -2° zero-lift
    (0.0000, 0.21),   // 0°
    (0.0873, 0.80),   // 5°
    (0.1745, 1.25),   // 10°
    (0.2443, 1.85),   // 14° (smoothed rise toward new peak)
    (0.2793, 2.00),   // 16° Cl_max (raised from 1.65 per Gap 3 tuning)
    (0.3142, 1.75),   // 18° post-stall (absolute drop -0.25 preserved)
    (0.3491, 1.45),   // 20° (absolute drop -0.30 preserved)
    (0.5236, 0.85),   // 30°
    (1.0472, 0.55),   // 60° deep stall
    (1.5708, 0.00),   // 90°
];

/// Linearly interpolate `Cl` from the table at the given α (radians).
fn lift_coefficient(alpha_rad: f32) -> f32 {
    let a = alpha_rad.clamp(CL_TABLE[0].0, CL_TABLE[CL_TABLE.len() - 1].0);
    let mut i = 0;
    while i + 1 < CL_TABLE.len() && CL_TABLE[i + 1].0 < a {
        i += 1;
    }
    let (x0, y0) = CL_TABLE[i];
    let (x1, y1) = CL_TABLE[(i + 1).min(CL_TABLE.len() - 1)];
    if (x1 - x0).abs() < 1e-6 {
        return y0;
    }
    let t = (a - x0) / (x1 - x0);
    y0 + (y1 - y0) * t
}

/// `Cd(α) = Cd0 + Cl² / (π · AR · e)` plus a post-stall parasite-drag spike.
/// Test-only convenience wrapper; production callers use
/// `drag_coefficient_with_induced_factor` directly so the flap `Cd0` contribution and
/// ground-effect induced-drag factor are always applied.
#[cfg(test)]
fn drag_coefficient(alpha_rad: f32, cl: f32, aspect_ratio: f32) -> f32 {
    drag_coefficient_with_induced_factor(alpha_rad, cl, aspect_ratio, 0.0, 1.0)
}

/// Full drag model: `Cd0 + Δ Cd0` parasite (with flap increment) plus
/// `induced_factor × Cl² / (π · AR · e)` (ground effect modifies the induced term)
/// plus post-stall parasite spike.
///
/// `induced_factor` is 1.0 outside ground effect; approaches 0.5 at ~1/10 wingspan
/// AGL (see `ground_effect_induced_drag_factor`).
fn drag_coefficient_with_induced_factor(
    alpha_rad: f32,
    cl: f32,
    aspect_ratio: f32,
    delta_cd0: f32,
    induced_factor: f32,
) -> f32 {
    let induced =
        induced_factor * cl * cl / (core::f32::consts::PI * aspect_ratio.max(1.0) * OSWALD_E);
    let stall_excess = (alpha_rad.abs() - 0.2793).max(0.0);
    let stall_drag = stall_excess * stall_excess * 2.0;
    CD0 + delta_cd0 + induced + stall_drag
}

/// Compute α (angle of attack, rad) and β (sideslip, rad) from body-frame velocity.
///
/// α > 0 when the relative wind comes from below the chord (nose above airflow).
/// β > 0 when the relative wind comes from the right (nose pointing left of velocity).
/// Returns `(0, 0)` when velocity is near zero or the forward component collapses
/// (flying sideways or backward); callers treat that as no aerodynamic force.
fn angle_of_attack_and_sideslip(state: &VehicleState) -> (f32, f32) {
    if state.velocity.length_squared() < 1e-3 {
        return (0.0, 0.0);
    }
    let body_vel = state.orientation.inverse() * state.velocity;
    let forward_speed = -body_vel.z;
    if forward_speed < 1e-3 {
        return (0.0, 0.0);
    }
    let alpha = f32::atan2(-body_vel.y, forward_speed);
    let beta = f32::atan2(body_vel.x, forward_speed);
    (alpha, beta)
}

/// Shaft power in kW at a given throttle position and fuel-availability flag. Throttle
/// is clamped to `[0, 1]`. Out of fuel → zero power regardless of throttle.
#[must_use]
pub fn engine_power_kw(throttle: f32, fuel_available: bool) -> f32 {
    if !fuel_available {
        return 0.0;
    }
    let t = throttle.clamp(0.0, 1.0);
    ENGINE_POWER_IDLE_KW + t * (ENGINE_POWER_MAX_KW - ENGINE_POWER_IDLE_KW)
}

/// Prop efficiency at the given airspeed (m/s). Piecewise-linear interpolation of the
/// `PROP_EFF_TABLE` keypoints; saturates at the table endpoints outside the range.
#[must_use]
pub fn prop_efficiency(airspeed_m_s: f32) -> f32 {
    let v = airspeed_m_s.max(0.0);
    if v <= PROP_EFF_TABLE[0].0 {
        return PROP_EFF_TABLE[0].1;
    }
    let last = PROP_EFF_TABLE.len() - 1;
    if v >= PROP_EFF_TABLE[last].0 {
        return PROP_EFF_TABLE[last].1;
    }
    let mut i = 0;
    while i + 1 < PROP_EFF_TABLE.len() && PROP_EFF_TABLE[i + 1].0 < v {
        i += 1;
    }
    let (x0, y0) = PROP_EFF_TABLE[i];
    let (x1, y1) = PROP_EFF_TABLE[i + 1];
    if (x1 - x0).abs() < 1e-6 {
        return y0;
    }
    let t = (v - x0) / (x1 - x0);
    y0 + (y1 - y0) * t
}

/// Thrust force in newtons.
///
/// `T = η_prop × P / V`, clamped so the denominator doesn't drop below
/// `V_MIN_FOR_THRUST` (prevents divergence at low airspeed). Takes throttle, airspeed,
/// and the fuel-availability flag.
#[must_use]
pub fn thrust_force_newtons(throttle: f32, airspeed_m_s: f32, fuel_available: bool) -> f32 {
    let power_watts = engine_power_kw(throttle, fuel_available) * 1000.0;
    let eta = prop_efficiency(airspeed_m_s);
    let v_eff = airspeed_m_s.max(V_MIN_FOR_THRUST);
    eta * power_watts / v_eff
}

/// Fuel flow in gallons-per-hour at the given throttle + fuel availability. Uses a
/// constant BSFC so burn scales linearly with shaft power. Out of fuel → zero burn.
#[must_use]
pub fn fuel_flow_gph(throttle: f32, fuel_available: bool) -> f32 {
    engine_power_kw(throttle, fuel_available) * BSFC_GPH_PER_KW
}

/// Contributions that sum to a single-axis aerodynamic moment (N·m).
///
/// Kept separate so the debug HUD can show which term dominates at the current flight
/// condition (e.g., "control is strong but damping cancels it", "stability is pulling
/// you back", "coupling from P-factor / adverse yaw / torque").
#[derive(Clone, Copy, Debug, Default)]
pub struct MomentBreakdown {
    /// Control-surface contribution (elevator / aileron / rudder × stick position).
    pub control: f32,
    /// Static-stability / α-β / flap / post-stall contribution.
    pub stability: f32,
    /// Damping contribution (opposes the current angular rate about this axis).
    pub damping: f32,
    /// Cross-axis coupling from secondary effects (2.4.6): adverse yaw (aileron →
    /// yaw), P-factor (throttle × α → yaw), torque reaction (engine power → roll),
    /// and future slipstream / dihedral effects. Kept separate so the HUD can show
    /// when the plane's behavior is driven by coupling rather than primary
    /// aerodynamics.
    pub coupling: f32,
}

impl MomentBreakdown {
    /// Sum of all four contributions.
    #[must_use]
    pub fn total(&self) -> f32 {
        self.control + self.stability + self.damping + self.coupling
    }
}

/// Result of one flight-model evaluation. The first two fields drive integration;
/// the remaining fields are diagnostics for the debug HUD.
#[derive(Clone, Copy, Debug, Default)]
pub struct FlightOutputs {
    /// World-space linear acceleration applied to the velocity this tick.
    pub linear_accel: Vec3,
    /// Body-frame angular acceleration (rad/s²). Integrated into `angular_velocity` in
    /// `state::tick_dynamics`. Replaces the old commanded-rate model.
    pub angular_accel: Vec3,

    // Diagnostic breakdown of the force terms that compose `linear_accel`. All in m/s².
    pub thrust: Vec3,
    pub lift: Vec3,
    pub drag: Vec3,
    pub gravity: Vec3,
    /// Total airspeed magnitude (m/s).
    pub airspeed: f32,
    /// Horizontal component of forward airspeed (m/s); feeds the placeholder lift model
    /// and the pre-2.4.4 airspeed-based stall alert heuristic.
    pub horizontal_airspeed: f32,
    /// Angle of attack (rad). Positive when nose points above relative wind.
    pub alpha: f32,
    /// Sideslip angle (rad). Positive when the aircraft has rightward lateral body
    /// velocity (wind from the right, nose left of velocity). Matches aerospace
    /// convention in sign -- the yaw-stability moment uses this value directly.
    pub beta: f32,
    /// Current lift coefficient (dimensionless). Driven by `alpha` on the airfoil path,
    /// zero on the no-spec placeholder path.
    pub cl: f32,
    /// Current drag coefficient (dimensionless).
    pub cd: f32,

    // --- Moment breakdown (N·m) per body axis. Physics-convention signs:
    //   pitch > 0 = nose up, roll > 0 = right wing down, yaw > 0 = nose right.
    /// Pitch moment M about the lateral axis (breakdown: elevator / α-stability / damping).
    pub pitch_moment: MomentBreakdown,
    /// Roll moment L about the longitudinal axis (breakdown: aileron / - / damping).
    pub roll_moment: MomentBreakdown,
    /// Yaw moment N about the vertical axis (breakdown: rudder / β-weathervane / damping).
    pub yaw_moment: MomentBreakdown,

    // --- Engine + prop diagnostics (Phase 2.4.3) ---
    /// Shaft power output (kW) this tick. Zero when out of fuel.
    pub engine_power_kw: f32,
    /// Propeller efficiency this tick (dimensionless, 0..~0.85).
    pub prop_efficiency: f32,
    /// Fuel flow (gallons per hour) at the current engine state.
    pub fuel_flow_gph: f32,

    /// α-based stall state (Phase 2.4.4d). Replaces the pre-2.4.4d airspeed-ratio
    /// heuristic; drives HUD alerts and (future) stall-warning audio.
    pub stall_state: StallState,
}

/// Compute flight forces for the current vehicle state.
///
/// `spec` carries aircraft-specific data (mass, airfoil, engine). Used for translational
/// force computation; rotational dynamics use hardcoded Beaver-ballpark inertia /
/// moment coefficients until `AircraftSpec` grows those fields in a 2.4.x follow-up.
///
/// Sign conventions (right-handed, Y-up, -Z forward):
///   pitch positive = nose up      -> body X rotation positive
///   yaw positive   = nose right   -> body Y rotation negative (storage negates)
///   roll positive  = roll right   -> body Z rotation negative (storage negates)
///
/// See the moment-coefficient block at the top of this module for the open TODOs that
/// close once this milestone's follow-ups (2.4.4d stall physics, 2.4.6 adverse
/// coupling, 2.4.x per-aircraft data) land.
#[must_use]
#[allow(clippy::too_many_lines)] // Linear physics pipeline -- splitting hurts readability.
pub fn compute_forces(
    state: &VehicleState,
    spec: Option<&AircraftSpec>,
    _dt: f32,
    altitude_agl_m: f32,
) -> FlightOutputs {
    let forward = state.orientation * Vec3::NEG_Z;
    let speed = state.velocity.length();

    // Density altitude (2.4.7): exponential atmosphere approximation. All aero
    // forces (q = 0.5 ρ V²) and the naturally-aspirated engine's output scale with
    // this ratio. Position.y is MSL altitude under the current "y=0 is sea level"
    // convention; proper density-altitude with non-ISA temp / pressure is a
    // post-MVP refinement.
    let rho = air_density(state.position.y);
    let rho_ratio = rho / AIR_DENSITY_SEA_LEVEL;

    // Always-useful diagnostics:
    let horiz_vel = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
    let horiz_fwd = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let horiz_airspeed = horiz_vel.dot(horiz_fwd).max(0.0);
    let (alpha, beta) = angle_of_attack_and_sideslip(state);

    // Thrust / lift / drag branch. Airfoil path is physically derived; the no-spec
    // placeholder keeps pure-sim tests deterministic. `if let / else` stays -- the two
    // paths compute fundamentally different physics.
    #[allow(clippy::option_if_let_else)]
    let (thrust, lift, drag, cl, cd, engine_power_kw_out, prop_eff_out, fuel_flow_out) =
        if let Some(s) = spec {
            let mass_kg = s.max_mass_wheels_kg.max(1.0);
            let wing_area = s.wing_area_m2.max(1.0);
            let aspect_ratio = s.aspect_ratio.max(1.0);

            // Engine + prop (Phase 2.4.3): thrust from R-985 power curve times prop
            // efficiency, clamped at low airspeed to avoid divergence. Out of fuel →
            // zero power → zero thrust.
            let fuel_ok = state.fuel_gal > 0.0;
            // Engine altitude derate (2.4.7): naturally-aspirated pistons lose power
            // roughly linearly with air density. At 3000 m, ρ/ρ_0 ≈ 0.71, so the
            // R-985's 450 hp drops to ~320 hp; at 5000 m ~56% → ~253 hp. Fuel flow
            // drops proportionally (BSFC ≈ constant). Prop efficiency is mostly a
            // function of advance ratio, not density, so we don't derate it.
            let engine_power_sl = engine_power_kw(state.controls.throttle, fuel_ok);
            let engine_power = engine_power_sl * rho_ratio;
            let prop_eff = prop_efficiency(speed);
            // Re-derive thrust from the altitude-corrected power rather than calling
            // thrust_force_newtons (which would re-compute at sea-level power).
            let v_eff = speed.max(V_MIN_FOR_THRUST);
            let thrust_n = prop_eff * engine_power * 1000.0 / v_eff;
            let thrust_vec = forward * (thrust_n / mass_kg);
            let fuel_gph = engine_power * BSFC_GPH_PER_KW;

            // Airfoil lift + drag polar (2.4.2) with flap deltas (2.4.5) and ground-
            // effect induced-drag reduction (2.4.6). Flaps shift the Cl curve up by
            // `delta_cl` (camber increase) and add a zero-lift drag penalty; ground
            // effect multiplies the induced portion of drag by Phillips' factor.
            let flap_deltas = flap_aero(state.controls.flaps);
            let cl = lift_coefficient(alpha) + flap_deltas.delta_cl;
            let ge_factor = ground_effect_induced_drag_factor(altitude_agl_m, WING_SPAN_M);
            let cd = drag_coefficient_with_induced_factor(
                alpha,
                cl,
                aspect_ratio,
                flap_deltas.delta_cd0,
                ge_factor,
            );
            let q = 0.5 * rho * speed * speed;
            // Lift uses chord-plane dynamic pressure `q_chord = q × cos²(β)`.
            // Wing generates lift from forward+vertical body flow, not lateral
            // crossflow. At β=0 this is identical to q; at high β, lift fades
            // naturally to zero as the wing enters bluff-body flow regime.
            // Drag keeps total q — parasite drag IS omnidirectional (fuselage
            // drags sideways too), and induced drag's correction from sideslip
            // is second-order. Phase A of the 2026-04-22 q_dyn → q_chord audit.
            let cos_beta = libm::cosf(beta);
            let q_chord = q * cos_beta * cos_beta;
            let lift_force_n = q_chord * wing_area * cl;
            let drag_force_n = q * wing_area * cd;

            // Lift direction: perpendicular to velocity, in the plane containing
            // velocity and body-up (2.4.2b). Falls back to body-up if velocity is
            // parallel to body-up.
            let body_up = state.orientation * Vec3::Y;
            let lift_dir = if speed > 1e-3 {
                let v_hat = state.velocity / speed;
                let perp = body_up - v_hat * body_up.dot(v_hat);
                if perp.length_squared() > 1e-6 {
                    perp.normalize()
                } else {
                    body_up
                }
            } else {
                body_up
            };
            // Tailplane lift: elevator deflection creates a small vertical force
            // at the tail, in addition to the pitching moment modeled via
            // CM_DELTA_E. Real tailplane surfaces contribute ~5-8% of
            // wing-equivalent lift per unit elevator deflection — a small
            // force individually, but load-bearing at low airspeed where the
            // main wing + buoyancy margin is thin (on-step plateau, rotation
            // from level attitude on floats). Missing this physics contributed
            // to the restrictive glassy-water takeoff failure (task #174):
            // elevator moment alone couldn't produce the pitch-up rotation
            // because it relied on buoyancy asymmetry that only appears
            // AFTER some pitch has developed (chicken-and-egg).
            //
            // Coefficient derivation: the moment from tail lift = L_tail ×
            // tail_arm. The existing Cm_δe already captures this moment;
            // so the tail lift magnitude is Cm_δe × c / tail_arm, referenced
            // to wing area. For Beaver: 0.24 × 1.52 / 5.5 ≈ 0.066.
            // Uses freestream q only (skipping slipstream boost — simplification
            // that under-estimates force by ~5-10% at high power).
            let tailplane_lift_n = q * wing_area * TAILPLANE_LIFT_COEFF * state.controls.pitch;
            let lift_vec = lift_dir * ((lift_force_n + tailplane_lift_n) / mass_kg);

            let drag_vec = if speed > 1e-3 {
                -state.velocity.normalize_or_zero() * (drag_force_n / mass_kg)
            } else {
                Vec3::ZERO
            };

            (
                thrust_vec,
                lift_vec,
                drag_vec,
                cl,
                cd,
                engine_power,
                prop_eff,
                fuel_gph,
            )
        } else {
            // No-spec fallback: horizontal-airspeed lift + constant thrust. Engine
            // diagnostics left at zero since there's no fuel / power model in this
            // path.
            let thrust_vec = forward * (state.controls.throttle * THRUST_ACCEL_MAX_FALLBACK);
            let lift_vec = Vec3::Y * (horiz_airspeed * LIFT_COEFF);
            let drag_vec = if speed > 1e-3 {
                -state.velocity.normalize_or_zero() * (speed * (LIN_DAMPING + QUAD_DRAG * speed))
            } else {
                Vec3::ZERO
            };
            (thrust_vec, lift_vec, drag_vec, 0.0, 0.0, 0.0, 0.0, 0.0)
        };

    // ----- Rotational dynamics (Phase 2.4.4): moments -> angular acceleration. -----
    //
    // Dynamic pressure and wing area are the scaling factors on all three axes. Wing
    // area falls back to the spec value; the no-spec placeholder path uses the
    // hardcoded Beaver area so the moment equations still produce sensible numbers
    // without an AircraftSpec loaded (e.g., in pure-sim tests).
    let wing_area_moments = spec.map_or(23.23_f32, |s| s.wing_area_m2.max(1.0));
    let q_dyn = 0.5 * rho * speed * speed;
    // Chord-plane dynamic pressure = q_dyn × cos²(β). Used throughout the
    // aero model for terms where the wing/tail surface responds to
    // forward-airspeed flow only (not lateral crossflow): lift, wing-drop,
    // aileron authority, roll damping, adverse yaw, pitch stability and
    // damping, elevator freestream component. Cross-flow terms (weathervane
    // CN_β, yaw damping CN_r, parasite drag) continue to use q_dyn.
    // See docs/physics notes for the 2026-04-22 q_dyn → q_chord audit.
    let cos_beta_outer = libm::cosf(beta);
    let q_chord = q_dyn * cos_beta_outer * cos_beta_outer;
    let v_damp = speed.max(MIN_DAMPING_AIRSPEED);

    // Angular rates in physics convention: pitch > 0 nose-up, roll > 0 right-wing-down,
    // yaw > 0 nose-right. flowplan stores yaw and roll negated in angular_velocity, so
    // flip back here.
    let pitch_rate = state.angular_velocity.x;
    let roll_rate = -state.angular_velocity.z;
    let yaw_rate = -state.angular_velocity.y;

    // Nondimensional damping factors ω·L/(2V). At cruise they're ~0.01--0.02 per rad/s
    // of rate, so moderate rates combined with large |Cm_q| / |Cl_p| / |Cn_r| produce
    // meaningful restoring moments.
    let pitch_damp_nd = pitch_rate * MEAN_CHORD_M / (2.0 * v_damp);
    let roll_damp_nd = roll_rate * WING_SPAN_M / (2.0 * v_damp);
    let yaw_damp_nd = yaw_rate * WING_SPAN_M / (2.0 * v_damp);

    // Precursors shared between the pitch/yaw control-surface boosts AND the
    // yaw/roll coupling terms (P-factor, spiral slipstream, torque roll).
    // Computed here so both rudder AND elevator can apply the same slipstream-
    // q-boost — real Beaver: both control surfaces sit in the prop wash and
    // get accelerated air proportional to thrust loading.
    let coupling_airspeed_factor = ((speed - COUPLING_AIRSPEED_RAMP_MIN)
        / (COUPLING_AIRSPEED_RAMP_MAX - COUPLING_AIRSPEED_RAMP_MIN))
        .clamp(0.0, 1.0);
    let thrust_n_this_tick =
        thrust.dot(forward) * spec.map_or(1.0, |s| s.max_mass_wheels_kg.max(1.0));
    let spiral_slipstream_factor =
        SPIRAL_SLIPSTREAM_V_REF / (SPIRAL_SLIPSTREAM_V_REF + speed.max(1.0));
    // Slipstream q-boost: prop wash over a control surface. Empirically split
    // between elevator and rudder:
    // - `q_local_at_elevator`: NOT gated by airspeed. The tail-up-balance
    //   maneuver (brakes locked, power up, forward stick) needs pitch
    //   authority at V=0. Pilot reports confirm this works at zero airspeed.
    //   Angular damping at V=0 comes from ground-contact lateral friction.
    // - `q_local_at_rudder`: IS gated by `coupling_airspeed_factor`. Real
    //   pilot reports describe rudder as "ineffective below 10 kt" on the
    //   Beaver; ground-taxi directional control comes from the TAILWHEEL
    //   (2.5b.3 physics), not the rudder. Without this gate the rudder
    //   overrides tailwheel steering at taxi speeds.
    let base_slipstream_q =
        RUDDER_SLIPSTREAM_Q_PER_N * thrust_n_this_tick.max(0.0) * spiral_slipstream_factor;
    // Phase D of 2026-04-22 q_chord audit: elevator splits freestream (q_chord,
    // horizontal surface responds to forward flow) from slipstream (q_prop,
    // always body-forward regardless of β). Rudder keeps q_dyn because the
    // vertical fin's effectiveness comes from lateral flow (β × q_total is
    // the fin's "lift") — same reason CN_β weathervaning uses q_dyn.
    let q_local_at_elevator = q_chord + base_slipstream_q;
    let q_local_at_rudder = q_dyn + base_slipstream_q * coupling_airspeed_factor;

    // Pitch: control (elevator) + α-stability (with post-stall nose-down and flap
    // pitching-moment contribution) + pitch-rate damping. Past α_stall, the center
    // of lift shifts aft, producing an additional nose-down moment that tries to
    // self-recover the aircraft. Flap deployment also produces a nose-down moment
    // (Cm_0 shift -- the lift increment acts aft of CG for positive-camber airfoils).
    //
    // Control uses `q_local_at_elevator` (freestream + ungated slipstream) so
    // the elevator retains authority at zero airspeed when prop wash dominates
    // — the physical basis for classic tail-up-balance / wheelie-standing
    // maneuvers: brakes locked, throttle up, stick forward → prop wash over
    // elevator pitches the tail up even with V=0.
    let stall_excess = (alpha.abs() - ALPHA_STALL_RAD).max(0.0);
    let stall_pitch_addend = CM_STALL_PITCH_SLOPE * stall_excess * alpha.signum();
    let flap_cm = flap_aero(state.controls.flaps).delta_cm;
    // Phase B of 2026-04-22 q_chord audit: pitch stability (α-driven) and
    // pitch damping use q_chord because both depend on chord-plane flow
    // (tail sees forward airspeed, not crossflow). At high β these fade
    // naturally. Control stays on q_local_at_elevator (Phase D will split
    // the q_dyn component of that; for now, slipstream dominates at the
    // low airspeeds where pitch control is most critical).
    let pitch_moment = MomentBreakdown {
        control: q_local_at_elevator
            * wing_area_moments
            * MEAN_CHORD_M
            * CM_DELTA_E
            * state.controls.pitch,
        stability: q_chord
            * wing_area_moments
            * MEAN_CHORD_M
            * (CM_ALPHA * alpha + stall_pitch_addend + flap_cm),
        damping: q_chord * wing_area_moments * MEAN_CHORD_M * CM_Q * pitch_damp_nd,
        coupling: 0.0, // Pitch coupling reserved for future slipstream effects.
    };

    // Roll: control (aileron) + roll-rate damping (Cl_p) + wing-drop asymmetry when
    // near or past stall + torque-reaction roll (2.4.6). The wing-drop term samples
    // Cl(α) at the left and right wing's quarter-span with roll-rate-induced α
    // perturbation, and uses the non-linear portion of that differential as a
    // destabilizing rolling moment past stall. Torque reaction is a LEFT-roll from
    // the right-rotating prop; scales linearly with shaft power AND by the
    // airspeed-ramp factor so it doesn't accumulate rotation at V=0.
    // `coupling_airspeed_factor` already computed above (pitch-block precursor).
    // Wing-drop uses q_chord (forward-airspeed dynamic pressure). See the
    // outer-scope q_chord declaration for the physics rationale. At β=90°
    // q_chord=0 → wing_drop=0, eliminating the need for hard clamps on
    // alpha (previously clamped ±25° to avoid bogus atan2 artifacts when
    // forward_speed collapsed).
    let wing_drop_moment =
        wing_drop_roll_moment(alpha, roll_rate, v_damp, q_chord, wing_area_moments);
    let torque_roll_moment =
        -TORQUE_ROLL_NM_PER_KW * engine_power_kw_out * coupling_airspeed_factor;
    // Dihedral effect: sideslip β produces a roll moment. Cl_β < 0 makes this
    // restoring (positive β = wind from right → negative moment = roll left,
    // reducing any right bank that caused the sideslip). This is the canonical
    // GA "banked aircraft rolls back to level" behavior. Combined with the
    // wing-drop moment (past-stall differential Cl) into the stability slot.
    //
    // Smooth aerodynamic falloff: replaces the 2026-04-22 hard β-clamp with
    // `sin(β)·cos(β) = 0.5·sin(2β)`. Physical basis:
    //   - Linear equivalence at small β: sin(β)·cos(β) ≈ β for |β| < ~15°,
    //     matching textbook linear `Cl_β × β` behavior in the normal envelope.
    //   - Peaks at β = 45°: the wing's asymmetric response grows with β
    //     (captured by sin), but lift authority shrinks as forward flow
    //     collapses (captured by cos). Product peaks at 45°.
    //   - Vanishes at β = 90°: wing in pure crossflow cannot generate
    //     asymmetric lift for dihedral response — matches physical reality
    //     of wing entering bluff-body drag regime.
    // Benefit over clamp: no discontinuity at ±20°, provides proper rolloff
    // for transient sideslip excursions (e.g. during gust response or
    // rapid maneuvers). Abusive-input cases that earlier drove β > 40° in
    // water have been fixed at the source (water physics), so smooth
    // aero response is safe here.
    // Dihedral's `sin(β)·cos(β)` factor already embeds the cross-flow effect;
    // dihedral moment itself is proportional to q_total because it's a
    // cross-wind response (the fin/wing sees asymmetric loading from lateral
    // flow, scaled by total dynamic pressure).
    let dihedral_factor = libm::sinf(beta) * libm::cosf(beta);
    let dihedral_roll_moment = q_dyn * wing_area_moments * WING_SPAN_M * CL_BETA * dihedral_factor;
    // Phase C of 2026-04-22 q_chord audit: roll control (aileron), roll
    // damping (CL_p), and adverse yaw all depend on chord-plane flow at
    // the wing surface. Use q_chord so these fade naturally at high β.
    let roll_moment = MomentBreakdown {
        control: q_chord * wing_area_moments * WING_SPAN_M * CL_DELTA_A * state.controls.roll,
        stability: wing_drop_moment + dihedral_roll_moment,
        damping: q_chord * wing_area_moments * WING_SPAN_M * CL_P * roll_damp_nd,
        coupling: torque_roll_moment,
    };

    // Yaw: control (rudder) + β-weathervane + yaw-rate damping + secondary coupling
    // (2.4.6: adverse yaw from aileron input + P-factor from throttle × α).
    //
    // Adverse yaw: positive aileron input rolls the aircraft into the turn; the
    // "into-turn" wing has higher Cl → higher induced drag → nose yaws OPPOSITE the
    // roll (e.g., right roll stick → nose yaws left unless rudder compensates).
    //
    // P-factor: the descending blade of a right-rotating prop is on the right; at
    // high α + high power, the descending blade bites into more air, producing
    // asymmetric thrust that yaws the nose LEFT. Strongest at takeoff climb out
    // (high α, full power); negligible at cruise.
    // Adverse yaw (Phase C): wing-drag asymmetry from aileron uses q_chord.
    let adverse_yaw_moment =
        q_chord * wing_area_moments * WING_SPAN_M * CN_DELTA_A_ADVERSE * state.controls.roll;
    let p_factor_moment =
        -P_FACTOR_ARM_M * thrust_n_this_tick * libm::sinf(alpha) * coupling_airspeed_factor;
    // Spiraling slipstream: nose-left yaw moment from prop-wash swirl hitting
    // vertical tail. Scales with thrust, decays with airspeed. The third
    // left-turning tendency (alongside P-factor + torque roll).
    let spiral_slipstream_moment = -SPIRAL_SLIPSTREAM_ARM_M
        * thrust_n_this_tick
        * spiral_slipstream_factor
        * coupling_airspeed_factor;

    // Rudder uses `q_local_at_rudder` (freestream + slipstream, gated by
    // airspeed). At taxi speeds the rudder is physically ineffective;
    // directional control comes from the tailwheel (2.5b.3 physics).
    let yaw_moment = MomentBreakdown {
        control: q_local_at_rudder
            * wing_area_moments
            * WING_SPAN_M
            * CN_DELTA_R
            * state.controls.yaw,
        stability: q_dyn * wing_area_moments * WING_SPAN_M * CN_BETA * beta,
        damping: q_dyn * wing_area_moments * WING_SPAN_M * CN_R * yaw_damp_nd,
        coupling: adverse_yaw_moment + p_factor_moment + spiral_slipstream_moment,
    };

    // Convert moments (physics convention) to body-frame angular acceleration. flowplan
    // storage negates Y (yaw) and Z (roll); negate those on the way in so the final
    // angular_velocity has the same sign convention the rest of the sim already uses.
    let angular_accel = Vec3::new(
        pitch_moment.total() / I_XX,
        -yaw_moment.total() / I_YY,
        -roll_moment.total() / I_ZZ,
    );

    FlightOutputs {
        linear_accel: thrust + lift + drag + GRAVITY,
        angular_accel,
        thrust,
        lift,
        drag,
        gravity: GRAVITY,
        airspeed: speed,
        horizontal_airspeed: horiz_airspeed,
        alpha,
        beta,
        cl,
        cd,
        pitch_moment,
        roll_moment,
        yaw_moment,
        engine_power_kw: engine_power_kw_out,
        prop_efficiency: prop_eff_out,
        fuel_flow_gph: fuel_flow_out,
        stall_state: StallState::from_alpha(alpha.abs()),
    }
}

/// Post-stall asymmetric-Cl rolling moment (wing drop).
///
/// Samples `Cl(α)` at each wing's quarter-span with the α perturbation induced by
/// roll rate (`Δα = ω_roll × y / V` at spanwise offset `y`), then returns the
/// rolling moment from the NON-linear portion of that differential. The linear
/// portion is already captured by the main `Cl_p` damping term.
///
/// In normal flight (α < 80% of stall), the Cl curve is near-linear so the
/// non-linear portion is small. Past stall, the stalled-wing Cl drops off sharply
/// while the other wing may still be making lift, producing a destabilizing
/// rolling moment that reinforces any small roll rate → wing drop / spin entry.
#[must_use]
fn wing_drop_roll_moment(
    alpha: f32,
    roll_rate: f32,
    v_damp: f32,
    q_dyn: f32,
    wing_area: f32,
) -> f32 {
    if alpha.abs() < ALPHA_APPROACH_RAD {
        // Well clear of stall: Cl curve is essentially linear, non-linear portion is
        // zero by definition. Skip the work.
        return 0.0;
    }
    // Spanwise α perturbation at the quarter-span representative point.
    let delta_alpha = roll_rate * WING_SPAN_M * WING_DROP_SAMPLE_FRAC / v_damp;
    let alpha_right = alpha + delta_alpha;
    let alpha_left = alpha - delta_alpha;
    let cl_right = lift_coefficient(alpha_right);
    let cl_left = lift_coefficient(alpha_left);
    // Linear-theory differential (captured by Cl_p): 2 × (dCl/dα) × Δα.
    // Estimate the local slope from a narrow two-point sample around the current α.
    let d = 0.01_f32;
    let cl_slope_local = (lift_coefficient(alpha + d) - lift_coefficient(alpha - d)) / (2.0 * d);
    let linear_diff = 2.0 * cl_slope_local * delta_alpha;
    // The non-linear "surprise" past stall: actual differential minus the linear
    // piece. Rolling moment follows with the physics-convention sign (more lift on
    // right → roll LEFT → negative L per our convention).
    let cl_nonlinear_diff = (cl_right - cl_left) - linear_diff;
    -q_dyn * wing_area * WING_SPAN_M * 0.125 * cl_nonlinear_diff
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
            fuel_gal: 0.0,
            config: crate::config::VehicleConfig::default(),
        }
    }

    #[test]
    fn at_rest_idle_only_gravity() {
        let out = compute_forces(&zero_state(), None, 1.0 / 120.0, 1000.0);
        assert!((out.linear_accel - GRAVITY).length() < 1e-5);
        // At rest (q = 0), every moment term should vanish.
        assert!(out.angular_accel.length() < 1e-6);
    }

    #[test]
    fn full_throttle_pushes_forward() {
        let mut s = zero_state();
        s.controls.throttle = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        // Forward is -Z at identity orientation, so horizontal component is negative Z.
        assert!(out.linear_accel.z < -1.0);
    }

    #[test]
    fn lift_rises_with_horizontal_airspeed() {
        // No-spec placeholder path: lift = horiz_airspeed * LIFT_COEFF.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -20.0);
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert!(
            out.linear_accel.y > GRAVITY.y + 3.0,
            "y accel {}",
            out.linear_accel.y
        );
    }

    #[test]
    fn alpha_zero_when_flying_level_forward() {
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -30.0);
        let (alpha, beta) = angle_of_attack_and_sideslip(&s);
        assert!(alpha.abs() < 1e-5);
        assert!(beta.abs() < 1e-5);
    }

    #[test]
    fn alpha_positive_when_velocity_has_body_neg_y() {
        // Plane pitched up 10°, moving in the pre-pitch world velocity direction: the
        // pitched plane sees airflow coming from below -> positive α.
        let mut s = zero_state();
        s.orientation = Quat::from_rotation_x(10_f32.to_radians());
        s.velocity = Vec3::new(0.0, 0.0, -30.0);
        let (alpha, _beta) = angle_of_attack_and_sideslip(&s);
        assert!(alpha > 0.08, "alpha {alpha}");
        assert!(alpha < 0.20);
    }

    #[test]
    fn cl_zero_at_minus_two_degrees() {
        let cl = lift_coefficient((-2.0_f32).to_radians());
        assert!(cl.abs() < 1e-3, "Cl at -2° should be ~0, got {cl}");
    }

    #[test]
    fn cl_peaks_near_sixteen_degrees() {
        let cl_peak = lift_coefficient(16.0_f32.to_radians());
        let cl_clean = lift_coefficient(5.0_f32.to_radians());
        assert!(cl_peak > cl_clean);
        // Whole-wing Cl_max target 2.0 after Phase 2.4.8 Gap 3 tune
        // (was 1.65 pre-tune; see PROGRESS.md § Bench-identified gaps).
        assert!(cl_peak > 1.9);
        assert!(cl_peak < 2.2);
    }

    #[test]
    fn lift_is_perpendicular_to_velocity_on_airfoil_path() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        // Plane pitched up 10°, moving level horizontally -- α > 0 so Cl > 0 so lift
        // exists. Assert lift vector dot velocity ≈ 0 (orthogonal).
        let mut s = zero_state();
        s.orientation = Quat::from_rotation_x(10_f32.to_radians());
        s.velocity = Vec3::new(0.0, 0.0, -40.0);
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2000.0,
            max_mass_floats_kg: 2000.0,
            wing_area_m2: 20.0,
            aspect_ratio: 9.0,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 70.0,
                cruise_mps: 60.0,
                vs_clean_mps: 23.0,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        let out = compute_forces(&s, Some(&spec), 1.0 / 120.0, 1000.0);
        assert!(out.lift.length() > 1.0, "lift should be non-trivial");
        let dot = out.lift.dot(s.velocity) / (out.lift.length() * s.velocity.length());
        assert!(
            dot.abs() < 1e-3,
            "lift should be perpendicular to velocity; cos θ = {dot}",
        );
    }

    #[test]
    fn cl_drops_post_stall() {
        let cl_peak = lift_coefficient(16.0_f32.to_radians());
        let cl_stalled = lift_coefficient(22.0_f32.to_radians());
        assert!(cl_stalled < cl_peak);
    }

    #[test]
    fn drag_grows_with_cl_squared() {
        let cd_low = drag_coefficient(5.0_f32.to_radians(), 0.75, 9.2);
        let cd_high = drag_coefficient(12.0_f32.to_radians(), 1.2, 9.2);
        assert!(cd_high > cd_low);
    }

    #[test]
    fn elevator_at_cruise_produces_nose_up_moment() {
        // At cruise airspeed with full nose-up stick, the control contribution of the
        // pitch moment should be positive (nose up in physics convention) and the
        // body-frame angular_accel.x component should be positive.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -50.0);
        s.controls.pitch = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert!(
            out.pitch_moment.control > 0.0,
            "pitch control moment should be positive with nose-up stick; got {}",
            out.pitch_moment.control,
        );
        assert!(
            out.angular_accel.x > 0.0,
            "pitch angular accel should be positive (nose up); got {}",
            out.angular_accel.x,
        );
    }

    #[test]
    fn rudder_at_cruise_produces_nose_right_yaw() {
        // Positive yaw stick -> positive yaw moment N (nose right in physics) ->
        // stored as negative Y angular accel (flowplan storage convention).
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -50.0);
        s.controls.yaw = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert!(
            out.yaw_moment.control > 0.0,
            "yaw control moment positive for nose-right pedal; got {}",
            out.yaw_moment.control,
        );
        assert!(
            out.angular_accel.y < 0.0,
            "nose-right yaw maps to negative Y angular accel; got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn aileron_at_cruise_produces_right_wing_down_roll() {
        // Positive roll stick -> positive roll moment L (right wing down) ->
        // stored as negative Z angular accel.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -50.0);
        s.controls.roll = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert!(
            out.roll_moment.control > 0.0,
            "roll control moment positive for right-wing-down stick; got {}",
            out.roll_moment.control,
        );
        assert!(
            out.angular_accel.z < 0.0,
            "right-wing-down roll maps to negative Z angular accel; got {}",
            out.angular_accel.z,
        );
    }

    #[test]
    fn pitch_damping_opposes_pitch_rate() {
        // With the aircraft already pitching nose-up (positive angular_velocity.x) and
        // neutral stick, the damping term should produce a nose-down moment to
        // oppose the motion.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -50.0);
        s.angular_velocity = Vec3::new(0.5, 0.0, 0.0); // 0.5 rad/s pitch-up
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert!(
            out.pitch_moment.damping < 0.0,
            "pitch damping should be negative when pitching up; got {}",
            out.pitch_moment.damping,
        );
    }

    #[test]
    fn weathervane_restores_to_velocity() {
        // Plane yawed 10° left of its velocity vector. flowplan's β is negative in this
        // case; aerospace β is positive (wind from the right). Restoring moment is
        // nose-right (positive N), which maps to negative Y angular accel.
        let mut s = zero_state();
        s.orientation = Quat::from_rotation_y(10_f32.to_radians());
        s.velocity = Vec3::new(0.0, 0.0, -50.0);
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert!(
            out.yaw_moment.stability > 0.0,
            "weathervane moment should be nose-right for nose-left-of-velocity; got {}",
            out.yaw_moment.stability,
        );
        assert!(
            out.angular_accel.y < 0.0,
            "weathervane-induced yaw accel should be negative Y (nose right); got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn alpha_stability_restores_toward_trim() {
        // Nose-up attitude relative to velocity (α > 0) should produce a nose-down
        // stability moment (pitch.stability < 0, angular_accel.x < 0).
        let mut s = zero_state();
        s.orientation = Quat::from_rotation_x(10_f32.to_radians());
        s.velocity = Vec3::new(0.0, 0.0, -50.0);
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert!(
            out.pitch_moment.stability < 0.0,
            "α-stability should be nose-down for α>0; got {}",
            out.pitch_moment.stability,
        );
    }

    // ---- Engine + prop model (2.4.3) ----

    #[test]
    fn engine_power_zero_throttle_returns_idle_when_fueled() {
        let p = engine_power_kw(0.0, true);
        assert!(
            (p - ENGINE_POWER_IDLE_KW).abs() < 1e-3,
            "idle should be {ENGINE_POWER_IDLE_KW} kW, got {p}",
        );
    }

    #[test]
    fn engine_power_full_throttle_returns_max_when_fueled() {
        let p = engine_power_kw(1.0, true);
        assert!(
            (p - ENGINE_POWER_MAX_KW).abs() < 1e-3,
            "full should be {ENGINE_POWER_MAX_KW} kW, got {p}",
        );
    }

    #[test]
    fn engine_power_zero_when_out_of_fuel() {
        let p = engine_power_kw(1.0, false);
        assert!(p < 1e-6, "expected 0 power out of fuel, got {p}");
    }

    #[test]
    fn prop_efficiency_peaks_at_cruise() {
        let eta_static = prop_efficiency(0.0);
        let eta_cruise = prop_efficiency(55.0);
        let eta_vne = prop_efficiency(85.0);
        assert!(eta_cruise > eta_static);
        assert!(eta_cruise > eta_vne);
    }

    #[test]
    fn thrust_finite_at_zero_airspeed() {
        // Classic prop-blade theory has T → ∞ as V → 0 for a fixed-power prop. Our
        // model clamps the denominator to prevent this; static thrust at max power
        // should be in the real-world ballpark (~9 kN for a 450 hp Beaver).
        let t = thrust_force_newtons(1.0, 0.0, true);
        assert!(t.is_finite());
        // Real Beaver static thrust ≈ 9 kN. Allow wide band while tuning.
        assert!(
            (5_000.0..=12_000.0).contains(&t),
            "static thrust should be in the 5-12 kN ballpark, got {t}",
        );
    }

    #[test]
    fn thrust_at_cruise_roughly_balances_drag() {
        // 0.55 throttle at 55 m/s cruise -> thrust should be ~1.2-1.5 m/s² accel.
        let t_n = thrust_force_newtons(0.55, 55.0, true);
        let a = t_n / 2313.0;
        assert!(
            (0.5..=2.5).contains(&a),
            "cruise thrust accel should be ~1-2 m/s², got {a}",
        );
    }

    #[test]
    fn fuel_flow_scales_with_power() {
        let gph_idle = fuel_flow_gph(0.0, true);
        let gph_cruise = fuel_flow_gph(0.55, true);
        let gph_full = fuel_flow_gph(1.0, true);
        assert!(gph_idle < gph_cruise);
        assert!(gph_cruise < gph_full);
        // Beaver cruise burn ~27 gph. Our model maps 0.55 throttle to ~195 kW ->
        // 0.124 × 195 = 24 gph. Close enough for a first cut.
        assert!(
            (18.0..=32.0).contains(&gph_cruise),
            "cruise burn should be 18-32 gph, got {gph_cruise}",
        );
    }

    #[test]
    fn fuel_burn_depletes_tank() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2000.0,
            max_mass_floats_kg: 2000.0,
            wing_area_m2: 20.0,
            aspect_ratio: 9.0,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 70.0,
                cruise_mps: 60.0,
                vs_clean_mps: 23.0,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        let mut s = crate::state::SimState::new(1);
        s.aircraft = Some(spec);
        s.vehicle.fuel_gal = 95.0;
        s.vehicle.velocity = Vec3::new(0.0, 0.0, -55.0);
        s.vehicle.controls.throttle = 1.0;
        // Run 60 s of simulated time; fuel should decrease measurably.
        let start_fuel = s.vehicle.fuel_gal;
        let d = crate::difficulty::DifficultyDials::default();
        let dt = 1.0 / 120.0;
        for _ in 0..(60 * 120) {
            s.tick(&crate::input::InputEvents::empty(), &d, dt, 0.0);
        }
        assert!(
            s.vehicle.fuel_gal < start_fuel,
            "fuel should deplete: start {start_fuel} end {}",
            s.vehicle.fuel_gal,
        );
        // At full throttle (~335 kW × 0.124 gph/kW = ~41.5 gph), 60 s burns ~0.69 gal.
        let burned = start_fuel - s.vehicle.fuel_gal;
        assert!(
            (0.4..=1.0).contains(&burned),
            "60 s full-power burn should be 0.4-1.0 gal, got {burned}",
        );
    }

    // ---- Secondary aero effects (2.4.6) ----

    #[test]
    fn ground_effect_saturates_outside_one_wingspan() {
        let span = 14.63;
        assert!((ground_effect_induced_drag_factor(span, span) - 1.0).abs() < 1e-5);
        assert!((ground_effect_induced_drag_factor(span * 2.0, span) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn ground_effect_cuts_induced_drag_close_to_surface() {
        let span = 14.63;
        let f_very_low = ground_effect_induced_drag_factor(1.0, span);
        let f_quarter = ground_effect_induced_drag_factor(span * 0.25, span);
        let f_half = ground_effect_induced_drag_factor(span * 0.5, span);
        assert!(
            f_very_low < 0.7,
            "1 m AGL factor should be <0.7; got {f_very_low}",
        );
        assert!(f_quarter < 0.9);
        assert!(f_half < 1.0 && f_half > 0.85);
    }

    #[test]
    fn adverse_yaw_opposes_roll_input() {
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -55.0);
        s.controls.roll = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        // Adverse yaw lives in yaw_moment.coupling. No-spec path has no P-factor
        // (legacy thrust constant), so coupling here is pure adverse yaw.
        assert!(
            out.yaw_moment.coupling < 0.0,
            "adverse yaw should be negative for positive aileron; got {}",
            out.yaw_moment.coupling,
        );
    }

    #[test]
    fn p_factor_yaws_nose_left_at_high_alpha_with_power() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.0,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 71.5,
                cruise_mps: 64.3,
                vs_clean_mps: 23.25,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        let mut s = zero_state();
        s.fuel_gal = 50.0;
        s.orientation = Quat::from_rotation_x(12_f32.to_radians());
        s.velocity = Vec3::new(0.0, 0.0, -30.0);
        s.controls.throttle = 1.0;
        let out = compute_forces(&s, Some(&spec), 1.0 / 120.0, 1000.0);
        assert!(
            out.yaw_moment.coupling < 0.0,
            "P-factor at high α + full power should yaw nose left; coupling = {}",
            out.yaw_moment.coupling,
        );
    }

    #[test]
    fn coupling_ramps_off_at_low_airspeed() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.0,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 71.5,
                cruise_mps: 64.3,
                vs_clean_mps: 23.25,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        let mut s = zero_state();
        s.fuel_gal = 50.0;
        s.controls.throttle = 1.0;
        // Stationary on ground. Without the ramp, torque and P-factor would still
        // fire; with it, coupling on both axes must be zero.
        let out = compute_forces(&s, Some(&spec), 1.0 / 120.0, 0.0);
        assert!(
            out.roll_moment.coupling.abs() < 1e-3,
            "torque roll should be zero at V=0; got {}",
            out.roll_moment.coupling,
        );
        assert!(
            out.yaw_moment.coupling.abs() < 1e-3,
            "P-factor + adverse yaw should be zero at V=0 with no aileron; got {}",
            out.yaw_moment.coupling,
        );
    }

    // ---- Density altitude (2.4.7) ----

    #[test]
    fn density_at_sea_level_matches_isa() {
        assert!((air_density(0.0) - 1.225).abs() < 1e-5);
    }

    #[test]
    fn density_drops_with_altitude() {
        let sl = air_density(0.0);
        let at_1km = air_density(1000.0);
        let at_3km = air_density(3000.0);
        let at_5km = air_density(5000.0);
        assert!(at_1km < sl);
        assert!(at_3km < at_1km);
        assert!(at_5km < at_3km);
        // Spot-check the exponential approximation at 3 km (~71% of SL).
        assert!(
            (density_ratio(3000.0) - 0.702).abs() < 0.02,
            "3 km density ratio should be ~0.71; got {}",
            density_ratio(3000.0),
        );
    }

    #[test]
    fn thrust_drops_with_altitude() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.0,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 71.5,
                cruise_mps: 64.3,
                vs_clean_mps: 23.25,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        // At SL, full throttle, cruise airspeed.
        let mut s_sl = zero_state();
        s_sl.fuel_gal = 50.0;
        s_sl.velocity = Vec3::new(0.0, 0.0, -55.0);
        s_sl.controls.throttle = 1.0;
        s_sl.position = Vec3::new(0.0, 0.0, 0.0);
        let out_sl = compute_forces(&s_sl, Some(&spec), 1.0 / 120.0, 1000.0);

        // Same state at 3 km altitude.
        let mut s_alt = s_sl;
        s_alt.position = Vec3::new(0.0, 3000.0, 0.0);
        let out_alt = compute_forces(&s_alt, Some(&spec), 1.0 / 120.0, 1000.0);

        // Engine power should be ~71% of sea-level.
        assert!(
            out_alt.engine_power_kw < out_sl.engine_power_kw * 0.80,
            "power should drop significantly at 3 km; SL {} alt {}",
            out_sl.engine_power_kw,
            out_alt.engine_power_kw,
        );
        // Thrust magnitude should also drop noticeably.
        let sl_thrust = out_sl.thrust.length();
        let alt_thrust = out_alt.thrust.length();
        assert!(
            alt_thrust < sl_thrust * 0.80,
            "thrust should drop with altitude; SL {sl_thrust} alt {alt_thrust}",
        );
    }

    #[test]
    fn lift_drops_with_altitude() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.0,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 71.5,
                cruise_mps: 64.3,
                vs_clean_mps: 23.25,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        let mut s_sl = zero_state();
        s_sl.fuel_gal = 50.0;
        s_sl.velocity = Vec3::new(0.0, 0.0, -55.0);
        s_sl.orientation = Quat::from_rotation_x(5_f32.to_radians());
        s_sl.position = Vec3::new(0.0, 0.0, 0.0);
        let out_sl = compute_forces(&s_sl, Some(&spec), 1.0 / 120.0, 1000.0);

        let mut s_alt = s_sl;
        s_alt.position = Vec3::new(0.0, 3000.0, 0.0);
        let out_alt = compute_forces(&s_alt, Some(&spec), 1.0 / 120.0, 1000.0);

        let sl_lift = out_sl.lift.length();
        let alt_lift = out_alt.lift.length();
        assert!(
            alt_lift < sl_lift * 0.80,
            "lift should drop ~29% at 3 km; SL {sl_lift} alt {alt_lift}",
        );
    }

    #[test]
    fn torque_roll_is_negative_under_power() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.0,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 71.5,
                cruise_mps: 64.3,
                vs_clean_mps: 23.25,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        let mut s = zero_state();
        s.fuel_gal = 50.0;
        s.velocity = Vec3::new(0.0, 0.0, -55.0);
        s.controls.throttle = 1.0;
        let out = compute_forces(&s, Some(&spec), 1.0 / 120.0, 1000.0);
        assert!(
            out.roll_moment.coupling < 0.0,
            "torque roll should be negative under power; got {}",
            out.roll_moment.coupling,
        );
    }

    // ---- Flaps (2.4.5) ----

    #[test]
    fn flap_deltas_grow_with_extension() {
        let up = flap_aero(FlapPosition::Up);
        let climb = flap_aero(FlapPosition::Climb);
        let takeoff = flap_aero(FlapPosition::Takeoff);
        let landing = flap_aero(FlapPosition::Landing);
        // Cl rises monotonically with extension.
        assert!(up.delta_cl < climb.delta_cl);
        assert!(climb.delta_cl < takeoff.delta_cl);
        assert!(takeoff.delta_cl < landing.delta_cl);
        // Cd0 rises monotonically with extension.
        assert!(up.delta_cd0 < climb.delta_cd0);
        assert!(climb.delta_cd0 < takeoff.delta_cd0);
        assert!(takeoff.delta_cd0 < landing.delta_cd0);
        // Cm (pitching moment) gets more nose-down (more negative) with extension.
        assert!(up.delta_cm > climb.delta_cm);
        assert!(climb.delta_cm > takeoff.delta_cm);
        assert!(takeoff.delta_cm > landing.delta_cm);
    }

    #[test]
    fn landing_flaps_raise_cl_in_compute_forces() {
        use crate::aircraft_spec::{AircraftSpec, EngineSpec, SpeedsSpec};
        let spec = AircraftSpec {
            name: "test".into(),
            inspiration: String::new(),
            empty_mass_kg: 1000.0,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.0,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: String::new(),
            airfoil_tip: String::new(),
            engine: EngineSpec {
                name: String::new(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: SpeedsSpec {
                vne_mps: 71.5,
                cruise_mps: 64.3,
                vs_clean_mps: 23.25,
                vso_mps: 21.0,
                vx_mps: 29.0,
                vy_mps: 38.0,
            },
            variants: Vec::new(),
            default_variant: String::new(),
        };
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -40.0);
        s.fuel_gal = 50.0;
        // Clean.
        s.controls.flaps = FlapPosition::Up;
        let out_clean = compute_forces(&s, Some(&spec), 1.0 / 120.0, 1000.0);
        // Landing.
        s.controls.flaps = FlapPosition::Landing;
        let out_land = compute_forces(&s, Some(&spec), 1.0 / 120.0, 1000.0);
        assert!(
            out_land.cl > out_clean.cl + 0.2,
            "landing flaps should add ≥0.2 to Cl; clean {} land {}",
            out_clean.cl,
            out_land.cl,
        );
        assert!(
            out_land.cd > out_clean.cd + 0.02,
            "landing flaps should add ≥0.02 to Cd; clean {} land {}",
            out_clean.cd,
            out_land.cd,
        );
        // Flaps down produces a nose-down pitching contribution (strictly more
        // negative stability term than clean at same α).
        assert!(
            out_land.pitch_moment.stability < out_clean.pitch_moment.stability,
            "flaps-down stability should be more nose-down; clean {} land {}",
            out_clean.pitch_moment.stability,
            out_land.pitch_moment.stability,
        );
    }

    #[test]
    fn flap_extend_retract_clamps_at_endpoints() {
        assert_eq!(FlapPosition::Landing.extended(), FlapPosition::Landing);
        assert_eq!(FlapPosition::Up.retracted(), FlapPosition::Up);
        assert_eq!(FlapPosition::Up.extended(), FlapPosition::Climb);
        assert_eq!(
            FlapPosition::Climb.extended().extended(),
            FlapPosition::Landing
        );
        assert_eq!(
            FlapPosition::Landing.retracted().retracted(),
            FlapPosition::Climb
        );
    }

    // ---- Real stall physics (2.4.4d) ----

    #[test]
    fn stall_state_classifies_correctly() {
        assert_eq!(StallState::from_alpha(0.0), StallState::Normal);
        assert_eq!(StallState::from_alpha(0.1), StallState::Normal);
        assert_eq!(StallState::from_alpha(0.23), StallState::Approach);
        assert_eq!(StallState::from_alpha(0.30), StallState::Stall);
    }

    #[test]
    fn past_stall_pitching_moment_is_nose_down() {
        // At α well past stall (20°), the stability moment should include a strong
        // nose-down (negative) contribution beyond linear Cm_α.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -40.0);
        s.orientation = Quat::from_rotation_x(20_f32.to_radians());
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert_eq!(out.stall_state, StallState::Stall);
        // Pre-stall only Cm_α × α would give a stability moment of size
        // |CM_ALPHA × α × q × S × c|. The post-stall addend should drive |stability|
        // higher than that.
        let q = 0.5 * AIR_DENSITY_SEA_LEVEL * 40.0 * 40.0;
        let linear_only = CM_ALPHA * 20.0_f32.to_radians() * q * 23.23 * MEAN_CHORD_M;
        assert!(
            out.pitch_moment.stability < linear_only,
            "stall-pitch should be more negative than pure Cm_α × α; got {} vs linear {}",
            out.pitch_moment.stability,
            linear_only,
        );
    }

    #[test]
    fn normal_alpha_produces_zero_wing_drop() {
        // At cruise α the Cl curve is near-linear, so wing-drop (which captures
        // non-linearity) should be approximately zero.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -55.0);
        s.angular_velocity.z = -0.3; // rolling right, our storage convention
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        // stability slot on roll moment is repurposed for wing drop.
        assert!(
            out.roll_moment.stability.abs() < 100.0,
            "normal-α wing drop should be ~0; got {}",
            out.roll_moment.stability,
        );
    }

    #[test]
    fn past_stall_rolling_produces_wing_drop() {
        // Past stall with positive roll rate, the right wing is deeper into stall
        // (higher α from the rolling motion). Its Cl drops more than the left's, so
        // lift asymmetry produces a destabilizing rolling moment (same sign as the
        // rate, reinforcing the drop). With our convention: right-wing-down roll is
        // L > 0, so a right-wing-down rate (ω_roll > 0 in physics convention, which
        // is -Z in storage) that is getting reinforced means wing_drop > 0.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -30.0);
        s.orientation = Quat::from_rotation_x(18_f32.to_radians()); // past stall
        s.angular_velocity.z = -0.5; // storage: negative Z = roll right = ω_roll +
        let out = compute_forces(&s, None, 1.0 / 120.0, 1000.0);
        assert_eq!(out.stall_state, StallState::Stall);
        // Non-zero wing-drop past stall. Direction verifies reinforcement.
        assert!(
            out.roll_moment.stability.abs() > 50.0,
            "past-stall wing drop should be non-trivial; got {}",
            out.roll_moment.stability,
        );
    }

    #[test]
    fn control_authority_scales_with_airspeed() {
        // Moments scale with q = 0.5 ρ V². Doubling speed should give ~4× moment.
        let mut slow = zero_state();
        slow.velocity = Vec3::new(0.0, 0.0, -25.0);
        slow.controls.pitch = 1.0;
        let out_slow = compute_forces(&slow, None, 1.0 / 120.0, 1000.0);

        let mut fast = zero_state();
        fast.velocity = Vec3::new(0.0, 0.0, -50.0);
        fast.controls.pitch = 1.0;
        let out_fast = compute_forces(&fast, None, 1.0 / 120.0, 1000.0);

        let ratio = out_fast.pitch_moment.control / out_slow.pitch_moment.control;
        assert!(
            (ratio - 4.0).abs() < 0.1,
            "pitch control moment should ~4× from 25 -> 50 m/s; got ratio {ratio}",
        );
    }
}
