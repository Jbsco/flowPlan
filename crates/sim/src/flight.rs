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
//!
//! Open (still to ship in Phase 2.4):
//!
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
/// Idle shaft power (kW). ~6% of max; represents residual combustion power that keeps
/// the engine running at idle RPM and generates a small amount of prop thrust.
const ENGINE_POWER_IDLE_KW: f32 = 20.0;

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
/// TODO(2.4.x): move to `AircraftSpec` so other aircraft tune independently.
const I_XX: f32 = 8000.0; // pitch (rotation about right-wing axis)
const I_YY: f32 = 12000.0; // yaw (rotation about up axis)
const I_ZZ: f32 = 5000.0; // roll (rotation about nose axis)

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
const CM_DELTA_E: f32 = 0.24;
const CM_Q: f32 = -25.0;

/// Roll: aileron command (`Cl_δa > 0`) rolls right wing down for positive stick;
/// damping (`Cl_p < 0`) opposes roll rate. No static roll stability term in the
/// baseline (small aircraft are roughly roll-neutral; dihedral-induced coupling
/// through β lives in 2.4.6).
///
/// `Cl_δa` tuning: 0.08 (too lively) → 0.07 (slightly weak) → 0.075 (smidge up).
const CL_DELTA_A: f32 = 0.075;
const CL_P: f32 = -0.5;

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

/// Stall α (rad). Matches the peak of the `CL_TABLE` (`Cl_max` at 16°). Past this α
/// the airfoil loses Cl and the center of lift shifts aft.
const ALPHA_STALL_RAD: f32 = 0.2793;

/// Approach-to-stall α (rad). 80% of stall; HUD flags this as "APPROACH".
const ALPHA_APPROACH_RAD: f32 = 0.2234;

/// Pitch-moment coefficient per radian of α past stall. Produces a nose-down
/// moment when α > `α_stall`, modeling the aft shift of the center of lift at stall.
/// Real aircraft self-recover from stall partly through this mechanism; small
/// deviations past stall generate restoring moment proportional to how far past.
const CM_STALL_PITCH_SLOPE: f32 = -2.0;

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
/// TODO(2.4.7): density altitude -- `ρ(h, T) = ρ₀ × (1 - Lh/T₀)^(g M / R L - 1)`.
const AIR_DENSITY_SEA_LEVEL: f32 = 1.225;

/// Zero-lift drag coefficient (clean airframe, no flaps).
///
/// Tuning history:
///   0.028 → "Cessna-retractable slippery"; `L/D_max` ~14 at cruise, plane barely slows
///           with throttle off (flight-test feedback: "feels like it glides forever").
///   0.045 → current; `L/D_max` ~11 at cruise, matches typical DHC-2 published figures.
///           A Beaver is a boxy airframe -- fixed gear, strut-braced wing, radial cowl,
///           external fuel caps, the whole lineup of parasite-drag sources.
///
/// TODO(2.4.5): flap-deployment bumps this by ~0.02 per notch.
const CD0: f32 = 0.045;

/// Oswald efficiency factor. Typical for small single-engine aircraft: 0.7-0.85.
/// Set slightly toward the low end (0.75) because strut-bracing and fixed gear
/// disturb the lift distribution away from ideal elliptical.
const OSWALD_E: f32 = 0.75;

/// NACA 64A416-shaped `Cl(α)` table adjusted for finite-span whole-wing effects.
/// `α` in radians; `Cl` dimensionless. Piecewise linearly interpolated. Zero-lift at
/// ~-2°, linear slope ~0.10/deg through the clean range, peak `Cl_max` ≈ 1.65 at 16°
/// (2D section `Cl_max` ~1.4 lifted to ~1.65 to approximate whole-wing max with fuselage
/// lift contribution and without flaps), post-stall drop to 1.25 by 18° and 1.0 by
/// 20°, deep-stall plateau near 0.55 out to 60°. Mirror-extended to negative α for
/// inverted flight.
///
/// TODO(2.4.2): per-aircraft tables should live in `AircraftSpec` rather than being
/// hard-coded here. For the MVP (Beaver) this matches published NACA TN-1945-ish
/// figures closely enough.
#[allow(clippy::approx_constant)]
const CL_TABLE: &[(f32, f32)] = &[
    (-1.5708, 0.0),   // -90° (flying backward / full sideways)
    (-0.5236, -0.85), // -30°
    (-0.3491, -1.25), // -20°
    (-0.2793, -1.50), // -16° (inverted Cl_max -- mirrored but still real)
    (-0.1745, -1.10), // -10°
    (-0.0873, -0.65), // -5°
    (-0.0349, 0.00),  // -2° zero-lift
    (0.0000, 0.21),   // 0°
    (0.0873, 0.80),   // 5°
    (0.1745, 1.25),   // 10°
    (0.2443, 1.55),   // 14°
    (0.2793, 1.65),   // 16° Cl_max
    (0.3142, 1.40),   // 18° post-stall
    (0.3491, 1.10),   // 20°
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
/// `drag_coefficient_with_cd0_offset` directly so the flap `Cd0` contribution is
/// always applied.
#[cfg(test)]
fn drag_coefficient(alpha_rad: f32, cl: f32, aspect_ratio: f32) -> f32 {
    drag_coefficient_with_cd0_offset(alpha_rad, cl, aspect_ratio, 0.0)
}

/// Same as `drag_coefficient`, with a `Cd0` offset (flap-deployment contribution).
fn drag_coefficient_with_cd0_offset(
    alpha_rad: f32,
    cl: f32,
    aspect_ratio: f32,
    delta_cd0: f32,
) -> f32 {
    let induced = cl * cl / (core::f32::consts::PI * aspect_ratio.max(1.0) * OSWALD_E);
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
/// you back").
#[derive(Clone, Copy, Debug, Default)]
pub struct MomentBreakdown {
    /// Control-surface contribution (elevator / aileron / rudder × stick position).
    pub control: f32,
    /// Static-stability contribution (α for pitch, β for yaw; zero for roll).
    pub stability: f32,
    /// Damping contribution (opposes the current angular rate about this axis).
    pub damping: f32,
}

impl MomentBreakdown {
    /// Sum of the three contributions.
    #[must_use]
    pub fn total(&self) -> f32 {
        self.control + self.stability + self.damping
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
) -> FlightOutputs {
    let forward = state.orientation * Vec3::NEG_Z;
    let speed = state.velocity.length();

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
            let engine_power = engine_power_kw(state.controls.throttle, fuel_ok);
            let prop_eff = prop_efficiency(speed);
            let thrust_n = thrust_force_newtons(state.controls.throttle, speed, fuel_ok);
            let thrust_vec = forward * (thrust_n / mass_kg);
            let fuel_gph = fuel_flow_gph(state.controls.throttle, fuel_ok);

            // Airfoil lift + drag polar (2.4.2) with flap deltas (2.4.5). Flaps shift
            // the Cl curve up by `delta_cl` (camber increase), add a zero-lift drag
            // penalty, and produce a nose-down pitching moment (applied later to
            // `pitch_moment.stability`).
            let flap_deltas = flap_aero(state.controls.flaps);
            let cl = lift_coefficient(alpha) + flap_deltas.delta_cl;
            // Induced drag uses the new (higher) Cl; parasite Cd0 gets the flap add.
            let cd =
                drag_coefficient_with_cd0_offset(alpha, cl, aspect_ratio, flap_deltas.delta_cd0);
            let q = 0.5 * AIR_DENSITY_SEA_LEVEL * speed * speed;
            let lift_force_n = q * wing_area * cl;
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
            let lift_vec = lift_dir * (lift_force_n / mass_kg);

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
    let q_dyn = 0.5 * AIR_DENSITY_SEA_LEVEL * speed * speed;
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

    // Pitch: control (elevator) + α-stability (with post-stall nose-down and flap
    // pitching-moment contribution) + pitch-rate damping. Past α_stall, the center
    // of lift shifts aft, producing an additional nose-down moment that tries to
    // self-recover the aircraft. Flap deployment also produces a nose-down moment
    // (Cm_0 shift -- the lift increment acts aft of CG for positive-camber airfoils).
    let stall_excess = (alpha.abs() - ALPHA_STALL_RAD).max(0.0);
    let stall_pitch_addend = CM_STALL_PITCH_SLOPE * stall_excess * alpha.signum();
    let flap_cm = flap_aero(state.controls.flaps).delta_cm;
    let pitch_moment = MomentBreakdown {
        control: q_dyn * wing_area_moments * MEAN_CHORD_M * CM_DELTA_E * state.controls.pitch,
        stability: q_dyn
            * wing_area_moments
            * MEAN_CHORD_M
            * (CM_ALPHA * alpha + stall_pitch_addend + flap_cm),
        damping: q_dyn * wing_area_moments * MEAN_CHORD_M * CM_Q * pitch_damp_nd,
    };

    // Roll: control (aileron) + roll-rate damping (Cl_p) + wing-drop asymmetry when
    // near or past stall. The wing-drop term samples Cl(α) at the left and right
    // wing's quarter-span with roll-rate-induced α perturbation, and uses the
    // non-linear portion of that differential as a destabilizing rolling moment past
    // stall. In normal flight the Cl curve is near-linear so this term is small;
    // past stall, Cl drops off sharply on the stalled wing and the imbalance
    // reinforces the roll instead of damping it (spin entry potential).
    let wing_drop_moment =
        wing_drop_roll_moment(alpha, roll_rate, v_damp, q_dyn, wing_area_moments);
    let roll_moment = MomentBreakdown {
        control: q_dyn * wing_area_moments * WING_SPAN_M * CL_DELTA_A * state.controls.roll,
        stability: wing_drop_moment, // reusing the "stability" slot for post-stall asymmetry
        damping: q_dyn * wing_area_moments * WING_SPAN_M * CL_P * roll_damp_nd,
    };

    // Yaw: control (rudder) + β-weathervane + yaw-rate damping.
    //
    // flowplan's `beta = atan2(body_vel.x, forward_speed)` matches aerospace β in sign:
    // both are positive when the aircraft has rightward lateral body velocity (wind
    // from the right). With `Cn_β > 0`, the stability term produces a positive N
    // (nose-right in physics convention) to restore.
    let yaw_moment = MomentBreakdown {
        control: q_dyn * wing_area_moments * WING_SPAN_M * CN_DELTA_R * state.controls.yaw,
        stability: q_dyn * wing_area_moments * WING_SPAN_M * CN_BETA * beta,
        damping: q_dyn * wing_area_moments * WING_SPAN_M * CN_R * yaw_damp_nd,
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
        }
    }

    #[test]
    fn at_rest_idle_only_gravity() {
        let out = compute_forces(&zero_state(), None, 1.0 / 120.0);
        assert!((out.linear_accel - GRAVITY).length() < 1e-5);
        // At rest (q = 0), every moment term should vanish.
        assert!(out.angular_accel.length() < 1e-6);
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
        // No-spec placeholder path: lift = horiz_airspeed * LIFT_COEFF.
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -20.0);
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        // Whole-wing Cl_max target ~1.65 (see CL_TABLE doc comment).
        assert!(cl_peak > 1.5);
        assert!(cl_peak < 1.8);
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
        };
        let out = compute_forces(&s, Some(&spec), 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        };
        let mut s = zero_state();
        s.velocity = Vec3::new(0.0, 0.0, -40.0);
        s.fuel_gal = 50.0;
        // Clean.
        s.controls.flaps = FlapPosition::Up;
        let out_clean = compute_forces(&s, Some(&spec), 1.0 / 120.0);
        // Landing.
        s.controls.flaps = FlapPosition::Landing;
        let out_land = compute_forces(&s, Some(&spec), 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out = compute_forces(&s, None, 1.0 / 120.0);
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
        let out_slow = compute_forces(&slow, None, 1.0 / 120.0);

        let mut fast = zero_state();
        fast.velocity = Vec3::new(0.0, 0.0, -50.0);
        fast.controls.pitch = 1.0;
        let out_fast = compute_forces(&fast, None, 1.0 / 120.0);

        let ratio = out_fast.pitch_moment.control / out_slow.pitch_moment.control;
        assert!(
            (ratio - 4.0).abs() < 0.1,
            "pitch control moment should ~4× from 25 -> 50 m/s; got ratio {ratio}",
        );
    }
}
