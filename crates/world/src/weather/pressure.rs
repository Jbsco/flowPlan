//! Atmospheric pressure field.
//!
//! Pure function of `(master_seed, world_xz, altitude, time)`. No global state,
//! no wall-clock reads. Uses `libm` for transcendentals.
//!
//! # Model
//!
//! Sea-level pressure = `BASELINE_HPA` + synoptic-scale variation + seasonal
//! trend. Altitude decrease applies the exponential ISA approximation
//! (`p(h) = p_0 · exp(-h / SCALE_HEIGHT_M)`) — the same scale-height used by
//! the flight model's density-altitude calc, so they stay coherent.
//!
//! # Characteristic scale
//!
//! Synoptic pressure variation wavelength ≈ 350 km — matches real highs/lows.
//! Visible on regional-scale visualization; less structure visible at 1 km
//! player-local scale (which is correct — pressure doesn't change much over a
//! 1 km traverse at constant altitude).
//!
//! # Time variation
//!
//! Slow advection via drift offset. At the pre-Voronoi W1 layer, pressure
//! systems drift at a constant world-prevailing rate (set per master seed).
//! When W2 weather cells land (Phase 3.2.2), per-cell storm state adds
//! faster pressure transients on top of this base.

use super::{field_seed, weather_seed};

/// ISA sea-level baseline pressure (hPa).
pub const BASELINE_HPA: f32 = 1013.25;

/// Synoptic-scale pressure variation amplitude (hPa peak-to-peak is ±this).
/// Real weather-system variation is typically ±30 hPa from baseline.
pub const SYNOPTIC_AMPLITUDE_HPA: f32 = 25.0;

/// Synoptic-scale wavelength (m). Originally 350 km (matches real
/// frontal-system spacing).
///
/// Quantized to `WORLD_EXTENT_M / 14` ≈ 357.14 km for clean toroidal-seam
/// wrap (integer cycles per world). 2.04% adjustment — well within the
/// ±15% natural variation of real synoptic systems. Phase 3.6.0
/// wrap-foundation requirement per `MAPGEN.md § Sampler-wrap requirements`.
pub const SYNOPTIC_WAVELENGTH_M: f32 = flowplan_core::WORLD_EXTENT_M / 14.0;

/// Exponential scale height for altitude pressure decrease (m). Matches
/// `flight.rs::ATMOSPHERE_SCALE_HEIGHT_M`.
pub const ATMOSPHERE_SCALE_HEIGHT_M: f32 = 8_500.0;

/// Prevailing drift speed for pressure systems (m/s). Real synoptic systems
/// move at mid-latitude westerly speeds ~10 m/s.
pub const PRESSURE_DRIFT_MPS: f32 = 10.0;

const PRESSURE_FIELD_TAG: u64 = u64::from_le_bytes(*b"pressure");

/// Sea-level pressure at world `(x, z)` at simulation `time` seconds.
///
/// Returns pressure in hectopascals (hPa). Normal range ~980-1040 hPa.
///
/// Perlin-fBm synoptic field (Phase 3.6.1a refactor). Octave offset
/// `[230..240)` avoids collision with terrain/temperature/humidity. Drift
/// via prevailing wind: query is offset backward in time by drift speed,
/// giving the apparent motion of pressure systems past a stationary point.
#[must_use]
pub fn sea_level_pressure(master_seed: u64, x_m: f32, z_m: f32, time_s: f32) -> f32 {
    let seed = field_seed(weather_seed(master_seed), PRESSURE_FIELD_TAG);
    let drift_x = -PRESSURE_DRIFT_MPS * time_s;
    let x_sample = x_m - drift_x;
    let variation = crate::noise::fbm_2d(seed, 230, x_sample, z_m, SYNOPTIC_WAVELENGTH_M, 4);
    BASELINE_HPA + SYNOPTIC_AMPLITUDE_HPA * variation
}

/// Pressure at world `(x, y_altitude, z)` at simulation `time` seconds.
///
/// Applies ISA exponential altitude decrease on top of the sea-level field.
#[must_use]
pub fn pressure_at(master_seed: u64, x_m: f32, y_altitude_m: f32, z_m: f32, time_s: f32) -> f32 {
    let p_sea = sea_level_pressure(master_seed, x_m, z_m, time_s);
    let altitude = y_altitude_m.max(0.0);
    p_sea * libm::expf(-altitude / ATMOSPHERE_SCALE_HEIGHT_M)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)] // Test grid iteration: i32→f32 cast is idiomatic.
#[allow(clippy::float_cmp)] // Tests compare floats via .to_bits() for determinism.
mod tests {
    use super::*;

    #[test]
    fn sea_level_pressure_in_plausible_range() {
        // Sample a 400 km grid, ensure all pressures fall within real-world
        // plausible bounds.
        let seed = 42;
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 20_000.0;
                let z = (iz as f32 - 10.0) * 20_000.0;
                let p = sea_level_pressure(seed, x, z, 0.0);
                let lo = BASELINE_HPA - SYNOPTIC_AMPLITUDE_HPA * 1.01;
                let hi = BASELINE_HPA + SYNOPTIC_AMPLITUDE_HPA * 1.01;
                assert!(
                    (lo..=hi).contains(&p),
                    "pressure {p} hPa at ({x}, {z}) outside expected range"
                );
            }
        }
    }

    #[test]
    fn pressure_decreases_with_altitude() {
        let seed = 42;
        let p_0 = pressure_at(seed, 0.0, 0.0, 0.0, 0.0);
        let p_3k = pressure_at(seed, 0.0, 3_000.0, 0.0, 0.0);
        let p_10k = pressure_at(seed, 0.0, 10_000.0, 0.0, 0.0);

        // ISA rough check: p(3km) ≈ 0.7 · p_0, p(10km) ≈ 0.3 · p_0.
        assert!(p_3k < p_0);
        assert!(p_10k < p_3k);
        assert!((p_3k / p_0 - 0.7).abs() < 0.05);
        assert!((p_10k / p_0 - 0.3).abs() < 0.05);
    }

    #[test]
    fn sea_level_pressure_deterministic() {
        let seed = 12_345;
        let p1 = sea_level_pressure(seed, 1_000.0, 2_000.0, 100.0);
        let p2 = sea_level_pressure(seed, 1_000.0, 2_000.0, 100.0);
        assert_eq!(
            p1.to_bits(),
            p2.to_bits(),
            "sea-level pressure must be bit-identical across calls"
        );
    }

    #[test]
    fn different_seeds_produce_different_patterns() {
        let p_seed_1 = sea_level_pressure(1, 50_000.0, 50_000.0, 0.0);
        let p_seed_2 = sea_level_pressure(2, 50_000.0, 50_000.0, 0.0);
        assert_ne!(p_seed_1.to_bits(), p_seed_2.to_bits());
    }

    #[test]
    fn pressure_drifts_over_time() {
        let seed = 42;
        let p_t0 = sea_level_pressure(seed, 100_000.0, 0.0, 0.0);
        // 1 hour later: drift ≈ 10 m/s * 3600 s = 36 km.
        let p_t1h = sea_level_pressure(seed, 100_000.0, 0.0, 3_600.0);
        // Non-zero drift should visibly move the pattern.
        assert_ne!(p_t0.to_bits(), p_t1h.to_bits());
    }

    #[test]
    fn altitude_clamp_negative_to_zero() {
        let seed = 42;
        let p_neg = pressure_at(seed, 0.0, -100.0, 0.0, 0.0);
        let p_zero = pressure_at(seed, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(p_neg.to_bits(), p_zero.to_bits());
    }
}
