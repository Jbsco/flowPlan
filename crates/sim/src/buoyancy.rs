//! Per-pontoon buoyancy for float configuration.
//!
//! Phase 2.5a.1: computes hydrostatic force + moment per pontoon based on the water
//! surface sampled at each float's world position. Output is summed with aero forces
//! in `state::tick_dynamics`. Per-pontoon physics up front means asymmetric behavior
//! (one-float dipped, one-pontoon takeoff in 2.5a.5) emerges for free rather than
//! requiring special-case code later.
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

/// Compute per-pontoon buoyancy for the current vehicle state.
///
/// Pure function: no hidden state; determinism-safe given the determinism of `wave_time`
/// and `water_height`. When no aircraft spec is loaded (pure-sim tests), returns zero
/// — floats are an aircraft configuration concept, not applicable to the generic cube.
///
/// Short-circuits cleanly when the aircraft is above water (no float submersion).
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

    for pontoon in &BEAVER_PONTOONS {
        // Pontoon center in world frame: aircraft position + orientation * body_offset.
        let world_offset = v.orientation * pontoon.body_offset;
        let world_center = v.position + world_offset;

        // Sample water surface at pontoon's world (x, z) position at the deterministic
        // wave clock. Y-only height is sufficient for immersion depth (the Gerstner
        // trochoidal X/Z component is a small secondary effect — see the note on
        // world::water_height).
        let water_y = water_height(world_center.x, world_center.z, wave_time);

        // Pontoon bottom approximation: pontoon center minus half its vertical extent.
        // Uses world-axis-aligned height, ignoring body tilt: acceptable for 2.5a.1
        // where we're validating the symmetric case. Tilted-float wetted-area modeling
        // is a 2.5a.3 (step transition) refinement.
        let pontoon_bottom_y = world_center.y - pontoon.height_m * 0.5;

        // Immersion depth: negative means the pontoon bottom is above water → zero
        // buoyancy. Positive → bottom is submerged by that much.
        let immersion_depth = (water_y - pontoon_bottom_y).max(0.0);
        let immersion_height = immersion_depth.min(pontoon.height_m);

        // Box approximation wetted volume, capped at real displacement.
        let box_volume = immersion_height * pontoon.length_m * pontoon.beam_m;
        let v_submerged = box_volume.min(pontoon.max_displacement_m3);

        if v_submerged <= 0.0 {
            continue;
        }

        // Archimedes' buoyancy force, world +Y direction.
        let force_magnitude = WATER_DENSITY_KG_M3 * v_submerged * GRAVITY.length();
        let pontoon_force_world = Vec3::new(0.0, force_magnitude, 0.0);

        // Moment in body frame: r × F_body. Rotate the world-frame force into body
        // frame via orientation.inverse() so the moment we accumulate is in the same
        // frame as angular_velocity / angular_accel. Cross product follows the
        // right-hand rule and produces a body-frame torque that, when divided by the
        // principal-axis inertia, gives the body-frame angular acceleration
        // contribution.
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
}
