//! Orientation widget: unit-vector direction display in the bottom-right corner.
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
}
