//! Semantic attitude helpers.
//!
//! Wraps `Quat::from_rotation_{x,y,z}` with names that match how a pilot (or
//! scenario author) reads attitudes: "15° nose-up", "10° right-bank", "5°
//! yaw-right." Eliminates the sign-convention footgun where `from_rotation_z`
//! with a positive angle produces LEFT bank because glam follows the physics
//! right-hand rule (positive rotation about +Z takes +X toward +Y, which
//! rotates the right wing UP).
//!
//! ## flowplan angular convention
//!
//! Body axes (flowplan): **+X = right wing, +Y = up, -Z = forward**. Aircraft
//! faces world -Z at identity orientation.
//!
//! Physics conventions (what moment calculations produce):
//! - Pitch moment positive → nose up (rotation about body +X axis).
//! - Yaw moment positive → nose right (rotation about body +Y axis by
//!   right-hand rule: +Z toward +X, which is... forward toward right,
//!   i.e. nose-right).
//! - Roll moment positive → right-wing down (rotation about body -Z axis...
//!   see note below).
//!
//! Glam's `Quat::from_rotation_{x,y,z}` always uses the standard
//! right-hand-rule mathematical convention. For yaw and roll, that produces
//! the *opposite* rotation from what the flowplan convention expects as
//! "positive":
//! - `from_rotation_y(+angle)` rotates +Z → +X → -Z → -X (looking from +Y
//!   down). Applied to body forward (-Z), it rotates -Z → -X, which is nose
//!   turning LEFT in body frame. So mathematical positive-Y-rotation = nose-
//!   LEFT = yaw-LEFT.
//! - `from_rotation_z(+angle)` rotates +X → +Y → -X → -Y. Applied to body
//!   right (+X), it goes +X → +Y (right wing rotates UP). So mathematical
//!   positive-Z-rotation = LEFT bank (right wing up).
//!
//! Because of this, `flight.rs` stores yaw and roll NEGATED in
//! `angular_velocity` relative to the positive-moment convention. For
//! ORIENTATION quats (representing actual attitudes, not rates), the helpers
//! below handle the sign so authors don't have to remember.
//!
//! ## Usage
//!
//! ```no_run
//! use flowplan_sim::attitude;
//! // 10° right bank. Semantically clear; the sign-flip on from_rotation_z is
//! // hidden inside the helper.
//! let bank = attitude::roll_right(10_f32.to_radians());
//! // Combined: 3-point rest pitch plus right-bank.
//! let attitude = attitude::pitch_up(0.113) * attitude::roll_right(0.175);
//! ```
//!
//! ## Radian inputs by convention
//!
//! All helpers take radians. If you want degrees, compose with
//! `f32::to_radians`. Rationale: most call sites are const contexts or
//! adjacent to other radian-valued scenario state; staying uniform avoids
//! the "which units?" reading cost. Helpers are `#[must_use]` + `const fn`
//! where glam permits (`from_rotation_*` is not currently const, so the
//! helpers pass through as plain functions).

use glam::Quat;

/// Quat representing nose-up pitch by `angle` radians (rotation about body
/// +X axis). Positive angle = nose up; no sign flip needed because
/// `from_rotation_x` matches the flowplan convention directly.
#[must_use]
pub fn pitch_up(angle_rad: f32) -> Quat {
    Quat::from_rotation_x(angle_rad)
}

/// Quat representing nose-down pitch by `angle` radians. Thin inverse helper —
/// equivalent to `pitch_up(-angle_rad)` but reads naturally at call sites.
#[must_use]
pub fn pitch_down(angle_rad: f32) -> Quat {
    Quat::from_rotation_x(-angle_rad)
}

/// Quat representing right-bank (right wing down) by `angle` radians.
///
/// Wraps `Quat::from_rotation_z(-angle)` — the negation is the flowplan
/// sign convention (see module docstring). Positive angle = right-wing-down.
#[must_use]
pub fn roll_right(angle_rad: f32) -> Quat {
    Quat::from_rotation_z(-angle_rad)
}

/// Quat representing left-bank (left wing down). Mirror of `roll_right`.
#[must_use]
pub fn roll_left(angle_rad: f32) -> Quat {
    Quat::from_rotation_z(angle_rad)
}

/// Quat representing nose-right yaw by `angle` radians.
///
/// Wraps `Quat::from_rotation_y(-angle)` — the negation is the flowplan
/// sign convention (see module docstring). Positive angle = nose-right.
#[must_use]
pub fn yaw_right(angle_rad: f32) -> Quat {
    Quat::from_rotation_y(-angle_rad)
}

/// Quat representing nose-left yaw. Mirror of `yaw_right`.
#[must_use]
pub fn yaw_left(angle_rad: f32) -> Quat {
    Quat::from_rotation_y(angle_rad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// A small f32 tolerance for vector-rotation equality checks.
    const EPS: f32 = 1e-5;

    #[test]
    fn pitch_up_tilts_body_forward_upward() {
        // +Z in body frame is backward per flowplan convention; body-forward
        // is -Z. Applying pitch_up(+10°) to body-forward should tilt it
        // slightly upward: expect positive y, still mostly -z.
        let q = pitch_up(10_f32.to_radians());
        let fwd = q * Vec3::NEG_Z;
        assert!(fwd.y > 0.0, "nose-up pitch should raise forward-y; got {fwd:?}");
        assert!(fwd.z < -0.9, "forward still mostly -Z; got {fwd:?}");
    }

    #[test]
    fn roll_right_drops_right_wing_below_horizon() {
        // Body-right is +X. After right-bank, the right-wing vector should
        // tilt downward in world frame (negative y).
        let q = roll_right(10_f32.to_radians());
        let right_wing_world = q * Vec3::X;
        assert!(
            right_wing_world.y < 0.0,
            "right-bank should drop right wing (+X → negative Y); got {right_wing_world:?}"
        );
    }

    #[test]
    fn roll_left_raises_right_wing() {
        let q = roll_left(10_f32.to_radians());
        let right_wing_world = q * Vec3::X;
        assert!(
            right_wing_world.y > 0.0,
            "left-bank should raise right wing; got {right_wing_world:?}"
        );
    }

    #[test]
    fn yaw_right_rotates_nose_toward_right() {
        // Body-forward is -Z. After nose-right yaw, forward direction
        // should have a positive X component (turned toward body-right).
        let q = yaw_right(10_f32.to_radians());
        let fwd_world = q * Vec3::NEG_Z;
        assert!(
            fwd_world.x > 0.0,
            "yaw-right should tilt forward toward +X; got {fwd_world:?}"
        );
    }

    #[test]
    fn yaw_left_rotates_nose_toward_left() {
        let q = yaw_left(10_f32.to_radians());
        let fwd_world = q * Vec3::NEG_Z;
        assert!(
            fwd_world.x < 0.0,
            "yaw-left should tilt forward toward -X; got {fwd_world:?}"
        );
    }

    #[test]
    fn pitch_down_inverts_pitch_up() {
        let a = 0.2_f32;
        let p = pitch_up(a) * pitch_down(a);
        // Should be identity (or numerically very close).
        let diff = p * Vec3::NEG_Z - Vec3::NEG_Z;
        assert!(
            diff.length() < EPS,
            "pitch_up(a)·pitch_down(a) should be identity rotation; diff={diff:?}"
        );
    }

    #[test]
    fn roll_mirror_inverts() {
        let a = 0.2_f32;
        let p = roll_right(a) * roll_left(a);
        let diff = p * Vec3::X - Vec3::X;
        assert!(diff.length() < EPS);
    }

    #[test]
    fn combined_pitch_and_roll_is_composable() {
        // The canonical 3-point rest attitude + 10° right bank, used by the
        // beaver_land_wingtip_scrape scenario. Verifying the helpers compose
        // in the same orientation the scenario's hand-written quat produces.
        let pitch = pitch_up(0.11279);
        let bank = roll_right(7.5_f32.to_radians());
        let combined = pitch * bank;

        // Body +X (right wing) after both rotations should be below horizon
        // (y < 0) AND tilted slightly backward (z > 0 — because the nose-up
        // pitch after the bank pushes the rotated +X slightly aft).
        let right_wing = combined * Vec3::X;
        assert!(
            right_wing.y < 0.0,
            "combined pitch + right-bank still drops right wing; got {right_wing:?}"
        );
    }
}
