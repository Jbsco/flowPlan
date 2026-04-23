//! Sky / sun rules.
//!
//! Phase 2 Milestone 2.6 sub-step 4: a minimal sun-direction function keyed to the sim's
//! normalized `time_of_day` in `[0, 1)`. Sun tracks a great circle tilted slightly toward
//! `-Z`: east (`+X`) at dawn, overhead at noon, west (`-X`) at dusk, below horizon at
//! night. Full atmospheric / latitude / seasonal model arrives with the celestial-sphere
//! work in Phase 5.
//!
//! Phase 3.6.0 added longitude-varying local time: [`local_time_of_day`] shifts
//! the global sim clock by a position-dependent offset so different world-x
//! coordinates see different solar phases simultaneously — fly east to chase
//! the sunset, west to race the sunrise. Crossing the toroidal seam preserves
//! continuity (time just wraps to the next cycle).

use core::f32::consts::TAU;

use flowplan_core::WORLD_EXTENT_M;
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
    let v = Vec3::new(libm::cosf(phi), libm::sinf(phi), SOLAR_TILT_Z);
    v.try_normalize().unwrap_or(Vec3::Y)
}

/// Local time-of-day at world-x coordinate `x_m`.
///
/// Derived from the sim's global `time_of_day` by subtracting a
/// longitude-phase offset. Each full `WORLD_EXTENT_M` of eastward motion
/// takes you through one full day cycle "earlier" (you're further east,
/// so the sun has moved less far in your local frame). Result wraps into
/// `[0, 1)` via `rem_euclid`. Crossing the toroidal seam is continuous —
/// time just rolls forward to the next cycle.
///
/// Phase 3.6.0 per `MAPGEN.md § Day/night cycle`: accelerated east-to-west
/// day cycle with 24:1 placeholder ratio. Placeholder: at `WORLD_EXTENT_M =
/// 5000 km` and `DAY_LENGTH_SECONDS = 1200 s`, traveling east at ferry speed
/// (~50 m/s) keeps you in the same local time (earth rotates under you).
#[must_use]
pub fn local_time_of_day(global_time_of_day: f32, x_m: f32) -> f32 {
    let longitude_phase = x_m / WORLD_EXTENT_M;
    (global_time_of_day - longitude_phase).rem_euclid(1.0)
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
            assert!(
                (v.length() - 1.0).abs() < 1e-5,
                "t={t} length={}",
                v.length()
            );
        }
    }

    #[test]
    fn midnight_sun_is_below_horizon() {
        let v = sun_direction(0.0);
        assert!(
            v.y < 0.0,
            "midnight sun should be below horizon; got y={}",
            v.y
        );
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

    #[test]
    fn local_time_at_origin_is_global_time() {
        // At x=0, local time equals the global time (zero longitude offset).
        for &t in &[0.0, 0.25, 0.5, 0.75] {
            let local = local_time_of_day(t, 0.0);
            assert!((local - t).abs() < 1e-5, "global={t} local={local}");
        }
    }

    #[test]
    fn local_time_halfworld_is_shifted_half_day() {
        // At x = WORLD_EXTENT_M/2, local time is 12 hours offset (half cycle).
        let half = WORLD_EXTENT_M * 0.5;
        let local = local_time_of_day(0.5, half);
        // 0.5 - 0.5 = 0.0 (midnight halfway around)
        assert!(local.abs() < 1e-5, "expected ~0 at half-world, got {local}");
    }

    #[test]
    fn local_time_wraps_across_seam() {
        // local_time_of_day at x and x+WORLD_EXTENT_M should be identical —
        // the longitude phase shift is exactly one full day cycle per world.
        for &x in &[0.0, 100_000.0, 2_500_000.0] {
            let t_global = 0.3;
            let local_a = local_time_of_day(t_global, x);
            let local_b = local_time_of_day(t_global, x + WORLD_EXTENT_M);
            assert!(
                (local_a - local_b).abs() < 1e-5,
                "x={x}: local_a={local_a} local_b={local_b}"
            );
        }
    }

    #[test]
    fn local_time_east_is_earlier() {
        // Moving east (larger x) puts you "earlier in the day" — the sun
        // hasn't risen as high there yet, so local time-of-day is smaller.
        let global = 0.5;
        let west = local_time_of_day(global, 0.0);
        let east = local_time_of_day(global, WORLD_EXTENT_M * 0.25);
        // East is x = W/4, so longitude_phase = 0.25, local = 0.5 - 0.25 = 0.25.
        assert!(east < west);
        assert!((east - 0.25).abs() < 1e-5);
        assert!((west - 0.5).abs() < 1e-5);
    }
}
