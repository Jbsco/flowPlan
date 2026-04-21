//! Per-pontoon hydrostatic (buoyancy) + hydrodynamic (drag, damping) forces for
//! float configuration.
//!
//! Phase 2.5a.1 (shipped): computes per-pontoon buoyancy force + moment from the
//! water surface sampled at each float's world position.
//!
//! Phase 2.5a.2: extends with per-pontoon horizontal water drag (quadratic, for
//! realistic plowing) and vertical water damping (linear, to stabilize the
//! buoyancy-gravity oscillation into settling behavior). Both scale with the
//! pontoon's wetted fraction — zero above water, full-strength when fully submerged.
//!
//! The function is still named `compute_buoyancy` because "buoyancy" is the public
//! umbrella term for the water-contact force model in this codebase. Module rename
//! to `hydrodynamics` / `water_contact` is a candidate follow-up once the step
//! transition (2.5a.3) grows the module further.
//!
//! Per-pontoon physics up front means asymmetric behavior (one-float dipped,
//! one-pontoon takeoff in 2.5a.5) emerges for free rather than requiring
//! special-case code later.
//!
//! Beaver float constants (Edo 4930 analogue geometry) are hardcoded here as
//! Beaver placeholders. They migrate to `AircraftSpec` when per-aircraft coefficient
//! migration lands (tracked alongside the other 2.4.x follow-ups).
//!
//! See [ROADMAP.md § Milestone 2.5](../../../ROADMAP.md#milestone-25-ground--water-contact-physics)
//! for the sub-milestone plan and the invariant that contact forces are a
//! deterministic function of `(state, water sample, dt)` — no wall-clock in the
//! contact path. Wave-phase determinism comes from `SimState.wave_time`, which is
//! advanced `dt` per tick and folded into `state_hash`.

use flowplan_world::water_height;
use glam::Vec3;

use crate::aircraft_spec::AircraftSpec;
use crate::flight::{GRAVITY, I_XX, I_YY, I_ZZ};
use crate::vehicle::VehicleState;

/// Fresh-water density (kg/m³). Salt water (~1025) slightly higher; not modeled until
/// water-body classification lands in Phase 3+ hydrology.
const WATER_DENSITY_KG_M3: f32 = 1000.0;

/// Per-pontoon vertical water damping coefficient (N·s/m, linear in velocity), scaled
/// by wetted fraction. Primary role is damping the buoyancy-gravity oscillation into
/// settling behavior. Sized empirically so a Beaver dropped from 1.5 m converges to
/// the 0.66 m analytic equilibrium within ~10 s.
///
/// Scaling by wetted fraction (range 0…1) means the effective damping coefficient
/// at equilibrium draft (~0.29 wet) is roughly a third of full-wet. At full wet the
/// two-pontoon system is supercritical (`c_total = 2·c_per_pontoon = 60 kN·s/m` vs.
/// `c_crit = 2mω ≈ 30.9 kN·s/m`); at equilibrium wet it's ~0.56 of critical.
///
/// Linear (rather than quadratic) for numerical stability near zero velocity — the
/// cost is unphysicality at high vertical speeds that real water drag would penalize
/// quadratically. Revisit if high-speed vertical scenarios (hard touchdown, porpoise
/// recovery) surface a feel gap.
const VERTICAL_DAMPING_N_PER_MPS: f32 = 30_000.0;

/// Per-pontoon horizontal water drag coefficient (N·s²/m², quadratic in velocity),
/// scaled by wetted fraction. Produces plowing drag proportional to `v²`; tuned so
/// the Beaver's idle thrust (~689 N at V<12 m/s with 0.413 prop efficiency) taxis
/// the aircraft at ~4 m/s on water at equilibrium draft (0.29 wetted fraction).
/// Quadratic drag self-stabilizes numerically without issue.
///
/// Step transition (2.5a.3) will modulate this by a planing factor — once the hull
/// rises onto the step, wetted area drops sharply and `c_h_effective` falls, letting
/// the aircraft accelerate out of plow mode. For 2.5a.2 the scaling is pure
/// immersion fraction without the planing correction.
const HORIZONTAL_DRAG_N_PER_MPS2: f32 = 75.0;

/// Per-pontoon geometry. One entry per float.
#[derive(Clone, Copy, Debug)]
struct Pontoon {
    /// Body-frame offset from the aircraft CG to the pontoon geometric center (m).
    body_offset: Vec3,
    /// Pontoon length along the body x-axis... actually along the aircraft longitudinal
    /// axis. Stored as meters.
    length_m: f32,
    /// Pontoon beam (lateral width) in meters.
    beam_m: f32,
    /// Pontoon height (vertical) in meters.
    height_m: f32,
    /// Maximum displacement volume (m³) when fully submerged. Less than the bounding
    /// box volume because the real hull is narrower than a box.
    max_displacement_m3: f32,
}

/// Beaver float set (Edo 4930 analogue geometry). Two pontoons, port/starboard.
/// Length ~6.9 m, beam ~0.76 m, height ~0.76 m per float; max displacement 2.236 m³
/// each (the "4930 lb" in the model number converts to ~2236 kg / 1000 kg·m⁻³).
/// TODO(2.4.x): move to per-aircraft `AircraftSpec` alongside moment coefficients.
const BEAVER_PONTOONS: [Pontoon; 2] = [
    // Port float (aircraft's left side, body -X).
    Pontoon {
        body_offset: Vec3::new(-1.25, -0.5, 0.0),
        length_m: 6.9,
        beam_m: 0.76,
        height_m: 0.76,
        max_displacement_m3: 2.236,
    },
    // Starboard float (aircraft's right side, body +X).
    Pontoon {
        body_offset: Vec3::new(1.25, -0.5, 0.0),
        length_m: 6.9,
        beam_m: 0.76,
        height_m: 0.76,
        max_displacement_m3: 2.236,
    },
];

/// Buoyancy outputs: force + moment integrated to body-frame accelerations, ready to
/// sum with `FlightOutputs.linear_accel` / `angular_accel` in `tick_dynamics`.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuoyancyOutputs {
    /// Linear acceleration contribution in world frame (m/s²).
    pub linear_accel: Vec3,
    /// Angular acceleration contribution in body frame (rad/s²).
    pub angular_accel: Vec3,
    /// Total buoyancy force magnitude across all pontoons (N). Diagnostic.
    pub total_force_n: f32,
}

/// Compute per-pontoon buoyancy + hydrodynamic drag for the current vehicle state.
///
/// Pure function: no hidden state; determinism-safe given the determinism of `wave_time`
/// and `water_height`. When no aircraft spec is loaded (pure-sim tests), returns zero
/// — floats are an aircraft configuration concept, not applicable to the generic cube.
///
/// Short-circuits cleanly when the aircraft is above water (no float submersion → no
/// hydrostatic force, no hydrodynamic drag).
///
/// Per-pontoon force model (summed across `BEAVER_PONTOONS`):
/// - Hydrostatic: `F_buoy = ρ_water · V_submerged · g` (world +Y).
/// - Hydrodynamic vertical damping: `F_v = -c_v · v_pontoon_y · wetted_frac` (world +Y).
/// - Hydrodynamic horizontal drag: `F_h = -c_h · |v_xz| · v_xz · wetted_frac`
///   (world XZ plane; quadratic).
///
/// Per-pontoon velocity includes angular contribution:
/// `v_pontoon = v_cg + (orient · ω_body) × r_world`, so angular rates couple into
/// pontoon drag and produce the expected angular damping (roll damping from asymmetric
/// vertical velocities at the port/starboard floats, yaw damping from asymmetric
/// horizontal velocities).
#[must_use]
pub fn compute_buoyancy(
    v: &VehicleState,
    spec: Option<&AircraftSpec>,
    wave_time: f32,
) -> BuoyancyOutputs {
    let Some(spec) = spec else {
        return BuoyancyOutputs::default();
    };
    let mass_kg = spec.max_mass_wheels_kg;

    let mut total_force_world = Vec3::ZERO;
    let mut total_moment_body = Vec3::ZERO;

    // World-frame angular velocity: rotate the body-frame ω by orientation so
    // cross-product contributions to pontoon velocity are in the right frame.
    let angular_velocity_world = v.orientation * v.angular_velocity;

    for pontoon in &BEAVER_PONTOONS {
        // Pontoon center in world frame: aircraft position + orientation * body_offset.
        let world_offset = v.orientation * pontoon.body_offset;
        let world_center = v.position + world_offset;

        // Sample water surface at pontoon's world (x, z) position at the deterministic
        // wave clock. Y-only height is sufficient for immersion depth (the Gerstner
        // trochoidal X/Z component is a small secondary effect — see the note on
        // world::water_height).
        let water_y = water_height(world_center.x, world_center.z, wave_time);

        // Pontoon bottom in world frame: center plus orientation-rotated -Y half-height.
        // Correctly accounts for aircraft pitch / roll — when the plane pitches nose-down
        // the body-Y axis tilts forward, so the pontoon "bottom" along body-Y is no longer
        // directly below the center in world frame. Full multi-point sampling along the
        // pontoon length lands in 2.5a.3 (step transition); this is already better than
        // the world-axis-aligned approximation in 2.5a.1.
        let pontoon_bottom_world =
            world_center + v.orientation * Vec3::new(0.0, -pontoon.height_m * 0.5, 0.0);
        let pontoon_bottom_y = pontoon_bottom_world.y;

        // Immersion depth: negative means the pontoon bottom is above water → zero
        // buoyancy and zero drag. Positive → bottom is submerged by that much.
        let immersion_depth = (water_y - pontoon_bottom_y).max(0.0);
        let immersion_height = immersion_depth.min(pontoon.height_m);

        if immersion_height <= 0.0 {
            continue;
        }

        let wetted_fraction = immersion_height / pontoon.height_m;

        // ── Hydrostatic: Archimedes' buoyancy ──────────────────────────────────
        let box_volume = immersion_height * pontoon.length_m * pontoon.beam_m;
        let v_submerged = box_volume.min(pontoon.max_displacement_m3);
        let buoyancy_magnitude = WATER_DENSITY_KG_M3 * v_submerged * GRAVITY.length();
        let buoyancy_force_world = Vec3::new(0.0, buoyancy_magnitude, 0.0);

        // ── Hydrodynamic: drag + damping ───────────────────────────────────────
        // Per-pontoon world-frame velocity accounts for aircraft rotation.
        let v_pontoon_world = v.velocity + angular_velocity_world.cross(world_offset);

        // Vertical damping (linear). Resists v_y; stabilizes buoyancy oscillation.
        let v_damp_y = -VERTICAL_DAMPING_N_PER_MPS * v_pontoon_world.y * wetted_fraction;

        // Horizontal drag (quadratic). Opposes pontoon's XZ motion.
        let v_horizontal = Vec3::new(v_pontoon_world.x, 0.0, v_pontoon_world.z);
        let v_h_speed = v_horizontal.length();
        let horizontal_drag_world = if v_h_speed > 1e-4 {
            let drag_magnitude =
                HORIZONTAL_DRAG_N_PER_MPS2 * v_h_speed * v_h_speed * wetted_fraction;
            -v_horizontal / v_h_speed * drag_magnitude
        } else {
            Vec3::ZERO
        };

        let pontoon_force_world = buoyancy_force_world
            + Vec3::new(horizontal_drag_world.x, v_damp_y, horizontal_drag_world.z);

        // Moment in body frame: r × F_body. Rotate the world-frame force into body
        // frame via orientation.inverse() so the moment we accumulate is in the same
        // frame as angular_velocity / angular_accel. Cross product produces a body-
        // frame torque; divided by principal-axis inertia gives angular acceleration.
        let force_body = v.orientation.inverse() * pontoon_force_world;
        let moment_body = pontoon.body_offset.cross(force_body);

        total_force_world += pontoon_force_world;
        total_moment_body += moment_body;
    }

    let linear_accel = total_force_world / mass_kg;
    let angular_accel = Vec3::new(
        total_moment_body.x / I_XX,
        total_moment_body.y / I_YY,
        total_moment_body.z / I_ZZ,
    );

    BuoyancyOutputs {
        linear_accel,
        angular_accel,
        total_force_n: total_force_world.length(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn beaver_spec() -> AircraftSpec {
        // Minimal spec with the fields buoyancy reads. Real RON loader produces the
        // full struct; for tests we only need max_mass_wheels_kg.
        AircraftSpec {
            name: "Test".to_string(),
            inspiration: "Test".to_string(),
            empty_mass_kg: 1315.5,
            max_mass_wheels_kg: 2313.0,
            max_mass_floats_kg: 2435.8,
            wing_area_m2: 23.23,
            aspect_ratio: 9.2,
            airfoil_root: "NACA 64A416".to_string(),
            airfoil_tip: "NACA 64A412".to_string(),
            engine: crate::aircraft_spec::EngineSpec {
                name: "Test".to_string(),
                power_hp: 450.0,
                cruise_fuel_gph: 27.0,
            },
            fuel_capacity_gal: 95.0,
            speeds: crate::aircraft_spec::SpeedsSpec {
                vne_mps: 71.53,
                cruise_mps: 64.31,
                vs_clean_mps: 23.25,
                vso_mps: 21.01,
                vx_mps: 29.06,
                vy_mps: 37.99,
            },
        }
    }

    fn level_vehicle_at(y: f32) -> VehicleState {
        let mut v = VehicleState::at_origin();
        v.position = Vec3::new(0.0, y, 0.0);
        v.orientation = Quat::IDENTITY;
        v.velocity = Vec3::ZERO;
        v.angular_velocity = Vec3::ZERO;
        v
    }

    #[test]
    fn no_spec_returns_zero() {
        let v = level_vehicle_at(0.0);
        let out = compute_buoyancy(&v, None, 0.0);
        assert_eq!(out.linear_accel, Vec3::ZERO);
        assert_eq!(out.angular_accel, Vec3::ZERO);
        assert!(out.total_force_n.abs() < f32::EPSILON);
    }

    #[test]
    fn airborne_aircraft_sees_zero_buoyancy() {
        // CG at y=50m → floats nowhere near water.
        let v = level_vehicle_at(50.0);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0);
        assert!(
            out.total_force_n.abs() < f32::EPSILON,
            "expected zero buoyancy above water, got {} N",
            out.total_force_n,
        );
    }

    #[test]
    fn submerged_aircraft_sees_capped_buoyancy() {
        // CG at y=0 → floats centered at -0.5m, fully submerged (bottom at -0.88m,
        // top at -0.12m, both below water level y=0). Expect the displacement cap.
        let v = level_vehicle_at(0.0);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0);

        // Max force = 2 pontoons × 2.236 m³ × 1000 kg/m³ × 9.81 m/s² ≈ 43,870 N.
        let expected_max = 2.0 * 2.236 * WATER_DENSITY_KG_M3 * GRAVITY.length();
        assert!(
            (out.total_force_n - expected_max).abs() < 50.0,
            "expected fully-submerged cap ~{expected_max:.0} N, got {:.0} N",
            out.total_force_n,
        );
        // Upward force > weight → plane would accelerate up.
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        assert!(
            out.total_force_n > weight_n,
            "submerged buoyancy ({:.0} N) should exceed weight ({:.0} N)",
            out.total_force_n,
            weight_n,
        );
    }

    #[test]
    fn equilibrium_draft_produces_weight_matching_force() {
        // Analytic equilibrium: per-float V = (mass/2) / ρ_water = 1157 / 1000 = 1.157 m³.
        // Per-float wetted area = 6.9 × 0.76 = 5.244 m². Depth = 1.157 / 5.244 = 0.221 m.
        // Float center_y = water_y - depth + height/2 = 0 - 0.221 + 0.38 = 0.159 m.
        // CG_y = center_y - body_offset.y = 0.159 - (-0.5) = 0.659 m.
        let v = level_vehicle_at(0.659);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0);

        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        // Expect buoyancy ≈ weight within a few percent (box-approximation tolerance).
        assert!(
            (out.total_force_n - weight_n).abs() < weight_n * 0.05,
            "at equilibrium draft, buoyancy ({:.0} N) should match weight ({:.0} N)",
            out.total_force_n,
            weight_n,
        );
    }

    #[test]
    fn symmetric_rest_produces_no_roll_moment() {
        // Level plane at partial submersion: port and starboard forces are equal, so
        // their roll moments cancel exactly. Asymmetric behavior emerges in 2.5a.5
        // scenarios; 2.5a.1 just validates the symmetric case.
        let v = level_vehicle_at(0.3);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0);
        // Roll moment is body-frame Z; yaw is body-frame Y. Both must be zero (up to
        // float-precision noise) when port/starboard are geometrically symmetric and
        // seeing the same water surface (wave_time=0 is a crest-neutral sample at origin).
        assert!(
            out.angular_accel.z.abs() < 1e-4,
            "expected zero roll accel, got {}",
            out.angular_accel.z,
        );
        assert!(
            out.angular_accel.y.abs() < 1e-4,
            "expected zero yaw accel, got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn wave_time_changes_sampled_surface() {
        // Pick CG in the unsaturated regime (immersion height < 0.426 m, where the
        // displacement cap is 2.236 m³ and the box cross-section is 6.9 × 0.76 m²)
        // so wave-phase-driven volume changes translate to force changes. At CG=0.58,
        // float bottom is at -0.30 m, immersion ≈ 0.30 m at water_y=0. Waves moving
        // the surface by ±0.2 m cause force to swing noticeably.
        let v = level_vehicle_at(0.58);
        let spec = beaver_spec();
        let out_t0 = compute_buoyancy(&v, Some(&spec), 0.0);
        let out_t1 = compute_buoyancy(&v, Some(&spec), 0.5);
        assert!(
            (out_t0.total_force_n - out_t1.total_force_n).abs() > 1.0,
            "buoyancy should differ with wave phase in the unsaturated regime; t0={:.1} t1={:.1}",
            out_t0.total_force_n,
            out_t1.total_force_n,
        );
    }

    // ── Phase 2.5a.2 tests: drag + damping ───────────────────────────────────

    #[test]
    fn vertical_damping_opposes_downward_motion() {
        // Plane partly submerged, sinking. Vertical damping should produce an upward
        // force on top of buoyancy — net linear_accel.y > buoyancy-alone.
        let spec = beaver_spec();
        let mut v = level_vehicle_at(0.58);
        v.velocity = Vec3::new(0.0, -1.0, 0.0); // sinking at 1 m/s

        let out = compute_buoyancy(&v, Some(&spec), 0.0);

        // Baseline buoyancy at y=0.58 with zero velocity, for comparison.
        let v_still = level_vehicle_at(0.58);
        let baseline = compute_buoyancy(&v_still, Some(&spec), 0.0);

        assert!(
            out.linear_accel.y > baseline.linear_accel.y,
            "damping should add upward accel when sinking; baseline={} with_drag={}",
            baseline.linear_accel.y,
            out.linear_accel.y,
        );
    }

    #[test]
    fn vertical_damping_opposes_upward_motion() {
        // Symmetric case: plane rising. Vertical damping should reduce the upward
        // acceleration.
        let spec = beaver_spec();
        let mut v = level_vehicle_at(0.58);
        v.velocity = Vec3::new(0.0, 1.0, 0.0); // rising at 1 m/s

        let out = compute_buoyancy(&v, Some(&spec), 0.0);
        let v_still = level_vehicle_at(0.58);
        let baseline = compute_buoyancy(&v_still, Some(&spec), 0.0);

        assert!(
            out.linear_accel.y < baseline.linear_accel.y,
            "damping should subtract upward accel when rising; baseline={} with_drag={}",
            baseline.linear_accel.y,
            out.linear_accel.y,
        );
    }

    #[test]
    fn horizontal_drag_opposes_forward_motion() {
        // Plane moving forward (-Z direction in body frame; velocity.z < 0 in world
        // frame with identity orientation). Drag produces +Z force component
        // (opposing motion).
        let spec = beaver_spec();
        let mut v = level_vehicle_at(0.58);
        v.velocity = Vec3::new(0.0, 0.0, -5.0); // 5 m/s forward

        let out = compute_buoyancy(&v, Some(&spec), 0.0);

        // With quadratic drag at 5 m/s, partial wet (0.58 CG → ~0.37 wetted):
        // F_drag per float = 75 × 25 × 0.37 = 694 N opposing motion.
        // Total ~1388 N backward, accel = 1388 / 2313 = 0.60 m/s² positive Z.
        assert!(
            out.linear_accel.z > 0.3 && out.linear_accel.z < 1.5,
            "expected drag accel ~0.6 m/s² in +Z (opposing -Z motion); got {}",
            out.linear_accel.z,
        );
    }

    #[test]
    fn horizontal_drag_scales_with_wetted_fraction() {
        // Compare drag at two different wetted fractions. Deeper submersion →
        // more drag at the same velocity.
        let spec = beaver_spec();
        let mut v_shallow = level_vehicle_at(0.75);
        v_shallow.velocity = Vec3::new(5.0, 0.0, 0.0);
        let out_shallow = compute_buoyancy(&v_shallow, Some(&spec), 0.0);

        let mut v_deep = level_vehicle_at(0.3);
        v_deep.velocity = Vec3::new(5.0, 0.0, 0.0);
        let out_deep = compute_buoyancy(&v_deep, Some(&spec), 0.0);

        assert!(
            out_deep.linear_accel.x.abs() > out_shallow.linear_accel.x.abs(),
            "deeper submersion should produce more drag; shallow={} deep={}",
            out_shallow.linear_accel.x,
            out_deep.linear_accel.x,
        );
    }

    #[test]
    fn above_water_has_zero_drag() {
        // Airborne aircraft moving fast: no water contact → zero drag contribution
        // from buoyancy module. (Aero drag is handled separately in flight.rs.)
        let spec = beaver_spec();
        let mut v = level_vehicle_at(50.0);
        v.velocity = Vec3::new(10.0, -5.0, -30.0);

        let out = compute_buoyancy(&v, Some(&spec), 0.0);
        assert!(out.linear_accel.length() < f32::EPSILON);
        assert!(out.total_force_n.abs() < f32::EPSILON);
    }
}
