//! Free-fly debug camera (Phase 3.6.3c.5a).
//!
//! 6-DoF camera decoupled from the aircraft. Useful for inspecting
//! streaming + placement-rule + palette-adaptation output in-engine
//! without having to pilot to the location.
//!
//! See:
//! - Memory `streaming_render_architecture` — the debug camera is a
//!   first-class consumer of `ActiveCamera` + both streaming layers.
//! - Memory `visual_bench_pattern` — complementary in-engine tool
//!   alongside the offline PNG artifacts.
//! - ROADMAP Milestone 3.6.3c.5a — full spec.
//!
//! # Bindings
//!
//! - **Backtick** ( `` ` `` ) — toggle debug mode on/off
//! - **ESC** — release mouse capture (stays in debug mode)
//! - **WASD** — camera-relative horizontal motion
//! - **Left Shift** — vertical up (+Y)
//! - **Left Ctrl** — vertical down (-Y)
//! - **Mouse Δ** — yaw (x) + pitch (y); clamped pitch to ±89°
//! - **Tab** — cycle speed: 50 → 100 → 500 → 50 m/s
//!
//! # Camera state
//!
//! Stored as (position, yaw, pitch) rather than orientation-quat
//! directly because mouse-look updates yaw/pitch as independent scalars.
//! Quat derived on demand via `Quat::from_euler(YXZ, yaw, pitch, 0)`.

#![allow(clippy::cast_precision_loss)]

use glam::{Quat, Vec3};

/// Mouse sensitivity (radians per pixel of mouse motion).
///
/// 0.002 = ~0.115° per pixel — FPS-game standard. 360° sweep needs
/// ~3142 pixels of mouse movement. Phase 4.8 settings UI will expose
/// this as a player-tunable slider.
pub const MOUSE_SENSITIVITY_RAD_PER_PX: f32 = 0.002;

/// Pitch clamp (radians). 89° = prevent gimbal-lock at straight-up /
/// straight-down, where yaw becomes ambiguous.
pub const PITCH_CLAMP_RAD: f32 = 89.0 * core::f32::consts::PI / 180.0;

/// Default FOV for the debug camera (radians). Matches sim-camera
/// default for seamless visual continuity on toggle.
pub const DEFAULT_FOV_Y_RAD: f32 = 1.221_730_5; // 70°

/// Speed tiers (m/s) cycled by Tab. Low tiers (50/100/500 m/s) cover
/// the 10 km streaming radius for near-camera inspection. High tiers
/// (1 km/s, 5 km/s) enable world-scale sweeps — the 5000 km toroidal
/// world takes ~17 min at 5 km/s, so biome-scale reconnaissance is
/// realistic in a dev session.
pub const SPEED_TIERS_MPS: [f32; 5] = [50.0, 100.0, 500.0, 1000.0, 5000.0];

/// 6-DoF free-fly camera state.
#[derive(Clone, Copy, Debug)]
pub struct DebugCamera {
    /// World-space camera position.
    pub position: Vec3,
    /// Yaw in radians (rotation around world Y). Zero = looking -Z.
    pub yaw: f32,
    /// Pitch in radians (rotation around camera-local X). Clamped to
    /// `[-PITCH_CLAMP_RAD, +PITCH_CLAMP_RAD]` to avoid gimbal lock.
    pub pitch: f32,
    /// Vertical FOV (radians). Held constant for 3.6.3c.5a MVP;
    /// potential zoom-in / FOV slider in a future polish pass.
    pub fov_y_radians: f32,
    /// Current speed-tier index into [`SPEED_TIERS_MPS`].
    pub speed_tier: u8,
}

impl Default for DebugCamera {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y_radians: DEFAULT_FOV_Y_RAD,
            speed_tier: 0,
        }
    }
}

impl DebugCamera {
    /// Spawn at the given position, aiming along -Z, level pitch.
    /// Used on first backtick toggle (entry from sim-camera pose).
    #[must_use]
    pub fn spawn_at(position: Vec3) -> Self {
        Self {
            position,
            ..Self::default()
        }
    }

    /// Current orientation quaternion, derived from yaw + pitch.
    /// YXZ Euler order: yaw (world Y), then pitch (local X), then
    /// roll (zero — debug camera doesn't roll).
    #[must_use]
    pub fn orientation(&self) -> Quat {
        Quat::from_euler(glam::EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    /// Current speed (m/s) from the active tier.
    #[must_use]
    pub fn current_speed_mps(&self) -> f32 {
        SPEED_TIERS_MPS[self.speed_tier as usize % SPEED_TIERS_MPS.len()]
    }

    /// Advance to the next speed tier (wraps around).
    pub fn cycle_speed(&mut self) {
        // SPEED_TIERS_MPS has 3 entries; the modulo result fits u8 by construction.
        #[allow(clippy::cast_possible_truncation)]
        let next = ((self.speed_tier as usize + 1) % SPEED_TIERS_MPS.len()) as u8;
        self.speed_tier = next;
    }

    /// Apply mouse Δ to yaw + pitch. Clamps pitch.
    pub fn apply_mouse_delta(&mut self, dx_px: f32, dy_px: f32) {
        self.yaw -= dx_px * MOUSE_SENSITIVITY_RAD_PER_PX;
        self.pitch = (self.pitch - dy_px * MOUSE_SENSITIVITY_RAD_PER_PX)
            .clamp(-PITCH_CLAMP_RAD, PITCH_CLAMP_RAD);
        // Normalize yaw to [-π, π] to avoid float drift on long
        // sessions (unlikely to matter in practice, cheap insurance).
        let two_pi = core::f32::consts::TAU;
        if self.yaw > core::f32::consts::PI {
            self.yaw -= two_pi;
        } else if self.yaw < -core::f32::consts::PI {
            self.yaw += two_pi;
        }
    }

    /// Advance position by integrating WASD + Shift/Ctrl input over `dt`.
    ///
    /// `forward_back`: +1 = W (forward along camera -Z); -1 = S (backward)
    /// `right_left`: +1 = D (strafe right along camera +X); -1 = A
    /// `up_down`: +1 = Shift (world +Y); -1 = Ctrl (world -Y)
    pub fn integrate(&mut self, forward_back: f32, right_left: f32, up_down: f32, dt: f32) {
        let speed = self.current_speed_mps();
        let orient = self.orientation();
        let forward = orient * Vec3::NEG_Z;
        let right = orient * Vec3::X;
        let horizontal = forward * forward_back + right * right_left;
        self.position += horizontal * speed * dt;
        self.position.y += up_down * speed * dt;
    }

    /// Heading in degrees (0-360) for HUD display. 0° = north (-Z),
    /// 90° = east (+X), etc.
    #[must_use]
    pub fn heading_deg(&self) -> f32 {
        let deg = -self.yaw * 180.0 / core::f32::consts::PI;
        deg.rem_euclid(360.0)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn default_camera_state() {
        let c = DebugCamera::default();
        assert_eq!(c.position, Vec3::ZERO);
        assert!(c.yaw.abs() < EPS);
        assert!(c.pitch.abs() < EPS);
        assert_eq!(c.speed_tier, 0);
        assert!((c.current_speed_mps() - 50.0).abs() < EPS);
    }

    #[test]
    fn cycle_speed_wraps_through_tiers() {
        let mut c = DebugCamera::default();
        assert!((c.current_speed_mps() - 50.0).abs() < EPS);
        c.cycle_speed();
        assert!((c.current_speed_mps() - 100.0).abs() < EPS);
        c.cycle_speed();
        assert!((c.current_speed_mps() - 500.0).abs() < EPS);
        c.cycle_speed();
        assert!((c.current_speed_mps() - 1000.0).abs() < EPS);
        c.cycle_speed();
        assert!((c.current_speed_mps() - 5000.0).abs() < EPS);
        c.cycle_speed();
        assert!((c.current_speed_mps() - 50.0).abs() < EPS);
    }

    #[test]
    fn mouse_delta_updates_yaw_and_pitch() {
        let mut c = DebugCamera::default();
        c.apply_mouse_delta(100.0, 0.0);
        assert!((c.yaw - (-100.0 * MOUSE_SENSITIVITY_RAD_PER_PX)).abs() < EPS);
        assert!(c.pitch.abs() < EPS);
        c.apply_mouse_delta(0.0, -50.0);
        assert!((c.pitch - (50.0 * MOUSE_SENSITIVITY_RAD_PER_PX)).abs() < EPS);
    }

    #[test]
    fn pitch_is_clamped_to_prevent_gimbal_lock() {
        let mut c = DebugCamera::default();
        // Apply enormous upward mouse motion; pitch should clamp.
        c.apply_mouse_delta(0.0, -100_000.0);
        assert!(c.pitch <= PITCH_CLAMP_RAD + EPS);
        // And downward.
        c.apply_mouse_delta(0.0, 200_000.0);
        assert!(c.pitch >= -PITCH_CLAMP_RAD - EPS);
    }

    #[test]
    fn forward_motion_moves_along_negative_z_at_default_orientation() {
        let mut c = DebugCamera::default();
        c.integrate(1.0, 0.0, 0.0, 1.0); // 1 sec forward at default tier (50 m/s)
        assert!(c.position.x.abs() < EPS);
        assert!(c.position.y.abs() < EPS);
        assert!((c.position.z - -50.0).abs() < EPS);
    }

    #[test]
    fn strafe_moves_along_positive_x_at_default_orientation() {
        let mut c = DebugCamera::default();
        c.integrate(0.0, 1.0, 0.0, 1.0);
        assert!((c.position.x - 50.0).abs() < EPS);
        assert!(c.position.y.abs() < EPS);
        assert!(c.position.z.abs() < EPS);
    }

    #[test]
    fn vertical_motion_uses_world_y() {
        let mut c = DebugCamera::default();
        // Tilt 45° down; vertical input should still move along world Y.
        c.apply_mouse_delta(0.0, 200.0);
        let pos_before = c.position;
        c.integrate(0.0, 0.0, 1.0, 1.0);
        assert!((c.position.x - pos_before.x).abs() < EPS);
        assert!((c.position.z - pos_before.z).abs() < EPS);
        assert!((c.position.y - (pos_before.y + 50.0)).abs() < EPS);
    }

    #[test]
    fn heading_at_default_orientation_is_north() {
        let c = DebugCamera::default();
        let heading = c.heading_deg();
        // 0 or 360 both acceptable; rem_euclid should give 0.
        assert!(!(1.0..=359.0).contains(&heading));
    }
}
