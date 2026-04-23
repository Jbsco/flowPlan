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
///
/// Quantized to `WORLD_EXTENT_M / 14` ≈ 357.14 km for clean toroidal-seam
/// wrap (originally 350 km; 2.04% adjustment). Matches
/// `pressure::SYNOPTIC_WAVELENGTH_M` so synoptic-scale fields align across
/// pressure/temperature/humidity.
pub const SYNOPTIC_WAVELENGTH_M: f32 = flowplan_core::WORLD_EXTENT_M / 14.0;

/// Altitude scale-height for humidity decrease (m).
///
/// Real atmospheric absolute humidity drops off faster than pressure;
/// relative humidity often peaks near cloud base. Simplified here as an
/// exponential decrease.
pub const HUMIDITY_SCALE_HEIGHT_M: f32 = 5_000.0;

const HUMIDITY_FIELD_TAG: u64 = u64::from_le_bytes(*b"humidity");

/// Sea-level relative humidity at world `(x, z)` at `time` seconds. `[0, 1]`.
///
/// Perlin-fBm regional + synoptic layers (Phase 3.6.1a refactor).
/// Octave offsets `[220..230)` to avoid collision with terrain layers
/// `[0..200)` and temperature layers `[200..220)`.
#[must_use]
pub fn sea_level_humidity(master_seed: u64, x_m: f32, z_m: f32, _time_s: f32) -> f32 {
    let seed = field_seed(weather_seed(master_seed), HUMIDITY_FIELD_TAG);
    let regional = crate::noise::fbm_2d(seed, 220, x_m, z_m, REGIONAL_WAVELENGTH_M, 4);
    let synoptic = crate::noise::fbm_2d(seed, 225, x_m, z_m, SYNOPTIC_WAVELENGTH_M, 3);
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

/// Sea-level humidity with terrain coupling — orographic lift on windward
/// slopes, rain shadow on lee sides. Delegates to
/// [`super::orographic::terrain_moisture_modifier`] for the bias shape.
#[must_use]
pub fn sea_level_humidity_with_terrain<T: crate::terrain::HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let base = sea_level_humidity(master_seed, x_m, z_m, time_s);
    let modifier =
        super::orographic::terrain_moisture_modifier(master_seed, terrain, x_m, z_m, time_s);
    (base * modifier).clamp(0.0, 1.0)
}

/// Relative humidity at altitude with orographic terrain coupling applied.
/// Mirrors [`humidity_at`] structure; adds the terrain sampler parameter.
#[must_use]
pub fn humidity_at_with_terrain<T: crate::terrain::HeightSampler>(
    master_seed: u64,
    terrain: &T,
    x_m: f32,
    y_altitude_m: f32,
    z_m: f32,
    time_s: f32,
) -> f32 {
    let rh_sea = sea_level_humidity_with_terrain(master_seed, terrain, x_m, z_m, time_s);
    let altitude = y_altitude_m.max(0.0);
    let rh_at_alt = rh_sea * libm::expf(-altitude / HUMIDITY_SCALE_HEIGHT_M);
    rh_at_alt.clamp(0.0, 1.0)
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

    #[test]
    fn with_terrain_equals_base_on_flat_terrain() {
        // Flat terrain produces modifier = 1.0, so terrain-aware humidity
        // matches the flat-humidity path exactly (bit-identical).
        use crate::terrain::Terrain;
        let flat = Terrain::flat_land();
        let seed = 42;
        for ix in 0..10 {
            for iz in 0..10 {
                let x = (ix as f32 - 5.0) * 10_000.0;
                let z = (iz as f32 - 5.0) * 10_000.0;
                let base = sea_level_humidity(seed, x, z, 0.0);
                let with_terrain = sea_level_humidity_with_terrain(seed, &flat, x, z, 0.0);
                assert_eq!(base.to_bits(), with_terrain.to_bits());
            }
        }
    }

    #[test]
    fn with_terrain_differs_over_seeded_terrain() {
        // Over seeded terrain, at least some points should differ from the
        // flat-humidity path (proves the modifier is actually engaging).
        // Sampling range widened to 50 km × 50 km post 3.6.0 terrain rewrite:
        // new Perlin-fBm terrain has gentler local slopes than the old
        // sum-of-sines, so need broader area to guarantee the modifier
        // threshold gets crossed somewhere.
        use crate::terrain::Terrain;
        let terrain = Terrain::from_seed(42);
        let seed = 42;
        let mut any_diff = false;
        for ix in 0..50 {
            for iz in 0..50 {
                let x = (ix as f32 - 25.0) * 2_000.0;
                let z = (iz as f32 - 25.0) * 2_000.0;
                let base = sea_level_humidity(seed, x, z, 0.0);
                let with_terrain = sea_level_humidity_with_terrain(seed, &terrain, x, z, 0.0);
                if base.to_bits() != with_terrain.to_bits() {
                    any_diff = true;
                    break;
                }
            }
            if any_diff {
                break;
            }
        }
        assert!(any_diff, "terrain-aware humidity never diverged from base");
    }

    #[test]
    fn with_terrain_deterministic() {
        use crate::terrain::Terrain;
        let terrain = Terrain::from_seed(7);
        let seed = 42;
        let a = humidity_at_with_terrain(seed, &terrain, 123.0, 100.0, -456.0, 0.0);
        let b = humidity_at_with_terrain(seed, &terrain, 123.0, 100.0, -456.0, 0.0);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn with_terrain_stays_in_unit_range() {
        use crate::terrain::Terrain;
        let terrain = Terrain::from_seed(42);
        for ix in 0..30 {
            for iz in 0..30 {
                let x = (ix as f32 - 15.0) * 500.0;
                let z = (iz as f32 - 15.0) * 500.0;
                let rh = sea_level_humidity_with_terrain(42, &terrain, x, z, 0.0);
                assert!(
                    (0.0..=1.0).contains(&rh),
                    "terrain-aware RH {rh} at ({x}, {z}) outside [0, 1]"
                );
            }
        }
    }
}
