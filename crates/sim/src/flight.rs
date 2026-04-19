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

// Shared placeholder constants. Some stay (angular rates) until 2.4.4 replaces them
// with airspeed-scaled / stall-aware authority; others are replaced in 2.4.2 below
// by airfoil-driven force computation when an `AircraftSpec` is provided.

/// Thrust acceleration at full throttle along body forward, m/s².
///
/// TODO(2.4.3): replace with engine power curve `T = P(rpm, map, alt) * η_prop / V`.
/// A real 450 hp R-985 at 80% prop efficiency produces roughly 5.8 m/s² at takeoff
/// airspeed (V=20 m/s, T=13.4 kN / 2313 kg) and ~1.95 m/s² at cruise (V=60 m/s);
/// the constant 3.5 is a compromise that gives the plane enough climb / maneuver
/// margin without overshooting Vne in level flight. A full power curve lands here
/// in 2.4.3.
const THRUST_ACCEL_MAX: f32 = 3.5;

/// Peak commanded angular rate per body axis (rad/s).
const ROLL_RATE_MAX: f32 = 1.6;
const PITCH_RATE_MAX: f32 = 1.0;
const YAW_RATE_MAX: f32 = 0.6;

/// First-order tracking rate toward commanded angular velocity (1/s).
const ANG_RESPONSE: f32 = 10.0;

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
/// TODO(2.4.5): flap-deployment bumps this by ~0.02 per notch.
const CD0: f32 = 0.028;

/// Oswald efficiency factor. Typical for small single-engine aircraft: 0.7-0.85.
const OSWALD_E: f32 = 0.8;

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
fn drag_coefficient(alpha_rad: f32, cl: f32, aspect_ratio: f32) -> f32 {
    let induced = cl * cl / (core::f32::consts::PI * aspect_ratio.max(1.0) * OSWALD_E);
    // Post-stall drag rises roughly quadratically in |α - α_stall|.
    let stall_excess = (alpha_rad.abs() - 0.2793).max(0.0); // 16° stall boundary
    let stall_drag = stall_excess * stall_excess * 2.0;
    CD0 + induced + stall_drag
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
    /// Angle of attack (rad). Positive when nose points above relative wind.
    pub alpha: f32,
    /// Sideslip angle (rad). Positive when relative wind comes from the right.
    pub beta: f32,
    /// Current lift coefficient (dimensionless). Driven by `alpha` on the airfoil path,
    /// zero on the no-spec placeholder path.
    pub cl: f32,
    /// Current drag coefficient (dimensionless).
    pub cd: f32,
}

/// Compute flight forces for the current vehicle state.
///
/// `spec` carries aircraft-specific data (mass, airfoil, engine). Phase 2.4.1 ignores
/// it; later sub-steps read lift / drag / thrust tables from it.
#[must_use]
pub fn compute_forces(
    state: &VehicleState,
    spec: Option<&AircraftSpec>,
    _dt: f32,
) -> FlightOutputs {
    // Commanded angular rates per body axis.
    // Sign conventions (right-handed, Y-up, -Z forward):
    //   pitch positive = nose up      -> rotate +X (right)
    //   yaw positive   = nose right   -> rotate -Y (positive rot around +Y yaws LEFT)
    //   roll positive  = roll right   -> rotate -Z (positive rot around +Z rolls LEFT)
    //
    // TODO(2.4.4): this entire block is fiat: stick position -> body-frame angular
    //   velocity directly, with no aero in the loop. Real control surfaces produce
    //   *moments* via pressure differentials; the body then experiences angular
    //   *acceleration* proportional to (moment / inertia). The cleanest refactor is:
    //     - rudder deflection δ_r (rad) + β + q (dynamic pressure) -> side force on
    //       vertical stabilizer -> yaw moment N -> body yaw accel (not rate).
    //     - elevator δ_e + α + q -> pitch moment M -> body pitch accel.
    //     - aileron δ_a + q -> differential lift on wings -> roll moment L -> body
    //       roll accel.
    //   Each surface also contributes damping proportional to (rate × q), so the
    //   plane self-stabilizes at altitude but mushes at low speed. This is the
    //   right physical frame for 2.4.4 to sit in; the current fixed-rate model
    //   produces nonphysical behaviors like "full rudder at any airspeed yaws
    //   identically" and "rudder yaw doesn't generate β, the plane just rotates".
    // TODO(2.4.4): airspeed-scaled control authority follows from the moment model
    //   above (q = 0.5 ρ V² is the scaling factor). Until 2.4.4 lands, even an
    //   interim ramp like `authority = (V / V_ref).clamp(0.0, 1.2)` on the rate
    //   constants would be an improvement over the flat rate.
    // TODO(2.4.4): post-stall, elevator authority is normal but ailerons lose authority
    //   (differentially: the retreating-inside-roll blade can be deep in stall). Rudder
    //   retains authority (hence PARE recovery). Falls out of the moment model
    //   naturally once `Cl(α)` on each wing half drives the aileron-moment coupling.
    // TODO(2.4.6): aileron input should couple into a yaw contribution (adverse yaw).
    //   Also falls out of the moment model (extra induced drag on the up-going wing).
    // TODO(2.4.6): throttle + nose-high attitude should couple into a yaw contribution
    //   (P-factor, right-rudder-required on takeoff). Requires prop model (2.4.3).
    // TODO(2.4.4 / UX): keyboard input is 0/1 step. Even under a proper moment model,
    //   "full rudder mashed in one frame" is a more violent command than any human
    //   could produce on a real pedal. An input-layer control ramp (surface deflection
    //   slews toward target at a bounded rate, e.g., ~1.5/s so full deflection needs
    //   ~0.7 s) would smooth this without faking physics. Gamepad / HOTAS analog
    //   support lands in a later UX pass; the ramp is useful regardless.
    let commanded = Vec3::new(
        state.controls.pitch * PITCH_RATE_MAX,
        -state.controls.yaw * YAW_RATE_MAX,
        -state.controls.roll * ROLL_RATE_MAX,
    );

    // Thrust along body forward.
    // TODO(2.4.3): replace constant `THRUST_ACCEL_MAX` with engine power curve.
    let forward = state.orientation * Vec3::NEG_Z;
    let thrust = forward * (state.controls.throttle * THRUST_ACCEL_MAX);

    let speed = state.velocity.length();

    // Always-useful diagnostics:
    let horiz_vel = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
    let horiz_fwd = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let horiz_airspeed = horiz_vel.dot(horiz_fwd).max(0.0);
    let (alpha, beta) = angle_of_attack_and_sideslip(state);

    // Lift / drag branch: airfoil path when an AircraftSpec is available, placeholder
    // otherwise. The airfoil path is physically derived (`0.5 ρ V² S C_*`) and uses
    // mass to convert force to acceleration. `if let / else` stays -- map_or_else
    // would obscure the two distinct physical paths.
    #[allow(clippy::option_if_let_else)]
    let (lift, drag, cl, cd) = if let Some(s) = spec {
        // Mass: MTOW on wheels as the Phase 2.4 placeholder (cargo / fuel update in
        // Phase 4). TODO(4.2): use dynamic mass from current fuel + cargo loadout.
        let mass_kg = s.max_mass_wheels_kg.max(1.0);
        let wing_area = s.wing_area_m2.max(1.0);
        // Aspect ratio comes from the spec; defaults to 1.0 if missing (won't happen
        // for sane RON input).
        let aspect_ratio = s.aspect_ratio.max(1.0);

        let cl = lift_coefficient(alpha);
        let cd = drag_coefficient(alpha, cl, aspect_ratio);
        let q = 0.5 * AIR_DENSITY_SEA_LEVEL * speed * speed; // dynamic pressure, Pa
        let lift_force_n = q * wing_area * cl;
        let drag_force_n = q * wing_area * cd;

        // Lift direction: perpendicular to velocity, in the plane containing velocity
        // and body-up. This is the textbook aerodynamic convention -- lift is by
        // definition orthogonal to the relative wind, not to the wing. Using body +Y
        // instead (the previous placeholder) produced a large spurious backward force
        // at high α (lift × sin α), which decelerated the plane every time the pilot
        // pulled up and triggered stall-cascade behavior with no real control
        // authority. Backward force on the plane now comes only from proper Cd.
        //
        // Construction: start with body-up, project out the velocity component, and
        // normalize. If body-up happens to be parallel to velocity (plane aligned with
        // its flight path in an unusual way), fall back to body-up.
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
        let lift = lift_dir * (lift_force_n / mass_kg);

        // Drag acts opposite velocity.
        let drag = if speed > 1e-3 {
            -state.velocity.normalize_or_zero() * (drag_force_n / mass_kg)
        } else {
            Vec3::ZERO
        };

        (lift, drag, cl, cd)
    } else {
        // No-spec fallback: the Phase 1 horizontal-airspeed placeholder. Kept so
        // pure-sim tests without an AircraftSpec still behave deterministically.
        let lift = Vec3::Y * (horiz_airspeed * LIFT_COEFF);
        let drag = if speed > 1e-3 {
            -state.velocity.normalize_or_zero() * (speed * (LIN_DAMPING + QUAD_DRAG * speed))
        } else {
            Vec3::ZERO
        };
        (lift, drag, 0.0, 0.0)
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
        alpha,
        beta,
        cl,
        cd,
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
    fn commanded_rate_follows_stick() {
        let mut s = zero_state();
        s.controls.pitch = 1.0;
        let out = compute_forces(&s, None, 1.0 / 120.0);
        assert!((out.commanded_angular_vel.x - PITCH_RATE_MAX).abs() < 1e-5);
    }
}
