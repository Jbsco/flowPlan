//! Per-pontoon hydrostatic (buoyancy) + hydrodynamic (drag, damping, planing lift)
//! forces for float configuration.
//!
//! Phase 2.5a.1 (shipped): computes per-pontoon buoyancy force + moment from the
//! water surface sampled at each float's world position.
//!
//! Phase 2.5a.2 (shipped): extends with per-pontoon horizontal water drag
//! (quadratic, for realistic plowing) and vertical water damping (linear, to
//! stabilize the buoyancy-gravity oscillation into settling behavior).
//!
//! Phase 2.5a.3: multi-point sampling along each pontoon's length (fore, fore-mid,
//! mid, aft-mid, aft) so buoyancy and drag distribute over the float's longitudinal
//! extent. When the aircraft pitches, forward sections dip deeper than aft sections
//! (or vice versa), producing a pitch-restoring buoyancy moment that bounds the
//! pitch-down tendency single-point drag would otherwise produce. Also adds a
//! simple hydrodynamic planing lift term — forward motion over wetted sections
//! generates vertical lift that rises onto the step as speed builds, driving the
//! characteristic plow → on-step behavior in water takeoff.
//!
//! The function is still named `compute_buoyancy` because "buoyancy" is the public
//! umbrella term for the water-contact force model in this codebase. Module rename
//! to `hydrodynamics` / `water_contact` is a candidate follow-up once this module
//! grows further (water rudder lands in 2.5a.6).
//!
//! Per-pontoon physics — now multi-point, not just multi-pontoon — means asymmetric
//! behavior (one-float dipped, one-pontoon takeoff in 2.5a.5) emerges for free
//! rather than requiring special-case code later.
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
/// Distributed over `PONTOON_SAMPLE_COUNT` sections per pontoon at runtime (each
/// section gets `/ PONTOON_SAMPLE_COUNT`) so total damping is preserved regardless
/// of sampling resolution.
///
/// Linear (rather than quadratic) for numerical stability near zero velocity — the
/// cost is unphysicality at high vertical speeds that real water drag would penalize
/// quadratically. Revisit if high-speed vertical scenarios (hard touchdown, porpoise
/// recovery) surface a feel gap.
const VERTICAL_DAMPING_N_PER_MPS: f32 = 30_000.0;

/// Per-pontoon horizontal water drag coefficient (N·s²/m², quadratic in velocity),
/// scaled by wetted fraction. Produces plowing drag proportional to `v²`; tuned so
/// the Beaver's idle thrust taxis the aircraft at a bounded speed on water at
/// equilibrium draft. Quadratic drag self-stabilizes numerically without issue.
///
/// Distributed over `PONTOON_SAMPLE_COUNT` sections per pontoon at runtime.
/// As the aircraft rises onto the step, aft sections stay wet while fore sections
/// rise out of water, so effective drag drops naturally from the wetted-fraction
/// per-section scaling — no separate step-transition coefficient needed.
///
/// Reduced from 75 → 40 in 2.5a.3c retune: with the deeper float position
/// (`body_offset.y = -2.24`) the drag pitch-down moment scaled 4.5× and overwhelmed
/// the buoyancy pitch-restoring moment around 14 m/s, causing porpoise instability
/// to grow into a flip. Reducing drag plus adding a forward offset to planing lift
/// (see `PLANING_LIFT_CENTROID_Z_FWD_M`) produces stable plow → on-step transition.
const HORIZONTAL_DRAG_N_PER_MPS2: f32 = 40.0;

/// Lateral (body-X, across-the-float) drag coefficient.
///
/// Real floats are long and narrow — their cross-axis hydrodynamic drag is
/// much higher than longitudinal (along-float) drag, because the wetted
/// cross-section is a deep hull resisting sideways push. Tuned at ~10× the
/// longitudinal coefficient to reproduce that anisotropy: aircraft resists
/// sideslip / lateral drift strongly while still rolling forward freely.
///
/// Without this, the aircraft slides sideways under water-rudder push or
/// crosswind — unphysical, and the "too easy to drift side to side" 3.2.3c
/// playtest observation. With it, the rudder produces tail-sweep rotation
/// without an equal amount of lateral translation, matching real float
/// behavior.
const LATERAL_HYDRO_DRAG_N_PER_MPS2: f32 = 400.0;

/// Per-pontoon planing lift coefficient (N·s²/m², quadratic in forward velocity),
/// scaled by wetted fraction. Hydrodynamic lift from water flowing past the pontoon
/// bottom — provides the physical mechanism by which the aircraft rises onto the
/// step during water takeoff. At low speeds (taxi regime, V < ~10 m/s) the lift is
/// negligible compared to weight; at planing speeds (15-25 m/s) it combines with
/// wing lift to progressively lift the aircraft out of displacement mode.
///
/// For v1 we model planing lift as world-frame +Y (approximately normal to water
/// surface). This is accurate for small pitch angles; larger pitch angles would
/// need a pitch-dependent Cl curve. Tune via the water-takeoff bench scenario.
///
/// Raised from 50 → 75 in 2.5a.3c retune: stronger lift enables actual rise onto
/// the step and provides the pitch-up balancing moment via `PLANING_LIFT_CENTROID_Z_FWD_M`.
const PLANING_LIFT_N_PER_MPS2: f32 = 75.0;

/// Inside-float hydrofoil stick coefficient.
///
/// Models the real-world hydrofoil effect that pulls the inside float DOWN
/// during a water turn and (symmetrically) pushes the outside float UP. The
/// inside float's keel acts as a downward-suction hydrofoil during yawing
/// motion; the outside float's as an upward-lift one. Combined, these create
/// a strong roll-restoring moment that keeps float-planes planted in turns
/// and prevents outward flips at higher throttle.
///
/// Force magnitude per sample: `coeff × inside_indicator × forward_body × wetted × per_section_scale`
/// where `inside_indicator = -pontoon.offset.x × yaw_rate` (positive on
/// inside of turn, negative on outside). Applied as world-Y (vertical);
/// accurate for moderate bank angles.
///
/// Tuned 2026-04-22 alongside water-rudder / anisotropic-drag pass to
/// prevent outward flips at 35%+ throttle without over-damping natural
/// turn dynamics.
///
/// **Task #179 iteration 3 (2026-04-22): reduced 70k → 15k → 3k** after
/// multiple flip-bug reports.
/// - 70k (original): `yaw_rate²` positive feedback → flip at any throttle with yaw
/// - 15k: better but abusive inputs (full throttle + full yaw) still flip
/// - 3k + hard clamp: bounded under all tested abusive inputs
///
/// Also clamped to `STICK_FORCE_MAX_N`. The "inside-float hydrofoil stick"
/// effect is now modest — a gentle turn-tightening, not a primary
/// restoring mechanism. Normal dihedral + lateral drag provide the
/// dominant roll stability.
const INSIDE_FLOAT_STICK_COEFF: f32 = 3_000.0;

/// Absolute-magnitude clamp on per-pontoon hydrofoil stick force.
/// Reduced 5000 → 2000 N alongside coefficient reduction.
const STICK_FORCE_MAX_N: f32 = 2_000.0;

/// Forward body-Z offset (m) where planing lift's center of pressure acts, relative
/// to each section's geometric center. Real planing-hull lift acts at the bow-wave
/// region (forward of the wetted area's geometric center) — modeling this with a
/// simple constant offset gives the natural pitch-up moment that balances the drag
/// pitch-down moment, producing the characteristic nose-up attitude of an aircraft
/// on the step. Negative = forward (per flowplan convention, aircraft faces -Z).
///
/// `-1.5 m` chosen so that at typical planing conditions (V ~14-20 m/s, wetted ~0.29),
/// planing pitch-up moment slightly exceeds drag pitch-down moment, allowing the
/// aircraft to settle into a stable on-step attitude rather than oscillating into
/// a flip. Buoyancy fore/aft asymmetry then bounds the pitch-up at a few degrees
/// nose-up — exactly the characteristic on-step posture.
const PLANING_LIFT_CENTROID_Z_FWD_M: f32 = -1.5;

/// Trim-angle (body pitch) dependence of planing lift.
///
/// Real planing hulls generate additional lift at nose-up attitudes — the trim
/// angle between the pontoon bottom and water surface determines hydrodynamic
/// lift coefficient (Savitsky planing-hull theory). Form:
/// `lift_multiplier = 1 + k × sin(pitch).max(0)` — multiplier ≥ 1 (never less
/// than baseline). Only boosts for nose-up attitudes; at level the factor is
/// exactly 1 so existing level-start scenarios are unchanged.
///
/// Task #174 iteration + playtest follow-up (2026-04-22):
/// - k=1.5: aircraft unblocked glassy takeoff but needed 15s saturated
///   elevator to rotate (unplayable for human pilot)
/// - k=3.0: stronger nose-up planing lift accelerates rise-out-of-water,
///   should reduce rotation authority demand without amplifying coefficient
///   values elsewhere. At k=3.0: 5° → 1.26×, 10° → 1.52×, 15° → 1.78×.
const ALPHA_PLANING_LIFT_GAIN: f32 = 3.0;

/// Water rudder yaw coefficient.
///
/// Produces directional control at low water speed where air rudder is
/// ineffective. Deployed rudder force scales with forward velocity² and
/// average pontoon wetted fraction. The resulting force is applied as BOTH
/// a moment around CG AND a linear push on the airframe (via the body-+Z
/// offset); this produces the real-world "rudder sweeps the tail around"
/// pivot behavior rather than a pirouette on the CG.
///
/// Must be RETRACTED (control state off) for takeoff or planing-speed
/// rudder moment would be excessive.
///
/// Tuned 2026-04-22 (32× original 25.0) paired with anisotropic float drag
/// (see `LATERAL_HYDRO_DRAG_N_PER_MPS2`) to match POH water-turn performance
/// at idle throttle: **~4°/s sustained yaw rate, 360° circle in ~90 s** —
/// POH center of range. Tuning is driven by the `beaver_water_circle_*` bench
/// scenarios; if those assertions drift, this coefficient is the first knob
/// to revisit (lateral drag is the second).
///
/// Retuned 800 → 500 in post-playtest 2026-04-22 (iteration 3) after
/// removing the spurious linear-lateral-force-at-CG component (player
/// feedback: "a lot of side sliding when pontoons should enforce mostly
/// forward motion"). Without lateral push at CG, the yaw moment converts
/// more efficiently to heading change, so coefficient drops ~38% to hold
/// POH 4°/s equilibrium turn rate (`circle_right`/`circle_left` bench scenarios
/// targeting 119.5° over 30s).
const WATER_RUDDER_COEFF: f32 = 450.0;

/// Aft lever arm (m) for water rudder moment, body +Z direction (aft of CG).
const WATER_RUDDER_ARM_M: f32 = 3.2;

/// Maximum side force (N) the water rudder may produce. The raw coefficient
/// × v² scaling is unbounded: tuned at taxi speed (~3 m/s → ~3.6 kN) but
/// grows to >100 kN at 20 m/s, which drives the abusive-input flip bug. Real
/// water rudders stall / cavitate long before that. 5000 N is taxi peak plus
/// headroom — enough for the POH-matching 4°/s yaw at idle, bounded at any
/// velocity so abusive inputs can't flip the aircraft.
const WATER_RUDDER_FORCE_MAX_N: f32 = 5_000.0;

/// Number of sample points along each pontoon's longitudinal axis. 5 gives good
/// pitch stability (fore / fore-mid / mid / aft-mid / aft) without excessive compute
/// cost — buoyancy+drag is called every 120 Hz tick, so ~10 samples total per tick
/// (2 pontoons × 5 points) stays well within the sim budget.
///
/// Samples are equally spaced along the pontoon at center fractions
/// `(i + 0.5) / N - 0.5` for i ∈ [0, N). With N=5, sample offsets along pontoon
/// length are `{-0.4, -0.2, 0, 0.2, 0.4}` × `length_m`.
const PONTOON_SAMPLE_COUNT: usize = 5;

/// Submerged airframe surface interaction point. When a wingtip, nose, or
/// tail extremum goes below the water surface (typically during extreme
/// bank or pitch), it experiences buoyancy + drag like the pontoons do.
/// Models the "wingtip digs into water, aircraft levels back out" physics.
#[derive(Clone, Copy, Debug)]
struct AirframeFloatPoint {
    /// Body-frame offset from CG (m).
    body_offset: Vec3,
    /// Max submersion depth for buoyancy integration — beyond this the
    /// surface is fully submerged and buoyancy plateaus. Approximates
    /// the surface's effective volume contribution.
    max_depth_m: f32,
    /// Buoyancy coefficient (N per meter of submersion depth).
    buoyancy_n_per_m: f32,
    /// Quadratic drag coefficient (N·s²/m² per m of submersion depth).
    drag_coeff: f32,
    /// Drag force magnitude clamp (N) to prevent divergence at extreme v.
    drag_max_n: f32,
}

/// Beaver airframe float points. Wingtips at ±7.3 m lateral × 1.0 m below
/// CG match the strike detection geometry. Tail and nose extrema also
/// modeled for pitch extrema (tail strike on hard flare, nose dig on
/// porpoise over-push). Coefficients tuned so that at 30°+ bank (wingtip
/// ~2m underwater) the resulting restoring moment overwhelms the water-
/// rudder-driven banking cascade but doesn't over-restore at light bank.
///
/// Tuning rationale:
/// - `buoyancy_n_per_m` 5000: at 1m submersion wingtip → 5 kN buoyancy
///   × 7.3m arm = 36 kNm restoring moment.
/// - `drag_coeff` 5000 + `drag_max_n` 15000: at extreme submersion and
///   velocity, each wingtip contributes max ~110 kNm counter-rotation.
/// - Tail and nose have smaller arms (5.5 m and 3 m) but still substantial.
///   These engage on pitch extremes.
const BEAVER_AIRFRAME_FLOAT_POINTS: [AirframeFloatPoint; 4] = [
    AirframeFloatPoint {
        body_offset: Vec3::new(-7.3, -1.0, 0.0), // left wingtip
        max_depth_m: 1.5,
        buoyancy_n_per_m: 8_000.0,
        drag_coeff: 8_000.0,
        drag_max_n: 25_000.0,
    },
    AirframeFloatPoint {
        body_offset: Vec3::new(7.3, -1.0, 0.0), // right wingtip
        max_depth_m: 1.5,
        buoyancy_n_per_m: 8_000.0,
        drag_coeff: 8_000.0,
        drag_max_n: 25_000.0,
    },
    AirframeFloatPoint {
        body_offset: Vec3::new(0.0, -0.3, 5.5), // tail end
        max_depth_m: 1.0,
        buoyancy_n_per_m: 3_000.0,
        drag_coeff: 3_000.0,
        drag_max_n: 10_000.0,
    },
    AirframeFloatPoint {
        body_offset: Vec3::new(0.0, -1.0, -3.0), // nose / prop area
        max_depth_m: 1.0,
        buoyancy_n_per_m: 3_000.0,
        drag_coeff: 3_000.0,
        drag_max_n: 10_000.0,
    },
];

const AIRFRAME_FLOAT_POINTS: &[AirframeFloatPoint] = &BEAVER_AIRFRAME_FLOAT_POINTS;

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
///
/// `body_offset.y = -2.24` aligns the pontoon center with the actual mesh's float
/// position: the loaded `data/aircraft/northlander.glb` has its lowest mesh point
/// (the float bottom) at body y = -2.62 (`bbox_min_y`), so the float center
/// (height 0.76) is at -2.62 + 0.38 = -2.24. Pre-2.5a.3 used a placeholder -0.5
/// that didn't match the visual mesh, causing the aircraft to settle with floats
/// 1.96 m underwater. Aligning physics geometry to the mesh fixes the visual.
///
/// Analytic equilibrium with this geometry: per-pontoon volume to support
/// 2313 kg / 2 = 1157 kg = 1.157 m³ → wetted height 0.221 m per pontoon →
/// float bottom 0.221 m below water → CG at y = 0.539 + 1.86 = 2.40 m.
///
/// TODO(2.4.x): move to per-aircraft `AircraftSpec` alongside moment coefficients,
/// and either expose `bbox_min_y` to the spec loader or compute pontoon position
/// from mesh introspection at startup so future aircraft don't need manual sync.
const BEAVER_PONTOONS: [Pontoon; 2] = [
    // Port float (aircraft's left side, body -X).
    Pontoon {
        body_offset: Vec3::new(-1.25, -2.24, 0.0),
        length_m: 6.9,
        beam_m: 0.76,
        height_m: 0.76,
        max_displacement_m3: 2.236,
    },
    // Starboard float (aircraft's right side, body +X).
    Pontoon {
        body_offset: Vec3::new(1.25, -2.24, 0.0),
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

/// Compute per-pontoon buoyancy + hydrodynamic drag + planing lift for the current
/// vehicle state.
///
/// Pure function: no hidden state; determinism-safe given the determinism of `wave_time`
/// and `water_height`. When no aircraft spec is loaded (pure-sim tests), returns zero
/// — floats are an aircraft configuration concept, not applicable to the generic cube.
///
/// Short-circuits cleanly when the aircraft is above water (no float submersion → no
/// hydrostatic force, no hydrodynamic drag, no planing lift).
///
/// Per-section force model (summed across `BEAVER_PONTOONS` × `PONTOON_SAMPLE_COUNT`):
/// - Hydrostatic: `F_buoy = ρ_water · V_submerged · g` (world +Y).
/// - Hydrodynamic vertical damping: `F_v = -c_v · v_sample_y · wetted_frac` (world +Y).
/// - Hydrodynamic horizontal drag: `F_h = -c_h · |v_xz| · v_xz · wetted_frac`
///   (world XZ plane; quadratic).
/// - Planing lift: `F_l = c_l · v_forward² · wetted_frac` (world +Y, only positive
///   when body-forward velocity > 0).
///
/// Per-section velocity includes angular contribution:
/// `v_sample = v_cg + (orient · ω_body) × r_sample_world`, so angular rates couple
/// into per-section drag/lift and produce angular damping naturally (no explicit
/// angular-damping term needed).
#[must_use]
#[allow(clippy::too_many_lines)] // multi-pass physics: buoyancy + drag + planing + rudder
pub fn compute_buoyancy(
    v: &VehicleState,
    spec: Option<&AircraftSpec>,
    wave_time: f32,
    chop_intensity: f32,
) -> BuoyancyOutputs {
    let Some(spec) = spec else {
        return BuoyancyOutputs::default();
    };
    // Config check: a wheeled aircraft has no pontoons — buoyancy must not
    // apply phantom float forces. This mirrors the `uses_ground_contact`
    // short-circuit in `compute_ground_contact`. 2.5b.5 audit caught the
    // missing check: pre-fix, a wheels-variant aircraft at CG ≈ 1.15 m got
    // +19 m/s² "buoyancy" from pontoons that don't exist in its mesh or
    // physics model, driving divergent at-rest drift.
    if !v.config.uses_buoyancy() {
        return BuoyancyOutputs::default();
    }
    let mass_kg = spec.max_mass_wheels_kg;

    let mut total_force_world = Vec3::ZERO;
    let mut total_moment_body = Vec3::ZERO;
    let mut wetted_fraction_accum = 0.0_f32;
    let mut wetted_sample_count = 0_u32;

    // World-frame angular velocity: rotate the body-frame ω by orientation so
    // cross-product contributions to pontoon velocity are in the right frame.
    let angular_velocity_world = v.orientation * v.angular_velocity;

    // Body pitch (trim-angle proxy) for α-dependent planing lift boost.
    // YXZ Euler decomposition: X component = pitch (+nose-up).
    let (_, pitch_rad, _) = v.orientation.to_euler(glam::EulerRot::YXZ);
    let pitch_lift_multiplier = 1.0 + ALPHA_PLANING_LIFT_GAIN * pitch_rad.sin().max(0.0);

    // Per-section normalization factor: force coefficients are specified per-pontoon
    // (whole-pontoon effective strength); divided across sections so summing all
    // sections returns the original per-pontoon total when fully wet. Max displacement
    // similarly divides per section.
    #[allow(clippy::cast_precision_loss)]
    let per_section_scale = 1.0 / PONTOON_SAMPLE_COUNT as f32;
    let section_length_factor = per_section_scale; // each section is 1/N of pontoon length

    for pontoon in &BEAVER_PONTOONS {
        let section_length = pontoon.length_m * section_length_factor;
        let section_max_displacement = pontoon.max_displacement_m3 * per_section_scale;
        // Per-pontoon wetted accumulator (for the whole-pontoon hydrofoil
        // stick force applied once after the sample loop — see below).
        let mut pontoon_wetted_accum = 0.0_f32;

        for i in 0..PONTOON_SAMPLE_COUNT {
            // Sample center fraction along pontoon length, body-Z offset. For N=5:
            // i=0: -0.4 (fore), i=4: +0.4 (aft).
            #[allow(clippy::cast_precision_loss)]
            let frac = (i as f32 + 0.5) * per_section_scale - 0.5;
            let sample_body_offset =
                pontoon.body_offset + Vec3::new(0.0, 0.0, frac * pontoon.length_m);

            // Sample center in world frame.
            let world_offset = v.orientation * sample_body_offset;
            let world_center = v.position + world_offset;

            // Water surface at this sample's (x, z). Scaled by `chop_intensity` so
            // the same deterministic wave phase can be damped (glassy water) or
            // amplified (heavy chop) without recomputing the Gerstner sum. The
            // function returns displacement around the y=0 rest plane, so scalar
            // multiplication correctly scales amplitude while preserving phase.
            let water_y = water_height(world_center.x, world_center.z, wave_time) * chop_intensity;

            // Orientation-aware section bottom: body -Y half-height rotated.
            let section_bottom_world =
                world_center + v.orientation * Vec3::new(0.0, -pontoon.height_m * 0.5, 0.0);

            // Immersion depth for this section.
            let immersion_depth = (water_y - section_bottom_world.y).max(0.0);
            let immersion_height = immersion_depth.min(pontoon.height_m);

            if immersion_height <= 0.0 {
                continue;
            }

            let wetted_fraction = immersion_height / pontoon.height_m;
            wetted_fraction_accum += wetted_fraction;
            wetted_sample_count += 1;

            // ── Hydrostatic buoyancy (per section) ────────────────────────────
            let box_volume = immersion_height * section_length * pontoon.beam_m;
            let v_submerged = box_volume.min(section_max_displacement);
            let buoyancy_magnitude = WATER_DENSITY_KG_M3 * v_submerged * GRAVITY.length();
            let buoyancy_force_world = Vec3::new(0.0, buoyancy_magnitude, 0.0);

            // ── Sample velocity (includes angular contribution) ───────────────
            let v_sample_world = v.velocity + angular_velocity_world.cross(world_offset);

            // ── Vertical damping (linear, distributed per section) ────────────
            let v_damp_y = -VERTICAL_DAMPING_N_PER_MPS
                * per_section_scale
                * v_sample_world.y
                * wetted_fraction;

            // ── Horizontal drag (quadratic, anisotropic, distributed) ─────────
            // Decompose into body-frame longitudinal (along-float) and lateral
            // (across-float). Real floats are long-and-narrow → low
            // longitudinal drag, high lateral drag. Isotropic drag produces
            // unphysical sideslip during water-rudder turns / crosswind.
            let v_horizontal = Vec3::new(v_sample_world.x, 0.0, v_sample_world.z);
            let v_horiz_body = v.orientation.inverse() * v_horizontal;
            let v_long = v_horiz_body.z; // forward-back along float (+Z = aft, –Z = nose-forward)
            let v_lat = v_horiz_body.x; // sideways across float
            let drag_long_body = -v_long.abs()
                * v_long
                * HORIZONTAL_DRAG_N_PER_MPS2
                * per_section_scale
                * wetted_fraction;
            let drag_lat_body = -v_lat.abs()
                * v_lat
                * LATERAL_HYDRO_DRAG_N_PER_MPS2
                * per_section_scale
                * wetted_fraction;
            let horizontal_drag_world =
                v.orientation * Vec3::new(drag_lat_body, 0.0, drag_long_body);

            // ── Planing lift (quadratic in body-forward speed, distributed) ───
            // Body-forward velocity at this sample (body -Z component). Rotate
            // sample velocity into body frame; forward component is -z.
            //
            // Applied in BODY +Y (perpendicular to float deck), NOT world +Y.
            // Hydrodynamic lift on a planing hull acts normal to the hull
            // surface (approximately body-Y for a floatplane's roughly-
            // horizontal pontoon bottom). World-vertical framing introduced
            // a bank-angle coupling: when banked, world-Y lift developed a
            // body-lateral component which combined with pontoon offset to
            // produce spurious roll moment that positive-fed bank angle at
            // high-throttle + high-yaw. Body-Y framing keeps lift aligned
            // with hull normal so asymmetric wetted-side lift produces
            // restoring roll moment (toward level), not destabilizing.
            // Added 2026-04-22 after abusive-input flip regression guard.
            let v_sample_body = v.orientation.inverse() * v_sample_world;
            let v_body_forward = -v_sample_body.z;
            let planing_lift_body_vec = if v_body_forward > 0.0 {
                // α-dependent boost (Savitsky planing-hull theory): nose-up
                // trim angles raise hydrodynamic lift coefficient. Multiplier
                // is clamped ≥ 1.0, so level/nose-down attitudes behave as
                // before and only nose-up gets the benefit.
                let lift_magnitude = PLANING_LIFT_N_PER_MPS2
                    * per_section_scale
                    * v_body_forward
                    * v_body_forward
                    * wetted_fraction
                    * pitch_lift_multiplier;
                Vec3::new(0.0, lift_magnitude, 0.0)
            } else {
                Vec3::ZERO
            };
            let planing_lift_world = v.orientation * planing_lift_body_vec;

            // Accumulate per-pontoon wetted for the stick force applied once
            // after the sample loop at pontoon center — see below.
            pontoon_wetted_accum += wetted_fraction * per_section_scale;

            // Buoyancy + drag + damping all act at the section geometric center.
            let other_force_world = buoyancy_force_world
                + Vec3::new(horizontal_drag_world.x, v_damp_y, horizontal_drag_world.z);
            let other_force_body = v.orientation.inverse() * other_force_world;
            let other_moment_body = sample_body_offset.cross(other_force_body);

            // Planing lift acts at a forward-offset point (PLANING_LIFT_CENTROID_Z_FWD_M
            // ahead of the section center) — models the bow-wave center-of-pressure
            // location on a real planing hull. This gives planing the natural pitch-up
            // moment that balances drag's pitch-down, enabling stable on-step attitude.
            let planing_offset_body =
                sample_body_offset + Vec3::new(0.0, 0.0, PLANING_LIFT_CENTROID_Z_FWD_M);
            let planing_force_body = v.orientation.inverse() * planing_lift_world;
            let planing_moment_body = planing_offset_body.cross(planing_force_body);

            total_force_world += other_force_world + planing_lift_world;
            total_moment_body += other_moment_body + planing_moment_body;
        }

        // ── Inside-float hydrofoil stick (per-pontoon, post-sample-loop) ──
        // Applied once at the pontoon's geometric center — NOT distributed
        // per-sample — so that per-sample wetted-fraction asymmetry during
        // pitch oscillation (porpoise) doesn't spawn spurious pitch moments.
        // Force is world-Y (vertical); moment comes from pontoon offset in
        // body frame so strong lateral arm → strong roll-restoring moment.
        //
        // Quadratic in yaw rate: matches hydrofoil lift ∝ v² physics.
        //
        // **HARD CLAMPED** to STICK_FORCE_MAX_N post playtest (task #179+):
        // unbounded quadratic scaling let any yaw input (even at 16%
        // throttle) drive runaway flips when players sent abusive inputs
        // — rapid rudder reversal, sustained max yaw at any throttle.
        // Physical max wetted-hydrofoil lift IS bounded by wetted area ×
        // max lift coefficient; clamp enforces that bound. Real physics
        // doesn't produce unbounded forces.
        let yaw_rate_body = v.angular_velocity.y;
        let inside_indicator = -pontoon.body_offset.x * yaw_rate_body;
        let v_cg_body_forward = (-(v.orientation.inverse() * v.velocity).z).max(0.0);
        let stick_force_raw = -INSIDE_FLOAT_STICK_COEFF
            * inside_indicator
            * yaw_rate_body.abs()
            * v_cg_body_forward
            * pontoon_wetted_accum;
        let stick_force_world_y = stick_force_raw.clamp(-STICK_FORCE_MAX_N, STICK_FORCE_MAX_N);
        // Roll-angle gate: the stick force is physically meaningful only in
        // near-level taxi attitudes. Applied in world-Y, at high bank angles
        // it acquires a body-lateral component which — combined with the
        // pontoon's longitudinal offset — produces a spurious ROLL moment that
        // positive-feeds into more bank. Fade linearly from full authority at
        // |roll| ≤ 30° to zero at |roll| ≥ 60°. Beyond 60° the aircraft is in
        // an out-of-normal-operating-envelope state where this "nominal taxi
        // stabilization" force has no physical basis and only drives flip.
        // Added 2026-04-22 after abusive-input regression scenario.
        let (_yaw, _pitch, roll_rad) = v.orientation.to_euler(glam::EulerRot::YXZ);
        let roll_abs_deg = roll_rad.to_degrees().abs();
        let stick_roll_gate = ((60.0 - roll_abs_deg) / 30.0).clamp(0.0, 1.0);
        let stick_force_world = Vec3::new(0.0, stick_force_world_y * stick_roll_gate, 0.0);
        let stick_force_body = v.orientation.inverse() * stick_force_world;
        let stick_moment_body = pontoon.body_offset.cross(stick_force_body);
        total_force_world += stick_force_world;
        total_moment_body += stick_moment_body;
    }

    // Water rudder: deployed state produces BOTH a yaw moment AND a lateral
    // push on the floats, scaled by forward speed² and average wetted
    // fraction. Retracted state contributes nothing — aircraft weathervanes
    // via existing aero stability (`CN_BETA`).
    //
    // Force-at-offset → both linear and angular accel: a rigid body with a
    // side-force applied aft of CG translates sideways (tail pushed aside)
    // AND rotates around CG. The combination produces a realistic pivot
    // point somewhere between the CG and the rudder, which is how real
    // float-plane water turns track — the aircraft doesn't pirouette around
    // its own center, it sweeps its tail around.
    //
    // Sign: positive yaw input (nose-right per flowplan convention) pushes
    // the aft pontoons leftward in body frame, which rotates the nose
    // rightward and translates the whole aircraft leftward. In body-frame
    // moment storage this is a NEGATIVE `moment_body.y` (because +Y body
    // angular velocity is yaw-LEFT per state.rs convention).
    if v.controls.water_rudder && wetted_sample_count > 0 {
        #[allow(clippy::cast_precision_loss)]
        let avg_wetted = wetted_fraction_accum / wetted_sample_count as f32;
        let v_body = v.orientation.inverse() * v.velocity;
        let v_body_forward = -v_body.z;
        if v_body_forward > 0.0 {
            let side_force_raw =
                WATER_RUDDER_COEFF * v_body_forward * v_body_forward * avg_wetted * v.controls.yaw;
            let side_force =
                side_force_raw.clamp(-WATER_RUDDER_FORCE_MAX_N, WATER_RUDDER_FORCE_MAX_N);
            let force_body = Vec3::new(-side_force, 0.0, 0.0);
            let arm_body = Vec3::new(0.0, 0.0, WATER_RUDDER_ARM_M);
            // YAW-ONLY framing: in real water, the rudder's lateral force
            // is resisted by the pontoon hull's enormous lateral drag (long/
            // narrow hulls act like keels). The aircraft rotates in place
            // more than it slides — then apparent sideslip emerges from
            // velocity-vector lag behind the rotating heading. Earlier we
            // added both the yaw moment AND a linear lateral push at CG,
            // which double-counted and produced visible side-sliding
            // (playtest feedback 2026-04-22). Pure moment framing: rudder
            // contributes yaw only, lateral motion comes from pontoon
            // lateral drag balance + centripetal response to forward motion.
            let moment_body = arm_body.cross(force_body);
            total_moment_body += moment_body;
        }
    }

    // ── Submerged-surface flotation+drag (wingtips, fuselage ends) ────────
    // When the aircraft banks past ~30° or pitches past ~20° on water, a
    // wingtip or fuselage end will dip below the water surface. Without
    // modeling that surface's interaction with water, nothing stops the
    // aircraft from continuing to rotate further — the pontoon-only model
    // is blind to below-pontoon geometry.
    //
    // This section adds per-point "drag + buoyancy" force at key airframe
    // extrema (wingtips, tail, nose). For each point: if submerged, produce
    //   - Buoyancy ~ submersion_depth × coeff (upward in world frame)
    //   - Quadratic drag ~ v² × submersion_depth × coeff opposing motion
    // The force is applied at the point's body offset so it produces both
    // linear and angular (restoring) effects. Models the physical
    // "wingtip digs into water and lifts aircraft back to level" mechanism.
    //
    // Added 2026-04-22 after abusive-input flip diagnostic: user correctly
    // identified that we had realistic ACTIVE forces (water rudder, stick)
    // but insufficient RESISTING forces at high-bank attitudes. Pontoon
    // lateral drag alone can't overwhelm sustained water-rudder push, but
    // wingtip-water drag at 30°+ bank is enormous and provides the missing
    // restoring torque that bounds the flip.
    for point in AIRFRAME_FLOAT_POINTS {
        let world_offset = v.orientation * point.body_offset;
        let world_pos = v.position + world_offset;
        let water_y = water_height(world_pos.x, world_pos.z, wave_time) * chop_intensity;
        let submersion = (water_y - world_pos.y).max(0.0);
        if submersion <= 0.0 {
            continue;
        }
        let submersion_clamped = submersion.min(point.max_depth_m);
        // Velocity at this point (CG velocity + rotational contribution).
        let v_point_world = v.velocity + angular_velocity_world.cross(world_offset);

        // Buoyancy: proportional to submersion depth × coefficient.
        let buoyancy_world = Vec3::new(0.0, submersion_clamped * point.buoyancy_n_per_m, 0.0);

        // Quadratic drag opposing velocity. Apply only horizontal + vertical
        // components; clamped magnitude per dimension to prevent explosion
        // in degenerate high-speed scenarios.
        let v_mag_sq = v_point_world.length_squared();
        let drag_world = if v_mag_sq > 1e-6 {
            let v_mag = v_mag_sq.sqrt();
            let drag_magnitude = point.drag_coeff * v_mag_sq * submersion_clamped;
            let drag_magnitude_clamped = drag_magnitude.min(point.drag_max_n);
            -v_point_world * (drag_magnitude_clamped / v_mag)
        } else {
            Vec3::ZERO
        };

        let total_point_force_world = buoyancy_world + drag_world;
        let force_body = v.orientation.inverse() * total_point_force_world;
        let moment_body = point.body_offset.cross(force_body);

        total_force_world += total_point_force_world;
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
            variants: Vec::new(),
            default_variant: String::new(),
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
        let out = compute_buoyancy(&v, None, 0.0, 1.0);
        assert_eq!(out.linear_accel, Vec3::ZERO);
        assert_eq!(out.angular_accel, Vec3::ZERO);
        assert!(out.total_force_n.abs() < f32::EPSILON);
    }

    #[test]
    fn airborne_aircraft_sees_zero_buoyancy() {
        // CG at y=50m → floats nowhere near water.
        let v = level_vehicle_at(50.0);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);
        assert!(
            out.total_force_n.abs() < f32::EPSILON,
            "expected zero buoyancy above water, got {} N",
            out.total_force_n,
        );
    }

    #[test]
    fn submerged_aircraft_sees_capped_buoyancy() {
        // CG at y=0 → floats centered at -0.5m, fully submerged (bottom at -0.88m,
        // top at -0.12m, both below water level y=0). Expect the displacement cap
        // PLUS contribution from airframe float points (wingtips, nose, tail) which
        // are now also modeled as flotation surfaces when submerged (post-2026-04-22
        // abusive-input fix — see BEAVER_AIRFRAME_FLOAT_POINTS).
        let v = level_vehicle_at(0.0);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        // Pontoon cap = 2 × 2.236 m³ × 1000 × 9.81 ≈ 43,870 N.
        let pontoon_cap = 2.0 * 2.236 * WATER_DENSITY_KG_M3 * GRAVITY.length();
        // Upward force must exceed pontoon cap (airframe points add buoyancy when
        // submerged) and must exceed weight.
        assert!(
            out.total_force_n >= pontoon_cap - 50.0,
            "expected at least pontoon cap ~{pontoon_cap:.0} N, got {:.0} N",
            out.total_force_n,
        );
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
        // Analytic equilibrium with new pontoon body_offset.y = -2.24:
        // Per-float V = mass/2 / ρ_water = 1157 / 1000 = 1.157 m³.
        // Per-float wetted area = 6.9 × 0.76 = 5.244 m². Depth = 1.157 / 5.244 = 0.221 m.
        // Float center_y = water_y - depth + height/2 = 0 - 0.221 + 0.38 = 0.159 m.
        // CG_y = center_y - body_offset.y = 0.159 - (-2.24) = 2.40 m.
        let v = level_vehicle_at(2.40);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

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
    fn deeply_submerged_symmetric_produces_no_roll_or_yaw() {
        // With multi-point sampling, partial immersion produces per-section wave-
        // phase asymmetry that correctly creates real (non-zero) roll moments — a
        // physically faithful behavior that would have been hidden by single-point
        // sampling when the centroid hits the displacement cap. To verify no
        // systematic bias in the port/starboard-symmetric buoyancy code, deeply
        // submerge the aircraft so every section saturates at its displacement cap;
        // wave phase then doesn't affect per-section buoyancy, and symmetric
        // port/starboard geometry must give zero roll and yaw moments. Pitch is
        // NOT symmetric because the airframe is fore/aft asymmetric (tail further
        // back than prop, wingtip ahead of tail) — pitch at deep submersion
        // reflects real airframe mass/buoyancy distribution.
        let v = level_vehicle_at(-1.0);
        let spec = beaver_spec();
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);
        assert!(
            out.angular_accel.z.abs() < 1e-4,
            "expected zero roll accel at deep submersion, got {}",
            out.angular_accel.z,
        );
        assert!(
            out.angular_accel.y.abs() < 1e-4,
            "expected zero yaw accel at deep submersion, got {}",
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
        let v = level_vehicle_at(2.32);
        let spec = beaver_spec();
        let out_t0 = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);
        let out_t1 = compute_buoyancy(&v, Some(&spec), 0.5, 1.0);
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
        let mut v = level_vehicle_at(2.32);
        v.velocity = Vec3::new(0.0, -1.0, 0.0); // sinking at 1 m/s

        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        // Baseline buoyancy at y=0.58 with zero velocity, for comparison.
        let v_still = level_vehicle_at(2.32);
        let baseline = compute_buoyancy(&v_still, Some(&spec), 0.0, 1.0);

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
        let mut v = level_vehicle_at(2.32);
        v.velocity = Vec3::new(0.0, 1.0, 0.0); // rising at 1 m/s

        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);
        let v_still = level_vehicle_at(2.32);
        let baseline = compute_buoyancy(&v_still, Some(&spec), 0.0, 1.0);

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
        let mut v = level_vehicle_at(2.32);
        v.velocity = Vec3::new(0.0, 0.0, -5.0); // 5 m/s forward

        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

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
        // more drag at the same velocity. New geometry: equilibrium at y=2.40,
        // shallow case slightly above (less wet), deep case below (more wet).
        let spec = beaver_spec();
        let mut v_shallow = level_vehicle_at(2.50);
        v_shallow.velocity = Vec3::new(5.0, 0.0, 0.0);
        let out_shallow = compute_buoyancy(&v_shallow, Some(&spec), 0.0, 1.0);

        let mut v_deep = level_vehicle_at(2.05);
        v_deep.velocity = Vec3::new(5.0, 0.0, 0.0);
        let out_deep = compute_buoyancy(&v_deep, Some(&spec), 0.0, 1.0);

        assert!(
            out_deep.linear_accel.x.abs() > out_shallow.linear_accel.x.abs(),
            "deeper submersion should produce more drag; shallow={} deep={}",
            out_shallow.linear_accel.x,
            out_deep.linear_accel.x,
        );
    }

    // ── Phase 2.5a.3 tests: multi-point sampling + planing lift ──────────────

    #[test]
    fn nose_down_pitch_produces_pitch_up_buoyancy_moment() {
        // THE test that would have caught the 2.5a.2 water-contact flip bug: when
        // the aircraft pitches nose-down, forward float sections dip deeper than aft
        // sections (because the float bottoms tilt with orientation). More buoyancy
        // forward than aft produces a pitch-up restoring moment. Without multi-point
        // sampling, all of a pontoon's buoyancy was applied at its geometric center,
        // so pitch pitched the float around its own centroid → zero restoring moment.
        let spec = beaver_spec();
        let mut v = level_vehicle_at(2.40);
        // Nose-down ~10° via rotation about body +X axis (right wing axis). Per
        // state.rs sign convention, positive X rotation is nose-up; apply -10° for
        // nose-down.
        v.orientation = Quat::from_axis_angle(Vec3::X, -10.0_f32.to_radians());
        v.velocity = Vec3::ZERO;
        v.angular_velocity = Vec3::ZERO;

        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        // Body-frame pitch moment: positive X → nose-up. Nose-down attitude should
        // produce positive pitch moment (restoring toward level).
        assert!(
            out.angular_accel.x > 0.01,
            "nose-down should produce pitch-up restoring moment, got {}",
            out.angular_accel.x,
        );
    }

    #[test]
    fn nose_up_pitch_produces_pitch_down_buoyancy_moment() {
        // Mirror of the nose-down test: nose-up attitude dips aft sections deeper
        // than fore sections, producing a pitch-down restoring moment.
        let spec = beaver_spec();
        let mut v = level_vehicle_at(2.40);
        v.orientation = Quat::from_axis_angle(Vec3::X, 10.0_f32.to_radians());
        v.velocity = Vec3::ZERO;
        v.angular_velocity = Vec3::ZERO;

        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        assert!(
            out.angular_accel.x < -0.01,
            "nose-up should produce pitch-down restoring moment, got {}",
            out.angular_accel.x,
        );
    }

    #[test]
    fn planing_lift_scales_with_forward_speed_squared() {
        // Planing lift is quadratic in forward body velocity. Compare a slow and a
        // fast case; the fast case should produce roughly 4× the lift (2× speed →
        // 4× v²).
        let spec = beaver_spec();

        let mut v_slow = level_vehicle_at(2.32);
        v_slow.velocity = Vec3::new(0.0, 0.0, -10.0); // 10 m/s forward (body -Z)
        let out_slow = compute_buoyancy(&v_slow, Some(&spec), 0.0, 1.0);

        let mut v_fast = level_vehicle_at(2.32);
        v_fast.velocity = Vec3::new(0.0, 0.0, -20.0); // 20 m/s forward
        let out_fast = compute_buoyancy(&v_fast, Some(&spec), 0.0, 1.0);

        // Extract the planing lift contribution by subtracting buoyancy-only baseline
        // (at rest at same draft).
        let v_still = level_vehicle_at(2.32);
        let baseline = compute_buoyancy(&v_still, Some(&spec), 0.0, 1.0);

        let slow_lift_component = out_slow.linear_accel.y - baseline.linear_accel.y;
        let fast_lift_component = out_fast.linear_accel.y - baseline.linear_accel.y;

        // Slow should be positive (planing lift adds upward accel).
        assert!(
            slow_lift_component > 0.1,
            "slow planing should add upward accel, got {slow_lift_component}",
        );
        // Fast should be roughly 4× slow (within numerical tolerance — there's also
        // horizontal drag and vertical damping contributing, but for zero velocity
        // baseline and symmetric motion those scale differently).
        let ratio = fast_lift_component / slow_lift_component;
        assert!(
            ratio > 3.0 && ratio < 5.0,
            "20 m/s lift should be ~4× 10 m/s lift (v² scaling); ratio={ratio:.2}",
        );
    }

    #[test]
    fn planing_lift_zero_for_backward_motion() {
        // Planing lift only applies to forward motion (body -Z). Reverse motion
        // shouldn't produce upward planing force (real floats don't plane backward).
        let spec = beaver_spec();
        let mut v_forward = level_vehicle_at(2.32);
        v_forward.velocity = Vec3::new(0.0, 0.0, -10.0); // body -Z (forward)
        let out_forward = compute_buoyancy(&v_forward, Some(&spec), 0.0, 1.0);

        let mut v_backward = level_vehicle_at(2.32);
        v_backward.velocity = Vec3::new(0.0, 0.0, 10.0); // body +Z (backward)
        let out_backward = compute_buoyancy(&v_backward, Some(&spec), 0.0, 1.0);

        // Forward produces planing lift on top of buoyancy; backward does not. So
        // forward vertical accel > backward vertical accel.
        assert!(
            out_forward.linear_accel.y > out_backward.linear_accel.y + 0.1,
            "forward motion should add planing lift; forward_y={}, backward_y={}",
            out_forward.linear_accel.y,
            out_backward.linear_accel.y,
        );
    }

    // ── Phase 2.5a.6 tests: water rudder ──────────────────────────────────

    #[test]
    fn water_rudder_retracted_produces_no_yaw_moment() {
        // With water_rudder = false (the default), no yaw moment contribution
        // from rudder even with yaw input. Weathervaning is handled by aero
        // stability (CN_BETA), not by this module.
        let spec = beaver_spec();
        let mut v = level_vehicle_at(2.40);
        v.velocity = Vec3::new(0.0, 0.0, -5.0); // forward 5 m/s
        v.controls.yaw = 1.0; // full right rudder
        v.controls.water_rudder = false;
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        assert!(
            out.angular_accel.y.abs() < 0.01,
            "retracted water rudder should produce ~zero yaw accel, got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn water_rudder_deployed_produces_yaw_moment_with_yaw_input() {
        // With water_rudder deployed + positive yaw input + forward motion:
        // expect NEGATIVE body-Y angular accel (which in flowplan convention
        // = yaw-RIGHT).
        let spec = beaver_spec();
        let mut v = level_vehicle_at(2.40);
        v.velocity = Vec3::new(0.0, 0.0, -5.0); // 5 m/s forward
        v.controls.yaw = 1.0; // full right rudder
        v.controls.water_rudder = true;
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        assert!(
            out.angular_accel.y < -0.01,
            "deployed water rudder + right yaw input should produce negative body-Y angular accel (yaw right), got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn water_rudder_reverses_with_opposite_yaw_input() {
        let spec = beaver_spec();
        let mut v = level_vehicle_at(2.40);
        v.velocity = Vec3::new(0.0, 0.0, -5.0);
        v.controls.yaw = -1.0; // full left rudder
        v.controls.water_rudder = true;
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        assert!(
            out.angular_accel.y > 0.01,
            "deployed water rudder + left yaw input should produce positive body-Y angular accel (yaw left), got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn water_rudder_scales_with_forward_speed_squared() {
        // Test v² scaling at speeds BELOW the WATER_RUDDER_FORCE_MAX_N clamp.
        // At 1 m/s, raw side force ≈ 800 × 1 × 0.5 = 400 N (below 5000 N cap).
        // At 2 m/s, raw side force ≈ 800 × 4 × 0.5 = 1600 N (still below cap).
        // Ratio should be ~4× for clean v² verification. Post-2026-04-22 clamp
        // saturates high speeds, so the test uses low speeds to verify formula.
        let spec = beaver_spec();
        let mut v_slow = level_vehicle_at(2.40);
        v_slow.velocity = Vec3::new(0.0, 0.0, -1.0);
        v_slow.controls.yaw = 1.0;
        v_slow.controls.water_rudder = true;
        let slow = compute_buoyancy(&v_slow, Some(&spec), 0.0, 1.0);

        let mut v_fast = level_vehicle_at(2.40);
        v_fast.velocity = Vec3::new(0.0, 0.0, -2.0); // 2× speed
        v_fast.controls.yaw = 1.0;
        v_fast.controls.water_rudder = true;
        let fast = compute_buoyancy(&v_fast, Some(&spec), 0.0, 1.0);

        // 2× speed → ~4× moment (quadratic scaling, pre-clamp regime).
        let ratio = fast.angular_accel.y / slow.angular_accel.y;
        assert!(
            ratio > 3.0 && ratio < 5.0,
            "water rudder should scale v² (fast/slow ratio ~4), got {ratio}",
        );
    }

    #[test]
    fn water_rudder_zero_above_water() {
        // Aircraft airborne: pontoons dry → no wetted fraction → no rudder moment
        // even with deployed rudder + yaw input.
        let spec = beaver_spec();
        let mut v = level_vehicle_at(50.0); // way above water
        v.velocity = Vec3::new(0.0, 0.0, -30.0);
        v.controls.yaw = 1.0;
        v.controls.water_rudder = true;
        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);

        assert!(out.angular_accel.y.abs() < 1e-6);
    }

    // ── Phase 2.5a.7 tests: chop-intensity scalar ────────────────────────────

    #[test]
    fn chop_intensity_zero_freezes_wave_phase_effect() {
        // With chop_intensity = 0, the sampled water surface collapses to y=0
        // regardless of wave_time. So buoyancy at t=0 and t=0.5 must be identical
        // (the same test that `wave_time_changes_sampled_surface` relies on would
        // fail here — which is exactly the glassy-water guarantee we want).
        let v = level_vehicle_at(2.32);
        let spec = beaver_spec();
        let out_t0 = compute_buoyancy(&v, Some(&spec), 0.0, 0.0);
        let out_t1 = compute_buoyancy(&v, Some(&spec), 0.5, 0.0);
        assert!(
            (out_t0.total_force_n - out_t1.total_force_n).abs() < f32::EPSILON,
            "chop=0 should freeze wave phase; t0={:.3} t1={:.3}",
            out_t0.total_force_n,
            out_t1.total_force_n,
        );
    }

    #[test]
    fn chop_intensity_amplifies_wave_phase_swing() {
        // Heavier chop → larger per-tick force swings as the wave phase sweeps.
        // Same draft, same wave_times, but chop=2 should produce a bigger delta
        // between t=0 and t=0.5 than chop=1. Uses the unsaturated regime (y=2.32)
        // where phase differences translate to volume differences.
        let v = level_vehicle_at(2.32);
        let spec = beaver_spec();
        let nominal_t0 = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);
        let nominal_t1 = compute_buoyancy(&v, Some(&spec), 0.5, 1.0);
        let nominal_swing = (nominal_t0.total_force_n - nominal_t1.total_force_n).abs();

        let heavy_t0 = compute_buoyancy(&v, Some(&spec), 0.0, 2.0);
        let heavy_t1 = compute_buoyancy(&v, Some(&spec), 0.5, 2.0);
        let heavy_swing = (heavy_t0.total_force_n - heavy_t1.total_force_n).abs();

        assert!(
            heavy_swing > nominal_swing * 1.5,
            "chop=2 should amplify wave-phase force swing vs chop=1; \
             nominal_swing={nominal_swing:.1} heavy_swing={heavy_swing:.1}",
        );
    }

    #[test]
    fn above_water_has_zero_drag() {
        // Airborne aircraft moving fast: no water contact → zero drag contribution
        // from buoyancy module. (Aero drag is handled separately in flight.rs.)
        let spec = beaver_spec();
        let mut v = level_vehicle_at(50.0);
        v.velocity = Vec3::new(10.0, -5.0, -30.0);

        let out = compute_buoyancy(&v, Some(&spec), 0.0, 1.0);
        assert!(out.linear_accel.length() < f32::EPSILON);
        assert!(out.total_force_n.abs() < f32::EPSILON);
    }
}
