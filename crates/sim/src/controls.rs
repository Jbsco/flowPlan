//! Pending control surface values.
//!
//! `Controls` is the continuous state of the primary axes: throttle, pitch, yaw, roll.
//! It carries the player's current commanded values. The sim tick reads this, integrates
//! dynamics, and updates the vehicle. Phase 2's flight model extends this with flap
//! notches, trim, gear position, mixture, feather, etc.

/// Clamped continuous control inputs as they stand on this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// Throttle in `[0.0, 1.0]`.
    pub throttle: f32,
    /// Pitch command in `[-1.0, 1.0]` (positive = nose up).
    pub pitch: f32,
    /// Yaw command in `[-1.0, 1.0]` (positive = yaw right).
    pub yaw: f32,
    /// Roll command in `[-1.0, 1.0]` (positive = roll right).
    pub roll: f32,
}

impl Controls {
    /// Clamp all axes to their documented ranges. Called by the tick after applying
    /// `SetX`/`XDelta` events so downstream math can assume bounded values.
    pub fn clamp(&mut self) {
        self.throttle = self.throttle.clamp(0.0, 1.0);
        self.pitch = self.pitch.clamp(-1.0, 1.0);
        self.yaw = self.yaw.clamp(-1.0, 1.0);
        self.roll = self.roll.clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_neutral() {
        let c = Controls::default();
        assert!((c.throttle - 0.0).abs() < f32::EPSILON);
        assert!((c.pitch - 0.0).abs() < f32::EPSILON);
        assert!((c.yaw - 0.0).abs() < f32::EPSILON);
        assert!((c.roll - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_throttle_upper() {
        let mut c = Controls {
            throttle: 1.5,
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
        };
        c.clamp();
        assert!((c.throttle - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_throttle_lower() {
        let mut c = Controls {
            throttle: -0.5,
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
        };
        c.clamp();
        assert!((c.throttle - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_axes_bidirectional() {
        let mut c = Controls {
            throttle: 0.5,
            pitch: 2.0,
            yaw: -2.5,
            roll: 1.25,
        };
        c.clamp();
        assert!((c.pitch - 1.0).abs() < f32::EPSILON);
        assert!((c.yaw - -1.0).abs() < f32::EPSILON);
        assert!((c.roll - 1.0).abs() < f32::EPSILON);
    }
}
