//! Atmospheric temperature field.
//!
//! Pure function of `(master_seed, world_xz, altitude, time)`.
//!
//! # Model
//!
//! Sea-level temperature = `BASELINE_KELVIN` + latitude-band gradient (real
//! world latitude would be the driver; here we synthesize a slow varying
//! "climate-gradient" field since flowPlan doesn't track latitude explicitly
//! at the world-bake level) + synoptic variation + seasonal + diurnal.
//! Altitude lapse rate applies ISA standard ~6.5 K / km troposphere.
//!
//! # Characteristic scale
//!
//! Climate-gradient wavelength ≈ 500 km (slow continental-scale variation).
//! Synoptic variation at ~300 km (same scale as pressure, since temperature +
//! pressure correlate at synoptic scales). Diurnal cycle period 1 day.

use super::{field_seed, weather_seed};

/// ISA baseline sea-level temperature (K) — 15 °C = 288.15 K.
pub const BASELINE_KELVIN: f32 = 288.15;

/// Climate-gradient amplitude (K peak-to-peak is ±this from baseline).
/// Real continental climate variation is typically ±15 K seasonal, ±5 K daily.
pub const CLIMATE_AMPLITUDE_K: f32 = 12.0;

/// Diurnal amplitude (K peak-to-peak is ±this).
pub const DIURNAL_AMPLITUDE_K: f32 = 5.0;

/// Synoptic variation amplitude (K).
pub const SYNOPTIC_AMPLITUDE_K: f32 = 3.0;

/// Climate gradient wavelength (m). Continental-scale slow variation.
pub const CLIMATE_WAVELENGTH_M: f32 = 500_000.0;

/// Synoptic wavelength (m) — same scale as pressure for coupled variation.
/// Quantized to `WORLD_EXTENT_M / 14` ≈ 357.14 km for clean toroidal-seam
/// wrap (originally 350 km; 2.04% adjustment).
pub const SYNOPTIC_WAVELENGTH_M: f32 = flowplan_core::WORLD_EXTENT_M / 14.0;

/// ISA tropospheric lapse rate (K / m). Standard ~6.5 K / km.
pub const LAPSE_RATE_K_PER_M: f32 = 0.006_5;

/// Day length (s) — placeholder; real time-of-day lives in `SimState`. Used
/// here to synthesize a diurnal cycle when bench-testing.
pub const DAY_LENGTH_S: f32 = 24.0 * 3_600.0;

const TEMPERATURE_FIELD_TAG: u64 = u64::from_le_bytes(*b"tempera_");

/// Sea-level temperature at world `(x, z)` at `time` seconds. Kelvin.
#[must_use]
pub fn sea_level_temperature(master_seed: u64, x_m: f32, z_m: f32, time_s: f32) -> f32 {
    let seed = field_seed(weather_seed(master_seed), TEMPERATURE_FIELD_TAG);

    // Climate-gradient component (slow continental scale).
    let k_climate = core::f32::consts::TAU / CLIMATE_WAVELENGTH_M;
    let phase_c = hash_to_unit(seed, 1) * core::f32::consts::TAU;
    let angle_c = hash_to_unit(seed, 2) * core::f32::consts::TAU;
    let (dx_c, dz_c) = (libm::cosf(angle_c), libm::sinf(angle_c));
    let climate = libm::sinf(k_climate * (x_m * dx_c + z_m * dz_c) + phase_c);

    // Synoptic component (faster, smaller).
    let k_syn = core::f32::consts::TAU / SYNOPTIC_WAVELENGTH_M;
    let phase_s = hash_to_unit(seed, 3) * core::f32::consts::TAU;
    let angle_s = hash_to_unit(seed, 4) * core::f32::consts::TAU;
    let (dx_s, dz_s) = (libm::cosf(angle_s), libm::sinf(angle_s));
    let synoptic = libm::sinf(k_syn * (x_m * dx_s + z_m * dz_s) + phase_s);

    // Diurnal component — centered on noon (t = 12 h).
    let diurnal_phase = core::f32::consts::TAU * (time_s / DAY_LENGTH_S - 0.25);
    let diurnal = libm::sinf(diurnal_phase);

    BASELINE_KELVIN
        + CLIMATE_AMPLITUDE_K * climate
        + SYNOPTIC_AMPLITUDE_K * synoptic
        + DIURNAL_AMPLITUDE_K * diurnal
}

/// Temperature at world `(x, y_altitude, z)` at simulation `time` seconds.
/// Applies ISA tropospheric lapse rate on top of the sea-level field.
#[must_use]
pub fn temperature_at(master_seed: u64, x_m: f32, y_altitude_m: f32, z_m: f32, time_s: f32) -> f32 {
    let t_sea = sea_level_temperature(master_seed, x_m, z_m, time_s);
    let altitude = y_altitude_m.max(0.0);
    t_sea - LAPSE_RATE_K_PER_M * altitude
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
    fn sea_level_temperature_in_plausible_range() {
        // Sample a 500 km grid; all temperatures should fall within a realistic
        // continental range.
        let seed = 42;
        let max_expected = BASELINE_KELVIN
            + CLIMATE_AMPLITUDE_K
            + SYNOPTIC_AMPLITUDE_K
            + DIURNAL_AMPLITUDE_K
            + 0.1;
        let min_expected = BASELINE_KELVIN
            - CLIMATE_AMPLITUDE_K
            - SYNOPTIC_AMPLITUDE_K
            - DIURNAL_AMPLITUDE_K
            - 0.1;
        for ix in 0..25 {
            for iz in 0..25 {
                let x = (ix as f32 - 12.5) * 20_000.0;
                let z = (iz as f32 - 12.5) * 20_000.0;
                let t = sea_level_temperature(seed, x, z, 12.0 * 3_600.0);
                assert!(
                    (min_expected..=max_expected).contains(&t),
                    "temperature {t} K at ({x}, {z}) outside expected range"
                );
            }
        }
    }

    #[test]
    fn temperature_decreases_with_altitude() {
        let seed = 42;
        let t_0 = temperature_at(seed, 0.0, 0.0, 0.0, 0.0);
        let t_3k = temperature_at(seed, 0.0, 3_000.0, 0.0, 0.0);
        let t_10k = temperature_at(seed, 0.0, 10_000.0, 0.0, 0.0);

        // ISA: 3 km → -19.5 K; 10 km → -65 K. Within a few K.
        assert!(t_3k < t_0);
        assert!(t_10k < t_3k);
        assert!((t_0 - t_3k - 19.5).abs() < 0.5);
        assert!((t_0 - t_10k - 65.0).abs() < 0.5);
    }

    #[test]
    fn diurnal_cycle_peaks_at_noon() {
        let seed = 42;
        let t_midnight = temperature_at(seed, 0.0, 0.0, 0.0, 0.0);
        let t_noon = temperature_at(seed, 0.0, 0.0, 0.0, 12.0 * 3_600.0);
        // Noon ≈ baseline + diurnal amplitude; midnight ≈ baseline - amplitude.
        assert!(
            t_noon > t_midnight,
            "noon ({t_noon}) should exceed midnight ({t_midnight})"
        );
    }

    #[test]
    fn sea_level_temperature_deterministic() {
        let seed = 12_345;
        let t1 = sea_level_temperature(seed, 1_000.0, 2_000.0, 100.0);
        let t2 = sea_level_temperature(seed, 1_000.0, 2_000.0, 100.0);
        assert_eq!(t1.to_bits(), t2.to_bits());
    }

    #[test]
    fn different_seeds_produce_different_patterns() {
        let t1 = sea_level_temperature(1, 50_000.0, 50_000.0, 0.0);
        let t2 = sea_level_temperature(2, 50_000.0, 50_000.0, 0.0);
        assert_ne!(t1.to_bits(), t2.to_bits());
    }

    #[test]
    fn altitude_clamp_negative_to_zero() {
        let seed = 42;
        let t_neg = temperature_at(seed, 0.0, -100.0, 0.0, 0.0);
        let t_zero = temperature_at(seed, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(t_neg.to_bits(), t_zero.to_bits());
    }
}
