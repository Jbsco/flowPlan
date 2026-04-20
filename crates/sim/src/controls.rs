//! Pending control surface values.
//!
//! `Controls` is the continuous state of the primary axes (throttle, pitch, yaw, roll)
//! plus discrete configuration state (flaps). It carries the player's current
//! commanded values. The sim tick reads this, integrates dynamics, and updates the
//! vehicle. Phase 2's flight model extends this with flap notches (2.4.5); trim /
//! gear / mixture / feather land later.

/// Discrete flap positions. Beaver-style 4-detent handle: UP (clean) through LANDING
/// (full extension). Aero effects (Cl / Cd0 / Cm deltas) live in
/// `flight::flap_aero()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlapPosition {
    #[default]
    Up,
    Climb,
    Takeoff,
    Landing,
}

impl FlapPosition {
    /// Next position toward LANDING (more flap). Saturates at LANDING.
    #[must_use]
    pub fn extended(self) -> Self {
        match self {
            Self::Up => Self::Climb,
            Self::Climb => Self::Takeoff,
            Self::Takeoff | Self::Landing => Self::Landing,
        }
    }

    /// Next position toward UP (less flap). Saturates at UP.
    #[must_use]
    pub fn retracted(self) -> Self {
        match self {
            Self::Up | Self::Climb => Self::Up,
            Self::Takeoff => Self::Climb,
            Self::Landing => Self::Takeoff,
        }
    }

    /// Approximate deflection in degrees. Purely for HUD display; the flight model
    /// reads `flap_aero()` by position, not by angle.
    #[must_use]
    pub fn deflection_deg(self) -> f32 {
        match self {
            Self::Up => 0.0,
            Self::Climb => 20.0,
            Self::Takeoff => 35.0,
            Self::Landing => 58.0,
        }
    }

    /// Short label for HUD / logs.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Climb => "CLIMB",
            Self::Takeoff => "TAKEOFF",
            Self::Landing => "LANDING",
        }
    }
}

/// Clamped continuous control inputs + discrete configuration as they stand on this
/// tick.
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
    /// Current flap detent.
    pub flaps: FlapPosition,
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
            ..Default::default()
        };
        c.clamp();
        assert!((c.throttle - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn clamp_throttle_lower() {
        let mut c = Controls {
            throttle: -0.5,
            ..Default::default()
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
            ..Default::default()
        };
        c.clamp();
        assert!((c.pitch - 1.0).abs() < f32::EPSILON);
        assert!((c.yaw - -1.0).abs() < f32::EPSILON);
        assert!((c.roll - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flap_position_default_is_up() {
        assert_eq!(Controls::default().flaps, FlapPosition::Up);
    }

    #[test]
    fn flap_extended_steps_through_all_detents() {
        assert_eq!(FlapPosition::Up.extended(), FlapPosition::Climb);
        assert_eq!(FlapPosition::Climb.extended(), FlapPosition::Takeoff);
        assert_eq!(FlapPosition::Takeoff.extended(), FlapPosition::Landing);
        assert_eq!(FlapPosition::Landing.extended(), FlapPosition::Landing);
    }
}
