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

use glam::{Quat, Vec3};

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

/// Per-gear vertical damping during COMPRESSION (gear sinking into ground,
/// `v_gear_world.y < 0`). Combines with `GEAR_SPRING_N_PER_M` to absorb
/// impact energy. Moderate — enough to catch a normal landing without a
/// secondary bounce, but soft enough that gentle settling still feels
/// springy.
const GEAR_DAMPING_COMPRESSION_N_PER_MPS: f32 = 25_000.0;

/// Per-gear vertical damping during REBOUND (gear lifting away from ground,
/// `v_gear_world.y > 0`). Significantly HIGHER than compression damping —
/// real oleo struts bleed compression energy through a one-way orifice so
/// compression stores energy moderately and rebound dissipates the stored
/// energy aggressively. The asymmetry is THE reason real landing gear
/// doesn't trampoline on impact.
///
/// 2.5b.5 calibration found that symmetric damping (the pre-2.5b.5 model
/// with a single `GEAR_DAMPING_N_PER_MPS`) elastically returned hard-impact
/// energy, lofting the aircraft off its gear during brake-induced pitch
/// transients. Asymmetric damping at a 3:1 rebound:compression ratio fixes
/// that without overdamping gentle settling.
///
/// **Not a friction model** — the tire/ground contact point itself has
/// kinetic friction (handled by `ROLLING_FRICTION_COEFF`, `LATERAL_FRICTION_COEFF`).
/// This is the strut's hydraulic damping between the contact patch and the
/// airframe CG. Real oleo struts have this one-way-orifice behavior;
/// leaf-spring or rubber-cord gear approximates with stiffer bushings.
const GEAR_DAMPING_REBOUND_N_PER_MPS: f32 = 75_000.0;

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

/// Lateral (cornering) friction coefficient on paved surface.
///
/// Tires resist side-slip much more than they resist rolling — that asymmetry
/// IS what makes wheeled vehicles track straight. A rolling tire has an
/// effective `µ_r ≈ 0.02`; a laterally-slipping tire grips up to a peak
/// sliding coefficient of `µ_s ≈ 0.7-0.9` on dry paved, which decays to
/// ~0.6 under sustained slip.
///
/// For 2.5b.3, we model this as a simple kinetic-friction approximation:
/// lateral force magnitude per gear is `µ_lat · N`, direction opposing the
/// lateral-slip component of the gear's horizontal velocity. This is
/// anisotropic (different forward vs lateral coefficients) without the full
/// slip-angle / cornering-stiffness tire model — which would be 2.5b.6 or
/// later if vehicle-dynamics realism demands it.
///
/// `0.6` chosen mid-range for dry paved; low enough that a modest crosswind
/// can still weathervane the aircraft, high enough that pilots tracking
/// straight don't see drift at taxi speed.
const LATERAL_FRICTION_COEFF: f32 = 0.6;

/// Maximum tailwheel deflection from neutral (radians). Real Beaver tailwheel
/// steering range is approximately ±30° (0.524 rad) before the linkage bottoms
/// out. Deflection commanded by yaw input × this angle when the tailwheel is
/// locked (low ground speed); zero when castoring above threshold.
const TAILWHEEL_MAX_STEER_RAD: f32 = 0.524; // ~30°

/// Ground speed at which tailwheel starts transitioning from locked (steering)
/// to fully castoring (free-pivot aligned with velocity). Real Beaver behavior:
/// at taxi speeds (< ~15 kt / 7.7 m/s) the tailwheel is locked and responds to
/// pedal input; above ~20 kt (~10 m/s) the rudder has enough authority that the
/// tailwheel disengages and castors — pilot steers with rudder instead.
const V_STEER_FULL_MPS: f32 = 7.7;
const V_CASTOR_FULL_MPS: f32 = 10.0;

/// Velocity threshold below which friction smoothly ramps through zero, so a
/// stopped aircraft doesn't jitter between +eps / -eps sign flips. At
/// `|v_component|` ≥ `V_EPS` the friction magnitude is the full `µ·N`;
/// between 0 and `V_EPS` it scales linearly.
const V_EPS: f32 = 0.1;

/// Peak brake friction coefficient on paved surface.
///
/// When `Controls.brakes` is held, each brakable gear adds an additional
/// forward drag `F_brake = -µ_brake · N · brake_factor` opposing the forward
/// velocity component at that gear. `0.5` is a typical dry-paved brake µ for
/// a well-maintained tire before lockup — enough for a Beaver at taxi speed
/// to stop in a few meters. Skid / lockup dynamics (where lateral grip
/// collapses) arrive with 2.5b.6's per-surface model; v1 assumes the pilot
/// modulates to stay below lockup.
///
/// Real-world sanity: weight 22,700 N × 0.5 / 2313 kg = 4.9 m/s² decel ≈ 0.5 g.
/// Aggressive but within aircraft brake design envelope.
const BRAKE_FRICTION_COEFF: f32 = 0.5;

/// Per-contact geometry. One entry per wheel / tailwheel.
#[derive(Clone, Copy, Debug)]
struct GearPoint {
    /// Body-frame offset from the aircraft CG to the gear contact point (m). This
    /// is the **fully-extended** tire-to-ground point; compression shortens the
    /// effective y-offset, but the spring model applies force at this attach
    /// point (rigid-body approximation — strut compression is not modeled as a
    /// separate degree of freedom in v1).
    body_offset: Vec3,
    /// Spring rate multiplier relative to `GEAR_SPRING_N_PER_M`. Mains use 1.0;
    /// tailwheel uses a softer value (real tailwheel springs ARE softer because
    /// they carry less weight and need to absorb lighter impacts — a Beaver
    /// tailwheel is a coiled-leaf or rubber-cord spring, much less stiff than
    /// the oleo mains).
    ///
    /// 2.5b.5 calibration fix: at the geometry-constrained 3-point pitch
    /// (θ=6.46°, `main_world_y ≈ tail_world_y`), both gears have equal
    /// compression `c_m = c_t`. With equal spring rates the forces would
    /// also be equal, violating moment balance (2·N·0.942 ≠ N·4.39 →
    /// large nose-down moment). With
    /// `k_tail/k_main = 2·(1.3·sin+0.8·cos)/(4.5·cos-0.7·sin) ≈ 0.428` at
    /// θ=6.46°, forces satisfy moment balance at equal compression: weight
    /// split 82% mains / 18% tail (near the real-Beaver 85/15).
    ///
    /// This converts the previous metastable equilibrium (tail leaves
    /// contact first on small perturbation → mains-only nose-up moment →
    /// divergent rotation) into a stable one: both gears lose contact
    /// simultaneously on upward perturbation.
    spring_factor: f32,
    /// Steering authority (0.0 = fixed, 1.0 = full). Mains are fixed; tailwheel
    /// is 1.0 but its effective steering ramps down via the castor-threshold
    /// gate — this field only scales the *maximum* commanded deflection.
    steer_factor: f32,
    /// Brake authority (0.0 = unbraked, 1.0 = full). Classic taildragger has
    /// toe-brakes on mains only; tailwheel doesn't brake. Trike configurations
    /// (future) would set mains + nose wheel to 1.0 and mains alone may be 1.0
    /// for trainers without nose-gear brakes.
    brake_factor: f32,
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
/// Airframe points that shouldn't normally contact the ground. When one of
/// them drops below ground during unusual attitudes (tail-strike on over-
/// rotation, wingtip-scrape during crosswind landing, prop-strike during
/// nose-over), the strike is detected and flagged on `GroundContactOutputs`.
/// No force or moment is applied — detection only. Damage consequences land
/// with Phase 4 ([ROADMAP.md § Milestone 4.3 Damage](../../../ROADMAP.md#milestone-43-damage-and-wear)).
///
/// Body-frame offsets are placeholder DHC-2 Beaver dimensions, subject to
/// mesh-alignment against `northlander_land.glb` in the same follow-up that
/// tracks gear-geometry migration to `AircraftSpec.strike_geometry`.
#[derive(Clone, Copy, Debug)]
struct StrikePoint {
    /// Body-frame offset from CG (m).
    body_offset: Vec3,
    /// Which kind of strike — lets the outputs struct expose individual flags
    /// so a UI / damage system can map strikes to specific failure modes
    /// (prop bent ≠ tail bulkhead torn ≠ wingtip scraped).
    kind: StrikeKind,
    /// Label for diagnostics / plots. Not functional.
    #[allow(dead_code)]
    label: &'static str,
}

/// Which airframe structure a strike point represents. Damage consequences
/// in Phase 4 map per-kind to different failure modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StrikeKind {
    /// Prop disk — forward of CG, low. Hits ground on hard nose-down pitch
    /// or steep forward slopes.
    Prop,
    /// Tail cone / rear fuselage — aft of CG, low. Hits ground on excessive
    /// nose-up rotation (stall on takeoff, over-flared landing).
    Tail,
    /// Left wingtip — lateral left, low. Hits ground on excessive right bank
    /// during ground roll (crosswind landing gone wrong).
    WingtipLeft,
    /// Right wingtip — lateral right, mirror of `WingtipLeft`.
    WingtipRight,
}

/// DHC-2 Beaver strike points (placeholder DHC-2 dimensions).
///
/// **Calibration constraint**: all strike points must have world-y > 0 at the
/// 3-point rest attitude (θ=6.46°, CG=1.155 m) so they don't false-positive
/// at rest. The tail offset `y = -0.3` (vs a naive "fuselage bottom ≈ main
/// gear level" of `-0.8`) reflects that the rear fuselage underside sits
/// ABOVE the tailwheel at rest — the tailwheel attach is the lowest point
/// of the empennage. Over-rotation past ~9° nose-up brings the rear
/// fuselage bottom to the ground.
///
/// - Prop: 3.0 m forward of CG, 1.0 m below (prop disk center; blade tips
///   sweep to radius ~1.3 m, so actual blade-tip contact at nose-down
///   attitude happens slightly before the disk-center crosses ground —
///   approximation).
/// - Tail: 5.5 m aft, 0.3 m below (rear fuselage underside, NOT the
///   tailwheel contact — that's handled by `BEAVER_GEAR`).
/// - Wingtips: ±7.3 m lateral (half of 14.6 m wingspan), 1.0 m below
///   (wingtip hangs below CG at level attitude; bank pushes one tip down).
const BEAVER_STRIKES: [StrikePoint; 4] = [
    StrikePoint {
        body_offset: Vec3::new(0.0, -1.0, -3.0),
        kind: StrikeKind::Prop,
        label: "prop",
    },
    StrikePoint {
        body_offset: Vec3::new(0.0, -0.3, 5.5),
        kind: StrikeKind::Tail,
        label: "tail",
    },
    StrikePoint {
        body_offset: Vec3::new(-7.3, -1.0, 0.0),
        kind: StrikeKind::WingtipLeft,
        label: "wingtip_left",
    },
    StrikePoint {
        body_offset: Vec3::new(7.3, -1.0, 0.0),
        kind: StrikeKind::WingtipRight,
        label: "wingtip_right",
    },
];

const BEAVER_GEAR: [GearPoint; 3] = [
    // Left main wheel (fixed — mains don't steer on a taildragger; fully
    // braked — toe brakes on mains is standard Beaver arrangement).
    GearPoint {
        body_offset: Vec3::new(-1.65, -1.3, -0.8),
        spring_factor: 1.0,
        steer_factor: 0.0,
        brake_factor: 1.0,
        label: "left_main",
    },
    // Right main wheel.
    GearPoint {
        body_offset: Vec3::new(1.65, -1.3, -0.8),
        spring_factor: 1.0,
        steer_factor: 0.0,
        brake_factor: 1.0,
        label: "right_main",
    },
    // Tailwheel — steerable via yaw input at taxi speed; castors above.
    // No brake: classic taildragger has no tailwheel brake.
    // Softer spring (factor 0.428): gives correct 82/18 weight distribution
    // at the geometry-constrained 3-point pitch, stable equilibrium.
    GearPoint {
        body_offset: Vec3::new(0.0, -0.7, 4.5),
        spring_factor: 0.428,
        steer_factor: 1.0,
        brake_factor: 0.0,
        label: "tailwheel",
    },
];

/// Per-kind strike flags for the current tick.
///
/// Records when non-gear airframe points touch the ground. Cleared and
/// recomputed each tick. Damage consequences land in Phase 4; for now these
/// feed the bench CSV (for post-scenario analysis) and can be wired to
/// audio / visual SFX without needing any new plumbing.
///
/// The struct-of-bools shape (vs a bitmask) is deliberate: this is not a
/// hot-path memory layout (one per-tick copy per aircraft) and named fields
/// make call sites `strikes.tail` instead of `strikes & STRIKE_TAIL`, which
/// matches how Phase 4 damage handlers will dispatch per-kind.
#[allow(clippy::struct_excessive_bools)] // per-kind enum-of-flags; see doc.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StrikeFlags {
    /// Prop disk has descended below ground — typically nose-over or a
    /// steep-forward taxi slope.
    pub prop: bool,
    /// Tail cone / empennage below ground — over-rotation on takeoff, or
    /// an over-flared landing.
    pub tail: bool,
    /// Left wingtip touching ground — too much right-bank during ground roll.
    pub wingtip_left: bool,
    /// Right wingtip touching ground — mirror of `wingtip_left`.
    pub wingtip_right: bool,
}

impl StrikeFlags {
    /// True when ANY strike point hit the ground this tick. Useful for a
    /// coarse "is something bad happening" signal.
    #[must_use]
    pub const fn any(self) -> bool {
        self.prop || self.tail || self.wingtip_left || self.wingtip_right
    }
}

/// Ground-contact outputs.
///
/// Linear + angular acceleration contributions, a diagnostic total-force
/// magnitude, and strike-detection flags. Mirrors `BuoyancyOutputs`
/// field-for-field on the physics side so `tick_dynamics` can sum them
/// uniformly; strike flags are sim-side observations that don't affect
/// physics.
#[derive(Clone, Copy, Debug, Default)]
pub struct GroundContactOutputs {
    /// Linear acceleration contribution in world frame (m/s²).
    pub linear_accel: Vec3,
    /// Angular acceleration contribution in body frame (rad/s²).
    pub angular_accel: Vec3,
    /// Sum of gear normal-force magnitudes across all contacts (N). Diagnostic.
    pub total_force_n: f32,
    /// Per-kind strike-detection flags for this tick. All-false at rest /
    /// airborne / normal ground contact. Non-force: Phase 4 damage system
    /// will map these to consequences.
    pub strikes: StrikeFlags,
}

/// Compute per-gear ground-contact forces + strike flags.
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
#[allow(clippy::too_many_lines)] // multi-pass physics + strike detection
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

        // Spring pushback + asymmetric damping.
        //
        // Sign: positive compression pushes world +Y. For v_gear_world.y:
        //   < 0 (sinking into ground, compressing further) → damper adds to
        //     spring pushback with `COMPRESSION` coefficient (moderate).
        //   > 0 (lifting away from ground, rebounding) → damper subtracts
        //     from spring pushback with `REBOUND` coefficient (stronger).
        // The asymmetry mimics an oleo strut's one-way orifice — compression
        // stores energy moderately, rebound bleeds it out aggressively.
        // Without this, hard impacts would elastically return their energy,
        // lofting the aircraft after brake-transient or landing loads.
        //
        // Clamped to non-negative: ground pushes up, never pulls. If rebound
        // damping would drive the total negative (gear lifting faster than
        // spring can push back), contact is effectively lost and the gear
        // separates cleanly.
        let spring_n = GEAR_SPRING_N_PER_M * gear.spring_factor * compression;
        let damp_coeff = if v_gear_world.y < 0.0 {
            GEAR_DAMPING_COMPRESSION_N_PER_MPS
        } else {
            GEAR_DAMPING_REBOUND_N_PER_MPS
        };
        let damper_n = -damp_coeff * v_gear_world.y;
        let normal_n = (spring_n + damper_n).max(0.0);
        if normal_n <= 0.0 {
            continue;
        }
        total_normal_magnitude += normal_n;

        // Normal force acts at the gear contact point in world +Y.
        let normal_force_world = Vec3::new(0.0, normal_n, 0.0);

        // ── Wheel-frame velocity decomposition ────────────────────────────
        //
        // Tires are anisotropic: they roll freely along their forward axis
        // (low µ_r) and grip laterally (high µ_lat). Decompose the gear's
        // horizontal velocity into wheel-frame (forward, lateral) and apply
        // the two coefficients independently.
        //
        // Wheel heading in BODY frame:
        // - Mains: fixed at body -Z (straight ahead), `steer_factor = 0`.
        // - Tailwheel: body -Z rotated by `yaw_input × max_angle × lock_factor`.
        //   Sign flipped so positive yaw input (nose-right per flowplan) drives
        //   the tailwheel to deflect body-left; the resulting lateral friction
        //   at the tail produces a positive-body-Y moment that translates to
        //   nose-right rotation under the "yaw > 0 → angular_velocity.y < 0"
        //   storage convention (see flight.rs docs around angular_velocity.y).

        // Aircraft ground speed (body-forward component). Used to gate
        // tailwheel castoring: below V_STEER_FULL the tailwheel is locked and
        // responds to yaw input; above V_CASTOR_FULL it free-pivots (no
        // steering authority, negligible lateral grip).
        let v_cg_body = v.orientation.inverse() * v.velocity;
        let ground_speed = (-v_cg_body.z).max(0.0); // body -Z is forward

        let lock_factor = if gear.steer_factor > 0.0 {
            // smoothstep from V_STEER_FULL (1.0) to V_CASTOR_FULL (0.0).
            let t = ((ground_speed - V_STEER_FULL_MPS) / (V_CASTOR_FULL_MPS - V_STEER_FULL_MPS))
                .clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t) // smoothstep
        } else {
            1.0
        };

        // Sign: positive yaw input (nose-right per flowplan) needs the
        // tailwheel to deflect so its rolling direction has a BODY +X
        // component (right). When aircraft moves forward, the tailwheel
        // slips right; lateral friction points body-left at the tail;
        // r × F about +Y produces the correct-signed yaw moment for
        // nose-right rotation. Determined by convention chase + unit test,
        // not a-priori (flight.rs stores yaw negated — the friction-from-
        // lateral-slip path handles that implicitly).
        let steer_angle_rad =
            v.controls.yaw * TAILWHEEL_MAX_STEER_RAD * gear.steer_factor * lock_factor;
        let wheel_forward_body = Quat::from_rotation_y(steer_angle_rad) * Vec3::NEG_Z;
        let wheel_forward_world_raw = v.orientation * wheel_forward_body;
        // Project onto the horizontal ground plane: tires roll flat, so the
        // friction axes must be horizontal regardless of aircraft pitch.
        // Without this projection a pitched aircraft (e.g., 3-point taildragger
        // attitude) would get a spurious vertical component on the brake /
        // rolling-friction force that erroneously compresses the gear and
        // corrupts the decel calculation.
        let wheel_forward_world =
            Vec3::new(wheel_forward_world_raw.x, 0.0, wheel_forward_world_raw.z)
                .normalize_or_zero();
        let wheel_lateral_world = wheel_forward_world.cross(Vec3::Y).normalize_or_zero();

        // Horizontal gear velocity and its wheel-frame components.
        let v_horizontal = Vec3::new(v_gear_world.x, 0.0, v_gear_world.z);
        let v_forward = v_horizontal.dot(wheel_forward_world);
        let v_lateral = v_horizontal.dot(wheel_lateral_world);

        // Friction forces, smoothed through zero by clamping (v / V_EPS) to
        // [-1, 1]. At |v| >= V_EPS the magnitude is the full µ·N; at v=0 it's
        // zero (no jitter). Continuous through zero avoids integrator
        // oscillation at a stopped aircraft.
        let f_forward_roll =
            -ROLLING_FRICTION_COEFF * normal_n * (v_forward / V_EPS).clamp(-1.0, 1.0);

        // Brake force: an additional -forward drag at each brakable gear when
        // the pilot is holding brakes. Same smoothing as rolling friction — no
        // kick at zero velocity. Mains are brake_factor=1.0; tailwheel is 0.0
        // so this term is gated on gear type naturally. Combined with rolling
        // gives total -forward axis friction per gear.
        let f_forward_brake = if v.controls.brakes && gear.brake_factor > 0.0 {
            -BRAKE_FRICTION_COEFF
                * normal_n
                * gear.brake_factor
                * (v_forward / V_EPS).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let f_forward = f_forward_roll + f_forward_brake;

        // Tailwheel lateral coefficient scales with lock_factor: locked tailwheel
        // grips laterally (enables steering); castoring tailwheel free-pivots
        // with negligible lateral grip. Mains always use the full coefficient.
        let lateral_coeff = if gear.steer_factor > 0.0 {
            LATERAL_FRICTION_COEFF * lock_factor
        } else {
            LATERAL_FRICTION_COEFF
        };
        let f_lateral = -lateral_coeff * normal_n * (v_lateral / V_EPS).clamp(-1.0, 1.0);

        let friction_world = f_forward * wheel_forward_world + f_lateral * wheel_lateral_world;

        // Both forces act at the gear contact point (same lever arm). Sum in
        // world frame, rotate to body frame for r × F moment.
        let gear_force_world = normal_force_world + friction_world;
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

    // Strike detection: same world-y < ground test as gear compression, but
    // applied to prop / tail / wingtip offsets. No force or moment is
    // generated — detection only. Computed regardless of whether any gear
    // is compressed (prop-strike can precede gear contact during a nose-
    // over; wingtip-scrape happens during ground roll).
    let mut strikes = StrikeFlags::default();
    for point in &BEAVER_STRIKES {
        let world_offset = v.orientation * point.body_offset;
        let world_y = v.position.y + world_offset.y;
        if world_y < ground_y {
            match point.kind {
                StrikeKind::Prop => strikes.prop = true,
                StrikeKind::Tail => strikes.tail = true,
                StrikeKind::WingtipLeft => strikes.wingtip_left = true,
                StrikeKind::WingtipRight => strikes.wingtip_right = true,
            }
        }
    }

    GroundContactOutputs {
        linear_accel,
        angular_accel,
        total_force_n: total_normal_magnitude,
        strikes,
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

    /// Wheels-configured aircraft at the stable 3-point rest equilibrium.
    ///
    /// Derivation (2.5b.5 calibration): the geometry constraint for 3-point
    /// contact (both gears at same world-y) forces pitch
    /// `θ = atan2(y_main - y_tail, z_tail - z_main) = atan2(0.6, 5.3) ≈ 6.46°`.
    /// At that pitch, main and tail compressions are equal. Balance of weight
    /// and moment at equal compression requires unequal spring rates — the
    /// tail spring is softer by the factor `k_tail/k_main = 0.428` (encoded
    /// in `BEAVER_GEAR[2].spring_factor`). Equal compression × softer tail
    /// spring → correct 82/18 weight distribution mains/tail.
    ///
    /// `CG_y = z_m·cos(θ) - main.y·sin(θ) - compression`
    /// `     = 1.3·cos(6.46°) - 0.8·sin(6.46°) - 0.0468`
    /// `     ≈ 1.155 m`.
    ///
    /// **Stability**: with equal compressions, both gears lose contact
    /// simultaneously on small upward perturbation — no metastable
    /// "tail-lifts-first" cascade that the pre-2.5b.5 equal-spring
    /// geometry produced.
    fn wheels_three_point_rest() -> VehicleState {
        let mut v = VehicleState::at_origin();
        // θ = atan2(0.6, 5.3) = 0.11279 rad = 6.462°.
        v.orientation = Quat::from_rotation_x(0.112_79);
        v.position = Vec3::new(0.0, 1.155, 0.0);
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
    fn lateral_drag_exceeds_forward_drag_at_same_slip() {
        // 2.5b.3: tires are anisotropic — they roll forward (low µ_r = 0.02) and
        // grip laterally (high µ_lat = 0.6, ratio 30×). Compare forward-motion
        // decel vs pure-sideways-motion decel at the same speed; lateral
        // deceleration should be substantially larger.
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

        let fwd_mag = fwd.linear_accel.z.abs();
        let side_mag = side.linear_accel.x.abs();
        assert!(
            side_mag > fwd_mag * 10.0,
            "lateral drag should dominate forward drag (µ_lat/µ_r ≈ 30); \
             fwd_mag={fwd_mag} side_mag={side_mag}",
        );
    }

    // ── Phase 2.5b.3 tests: tailwheel steering ─────────────────────────────

    #[test]
    fn yaw_input_at_taxi_produces_nose_right_moment() {
        // Positive yaw input (nose-right per flowplan convention) + low
        // ground speed → tailwheel deflects, lateral friction at the tail
        // generates a body-Y moment stored as NEGATIVE angular_accel.y
        // (flowplan convention: yaw-right → angular_velocity.y < 0; see
        // flight.rs docs near line 753).
        //
        // Uses the 3-point rest attitude (tailwheel touching) with
        // world-frame horizontal velocity (no rotating through pitch — we
        // want the aircraft rolling on ground at constant y).
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.velocity = Vec3::new(0.0, 0.0, -3.0); // world-frame: 3 m/s forward, no rise
        v.controls.yaw = 1.0;
        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            out.angular_accel.y < -0.01,
            "yaw-right input at taxi should produce nose-right moment \
             (angular_accel.y < 0 per storage convention); got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn yaw_input_reverses_with_opposite_direction() {
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.velocity = Vec3::new(0.0, 0.0, -3.0);
        v.controls.yaw = -1.0;
        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            out.angular_accel.y > 0.01,
            "yaw-left input at taxi should produce nose-left moment \
             (angular_accel.y > 0); got {}",
            out.angular_accel.y,
        );
    }

    #[test]
    fn castoring_reduces_tailwheel_authority_at_speed() {
        // Tailwheel locks at low ground speed and castors above V_CASTOR_FULL.
        // Same yaw input should produce a larger yaw moment at taxi speed
        // than at castor speed (lock_factor smoothsteps 1 → 0 between 7.7
        // and 10 m/s). Uses 3-point attitude + world-frame velocity.
        let spec = beaver_spec();

        let mut v_taxi = wheels_three_point_rest();
        v_taxi.velocity = Vec3::new(0.0, 0.0, -3.0);
        v_taxi.controls.yaw = 1.0;
        let taxi = compute_ground_contact(&v_taxi, Some(&spec), 0.0);

        let mut v_fast = wheels_three_point_rest();
        v_fast.velocity = Vec3::new(0.0, 0.0, -15.0);
        v_fast.controls.yaw = 1.0;
        let fast = compute_ground_contact(&v_fast, Some(&spec), 0.0);

        assert!(
            taxi.angular_accel.y.abs() > fast.angular_accel.y.abs() * 2.0,
            "taxi tailwheel should produce >2× castor-speed moment \
             (lock_factor scales µ_lat); taxi={}, fast={}",
            taxi.angular_accel.y,
            fast.angular_accel.y,
        );
    }

    // ── Phase 2.5b.6 tests: strike detection ─────────────────────────

    #[test]
    fn no_strikes_at_rest() {
        let spec = beaver_spec();
        let v = wheels_three_point_rest();
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert_eq!(out.strikes, StrikeFlags::default());
        assert!(!out.strikes.any());
    }

    #[test]
    fn floats_config_produces_no_strikes() {
        // Strike detection is wheels-only. Floats-variant aircraft should
        // never raise strike flags from this function (pontoon-specific
        // strike geometry is a future concern).
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.config = VehicleConfig::Floats;
        // Put it in a pose that would strike if wheels config: over-rotated.
        v.orientation = Quat::from_rotation_x(30.0_f32.to_radians());
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert_eq!(out.strikes, StrikeFlags::default());
    }

    #[test]
    fn over_rotation_produces_tail_strike() {
        // Over-rotate on takeoff: tail should dip below ground. Tail body
        // offset (0, -0.8, 5.5). At pitch +θ (nose-up), tail world_y =
        // CG_y + rotated offset. For pitch+15° about +X:
        // y_rot = -0.8·cos(15°) - 5.5·sin(15°) = -0.773 - 1.424 = -2.197
        // At CG=1.155, tail world y = 1.155 - 2.197 = -1.042 → strike.
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.orientation = Quat::from_rotation_x(15.0_f32.to_radians());
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert!(
            out.strikes.tail,
            "over-rotated aircraft should raise tail strike; flags={:?}",
            out.strikes,
        );
        assert!(!out.strikes.prop, "prop should not strike on nose-up pitch");
        assert!(
            !out.strikes.wingtip_left && !out.strikes.wingtip_right,
            "wingtips should not strike on pure pitch",
        );
    }

    #[test]
    fn nose_down_produces_prop_strike() {
        // Strong nose-down pitch pushes prop disk below ground. Prop body
        // offset (0, -1.2, -3.0). At pitch -15° (nose-down):
        // y_rot = -1.2·cos(15°) - (-3.0)·sin(15°) = -1.159 + 0.776 = -0.383
        //         wait need to check sign. R_x(θ) applied:
        //         y' = y·cos - z·sin = -1.2·cos(-15°) - (-3.0)·sin(-15°)
        //            = -1.2·0.966 - (-3.0)·(-0.259) = -1.159 - 0.776 = -1.935
        // At CG=1.155, prop world y = 1.155 - 1.935 = -0.78 → strike.
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.orientation = Quat::from_rotation_x(-15.0_f32.to_radians());
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert!(
            out.strikes.prop,
            "nose-down pitch should raise prop strike; flags={:?}",
            out.strikes,
        );
    }

    #[test]
    fn right_bank_produces_right_wingtip_strike() {
        // Right-bank (right wing down) per flowplan convention:
        // angular_velocity.z is STORED NEGATED from physics convention, so
        // a body-frame rotation about +Z by positive angle actually rotates
        // the top of the aircraft toward body +X (right wing UP = LEFT bank).
        // To put the aircraft into right-bank attitude directly via a quat,
        // rotate about +Z by NEGATIVE angle (equivalent to +Z by negative
        // angle OR -Z by positive angle). Right wingtip at (7.3, -1.0, 0)
        // rotates DOWN; world_y drops past zero.
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.orientation *= Quat::from_rotation_z(-10.0_f32.to_radians());
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert!(
            out.strikes.wingtip_right,
            "right-banked aircraft should raise right wingtip strike; flags={:?}",
            out.strikes,
        );
        assert!(
            !out.strikes.wingtip_left,
            "left wingtip shouldn't strike on right bank; flags={:?}",
            out.strikes,
        );
    }

    #[test]
    fn left_bank_produces_left_wingtip_strike() {
        // Mirror of the right-bank test. Left-bank = positive Z rotation
        // in body frame (top tilts right toward +X = left wing drops).
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.orientation *= Quat::from_rotation_z(10.0_f32.to_radians());
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert!(
            out.strikes.wingtip_left,
            "left-banked aircraft should raise left wingtip strike; flags={:?}",
            out.strikes,
        );
        assert!(!out.strikes.wingtip_right);
    }

    #[test]
    fn airborne_aircraft_has_no_strikes() {
        // High-altitude aircraft at ANY attitude shouldn't register strikes.
        let spec = beaver_spec();
        let mut v = wheels_three_point_rest();
        v.position.y = 50.0; // airborne
        v.orientation = Quat::from_rotation_x(20.0_f32.to_radians());
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        assert_eq!(out.strikes, StrikeFlags::default());
    }

    // ── Phase 2.5b.5: at-rest invariant ──────────────────────────────

    #[test]
    fn wheels_pitch_converges_to_rest_from_perturbation() {
        // Spawn aircraft at +10° nose-up (above the 6.46° rest pitch) with zero
        // velocity, zero inputs, zero fuel. Gear + gravity should rotate it
        // back toward the rest attitude. Assert pitch is within 1° of the
        // 6.46° equilibrium within 5 s, with no overshoot below 5°. Validates
        // the "pitch attitude emerges from multi-point normal-force
        // distribution" claim in the 2.5b.6 ROADMAP.
        use crate::difficulty::DifficultyDials;
        use crate::input::InputEvents;
        use crate::state::{SimState, FIXED_DT};
        let spec = beaver_spec();
        let mut sim = SimState::new(0xC05A);
        sim.aircraft = Some(spec);
        sim.vehicle = wheels_three_point_rest();
        sim.vehicle.fuel_gal = 0.0; // isolate gear dynamics from thrust
        // Perturb: +10° pitch instead of the 6.46° rest attitude.
        sim.vehicle.orientation = Quat::from_rotation_x(10.0_f32.to_radians());
        sim.vehicle.position.y = 1.25; // slightly higher to avoid initial
                                       // over-compression from steep pitch

        let d = DifficultyDials::default();
        let batch = InputEvents::empty();
        let mut min_pitch_deg = f32::INFINITY;
        // 5 s = 600 ticks at 120 Hz.
        for _ in 0..600 {
            sim.tick(&batch, &d, FIXED_DT, 0.0);
            let (_, pitch, _) = sim.vehicle.orientation.to_euler(glam::EulerRot::YXZ);
            min_pitch_deg = min_pitch_deg.min(pitch.to_degrees());
        }
        let (_, final_pitch, _) = sim.vehicle.orientation.to_euler(glam::EulerRot::YXZ);
        let final_deg = final_pitch.to_degrees();

        // Rest pitch is 6.46°. Acceptance: within 1° of rest at end; no overshoot
        // below 5° (no bounce through the equilibrium by >1.5°).
        assert!(
            (final_deg - 6.46).abs() < 1.5,
            "pitch should converge to rest 6.46°; final={final_deg:+.2}°",
        );
        assert!(
            min_pitch_deg > 4.5,
            "pitch shouldn't overshoot >2° past rest; min={min_pitch_deg:+.2}°",
        );
    }

    #[test]
    fn wheels_at_rest_has_zero_drift() {
        // A wheels aircraft spawned at its declared 3-point rest state with
        // zero velocity, zero inputs, and zero fuel (so engine produces no
        // thrust — isolates the gear / buoyancy / aero-gravity loop from
        // powerplant creep) must stay put. Before 2.5b.5 calibration, the
        // equal-compression pitch wasn't a moment equilibrium AND buoyancy
        // ran on wheels aircraft (phantom pontoons), producing divergent
        // drift within 0.5 s. Guards gear equilibrium + buoyancy config
        // short-circuit + damping asymmetry against future regression.
        //
        // A real Beaver at idle would creep forward slightly (idle thrust
        // 250 N exceeds static-rolling-friction breakout on paved); that's
        // realistic behavior and tested elsewhere via the brake scenarios.
        use crate::difficulty::DifficultyDials;
        use crate::input::InputEvents;
        use crate::state::{SimState, FIXED_DT};
        let spec = beaver_spec();
        let mut sim = SimState::new(0xAB1E);
        sim.aircraft = Some(spec);
        sim.vehicle = wheels_three_point_rest();
        sim.vehicle.fuel_gal = 0.0;

        let (_, pitch0, _) = sim.vehicle.orientation.to_euler(glam::EulerRot::YXZ);
        let pos0 = sim.vehicle.position;

        let d = DifficultyDials::default();
        let batch = InputEvents::empty();
        // 240 ticks = 2.0 s at FIXED_DT=1/120.
        for _ in 0..240 {
            sim.tick(&batch, &d, FIXED_DT, 0.0);
        }

        let (_, pitch_final, _) = sim.vehicle.orientation.to_euler(glam::EulerRot::YXZ);
        let pitch_drift_deg = (pitch_final - pitch0).to_degrees().abs();
        let pos_drift = (sim.vehicle.position - pos0).length();

        // Thresholds are loose enough that small damped-oscillation settle
        // still passes, tight enough that the pre-2.5b.5 drift (0.69° in 0.5 s
        // → >2° in 2 s + 5 cm position rise) is detected.
        assert!(
            pitch_drift_deg < 0.5,
            "wheels-at-rest aircraft drifted {pitch_drift_deg:.3}° in pitch over \
             2 s (limit 0.5°). Check gear geometry equilibrium + damping asymmetry.",
        );
        assert!(
            pos_drift < 0.05,
            "wheels-at-rest aircraft moved {pos_drift:.3} m in 2 s (limit 5 cm). \
             Check moment balance at the declared rest state.",
        );
    }

    // ── Phase 2.5b.4 tests: brakes ───────────────────────────────────

    #[test]
    fn brakes_off_produces_rolling_friction_only() {
        // Baseline: brakes=false gives pure rolling friction. Used as the
        // reference for brakes_on tests.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v = wheels_vehicle_at(cg_y);
        v.velocity = Vec3::new(0.0, 0.0, -5.0);
        assert!(!v.controls.brakes, "default brakes should be off");
        let out = compute_ground_contact(&v, Some(&spec), 0.0);
        // Rolling-only decel: ~0.2 m/s² from `rolling_friction_opposes_forward_motion`.
        assert!(
            out.linear_accel.z > 0.05 && out.linear_accel.z < 0.5,
            "brakes-off should give pure rolling decel ~0.2 m/s², got {}",
            out.linear_accel.z,
        );
    }

    #[test]
    fn brakes_on_multiplies_deceleration() {
        // Brakes applied: each main adds -µ_brake·N forward drag on top of
        // rolling. Combined coefficient at the mains = µ_r + µ_brake = 0.62
        // (vs 0.02 rolling alone). Expect ~31× larger decel magnitude.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v_off = wheels_vehicle_at(cg_y);
        v_off.velocity = Vec3::new(0.0, 0.0, -5.0);
        let off = compute_ground_contact(&v_off, Some(&spec), 0.0);

        let mut v_on = wheels_vehicle_at(cg_y);
        v_on.velocity = Vec3::new(0.0, 0.0, -5.0);
        v_on.controls.brakes = true;
        let on = compute_ground_contact(&v_on, Some(&spec), 0.0);

        assert!(
            on.linear_accel.z > off.linear_accel.z * 10.0,
            "brakes-on should produce >10× rolling-alone decel; \
             off.z={}, on.z={}",
            off.linear_accel.z,
            on.linear_accel.z,
        );
        // Sanity: decel magnitude is in the 3-5 m/s² range for this setup.
        assert!(
            (3.0..=6.0).contains(&on.linear_accel.z),
            "brakes decel should be ~0.4 g (3-6 m/s²); got {}",
            on.linear_accel.z,
        );
    }

    #[test]
    fn brakes_at_zero_velocity_produce_no_force() {
        // Brakes held but aircraft not moving → no friction. Brake is a
        // kinetic drag (opposes motion), it can't accelerate a stopped
        // vehicle backward. Verified by the V_EPS ramp through zero.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v = wheels_vehicle_at(cg_y);
        v.controls.brakes = true;
        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            out.linear_accel.x.abs() < 1e-3 && out.linear_accel.z.abs() < 1e-3,
            "brakes at v=0 should produce no horizontal force; got ({}, _, {})",
            out.linear_accel.x,
            out.linear_accel.z,
        );
    }

    #[test]
    fn straight_rolling_produces_negligible_lateral_force() {
        // Pure body-forward motion with no yaw input → v_lateral in wheel
        // frame is ~0 at every gear → lateral friction is ~0. Use identity
        // orientation (mains-only) since we only need to verify lateral ≈ 0.
        let spec = beaver_spec();
        let weight_n = spec.max_mass_wheels_kg * GRAVITY.length();
        let main_compression = weight_n / (2.0 * GEAR_SPRING_N_PER_M);
        let cg_y = 1.3 - main_compression;

        let mut v = wheels_vehicle_at(cg_y);
        v.velocity = Vec3::new(0.0, 0.0, -5.0);
        let out = compute_ground_contact(&v, Some(&spec), 0.0);

        assert!(
            out.linear_accel.x.abs() < 0.05,
            "straight rolling should have near-zero lateral accel, got {}",
            out.linear_accel.x,
        );
        assert!(
            out.linear_accel.z > 0.1,
            "straight rolling should have forward drag (+Z accel) opposing \
             -Z motion, got {}",
            out.linear_accel.z,
        );
    }
}
