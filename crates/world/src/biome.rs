//! Biome classification (Phase 3.6.1a).
//!
//! Assigns one of the [`BiomeClass`] labels to each world coordinate based on
//! **climate only** — sea-level temperature + humidity + a seed-derived
//! continental-scale latitude proxy. Classification is **intentionally
//! independent of terrain elevation** so it can drive Phase 3.6.1b's
//! biome-parameterized terrain generation without circular dependency:
//! the classifier says "this region should be alpine" purely from climate,
//! then 3.6.1b produces sharp high-altitude terrain THERE because the
//! region is tagged alpine. Terrain shape reflects biome assignment, not
//! the other way around.
//!
//! Pure function of `(master_seed, x, z)` — deterministic per seed.
//! Feeds Phase 3.6.1b's biome-parameterized terrain generation, Phase
//! 3.6.2's sub-biome feature libraries, and Phase 4 mission-routing
//! biome constraints.
//!
//! # Biome catalog (Phase 3.6.0 pinned set)
//!
//! Four biomes ship initially, picked to span the extremes of the MAPGEN
//! altitude-envelope table without requiring the full 8-biome catalog:
//!
//! - [`BiomeClass::HighDesert`] — Colorado Plateau analog. Warm, arid, mid-
//!   altitude plateaus with mesa/canyon geometry (3.6.1b will apply ridged
//!   multifractal here).
//! - [`BiomeClass::RockyAlpine`] — Colorado Rockies analog. Cold + high-
//!   altitude mountains with sharp crests (3.6.1b: ridged multifractal +
//!   steep gradients).
//! - [`BiomeClass::CoastalBoreal`] — Alaska panhandle analog. Cool, very
//!   humid, low-altitude coastline (3.6.1b: domain-warped terrain for
//!   fjord-like coastlines).
//! - [`BiomeClass::MediterraneanArchipelago`] — Aegean analog. Warm,
//!   moderate humidity, low-altitude with island clusters (3.6.1b: domain
//!   warping for varied island shapes).
//!
//! Additional biomes (Canadian Boreal, Pacific Northwest, Subarctic,
//! Tropical Island) fill out the catalog in later milestones once the
//! classification pattern is validated. See [`MAPGEN.md § Altitude envelope`](../../../MAPGEN.md#altitude-envelope--real-world-possible-gate)
//! for the full catalog and real-world references.
//!
//! # Adjacency rules
//!
//! Forbidden biome transitions per [`MAPGEN.md § Biome system`](../../../MAPGEN.md#biome-system):
//! no direct snow→desert, no direct snow→tropical. The current threshold-
//! based classifier happens to keep these apart because the intervening
//! climate bands (e.g. cold-but-not-snow, warm-but-not-desert) land as
//! `CoastalBoreal` or `MediterraneanArchipelago`. The classifier doesn't
//! explicitly enforce adjacency; if the visual-bench output reveals
//! violations, add a post-classification smoothing step.

use crate::weather::{humidity, temperature};

/// Climate-based biome label. See module docs for the catalog rationale.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BiomeClass {
    /// Warm + arid + mid-altitude plateau. Colorado Plateau analog.
    HighDesert,
    /// Cold + high-altitude + moderate-humidity mountains. Colorado Rockies analog.
    RockyAlpine,
    /// Cool + very humid + low-altitude coastal. Alaska panhandle analog.
    CoastalBoreal,
    /// Warm + moderate humidity + low-altitude island cluster. Aegean analog.
    MediterraneanArchipelago,
}

/// Temperature thresholds (°C) at sea level.
///
/// Alpine below this = cold enough to be "snowy mountains" climate (we
/// use the sea-level temp so climate is terrain-independent; terrain
/// generation in 3.6.1b will raise peaks IN alpine regions, and lapse
/// rate automatically makes them colder at altitude). Hot threshold =
/// arid-regime onset.
///
/// Current temperature field: baseline 15°C ± 12K climate + ±3K
/// synoptic + ±5K diurnal. Thresholds at +10°C (alpine) and +20°C (hot)
/// are symmetric around baseline — each covers roughly the colder /
/// warmer 20% of world area, leaving the middle ~60% for Mediterranean
/// or `CoastalBoreal` depending on humidity.
const ALPINE_TEMP_C: f32 = 10.0;
const HOT_TEMP_C: f32 = 20.0;

/// Humidity thresholds (relative humidity fraction). Desert-dry and
/// coastal-moist cutoffs. The gap between `DESERT_HUMIDITY_CEILING` and
/// `COASTAL_HUMIDITY_FLOOR` is the "moderate" band that lands as
/// Mediterranean.
const DESERT_HUMIDITY_CEILING: f32 = 0.40;
const COASTAL_HUMIDITY_FLOOR: f32 = 0.55;

/// Kelvin-to-Celsius offset. Subtract from Kelvin-returning APIs.
const KELVIN_TO_CELSIUS: f32 = 273.15;

/// Edge softness (°C / humidity units) for soft biome classification.
/// Within ±this distance of a threshold, biome membership transitions
/// smoothly rather than hard-switching. 2°C temperature + 0.05 humidity
/// match the natural variance of the fBm fields so transitions are
/// perceptible but not abrupt.
const EDGE_SOFTNESS_TEMP_C: f32 = 2.0;
const EDGE_SOFTNESS_HUMIDITY: f32 = 0.05;

/// Soft biome membership weights — each field in `[0, 1]`, sum to 1.0.
///
/// Used by 3.6.1b per-biome terrain blending: terrain at `(x, z)` is
/// `Σ weight_i × biome_primitive_i(x, z)`, producing smooth biome
/// transitions without visible seams.
#[derive(Copy, Clone, Debug, Default)]
pub struct BiomeWeights {
    pub high_desert: f32,
    pub rocky_alpine: f32,
    pub coastal_boreal: f32,
    pub mediterranean: f32,
}

impl BiomeWeights {
    /// Sum of all weights (should be ~1.0 by normalization).
    #[must_use]
    pub fn sum(&self) -> f32 {
        self.high_desert + self.rocky_alpine + self.coastal_boreal + self.mediterranean
    }
}

/// Smoothstep from 0 to 1 as `x` goes from `edge_lo` to `edge_hi`.
fn smoothstep(edge_lo: f32, edge_hi: f32, x: f32) -> f32 {
    let t = ((x - edge_lo) / (edge_hi - edge_lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Soft biome membership weights at `(x, z)`.
///
/// Matches the `biome_at` decision tree but with smoothstep-softened
/// thresholds so weights transition smoothly across biome boundaries.
/// Always returns normalized weights summing to 1.0.
///
/// Used by 3.6.1b terrain blending. The discrete [`biome_at`]
/// classifier is retained for L2 feature-type assignment where a
/// single biome per cell is needed.
#[must_use]
pub fn biome_weights_at(master_seed: u64, x_m: f32, z_m: f32) -> BiomeWeights {
    let temp_k = temperature::sea_level_temperature(master_seed, x_m, z_m, 0.0);
    let temp_c = temp_k - KELVIN_TO_CELSIUS;
    let hum = humidity::sea_level_humidity(master_seed, x_m, z_m, 0.0);

    // Cold membership: 1 when very cold, 0 when warm-enough. Smoothed
    // across ±EDGE_SOFTNESS_TEMP_C around the alpine threshold.
    let cold = smoothstep(
        ALPINE_TEMP_C + EDGE_SOFTNESS_TEMP_C,
        ALPINE_TEMP_C - EDGE_SOFTNESS_TEMP_C,
        temp_c,
    );
    // Hot membership: 1 when hot, 0 when moderate.
    let hot = smoothstep(
        HOT_TEMP_C - EDGE_SOFTNESS_TEMP_C,
        HOT_TEMP_C + EDGE_SOFTNESS_TEMP_C,
        temp_c,
    );
    // Dry membership: 1 when dry, 0 when moist.
    let dry = smoothstep(
        DESERT_HUMIDITY_CEILING + EDGE_SOFTNESS_HUMIDITY,
        DESERT_HUMIDITY_CEILING - EDGE_SOFTNESS_HUMIDITY,
        hum,
    );
    // Humid membership: 1 when moist, 0 when dry.
    let humid = smoothstep(
        COASTAL_HUMIDITY_FLOOR - EDGE_SOFTNESS_HUMIDITY,
        COASTAL_HUMIDITY_FLOOR + EDGE_SOFTNESS_HUMIDITY,
        hum,
    );

    // Compose per the discrete decision tree, but preserving partial membership:
    //   alpine = cold (any humidity, overrides desert/coastal)
    //   desert = hot × dry × not-alpine
    //   coastal = humid × not-alpine × not-desert
    //   mediterranean = everything else (default / moderate climate)
    let alpine_w = cold;
    let desert_w = hot * dry * (1.0 - alpine_w);
    let coastal_w = humid * (1.0 - alpine_w) * (1.0 - desert_w);
    let mediterranean_w = (1.0 - alpine_w - desert_w - coastal_w).max(0.0);

    // Normalize (defensive — should sum to ≤1 by construction but clamp)
    let sum = alpine_w + desert_w + coastal_w + mediterranean_w;
    let inv_sum = if sum > 1e-6 { 1.0 / sum } else { 1.0 };

    BiomeWeights {
        rocky_alpine: alpine_w * inv_sum,
        high_desert: desert_w * inv_sum,
        coastal_boreal: coastal_w * inv_sum,
        mediterranean: mediterranean_w * inv_sum,
    }
}

/// Biome class at world coordinate `(x, z)`.
///
/// Pure function of climate fields (sea-level temperature + humidity) —
/// **intentionally independent of terrain elevation** so 3.6.1b's
/// biome-parameterized terrain can shape elevation TO MATCH the biome
/// assignment without circular dependency. Classifier says "this region
/// should be alpine"; 3.6.1b makes it so by applying ridged multifractal
/// + higher amplitude there.
///
/// Time parameter is 0.0 (classification is static — biomes don't drift
/// with weather evolution).
///
/// # Determinism
///
/// Pure function of `(master_seed, x, z)`. Bit-identical on every machine
/// given the same inputs.
#[must_use]
pub fn biome_at(master_seed: u64, x_m: f32, z_m: f32) -> BiomeClass {
    // Sea-level climate (altitude=0) — terrain-independent. Temperature
    // API returns Kelvin; convert to Celsius for comparison with thresholds.
    let temp_k = temperature::sea_level_temperature(master_seed, x_m, z_m, 0.0);
    let temp_c = temp_k - KELVIN_TO_CELSIUS;
    let hum = humidity::sea_level_humidity(master_seed, x_m, z_m, 0.0);

    // Decision tree: ordered from most specific to most permissive.
    //   1. Cold → alpine (snowy mountains climate).
    //   2. Hot + arid → desert.
    //   3. Humid + not-cold → coastal boreal.
    //   4. Default → Mediterranean archipelago (moderate climate).
    if temp_c < ALPINE_TEMP_C {
        BiomeClass::RockyAlpine
    } else if temp_c > HOT_TEMP_C && hum < DESERT_HUMIDITY_CEILING {
        BiomeClass::HighDesert
    } else if hum > COASTAL_HUMIDITY_FLOOR {
        BiomeClass::CoastalBoreal
    } else {
        BiomeClass::MediterraneanArchipelago
    }
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)] // test grid iteration: i32→f32 cast is idiomatic
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_inputs() {
        for &(x, z) in &[
            (0.0, 0.0),
            (12345.6, -9876.5),
            (500_000.0, 250_000.0),
            (-100_000.0, 100_000.0),
        ] {
            let a = biome_at(42, x, z);
            let b = biome_at(42, x, z);
            assert_eq!(a, b, "biome_at not deterministic at ({x}, {z})");
        }
    }

    #[test]
    fn classification_covers_multiple_biomes() {
        // Over a wide sample grid, expect multiple biomes to appear — the
        // classifier must actually discriminate. With sea-level climate
        // as the sole input + world-spanning synoptic variation, 2-4
        // biomes should appear across a 1000 km × 1000 km sample.
        use std::collections::HashSet;
        let mut seen: HashSet<BiomeClass> = HashSet::new();
        for ix in 0..40 {
            for iz in 0..40 {
                let x = (ix as f32 - 20.0) * 25_000.0;
                let z = (iz as f32 - 20.0) * 25_000.0;
                seen.insert(biome_at(42, x, z));
            }
        }
        assert!(
            seen.len() >= 2,
            "classifier produced only {} distinct biome(s): {:?}",
            seen.len(),
            seen
        );
    }

    #[test]
    fn biome_is_climate_independent_of_terrain() {
        // Classification depends only on (master_seed, x, z) via climate
        // fields. No terrain reference — verify the public API compiles
        // and behaves consistently without a terrain parameter.
        let a = biome_at(42, 0.0, 0.0);
        let b = biome_at(42, 0.0, 0.0);
        assert_eq!(a, b);
    }

    #[test]
    fn biome_weights_sum_to_one() {
        // Normalization invariant: sum of all weights ≈ 1 everywhere.
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 50_000.0;
                let z = (iz as f32 - 10.0) * 50_000.0;
                let w = biome_weights_at(42, x, z);
                assert!(
                    (w.sum() - 1.0).abs() < 1e-4,
                    "weights sum to {} at ({x}, {z}), want 1.0: {w:?}",
                    w.sum()
                );
            }
        }
    }

    #[test]
    fn biome_weights_all_nonnegative() {
        for ix in 0..10 {
            for iz in 0..10 {
                let x = (ix as f32) * 100_000.0;
                let z = (iz as f32) * 100_000.0;
                let w = biome_weights_at(42, x, z);
                assert!(w.rocky_alpine >= 0.0);
                assert!(w.high_desert >= 0.0);
                assert!(w.coastal_boreal >= 0.0);
                assert!(w.mediterranean >= 0.0);
            }
        }
    }

    #[test]
    fn biome_weights_span_multiple_biomes_over_world() {
        // Across a wide-range sample, each of the 4 biomes should attain
        // non-negligible weight SOMEWHERE. Proves the weight function
        // discriminates rather than collapsing to one biome.
        let mut max_alpine = 0.0_f32;
        let mut max_desert = 0.0_f32;
        let mut max_coastal = 0.0_f32;
        let mut max_mediterranean = 0.0_f32;
        for ix in 0..50 {
            for iz in 0..50 {
                let x = (ix as f32 - 25.0) * 100_000.0;
                let z = (iz as f32 - 25.0) * 100_000.0;
                let w = biome_weights_at(42, x, z);
                max_alpine = max_alpine.max(w.rocky_alpine);
                max_desert = max_desert.max(w.high_desert);
                max_coastal = max_coastal.max(w.coastal_boreal);
                max_mediterranean = max_mediterranean.max(w.mediterranean);
            }
        }
        assert!(
            max_alpine > 0.5,
            "alpine never substantial: max={max_alpine}"
        );
        assert!(
            max_desert > 0.3,
            "desert never substantial: max={max_desert}"
        );
        assert!(
            max_coastal > 0.5,
            "coastal never substantial: max={max_coastal}"
        );
        assert!(
            max_mediterranean > 0.3,
            "mediterranean never substantial: max={max_mediterranean}"
        );
    }
}
