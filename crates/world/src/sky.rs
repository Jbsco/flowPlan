//! Sky / sun rules.
//!
//! Phase 2 Milestone 2.6 sub-step 4: a minimal sun-direction function keyed to the sim's
//! normalized `time_of_day` in `[0, 1)`. Sun tracks a great circle tilted slightly toward
//! `-Z`: east (`+X`) at dawn, overhead at noon, west (`-X`) at dusk, below horizon at
//! night. Full atmospheric / latitude / seasonal model arrives with the celestial-sphere
//! work in Phase 5.

use core::f32::consts::TAU;

use glam::Vec3;

/// Fixed tilt of the solar great circle toward `-Z`. Keeps the sun off the exact zenith,
/// producing interesting diurnal shading regardless of the scene's forward direction.
const SOLAR_TILT_Z: f32 = -0.3;

/// Unit vector pointing FROM the scene TOWARD the sun for a given `time_of_day`.
///
/// `time_of_day` is a normalized phase in `[0, 1)`: 0.0 = midnight, 0.25 = dawn,
/// 0.5 = noon, 0.75 = dusk. Values outside the range are wrapped with `rem_euclid`.
#[must_use]
pub fn sun_direction(time_of_day: f32) -> Vec3 {
    let t = time_of_day.rem_euclid(1.0);
    // Phase so that noon (0.5) maps to azimuth 90 deg, putting the sun at (0, 1, tilt).
    let phi = TAU * (t - 0.25);
    let v = Vec3::new(phi.cos(), phi.sin(), SOLAR_TILT_Z);
    v.try_normalize().unwrap_or(Vec3::Y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_is_unit_length() {
        for i in 0..32_u32 {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 32.0;
            let v = sun_direction(t);
            assert!((v.length() - 1.0).abs() < 1e-5, "t={t} length={}", v.length());
        }
    }

    #[test]
    fn midnight_sun_is_below_horizon() {
        let v = sun_direction(0.0);
        assert!(v.y < 0.0, "midnight sun should be below horizon; got y={}", v.y);
    }

    #[test]
    fn noon_sun_is_overhead() {
        let v = sun_direction(0.5);
        assert!(v.y > 0.9, "noon sun should be near zenith; got y={}", v.y);
    }

    #[test]
    fn dawn_and_dusk_span_horizon() {
        let dawn = sun_direction(0.25);
        let dusk = sun_direction(0.75);
        assert!(dawn.y.abs() < 1e-5, "dawn y should be ~0; got {}", dawn.y);
        assert!(dusk.y.abs() < 1e-5, "dusk y should be ~0; got {}", dusk.y);
        // Dawn east (+X), dusk west (-X).
        assert!(dawn.x > 0.9);
        assert!(dusk.x < -0.9);
    }

    #[test]
    fn out_of_range_wraps() {
        let a = sun_direction(0.5);
        let b = sun_direction(1.5);
        assert!((a - b).length() < 1e-5);
    }
}
