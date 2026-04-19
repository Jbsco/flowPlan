//! Time-related newtypes for the simulation clock.
//!
//! `TickCount` counts ticks at the fixed timestep rate (target 120 Hz per
//! [`DESIGN.md § Invariants`](../../../DESIGN.md#invariants)). `Seconds` is an explicit unit on
//! wall-clock floats, used at boundaries where a physical duration is meaningful.

use core::fmt;
use core::ops::{Add, AddAssign, Mul, Sub};

/// Monotonic count of simulation ticks since start.
///
/// Tick 0 is the initial state; advancing the simulation increments by one. Wraps never
/// in practice (u64 at 120 Hz survives ~4.9 billion years before overflow).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TickCount(pub u64);

impl TickCount {
    /// The starting tick.
    pub const ZERO: Self = Self(0);

    /// Advance by one tick.
    #[inline]
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Distance in ticks from an earlier tick to `self`. Panics in debug if `self < earlier`.
    #[inline]
    #[must_use]
    pub const fn since(self, earlier: Self) -> u64 {
        self.0 - earlier.0
    }
}

impl fmt::Display for TickCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tick#{}", self.0)
    }
}

impl Add<u64> for TickCount {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u64) -> Self {
        Self(self.0 + rhs)
    }
}

impl AddAssign<u64> for TickCount {
    #[inline]
    fn add_assign(&mut self, rhs: u64) {
        self.0 += rhs;
    }
}

impl Sub for TickCount {
    type Output = u64;
    #[inline]
    fn sub(self, rhs: Self) -> u64 {
        self.0 - rhs.0
    }
}

/// Wall-clock duration in seconds. Used at boundaries (input timestamps, asset-load
/// durations, profiling) where a physical time unit is meaningful.
///
/// The simulation itself advances by fixed-timestep ticks, not by `Seconds`. Use
/// `TickCount` inside the sim.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Seconds(pub f64);

impl Seconds {
    pub const ZERO: Self = Self(0.0);

    /// Construct from a whole-second integer.
    #[inline]
    #[must_use]
    pub const fn from_secs(s: u64) -> Self {
        // Safe u64->f64 for values within practical duration range.
        #[allow(clippy::cast_precision_loss)]
        Self(s as f64)
    }

    /// Construct from milliseconds.
    #[inline]
    #[must_use]
    pub fn from_millis(ms: u64) -> Self {
        #[allow(clippy::cast_precision_loss)]
        Self(ms as f64 * 0.001)
    }

    /// Seconds as f32 (e.g., for shader uniforms).
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub const fn as_f32(self) -> f32 {
        self.0 as f32
    }
}

impl fmt::Display for Seconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.6}s", self.0)
    }
}

impl Add for Seconds {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Seconds {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Seconds {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Mul<f64> for Seconds {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f64) -> Self {
        Self(self.0 * rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_count_advances() {
        let t = TickCount::ZERO;
        let u = t.next().next().next();
        assert_eq!(u.0, 3);
        assert_eq!(u.since(t), 3);
        assert_eq!(u - t, 3);
    }

    #[test]
    fn tick_count_add_assign() {
        let mut t = TickCount::ZERO;
        t += 10;
        assert_eq!(t.0, 10);
    }

    #[test]
    fn seconds_arithmetic() {
        let half = Seconds::from_millis(500);
        let two = Seconds::from_secs(2);
        let sum = half + two;
        assert!((sum.0 - 2.5).abs() < 1e-9);
        let diff = two - half;
        assert!((diff.0 - 1.5).abs() < 1e-9);
        let scaled = half * 4.0;
        assert!((scaled.0 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn seconds_as_f32() {
        let s = Seconds::from_millis(1500);
        assert!((s.as_f32() - 1.5_f32).abs() < 1e-6);
    }

    #[test]
    fn tick_count_display() {
        assert_eq!(format!("{}", TickCount(42)), "tick#42");
    }

    #[test]
    fn seconds_display() {
        assert_eq!(format!("{}", Seconds::from_millis(1500)), "1.500000s");
    }
}
