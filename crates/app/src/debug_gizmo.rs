//! Debug widgets: orientation gizmo (bottom-right) + attitude indicator / glide-slope
//! (bottom-left).
//!
//! ## Orientation widget (`build`): unit-vector direction display.
//!
//! Phase 2.4.2c (first visual consumer of the telemetry plumbing). See
//! [`FEATURES.md § 3D Debug Gizmo`](../../../../FEATURES.md#3d-debug-gizmo-phase-242c-first-visual-consumer)
//! for the design and scaling rationale.
//!
//! All vectors are drawn at unit length so directions read cleanly at a glance; the
//! magnitudes are shown numerically in the top-right dynamics HUD. The widget is a
//! screen-space overlay rendered via a dedicated ortho projection (see
//! `flowplan_render::renderer::widget_view_proj`) -- the builder itself emits positions
//! in widget-local `[-1, 1]³` space.
//!
//! This module is pure CPU. Determinism-safe: no sim-side state, no state-hash
//! contribution, always on.

#![deny(unsafe_code)]

use flowplan_render::{DebugLineBatch, Telemetry};
use glam::{Quat, Vec3};

// Body-axis colors: Blender-style RGB convention so the gizmo reads at a glance.
const COLOR_X: [f32; 3] = [1.0, 0.25, 0.25]; // +X right wing = red
const COLOR_Y: [f32; 3] = [0.25, 1.0, 0.25]; // +Y up = green
const COLOR_Z: [f32; 3] = [0.35, 0.55, 1.0]; // -Z nose (forward) = blue
const COLOR_VELOCITY: [f32; 3] = [1.0, 1.0, 1.0];
const COLOR_THRUST: [f32; 3] = [1.0, 0.95, 0.25];
const COLOR_LIFT: [f32; 3] = [0.30, 0.95, 1.0];
const COLOR_DRAG: [f32; 3] = [1.0, 0.55, 0.15];
const COLOR_GRAVITY: [f32; 3] = [0.55, 0.55, 0.55];
const COLOR_TOTAL: [f32; 3] = [1.0, 0.30, 1.0];

/// Length of each vector in widget-local space. The widget cube spans `[-1, 1]`; axes
/// and unit-length force vectors end at ±`ARROW_LEN` so there's a small margin inside
/// the cube for readability.
const ARROW_LEN: f32 = 0.9;

/// Widget display rotation -- rotates the full vector set by a fixed "isometric" angle
/// before widening happens in widget-local XY. Without this, any vector aligned with
/// world Z (e.g., the nose axis at level flight) projects to zero XY and gets widened
/// to an invisible strip. The tilt guarantees all three body axes + all world-frame
/// force vectors have a visible screen-plane projection at every aircraft attitude.
///
/// Pitch -25° + yaw +30° picks a standard front-elevated view. Swap if a different
/// angle reads better.
const WIDGET_PITCH_RAD: f32 = -0.45; // ~25.8°
const WIDGET_YAW_RAD: f32 = 0.55; // ~31.5°

fn widget_display_rot() -> Quat {
    Quat::from_rotation_x(WIDGET_PITCH_RAD) * Quat::from_rotation_y(WIDGET_YAW_RAD)
}

/// Fill `batch` with the orientation widget for the given telemetry snapshot.
///
/// All vectors are unit-length in widget-local `[-1, 1]³` space. The renderer's widget
/// ortho projection maps that cube into a fixed screen-space rectangle. Directions
/// reflect the aircraft's world-frame velocity / forces, rotated INTO the widget --
/// not body-frame; the widget is effectively a world-orientation compass that moves
/// with the plane.
///
/// The batch is cleared before populating. Called once per frame; the widget is always
/// drawn (no toggle in Phase 2.4.2c).
pub fn build(batch: &mut DebugLineBatch, t: &Telemetry) {
    batch.clear();

    let origin = Vec3::ZERO;
    // Compose the widget display rotation with the aircraft orientation so body axes
    // reflect both the aircraft's attitude and the fixed isometric tilt of the widget.
    let display = widget_display_rot();
    let body_rot = display * t.orientation;

    // Body axes: +X (right wing), +Y (up), -Z (nose forward).
    batch.push_vector(origin, body_rot * (Vec3::X * ARROW_LEN), COLOR_X);
    batch.push_vector(origin, body_rot * (Vec3::Y * ARROW_LEN), COLOR_Y);
    batch.push_vector(origin, body_rot * (Vec3::NEG_Z * ARROW_LEN), COLOR_Z);

    // Velocity direction (unit-length). World-frame vectors only get the display
    // rotation applied (no aircraft orientation -- those are already world-space).
    if let Some(v_hat) = try_normalize(t.velocity) {
        batch.push_vector(origin, display * v_hat * ARROW_LEN, COLOR_VELOCITY);
    }

    // Force directions (unit-length each). Gravity included -- the widget is a
    // direction compass, so seeing gravity's unit vector helps judge orientation.
    if let Some(dir) = try_normalize(t.thrust) {
        batch.push_vector(origin, display * dir * ARROW_LEN, COLOR_THRUST);
    }
    if let Some(dir) = try_normalize(t.lift) {
        batch.push_vector(origin, display * dir * ARROW_LEN, COLOR_LIFT);
    }
    if let Some(dir) = try_normalize(t.drag) {
        batch.push_vector(origin, display * dir * ARROW_LEN, COLOR_DRAG);
    }
    if let Some(dir) = try_normalize(t.gravity) {
        batch.push_vector(origin, display * dir * ARROW_LEN, COLOR_GRAVITY);
    }
    if let Some(dir) = try_normalize(t.linear_accel) {
        batch.push_vector(origin, display * dir * ARROW_LEN, COLOR_TOTAL);
    }
}

fn try_normalize(v: Vec3) -> Option<Vec3> {
    if v.length_squared() > 1e-6 {
        Some(v.normalize())
    } else {
        None
    }
}

// ====================================================================================
// Attitude indicator / glide-slope widget (Phase 2.4.4e)
// ====================================================================================

// Colors: picked to match AI / HUD conventions while staying within the orientation
// widget's palette family. All rendered by the same `TriangleList` pipeline with the
// cel-style dark outline beneath each line.
const COLOR_HORIZON: [f32; 3] = [1.0, 1.0, 1.0];
const COLOR_AIRCRAFT: [f32; 3] = [1.0, 0.9, 0.2]; // yellow, traditional AI aircraft symbol
const COLOR_FPV: [f32; 3] = [0.3, 1.0, 0.3]; // green, flight-path marker
const COLOR_LADDER: [f32; 3] = [0.6, 0.6, 0.6]; // grey, pitch-ladder ticks

/// Widget-local units per radian of pitch / α / β. Chosen so ±30° of pitch
/// fills roughly the visible widget area (`0.9 units ≈ 30° × SCALE`).
const ATTITUDE_SCALE: f32 = 1.72;
/// Horizon line half-length in widget-local units. Extends beyond the visible
/// `[-1, 1]` cube so the line still covers the widget when rotated at extreme bank
/// angles; the renderer applies a scissor to clip overflow.
const HORIZON_HALF_LEN: f32 = 1.6;
/// Length of each aircraft-symbol wing.
const WING_OUTER: f32 = 0.22;
const WING_INNER: f32 = 0.06;
/// Half-size of the flight-path-vector crosshair.
const FPV_HALF: f32 = 0.05;
/// Pitch-ladder tick lengths (widget-local units).
const LADDER_TICK_MAJOR: f32 = 0.12;
const LADDER_TICK_MINOR: f32 = 0.06;

/// Fill `batch` with the attitude indicator + flight-path-vector widget for the
/// given telemetry. Widget-local coordinates in `[-1, 1]²`; everything outside that
/// box is scissor-clipped by the renderer.
///
/// Layout:
/// - Horizon line rotates with bank and translates with pitch.
/// - Pitch ladder: short ticks at ±5° (minor), ±10°, ±20° (major).
/// - Aircraft symbol: yellow wings + short fuselage, fixed in display.
/// - Flight-path vector: green crosshair offset from aircraft symbol by (β, -α) × scale.
///
/// Determinism: pure function of `Telemetry`. Never touches sim state.
pub fn build_attitude(batch: &mut DebugLineBatch, t: &Telemetry) {
    batch.clear();

    let pitch = t.pitch_rad;
    let bank = t.bank_rad;

    // --- Horizon + pitch ladder geometry ---
    // "up-in-display" after bank: world-up projected into the aircraft body's XY plane.
    // At bank=0 this is (0, 1); at bank=+90° (roll left) it's (1, 0).
    let (sin_b, cos_b) = bank.sin_cos();
    let up_display = Vec3::new(sin_b, cos_b, 0.0);
    // "horizon right" = 90° clockwise from up-in-display.
    let horizon_dir = Vec3::new(cos_b, -sin_b, 0.0);

    // Horizon center: moves in the ground direction (-up_display) proportional to pitch.
    let horizon_center = -up_display * (pitch * ATTITUDE_SCALE);

    // Horizon line itself.
    let h_start = horizon_center - horizon_dir * HORIZON_HALF_LEN;
    let h_end = horizon_center + horizon_dir * HORIZON_HALF_LEN;
    batch.push_segment(h_start, h_end, COLOR_HORIZON);

    // Pitch-ladder ticks. Major ticks at ±10, ±20; minor at ±5. Skip 0° (that's the
    // horizon itself).
    for (deg, is_major) in [
        (-20.0_f32, true),
        (-10.0, true),
        (-5.0, false),
        (5.0, false),
        (10.0, true),
        (20.0, true),
    ] {
        let tick_offset_rad = deg.to_radians();
        // Displacement from horizon_center along +up_display by (tick_offset * scale).
        let tick_center = horizon_center + up_display * (tick_offset_rad * ATTITUDE_SCALE);
        // Tick runs parallel to horizon, half the length either side.
        let half = if is_major {
            LADDER_TICK_MAJOR
        } else {
            LADDER_TICK_MINOR
        } * 0.5;
        let t_start = tick_center - horizon_dir * half;
        let t_end = tick_center + horizon_dir * half;
        batch.push_segment(t_start, t_end, COLOR_LADDER);
    }

    // --- Aircraft symbol (fixed in display) ---
    // Two wings + short fuselage nub. Widget-local X is display-right, Y is display-up.
    batch.push_segment(
        Vec3::new(-WING_OUTER, 0.0, 0.0),
        Vec3::new(-WING_INNER, 0.0, 0.0),
        COLOR_AIRCRAFT,
    );
    batch.push_segment(
        Vec3::new(WING_INNER, 0.0, 0.0),
        Vec3::new(WING_OUTER, 0.0, 0.0),
        COLOR_AIRCRAFT,
    );
    // Small inverted-V tail hint for orientation cue.
    batch.push_segment(
        Vec3::new(-WING_INNER, 0.0, 0.0),
        Vec3::new(0.0, 0.04, 0.0),
        COLOR_AIRCRAFT,
    );
    batch.push_segment(
        Vec3::new(WING_INNER, 0.0, 0.0),
        Vec3::new(0.0, 0.04, 0.0),
        COLOR_AIRCRAFT,
    );

    // --- Flight-path vector ---
    // Position in display: (β, -α) × scale. Always drawn, even when α and β are near
    // zero (marker overlaps aircraft symbol at level cruise, which is the correct
    // read -- we are going where we're pointed).
    let fpv_x = t.beta_rad * ATTITUDE_SCALE;
    let fpv_y = -t.alpha_rad * ATTITUDE_SCALE;
    let fpv_center = Vec3::new(fpv_x, fpv_y, 0.0);
    // Cross shape.
    batch.push_segment(
        fpv_center - Vec3::new(FPV_HALF, 0.0, 0.0),
        fpv_center + Vec3::new(FPV_HALF, 0.0, 0.0),
        COLOR_FPV,
    );
    batch.push_segment(
        fpv_center - Vec3::new(0.0, FPV_HALF, 0.0),
        fpv_center + Vec3::new(0.0, FPV_HALF, 0.0),
        COLOR_FPV,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowplan_render::Telemetry;
    use glam::Quat;

    fn level_cruise_telemetry() -> Telemetry {
        Telemetry {
            orientation: Quat::IDENTITY,
            position: Vec3::new(10.0, 50.0, 20.0),
            velocity: Vec3::new(0.0, 0.0, -50.0),
            thrust: Vec3::new(0.0, 0.0, -2.0),
            lift: Vec3::new(0.0, 9.81, 0.0),
            drag: Vec3::new(0.0, 0.0, 0.7),
            gravity: Vec3::new(0.0, -9.81, 0.0),
            linear_accel: Vec3::new(0.0, 0.0, -1.3),
            ..Telemetry::default()
        }
    }

    #[test]
    fn build_populates_expected_segment_count() {
        let mut batch = DebugLineBatch::new();
        build(&mut batch, &level_cruise_telemetry());
        // 3 body axes + velocity + thrust + lift + drag + gravity + total = 9 segments.
        // Each segment emits 12 verts (6 outline + 6 fill).
        assert_eq!(batch.len_vertices(), 9 * 12);
    }

    #[test]
    fn build_clears_previous_batch() {
        let mut batch = DebugLineBatch::new();
        batch.push_segment(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), [1.0, 1.0, 1.0]);
        build(&mut batch, &level_cruise_telemetry());
        assert_eq!(batch.len_vertices(), 9 * 12);
    }

    #[test]
    fn build_skips_zero_magnitude_vectors() {
        let t = Telemetry {
            orientation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            thrust: Vec3::ZERO,
            drag: Vec3::ZERO,
            // lift / gravity / linear_accel stay at Telemetry::default() = zero
            ..Telemetry::default()
        };
        let mut batch = DebugLineBatch::new();
        build(&mut batch, &t);
        // 3 body axes only.
        assert_eq!(batch.len_vertices(), 3 * 12);
    }

    #[test]
    fn all_three_body_axes_render_at_any_attitude() {
        // The widget display rotation guarantees all three axes have a visible XY
        // projection even at extreme attitudes (nose straight up/down, plane on its
        // back). Sample several attitudes and verify all three axes emit 12 verts each.
        for (name, orientation) in [
            ("level", Quat::IDENTITY),
            (
                "nose-up",
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            ),
            (
                "nose-down",
                Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            ),
            ("inverted", Quat::from_rotation_z(std::f32::consts::PI)),
            (
                "90° bank",
                Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            ),
        ] {
            let t = Telemetry {
                orientation,
                ..Telemetry::default()
            };
            let mut batch = DebugLineBatch::new();
            build(&mut batch, &t);
            assert_eq!(
                batch.len_vertices(),
                3 * 12,
                "attitude {name}: expected 3 body-axis quads, got {} verts",
                batch.len_vertices(),
            );
        }
    }

    // ---- Attitude-indicator builder ----

    #[test]
    fn attitude_emits_expected_segments() {
        let mut batch = DebugLineBatch::new();
        build_attitude(&mut batch, &Telemetry::default());
        // 1 horizon + 6 ladder ticks + 4 aircraft segs + 2 FPV segs = 13 segments
        // × 12 verts per segment = 156 verts.
        assert_eq!(batch.len_vertices(), 13 * 12);
    }

    #[test]
    fn attitude_horizon_tilts_with_bank() {
        // At zero bank the horizon line is horizontal (delta y between endpoints ≈ 0).
        // At +30° bank (roll left) the horizon should tilt such that the right side
        // is below the left (since pilot's head tilts left, world tilts clockwise).
        let mut batch = DebugLineBatch::new();
        build_attitude(
            &mut batch,
            &Telemetry {
                bank_rad: 30.0_f32.to_radians(),
                ..Telemetry::default()
            },
        );
        // Horizon is the first segment pushed. Its outline quad starts at verts 0..6
        // with a_left at index 0 and b_left at index 2. The horizon end at +X direction
        // corresponds to b; at bank=+30° (left roll), b_left.y should be negative and
        // a_left.y should be positive.
        let a_left_y = batch.outline_vertices[0].position[1];
        let b_left_y = batch.outline_vertices[2].position[1];
        assert!(
            a_left_y > 0.01 && b_left_y < -0.01,
            "horizon should tilt CW at left bank; a_left.y={a_left_y} b_left.y={b_left_y}",
        );
    }

    #[test]
    fn attitude_horizon_drops_at_pitch_up() {
        // At pitch up 20°, bank 0, the horizon center should be below widget origin.
        let mut batch = DebugLineBatch::new();
        build_attitude(
            &mut batch,
            &Telemetry {
                pitch_rad: 20.0_f32.to_radians(),
                ..Telemetry::default()
            },
        );
        // Sample a center-ish vertex of the horizon (vertex 0 and 1 are a_left and
        // a_right around the start of the horizon quad; y of either reflects horizon
        // baseline at pitch=20°). Expected: y = -0.20 × 1.72 = -0.344.
        let y_a = batch.outline_vertices[0].position[1];
        assert!(
            y_a < -0.3,
            "horizon baseline should drop at pitch-up; got {y_a}",
        );
    }

    #[test]
    fn attitude_fpv_tracks_alpha() {
        // With α=+10°, FPV should sit below aircraft symbol by 10° × ATTITUDE_SCALE.
        let mut batch = DebugLineBatch::new();
        build_attitude(
            &mut batch,
            &Telemetry {
                alpha_rad: 10.0_f32.to_radians(),
                ..Telemetry::default()
            },
        );
        // FPV is the last two segments (cross horizontal + cross vertical). Each
        // segment = 6 outline + 6 fill. Horizontal cross fill_vertices at the end.
        // Grab the fill vertices of the last segment's center (vertical cross).
        let n = batch.fill_vertices.len();
        let vert_cross_center_y = f32::midpoint(
            batch.fill_vertices[n - 6].position[1],
            batch.fill_vertices[n - 1].position[1],
        );
        let expected = -10.0_f32.to_radians() * ATTITUDE_SCALE;
        assert!(
            (vert_cross_center_y - expected).abs() < 0.05,
            "FPV vertical center y should be ~{expected}; got {vert_cross_center_y}",
        );
    }
}
