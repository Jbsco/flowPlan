//! Atmospheric humidity field.
//!
//! Pure function of `(master_seed, world_xz, altitude, time)`.
//!
//! Returns relative humidity as a fraction in `[0, 1]`.
//!
//! # Model
//!
//! Sea-level RH = `BASELINE_RH` + regional variation + synoptic variation.
//! Altitude-decreasing (higher altitudes hold less absolute moisture).
//!
//! # Characteristic scale
//!
//! Regional variation wavelength ≈ 200 km — humidity follows moisture
//! advection patterns, smaller scale than continental climate gradient but
//! larger than synoptic pressure systems.

use super::{field_seed, weather_seed};

/// Baseline sea-level relative humidity (fraction).
pub const BASELINE_RH: f32 = 0.55;

/// Regional variation amplitude (RH fraction).
pub const REGIONAL_AMPLITUDE: f32 = 0.30;

/// Synoptic variation amplitude (RH fraction).
pub const SYNOPTIC_AMPLITUDE: f32 = 0.10;

/// Regional wavelength (m). Moisture advection scale.
pub const REGIONAL_WAVELENGTH_M: f32 = 200_000.0;

/// Synoptic wavelength (m). Coupled with pressure variation.
pub const SYNOPTIC_WAVELENGTH_M: f32 = 350_000.0;

/// Altitude scale-height for humidity decrease (m).
///
/// Real atmospheric absolute humidity drops off faster than pressure;
/// relative humidity often peaks near cloud base. Simplified here as an
/// exponential decrease.
pub const HUMIDITY_SCALE_HEIGHT_M: f32 = 5_000.0;

const HUMIDITY_FIELD_TAG: u64 = u64::from_le_bytes(*b"humidity");

/// Sea-level relative humidity at world `(x, z)` at `time` seconds. `[0, 1]`.
#[must_use]
pub fn sea_level_humidity(master_seed: u64, x_m: f32, z_m: f32, _time_s: f32) -> f32 {
    let seed = field_seed(weather_seed(master_seed), HUMIDITY_FIELD_TAG);

    // Regional variation (moisture advection).
    let k_reg = core::f32::consts::TAU / REGIONAL_WAVELENGTH_M;
    let phase_r = hash_to_unit(seed, 1) * core::f32::consts::TAU;
    let angle_r = hash_to_unit(seed, 2) * core::f32::consts::TAU;
    let (dx_r, dz_r) = (libm::cosf(angle_r), libm::sinf(angle_r));
    let regional = libm::sinf(k_reg * (x_m * dx_r + z_m * dz_r) + phase_r);

    // Synoptic variation (storm-system-associated moisture).
    let k_syn = core::f32::consts::TAU / SYNOPTIC_WAVELENGTH_M;
    let phase_s = hash_to_unit(seed, 3) * core::f32::consts::TAU;
    let angle_s = hash_to_unit(seed, 4) * core::f32::consts::TAU;
    let (dx_s, dz_s) = (libm::cosf(angle_s), libm::sinf(angle_s));
    let synoptic = libm::sinf(k_syn * (x_m * dx_s + z_m * dz_s) + phase_s);

    let rh = BASELINE_RH + REGIONAL_AMPLITUDE * regional + SYNOPTIC_AMPLITUDE * synoptic;
    rh.clamp(0.0, 1.0)
}

/// Relative humidity at world `(x, y_altitude, z)` at `time` seconds. `[0, 1]`.
#[must_use]
pub fn humidity_at(master_seed: u64, x_m: f32, y_altitude_m: f32, z_m: f32, time_s: f32) -> f32 {
    let rh_sea = sea_level_humidity(master_seed, x_m, z_m, time_s);
    let altitude = y_altitude_m.max(0.0);
    let rh_at_alt = rh_sea * libm::expf(-altitude / HUMIDITY_SCALE_HEIGHT_M);
    rh_at_alt.clamp(0.0, 1.0)
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
fn hash_to_unit(seed: u64, tag: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(tag);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    ((x >> 40) as f32) / ((1u32 << 24) as f32)
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn humidity_in_unit_range() {
        let seed = 42;
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 20_000.0;
                let z = (iz as f32 - 10.0) * 20_000.0;
                let rh = sea_level_humidity(seed, x, z, 0.0);
                assert!(
                    (0.0..=1.0).contains(&rh),
                    "RH {rh} at ({x}, {z}) outside [0,1]"
                );
            }
        }
    }

    #[test]
    fn humidity_decreases_with_altitude() {
        let seed = 42;
        let rh_0 = humidity_at(seed, 0.0, 0.0, 0.0, 0.0);
        let rh_5k = humidity_at(seed, 0.0, 5_000.0, 0.0, 0.0);
        let rh_10k = humidity_at(seed, 0.0, 10_000.0, 0.0, 0.0);
        assert!(rh_5k < rh_0);
        assert!(rh_10k < rh_5k);
    }

    #[test]
    fn humidity_deterministic() {
        let seed = 12_345;
        let rh1 = sea_level_humidity(seed, 1_000.0, 2_000.0, 100.0);
        let rh2 = sea_level_humidity(seed, 1_000.0, 2_000.0, 100.0);
        assert_eq!(rh1.to_bits(), rh2.to_bits());
    }

    #[test]
    fn different_seeds_produce_different_patterns() {
        let rh1 = sea_level_humidity(1, 50_000.0, 50_000.0, 0.0);
        let rh2 = sea_level_humidity(2, 50_000.0, 50_000.0, 0.0);
        assert_ne!(rh1.to_bits(), rh2.to_bits());
    }
}
