//! Per-gear ground contact forces for wheeled / taildragger aircraft.
//!
//! Phase 2.5b.1 (shipped here): computes per-contact-point normal force from a
//! spring-damper compression model when a gear point is below ground level. The
//! `buoyancy.rs` structure is the template: per-contact geometry in a const array,
//! a pure-function entry point, world-frame force + body-frame moment outputs
//! summed into linear + angular accelerations.
//!
//! **Scope of 2.5b.1** — only the normal-force component. Rolling resistance,
//! lateral friction, brakes, tailwheel steering, and per-surface friction tables
//! land in 2.5b.2–2.5b.6. Attitude constraint emerges automatically from
//! multi-point normal-force distribution (same pattern as multi-point buoyancy
//! producing pitch / roll restoring moments in 2.5a.3) — no explicit constraint
//! solver.
//!
//! Gear geometry constants (`BEAVER_GEAR`) are placeholder values anchored to
//! DHC-2 real-world specs. A mesh-alignment follow-up reads `northlander_land.glb`
//! bbox and adjusts these to match the loaded visual (same pattern as 2.5a.3's
//! float realignment to `bbox_min_y = -2.62`). Until then, the placeholders put
//! the gear at physically plausible positions for a Beaver taildragger at rest.
//!
//! See [ROADMAP.md § Milestone 2.5b](../../../ROADMAP.md#milestone-25-ground--water-contact-physics)
//! for the sub-milestone breakdown and the determinism invariant that contact
//! forces are a pure function of `(state, surface sample, dt)` — the surface
//! sample is `ground_y` today; per-surface terrain classification arrives with
//! 2.5b.6.

use glam::Vec3;

use crate::aircraft_spec::AircraftSpec;
use crate::flight::{I_XX, I_YY, I_ZZ};
use crate::vehicle::VehicleState;

/// Per-gear vertical spring stiffness (N/m). Sized so the Beaver's main gear
/// compresses ~4-5 cm under its share of weight (~47% of 2313 kg × 9.81 m/s² per
/// main = ~10,700 N; compression ≈ 10,700 / 200,000 = 0.053 m). Tailwheel
/// compresses less because it carries ~15% of weight.
///
/// Stiff enough that the gear acts "nearly rigid" visually but soft enough that
/// a hard landing (3 m/s touchdown) doesn't produce an instantaneous force spike
/// that pushes the integrator into oscillation. Revisit if playtest reveals
/// bounce / porpoise on landing — the damping constant is the more likely knob.
const GEAR_SPRING_N_PER_M: f32 = 200_000.0;

/// Per-gear vertical damping coefficient (N·s/m, linear in compression-rate).
/// Combines with `GEAR_SPRING_N_PER_M` to form an underdamped spring-damper pair
/// at the effective mass per gear (~770 kg for main, ~350 kg for tail). Damping
/// ratio ~0.25 — enough to kill the second bounce on a normal landing but soft
/// enough to feel springy; hard landings bleed energy through the drag/pitch
/// paths that 2.5b.2+ add, not through overstiffening this damper.
const GEAR_DAMPING_N_PER_MPS: f32 = 10_000.0;

/// Rolling-friction coefficient on a paved surface (dimensionless, `µ_r`).
///
/// Produces a horizontal drag force per gear proportional to the gear's normal
/// force: `F_roll = -µ_r · N · v̂_ground`. At idle thrust this balances the
/// engine's output at a bounded taxi speed — the 2.5c invariant that a wheeled
/// plane taxiing straight under low throttle reaches a steady speed without
/// runaway.
///
/// `0.02` is the canonical `µ_r` for an aircraft tire on a well-maintained
/// paved runway (concrete / asphalt). Per-surface variation (grass ≈ 0.04,
/// gravel ≈ 0.07, sand ≈ 0.10-0.15, tundra ≈ 0.15-0.25) lands with 2.5b.6 when
/// the terrain classifier can tell the `ground_contact` module which surface
/// the gear is touching. Until then, every ground contact uses the paved
/// baseline.
///
/// The coefficient acts on the horizontal-velocity magnitude (linear in v),
/// not on v² — this is the standard low-speed model. High-speed tire-squeal
/// rolling drag is speed-dependent (rises with v²) and arrives with 2.5b.3's
/// cornering / tire-slip model.
const ROLLING_FRICTION_COEFF: f32 = 0.02;

/// Per-contact geometry. One entry per wheel / tailwheel.
#[derive(Clone, Copy, Debug)]
struct GearPoint {
    /// Body-frame offset from the aircraft CG to the gear contact point (m). This
    /// is the **fully-extended** tire-to-ground point; compression shortens the
    /// effective y-offset, but the spring model applies force at this attach
    /// point (rigid-body approximation — strut compression is not modeled as a
    /// separate degree of freedom in v1).
    body_offset: Vec3,
    /// Label for diagnostics / plots. Not functional.
    #[allow(dead_code)]
    label: &'static str,
}

/// Beaver taildragger gear set: two mains forward of CG, one tailwheel aft.
///
/// Real DHC-2 dimensions: wheel track ~3.3 m (±1.65 m lateral), main gear ~0.8 m
/// forward of CG, wheelbase ~5.3 m (tailwheel 4.5 m aft of the mains). Static
/// weight distribution works out to ~85% on the mains (split ~42.5% / 42.5%) and
/// ~15% on the tail, which matches real Beaver load figures.
///
/// Vertical offsets (y): mains deeper than tail so the static attitude at rest
/// is tail-down (~6.5° nose-up). Values chosen so that at zero compression the
/// aircraft touches all three gear points with the CG at the expected ground
/// clearance for a taildragger at rest.
///
/// TODO(2.5b follow-up): read `northlander_land.glb` bbox at load time and
/// realign these to the actual mesh geometry, mirroring the 2.5a.3 float
/// realignment. Same applies if an `AircraftSpec.gear_geometry` migration lands.
const BEAVER_GEAR: [GearPoint; 3] = [
    // Left main wheel.
    GearPoint {
        body_offset: Vec3::new(-1.65, -1.3, -0.8),
        label: "left_main",
    },
    // Right main wheel.
    GearPoint {
        body_offset: Vec3::new(1.65, -1.3, -0.8),
        label: "right_main",
    },
    // Tailwheel — higher (closer to CG in y) because attached to tail structure.
    GearPoint {
        body_offset: Vec3::new(0.0, -0.7, 4.5),
        label: "tailwheel",
    },
];

/// Ground-contact outputs: linear + angular acceleration contributions and a
/// diagnostic total-force magnitude. Mirrors `BuoyancyOutputs` field-for-field so
/// `tick_dynamics` can sum them uniformly.
#[derive(Clone, Copy, Debug, Default)]
pub struct GroundContactOutputs {
    /// Linear acceleration contribution in world frame (m/s²).
    pub linear_accel: Vec3,
    /// Angular acceleration contribution in body frame (rad/s²).
    pub angular_accel: Vec3,
    /// Sum of gear normal-force magnitudes across all contacts (N). Diagnostic.
    pub total_force_n: f32,
}

/// Compute per-gear normal force for the current vehicle state.
///
/// Short-circuits to zero when:
/// - No aircraft spec is loaded (pure-sim tests without a Beaver configuration).
/// - `v.config != VehicleConfig::Wheels` — float-configured aircraft skip this
///   function entirely (buoyancy is on instead).
/// - All gear contact points sit above `ground_y` (aircraft is airborne) — no
///   compression, no force.
///
/// Per-gear force model:
/// - Compression: `c = max(0, ground_y - contact_y_world)` where `contact_y_world`
///   is the gear point's y-coordinate after rotation by the aircraft orientation.
/// - Normal force magnitude: `F = k·c − c_d·v_gear_y` (damping opposes downward
///   velocity at the contact point, adding to the spring pushback when the gear
///   is sinking). Clamped to zero if negative — ground pushes up, never pulls
///   down, so a gear lifting off (positive `v_gear_y`) while still compressed
///   sheds force rather than latching.
/// - Force direction: world +Y (flat-ground approximation). Sloped-terrain
///   normal tracking is deferred to the per-surface work in 2.5b.6.
///
/// Velocity at each gear includes angular contribution:
/// `v_gear = v_cg + (orient · ω_body) × r_gear_world`, so pitch / roll / yaw
/// rates show up in vertical damping and produce the angular-damping behavior
/// that an attitude-constraint solver would otherwise require (same pattern as
/// buoyancy's per-sample velocity).
///
/// ### Rolling friction (2.5b.2)
///
/// Each gear in ground contact contributes a horizontal drag
/// `F_roll = -ROLLING_FRICTION_COEFF · N · v̂_ground`, opposing the gear's
/// horizontal velocity. Cornering / lateral tire slip is deferred to 2.5b.3
/// — v1 models rolling friction isotropically (same `µ_r` along forward and
/// lateral axes). For straight-line taxi + takeoff this is indistinguishable
/// from a direction-separated model; cornering scenarios in 2.5b.3 will
/// introduce side-slip separately.
#[must_use]
pub fn compute_ground_contact(
    v: &VehicleState,
    spec: Option<&AircraftSpec>,
    ground_y: f32,
) -> GroundContactOutputs {
    let Some(spec) = spec else {
        return GroundContactOutputs::default();
    };
    if !v.config.uses_ground_contact() {
        return GroundContactOutputs::default();
    }
    let mass_kg = spec.max_mass_wheels_kg;

    let mut total_force_world = Vec3::ZERO;
    let mut total_moment_body = Vec3::ZERO;
    let mut total_normal_magnitude = 0.0_f32;

    let angular_velocity_world = v.orientation * v.angular_velocity;

    for gear in &BEAVER_GEAR {
        let world_offset = v.orientation * gear.body_offset;
        let contact_world = v.position + world_offset;

        let compression = (ground_y - contact_world.y).max(0.0);
        if compression <= 0.0 {
            continue;
        }

        // Velocity at this gear (CG linear + angular cross-product).
        let v_gear_world = v.velocity + angular_velocity_world.cross(world_offset);

        // Spring pushback + damping opposing downward motion at contact.
        // Sign: positive compression pushes +Y; negative v_gear_y (sinking) adds
        // to the pushback; positive v_gear_y (lifting) subtracts but never lets
        // the normal force go negative (ground can't pull).
        let spring_n = GEAR_SPRING_N_PER_M * compression;
        let damper_n = -GEAR_DAMPING_N_PER_MPS * v_gear_world.y;
        let normal_n = (spring_n + damper_n).max(0.0);
        if normal_n <= 0.0 {
            continue;
        }
        total_normal_magnitude += normal_n;

        // Normal force acts at the gear contact point in world +Y.
        let normal_force_world = Vec3::new(0.0, normal_n, 0.0);

        // Rolling friction: horizontal drag proportional to the normal force
        // this gear carries, opposing the gear's horizontal velocity. Skip
        // when essentially stationary so a stopped aircraft doesn't oscillate
        // between +eps / -eps via an active-at-zero friction term.
        let v_horizontal = Vec3::new(v_gear_world.x, 0.0, v_gear_world.z);
        let v_h_speed = v_horizontal.length();
        let roll_force_world = if v_h_speed > 1e-4 {
            let drag_magnitude = ROLLING_FRICTION_COEFF * normal_n;
            -v_horizontal / v_h_speed * drag_magnitude
        } else {
            Vec3::ZERO
        };

        // Both forces act at the gear contact point (same lever arm). Sum in
        // world frame, rotate to body frame for r × F moment.
        let gear_force_world = normal_force_world + roll_force_world;
        let gear_force_body = v.orientation.inverse() * gear_force_world;
        let moment_body = gear.body_offset.cross(gear_force_body);

        total_force_world += gear_force_world;
        total_moment_body += moment_body;
    }

    let linear_accel = total_force_world / mass_kg;
    let angular_accel = Vec3::new(
        total_moment_body.x / I_XX,
        total_moment_body.y / I_YY,
        total_moment_body.z / I_ZZ,
    );

    GroundContactOutputs {
        linear_accel,
        angular_accel,
        total_force_n: total_normal_magnitude,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VehicleConfig;
    use crate::flight::GRAVITY;
    use glam::Quat;

    fn beaver_spec() -> AircraftSpec {
        // Same minimal spec used by buoyancy tests; ground_contact only reads
        // `max_mass_wheels_kg`, but we carry the rest for schema completeness.
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
            variants: Vec::new(),
            default_variant: String::new(),
        }
    }

    fn wheels_vehicle_at(y: f32) -> VehicleState {
        let mut v = VehicleState::at_origin();
        v.position = Vec3::new(0.0, y, 0.0);
        v.orientation = Quat::IDENTITY;
        v.velocity = Vec3::ZERO;
        v.angular_velocity = Vec3::ZERO;
        v.config = VehicleConfig::Wheels;
        v
    }

    #[test]
    fn no_spec_returns_zero() {
        let v = wheels_vehicle_at(0.0);
        let out = compute_ground_contact(&v, None, 0.0);
        assert_eq!(out.linear_accel, Vec3::ZERO);
        assert_eq!(out.angular_accel, Vec3::ZERO);
        assert!(out.total_force_n.abs() < f32::EPSILON);
    }

    #[test]
    fn floats_config_returns_zero_even_with_spec() {
        // Aircraft configured as floats → ground contact short-circuits even if
        // the gear points are below ground. Buoyancy is the active model.
        let mut v = wheels_vehicle_at(0.0);
        v.config = VehicleConfig::Floats;
        let spec = beaver_spec();
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert!(
            out.total_force_n.abs() < f32::EPSILON,
            "floats config should produce zero ground force, got {} N",
            out.total_force_n,
        );
    }

    #[test]
    fn airborne_aircraft_sees_zero_normal_force() {
        // CG at y=50 m → all gear contact points at y ≈ 48.7 m (well above ground=0).
        let v = wheels_vehicle_at(50.0);
        let spec = beaver_spec();
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert!(
            out.total_force_n.abs() < f32::EPSILON,
            "airborne aircraft should see zero ground force, got {} N",
            out.total_force_n,
        );
    }

    #[test]
    fn at_rest_on_ground_produces_weight_matching_force() {
        // Analytic rest position: with mains at body y=-1.3 and tail at y=-0.7, a
        // level attitude puts the mains below the tail. The aircraft must tilt
        // nose-up until all three touch. We bypass that here by running at level
        // orientation with CG positioned so the MAINS touch ground exactly. In
        // that posture only the mains compress, and at rest the spring force
        // through the mains equals weight.
        //
        // CG height for mains-just-touching: main y_body = -1.3, so CG_y = 1.3.
        // At 1.3 m, compression_main = 1.3 - 1.3 = 0 → we need to sink deeper
        // for force. Per-main compression under half-weight: 11,650 / 200,000
        // = 0.058 m. So CG_y = 1.3 - 0.058 = 1.242 m puts each main compressed
        // 0.058 m. Tailwheel at body y=-0.7 → world y=0.542, still above ground
        // (no tail contribution). Total spring force = 2 × 200,000 × 0.058
        // ≈ weight (22,700 N at 2313 kg × 9.81).
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;
        let v = wheels_vehicle_at(cg_y);

        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            (out.total_force_n - weight_n).abs() < weight_n * 0.01,
            "at rest on mains, normal force ({:.0} N) should match weight ({:.0} N)",
            out.total_force_n,
            weight_n,
        );
    }

    #[test]
    fn symmetric_level_at_rest_produces_no_roll_or_yaw() {
        // Level orientation + lateral-symmetric gear → left + right mains produce
        // equal + opposite roll moments → zero roll accel. Both mains are at the
        // same forward z → no yaw asymmetry. Pitch CAN be nonzero because the
        // mains are forward of CG (produces a pitch-up moment from the normal
        // force); this test asserts zero roll and yaw only.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;
        let v = wheels_vehicle_at(cg_y);

        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            out.angular_accel.z.abs() < 1e-4,
            "symmetric gear should produce zero roll accel, got {}",
            out.angular_accel.z,
        );
        assert!(
            out.angular_accel.y.abs() < 1e-4,
            "symmetric gear should produce zero yaw accel, got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn downward_velocity_adds_damping_force() {
        // Aircraft with mains compressed and sinking → damping contribution
        // makes the normal force larger than the spring-alone baseline.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let v_still = wheels_vehicle_at(cg_y);
        let baseline = compute_ground_contact(&v_still, Some(&spec), 0.0);

        let mut v_sinking = wheels_vehicle_at(cg_y);
        v_sinking.velocity = Vec3::new(0.0, -1.0, 0.0); // sinking at 1 m/s

        let sinking = compute_ground_contact(&v_sinking, Some(&spec), 0.0);

        assert!(
            sinking.total_force_n > baseline.total_force_n + 100.0,
            "sinking should add damping force; baseline={:.0} sinking={:.0}",
            baseline.total_force_n,
            sinking.total_force_n,
        );
    }

    #[test]
    fn upward_velocity_removes_damping_but_never_pulls() {
        // Aircraft compressed + lifting → damping subtracts from the spring
        // force, but the total normal magnitude stays non-negative (ground
        // cannot pull the aircraft down).
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v_lifting = wheels_vehicle_at(cg_y);
        v_lifting.velocity = Vec3::new(0.0, 1.0, 0.0); // lifting at 1 m/s

        let out = compute_ground_contact(&v_lifting, Some(&spec), 0.0);
        // Total must be non-negative and less than the still baseline.
        let v_still = wheels_vehicle_at(cg_y);
        let baseline = compute_ground_contact(&v_still, Some(&spec), 0.0);

        assert!(
            out.total_force_n >= 0.0,
            "normal magnitude must never be negative, got {}",
            out.total_force_n,
        );
        assert!(
            out.total_force_n < baseline.total_force_n,
            "lifting should subtract damping vs still; lifting={:.0} still={:.0}",
            out.total_force_n,
            baseline.total_force_n,
        );
    }

    #[test]
    fn one_wheel_in_pit_produces_roll_toward_dipped_side() {
        // Offset only the left main's world position below ground (simulate a
        // pothole) by rolling the aircraft slightly. After rolling left by a
        // few degrees, the left wheel compresses more than the right → left
        // side gets more upward force → roll moment pushes right (nose-up from
        // the aircraft's body frame means rolling the left wing up, away from
        // the ground — the restoring direction). Sign check per flowplan
        // convention: body-frame angular_accel.z > 0 means roll-left; the
        // restoring moment for a left-banked aircraft is roll-right, so
        // angular_accel.z should be NEGATIVE here.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v = wheels_vehicle_at(cg_y);
        // Bank left 5° via Z-axis rotation. Left main drops, right main rises.
        v.orientation = Quat::from_axis_angle(Vec3::Z, 5.0_f32.to_radians());

        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            out.angular_accel.z < -0.01,
            "left-banked aircraft should produce roll-right restoring moment \
             (angular_accel.z < 0), got {}",
            out.angular_accel.z,
        );
    }

    // ── Phase 2.5b.2 tests: rolling friction ─────────────────────────────

    #[test]
    fn airborne_produces_zero_rolling_friction() {
        // Airborne aircraft with horizontal velocity — no gear contact → no
        // rolling drag. (Also zero normal force, verified in the earlier test.)
        let spec = beaver_spec();
        let mut v = wheels_vehicle_at(50.0); // well above ground
        v.velocity = Vec3::new(5.0, 0.0, -30.0);
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert!(out.linear_accel.length() < f32::EPSILON);
    }

    #[test]
    fn stationary_on_ground_has_no_horizontal_friction() {
        // Parked aircraft, gear compressed under weight, zero horizontal
        // velocity. Expect zero horizontal linear acceleration (only vertical
        // normal force equals weight → net linear_accel.xz ≈ 0).
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;
        let v = wheels_vehicle_at(cg_y);

        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            out.linear_accel.x.abs() < 1e-3 && out.linear_accel.z.abs() < 1e-3,
            "stationary aircraft should have no horizontal friction force; \
             accel=({}, _, {})",
            out.linear_accel.x,
            out.linear_accel.z,
        );
    }

    #[test]
    fn rolling_friction_opposes_forward_motion() {
        // Aircraft moving forward on its mains (body -Z with identity
        // orientation = world -Z). Rolling friction should add a +Z world
        // acceleration component (opposing the motion). Magnitude check:
        // with main_compression under weight → normal ≈ weight, and µ_r ≈ 0.02,
        // total rolling drag force ≈ 0.02 × 22700 = 454 N; mass 2313 kg →
        // accel ≈ 0.20 m/s² in +Z. Bounds loose to catch direction + rough
        // magnitude without coupling to exact compression.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v = wheels_vehicle_at(cg_y);
        v.velocity = Vec3::new(0.0, 0.0, -5.0); // 5 m/s forward (body -Z)

        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        // Expect positive x-world-Z component (drag opposes -Z motion → +Z).
        assert!(
            out.linear_accel.z > 0.05 && out.linear_accel.z < 0.5,
            "rolling friction should produce ~0.2 m/s² +Z accel (opposing -Z \
             motion); got {}",
            out.linear_accel.z,
        );
    }

    #[test]
    fn rolling_friction_direction_matches_velocity() {
        // Same deceleration magnitude in any horizontal direction (isotropic
        // paved-surface model for 2.5b.2). Compare forward-motion accel.z to
        // sideways-motion accel.x; should be symmetric in magnitude.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v_fwd = wheels_vehicle_at(cg_y);
        v_fwd.velocity = Vec3::new(0.0, 0.0, -5.0);
        let fwd = compute_ground_contact(&v_fwd, Some(&spec), 0.0);

        let mut v_side = wheels_vehicle_at(cg_y);
        v_side.velocity = Vec3::new(5.0, 0.0, 0.0);
        let side = compute_ground_contact(&v_side, Some(&spec), 0.0);

        // Forward motion → +Z drag; lateral motion → -X drag. Magnitudes equal.
        let fwd_mag = fwd.linear_accel.z.abs();
        let side_mag = side.linear_accel.x.abs();
        let ratio = fwd_mag / side_mag;
        assert!(
            (0.9..=1.1).contains(&ratio),
            "rolling friction should be isotropic; fwd_mag={fwd_mag} side_mag={side_mag}",
        );
    }
}
