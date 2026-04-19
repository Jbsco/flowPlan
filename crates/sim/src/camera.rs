//! Camera state and controllers.
//!
//! The sim owns camera state. Render reads a snapshot. The camera supports three view
//! kinds in Phase 1 (first-person cockpit placeholder, third-person chase, exterior
//! orbit) plus POV-hat look-around per view. View switching is a simple discrete change;
//! the render layer smooths visual transitions via its own tween.
//!
//! See [`FEATURES.md § Camera & Views`](../../../FEATURES.md#camera--views).

use glam::{Quat, Vec2, Vec3};

/// One of the supported camera frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewKind {
    /// Head-in-the-cockpit frame. Placeholder rig for Phase 1 (no cockpit model yet);
    /// a small offset from the vehicle origin stands in.
    FirstPerson,
    /// Behind-and-above chase camera with smooth damping. Default for Phase 1 -- the
    /// chase view is the most legible for a pilot familiarizing themselves with the
    /// vehicle.
    #[default]
    ChaseThirdPerson,
    /// Free-orbit around the vehicle; useful for pre-flight inspection.
    ExteriorOrbit,
}

impl ViewKind {
    /// Advance to the next view in the cycle.
    #[must_use]
    pub const fn cycled(self) -> Self {
        match self {
            Self::FirstPerson => Self::ChaseThirdPerson,
            Self::ChaseThirdPerson => Self::ExteriorOrbit,
            Self::ExteriorOrbit => Self::FirstPerson,
        }
    }
}

/// Chase-camera tuning parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChaseConfig {
    /// Distance behind the target along its local backward axis (meters).
    pub distance: f32,
    /// Height above the target (meters).
    pub height: f32,
    /// Smoothing stiffness (higher = tighter follow). Dimensionally 1/seconds.
    pub stiffness: f32,
    /// Critical-damping factor on the exponential smoothing. 1.0 is critical.
    pub damping: f32,
}

impl Default for ChaseConfig {
    fn default() -> Self {
        Self {
            distance: 12.0,
            height: 3.5,
            stiffness: 6.0,
            damping: 1.0,
        }
    }
}

/// Orbit camera tuning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitConfig {
    pub distance: f32,
    pub pitch: f32, // elevation in radians
    pub yaw: f32,   // azimuth in radians
}

impl Default for OrbitConfig {
    fn default() -> Self {
        Self {
            distance: 18.0,
            pitch: 0.3,
            yaw: 0.0,
        }
    }
}

/// Full camera state.
///
/// `position` and `orientation` are the current-tick pose of the camera in world space.
/// `view_kind` selects which controller drives them. `pov_look` is the POV-hat offset
/// (yaw, pitch in radians), applied on top of the view's natural orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub orientation: Quat,
    pub fov_y_radians: f32,
    pub view_kind: ViewKind,
    pub pov_look: Vec2,
    pub chase: ChaseConfig,
    pub orbit: OrbitConfig,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 5.0, 15.0),
            orientation: Quat::IDENTITY,
            fov_y_radians: 60_f32.to_radians(),
            view_kind: ViewKind::default(),
            pov_look: Vec2::ZERO,
            chase: ChaseConfig::default(),
            orbit: OrbitConfig::default(),
        }
    }
}

impl Camera {
    /// Apply an accumulated POV-hat delta. Clamped so the hat cannot swing past the
    /// natural cockpit range.
    pub fn apply_look_delta(&mut self, delta: Vec2) {
        const MAX_YAW: f32 = 2.35; // ~135 deg
        const MAX_PITCH: f32 = 1.05; // ~60 deg
        self.pov_look.x = (self.pov_look.x + delta.x).clamp(-MAX_YAW, MAX_YAW);
        self.pov_look.y = (self.pov_look.y + delta.y).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Snap the POV-hat back to the view's natural orientation.
    pub fn reset_look(&mut self) {
        self.pov_look = Vec2::ZERO;
    }

    /// Clamp the camera's Y position to be at least `min_y` (meters in world space).
    ///
    /// Called by the app layer post-tick with a terrain-sampled ground height plus a
    /// small margin, to prevent the chase camera from clipping through hills. Kept
    /// external to `follow()` so that the sim tick remains independent of terrain
    /// geometry; terrain is an app-owned concern.
    pub fn clamp_above(&mut self, min_y: f32) {
        if self.position.y < min_y {
            self.position.y = min_y;
        }
    }

    /// Advance the camera one tick toward following `target_pos` / `target_orient`.
    ///
    /// Uses exponential smoothing (critical-damping factor) for the chase view; orbit
    /// moves instantly; first-person pins to the target. POV-hat is applied after.
    pub fn follow(&mut self, target_pos: Vec3, target_orient: Quat, dt: f32) {
        match self.view_kind {
            ViewKind::FirstPerson => {
                // Seat position: a small offset from the vehicle origin. Will be replaced
                // by per-aircraft cockpit rig data in Phase 2.
                let seat_offset = Vec3::new(0.0, 0.6, 0.0);
                let world_seat = target_pos + target_orient * seat_offset;
                self.position = world_seat;
                let pov =
                    Quat::from_euler(glam::EulerRot::YXZ, self.pov_look.x, self.pov_look.y, 0.0);
                self.orientation = target_orient * pov;
            },
            ViewKind::ChaseThirdPerson => {
                // Body-frame chase: back direction tracks the full body orientation,
                // including bank and pitch. The camera leans with the vehicle, which
                // gives a cinematic feel when banking into a turn and makes the vehicle's
                // orientation immediately readable without needing an artificial
                // horizon.
                let back = target_orient * Vec3::Z; // local +Z is backward
                let up = Vec3::Y;
                let desired_pos = target_pos + back * self.chase.distance + up * self.chase.height;
                // Exponential smoothing -- damping factor 1.0 = critical.
                let alpha = 1.0 - (-self.chase.stiffness * self.chase.damping * dt).exp();
                self.position = self.position.lerp(desired_pos, alpha);
                // Look at a point slightly above the target to avoid looking at the ground.
                let look_target = target_pos + Vec3::new(0.0, 0.8, 0.0);
                let forward_look = (look_target - self.position).normalize_or_zero();
                let look = look_at_quat(forward_look, Vec3::Y);
                let pov =
                    Quat::from_euler(glam::EulerRot::YXZ, self.pov_look.x, self.pov_look.y, 0.0);
                self.orientation = look * pov;
            },
            ViewKind::ExteriorOrbit => {
                let pitch = self.orbit.pitch + self.pov_look.y;
                let yaw = self.orbit.yaw + self.pov_look.x;
                let offset = Vec3::new(
                    self.orbit.distance * yaw.cos() * pitch.cos(),
                    self.orbit.distance * pitch.sin(),
                    self.orbit.distance * yaw.sin() * pitch.cos(),
                );
                self.position = target_pos + offset;
                let forward = (target_pos - self.position).normalize_or_zero();
                self.orientation = look_at_quat(forward, Vec3::Y);
            },
        }
    }
}

/// Build a world-space rotation quaternion that looks along `forward` with `up` as the
/// reference up vector. Returns identity for zero or degenerate inputs.
///
/// Convention: camera looks along its local `-Z`; local `+X` is right; local `+Y` is up.
/// The cross-product order is `right = forward x up`, not `up x forward` -- swapping
/// them flips the camera's up vector and renders the scene upside-down. Corresponding
/// test: `up_stays_above`.
fn look_at_quat(forward: Vec3, up: Vec3) -> Quat {
    let forward = forward.normalize_or_zero();
    if forward.length_squared() < f32::EPSILON {
        return Quat::IDENTITY;
    }
    // Right-handed camera basis: local +X = right, local +Y = up, local -Z = view.
    let right = forward.cross(up).normalize_or_zero();
    if right.length_squared() < f32::EPSILON {
        return Quat::IDENTITY;
    }
    let corrected_up = right.cross(forward);
    let m = glam::Mat3::from_cols(right, corrected_up, -forward);
    Quat::from_mat3(&m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_cycles() {
        let v = ViewKind::FirstPerson;
        assert_eq!(v.cycled(), ViewKind::ChaseThirdPerson);
        assert_eq!(v.cycled().cycled(), ViewKind::ExteriorOrbit);
        assert_eq!(v.cycled().cycled().cycled(), ViewKind::FirstPerson);
    }

    #[test]
    fn pov_look_clamps() {
        let mut cam = Camera::default();
        cam.apply_look_delta(Vec2::new(10.0, 10.0));
        assert!(cam.pov_look.x < 2.36);
        assert!(cam.pov_look.y < 1.06);
        cam.apply_look_delta(Vec2::new(-100.0, -100.0));
        assert!(cam.pov_look.x > -2.36);
        assert!(cam.pov_look.y > -1.06);
    }

    #[test]
    fn reset_look_zeros_pov() {
        let mut cam = Camera::default();
        cam.apply_look_delta(Vec2::new(1.0, 0.5));
        cam.reset_look();
        assert!(cam.pov_look.x.abs() < f32::EPSILON);
        assert!(cam.pov_look.y.abs() < f32::EPSILON);
    }

    #[test]
    fn chase_converges_to_desired() {
        let mut cam = Camera {
            view_kind: ViewKind::ChaseThirdPerson,
            ..Camera::default()
        };
        let target_pos = Vec3::new(100.0, 50.0, 0.0);
        let target_orient = Quat::IDENTITY;
        // Step many times at small dt; camera should approach desired offset.
        for _ in 0..300 {
            cam.follow(target_pos, target_orient, 0.016);
        }
        let expected = target_pos + Vec3::Z * cam.chase.distance + Vec3::Y * cam.chase.height;
        let diff = (cam.position - expected).length();
        assert!(diff < 0.5, "chase camera did not converge; diff={diff}");
    }

    #[test]
    fn orbit_distance_preserved() {
        let mut cam = Camera {
            view_kind: ViewKind::ExteriorOrbit,
            ..Camera::default()
        };
        let target = Vec3::new(10.0, 10.0, 10.0);
        cam.follow(target, Quat::IDENTITY, 0.016);
        let actual_distance = (cam.position - target).length();
        assert!((actual_distance - cam.orbit.distance).abs() < 0.01);
    }

    /// Regression guard: camera's local +Y axis must point upward in world space after
    /// `follow()`. A swapped cross-product in `look_at_quat` flipped this and rendered
    /// the scene upside-down; this test fires if that bug returns.
    #[test]
    fn chase_up_stays_above() {
        let mut cam = Camera {
            view_kind: ViewKind::ChaseThirdPerson,
            ..Camera::default()
        };
        let target_pos = Vec3::new(0.0, 50.0, 0.0);
        for _ in 0..300 {
            cam.follow(target_pos, Quat::IDENTITY, 0.016);
        }
        let cam_up = cam.orientation * Vec3::Y;
        assert!(
            cam_up.y > 0.7,
            "camera local +Y must point upward in world space; got {cam_up:?}"
        );
    }

    #[test]
    fn orbit_up_stays_above() {
        let mut cam = Camera {
            view_kind: ViewKind::ExteriorOrbit,
            ..Camera::default()
        };
        let target = Vec3::new(5.0, 20.0, 5.0);
        cam.follow(target, Quat::IDENTITY, 0.016);
        let cam_up = cam.orientation * Vec3::Y;
        assert!(
            cam_up.y > 0.7,
            "orbit camera local +Y must point upward; got {cam_up:?}"
        );
    }

    #[test]
    fn first_person_pins_to_target() {
        let mut cam = Camera {
            view_kind: ViewKind::FirstPerson,
            ..Camera::default()
        };
        let target = Vec3::new(5.0, 2.0, 3.0);
        cam.follow(target, Quat::IDENTITY, 0.016);
        // First-person rides inside the target; position should be within a small seat offset.
        assert!((cam.position - target).length() < 2.0);
    }
}
