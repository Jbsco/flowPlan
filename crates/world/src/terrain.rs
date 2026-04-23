//! Procedural heightmap.
//!
//! Phase 3.6.0 terrain is **Perlin-noise fractional Brownian motion (fBm)**:
//! six octaves of 2D Perlin noise at halving spacing and halving amplitude,
//! summed for naturally-varied landscape. Toroidal-wrap-continuous by
//! construction via periodic gradient-grid lookup.
//!
//! Previous Phase 1 terrain was sum-of-sines with seeded phases — produced
//! visible diamond-lattice patterns at octave boundaries. Perlin noise
//! eliminates that periodic structure: gradients are pseudo-random per
//! grid cell with no underlying symmetry, so terrain looks organic.
//!
//! Per-biome parameterization and specialized noise primitives (ridged
//! multifractal for alpine / mesa crests, domain warping for organic
//! coastlines) are reserved for Phase 3.6.1. Formulas, parameters, and
//! per-biome application rules are specified at
//! [`MAPGEN.md § Layer 1 noise primitives (reference)`](../../../MAPGEN.md#layer-1-noise-primitives-reference)
//! — that section is the spec for 3.6.1's biome noise catalog.
//!
//! Current Phase 3.6.0 terrain is uniform — one `FbmConfig::DEFAULT`
//! world-wide.
//!
//! # Determinism
//!
//! Given the same `master_seed`, the heightfield is bit-identical on every
//! machine. This is Invariant #8 from `DESIGN.md` extended to terrain.
//! f32 precision in world-coord → sample-space conversion accumulates ~ULPs
//! at the 5000 km world scale — seam continuity is maintained to within ~1 m
//! (well below visual threshold against ±275 m peak amplitudes).

use crate::noise::fbm_2d;

/// A sampleable height function.
pub trait HeightSampler: Send + Sync {
    /// World-space height at `(x, z)`.
    fn height(&self, x: f32, z: f32) -> f32;
}

// Noise primitives are in `crate::noise` — Perlin-fBm shared between
// terrain and weather subsystems. Terrain uses octave-offset ranges
// `[0..100)` for the regional layer and `[100..200)` for the fine layer;
// weather and biome layers use disjoint offset ranges per the convention
// in `crate::noise` docs.

/// fBm terrain configuration: two-layer regional + fine composition
/// per [`MAPGEN.md § Altitude envelope`](../../../MAPGEN.md#altitude-envelope--real-world-possible-gate)
/// "regional trend + fine variation" pattern. Regional layer sets
/// continental-scale mean altitude (mountain ranges, basins); fine layer
/// adds local variation (hills, valleys).
#[derive(Clone, Copy, Debug)]
struct FbmConfig {
    /// Regional layer: broad mountain-range scale.
    regional_spacing_m: f32,
    regional_octaves: u32,
    regional_amplitude_m: f32,
    /// Fine layer: local terrain detail.
    fine_spacing_m: f32,
    fine_octaves: u32,
    fine_amplitude_m: f32,
}

impl FbmConfig {
    const DEFAULT: Self = Self {
        // Regional: 100 km → 12.5 km at octave 3. Continental mountain
        // range scale. 100,000 divides WORLD_EXTENT_M = 5,000,000 evenly
        // (50 cells at octave 0).
        regional_spacing_m: 100_000.0,
        regional_octaves: 4,
        regional_amplitude_m: 200.0,
        // Fine: 2 km → 62 m at octave 5. Local hills + surface texture.
        // 2000 divides 5,000,000 evenly (2500 cells at octave 0).
        // Bumped from 80 to 150 m for visible hills at 10 km player
        // view — 80 m was being swallowed by beach_flatten.
        fine_spacing_m: 2000.0,
        fine_octaves: 6,
        fine_amplitude_m: 150.0,
    };

    /// Theoretical max height. Sum of both layers' worst-case peaks.
    #[allow(clippy::cast_possible_wrap)] // octave counts are small (≤16)
    fn theoretical_max(self) -> f32 {
        let regional_sum: f32 = (0..self.regional_octaves)
            .map(|i| 0.5_f32.powi(i as i32))
            .sum();
        let fine_sum: f32 = (0..self.fine_octaves)
            .map(|i| 0.5_f32.powi(i as i32))
            .sum();
        // 2D Perlin peak ≈ 1/√2 ≈ 0.7071; 1.2× fudge for correlated-octave
        // cases that slightly exceed per-octave bounds.
        (self.regional_amplitude_m * regional_sum + self.fine_amplitude_m * fine_sum)
            * core::f32::consts::FRAC_1_SQRT_2
            * 1.2
    }
}

/// Procedural heightmap from a seed. See module docs.
#[derive(Clone, Debug)]
pub struct Terrain {
    seed: u64,
    config: FbmConfig,
    /// Constant offset applied to every height query. Zero for `from_seed()`
    /// (terrain centered around y=0); negative for `flat()` to put the visible
    /// terrain below the water surface.
    base_height: f32,
    /// Amplitude scaling factor. 1.0 for normal terrain; 0.0 for `flat*`.
    amplitude_scale: f32,
}

impl Terrain {
    /// Build a seeded procedural terrain using Perlin-fBm noise.
    ///
    /// Default configuration produces ~±275 m peak amplitudes with natural
    /// variation across six octaves. Per-biome parameterization lands in
    /// Phase 3.6.1 per `MAPGEN.md § Layer 1`.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        Self {
            seed,
            config: FbmConfig::DEFAULT,
            base_height: 0.0,
            amplitude_scale: 1.0,
        }
    }

    /// Flat terrain at a constant depth below the water surface. Used for
    /// scenario-bench visual playback of float-configuration scenarios —
    /// water surface at y=0, terrain mesh 10 m below.
    #[must_use]
    pub const fn flat() -> Self {
        Self {
            seed: 0,
            config: FbmConfig::DEFAULT,
            base_height: -10.0,
            amplitude_scale: 0.0,
        }
    }

    /// Flat terrain at y=0 — the "ground" sibling of [`Terrain::flat`].
    /// Used for scenario-bench visual playback of wheels-configuration
    /// scenarios so the gear rests on a visible solid surface.
    #[must_use]
    pub const fn flat_land() -> Self {
        Self {
            seed: 0,
            config: FbmConfig::DEFAULT,
            base_height: 0.0,
            amplitude_scale: 0.0,
        }
    }

    /// The seed this terrain was built from.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The maximum height achievable (theoretical peak of the fBm sum
    /// scaled by `amplitude_scale`, plus `base_height`). Used for safe
    /// spawn-altitude computation in the app.
    #[must_use]
    pub fn max_height(&self) -> f32 {
        self.base_height + self.amplitude_scale * self.config.theoretical_max()
    }
}

/// Width of the beach-flattening zone, in meters. Terrain heights within
/// ± this distance from mean sea level get compressed toward zero so
/// shores rise gently out of the water before the steeper octaves take
/// over.
///
/// Reduced from 120 → 20 in 3.6.0: the old width was calibrated against
/// the sharp-gradient sum-of-sines terrain where 120 m of vertical
/// flatten-zone produced the right horizontal shore width. Perlin-fBm
/// has much gentler local gradients, so 120 m was over-compressing
/// tens of kilometers of rolling terrain down to near-zero heights.
/// 20 m only flattens the immediate shoreline; real hydrology-aware
/// shore classification lands in 3.6.1d.
const BEACH_WIDTH_M: f32 = 20.0;

/// `raw * smoothstep(0, width, |raw|)^2`. Below `width` heights are
/// compressed with a cubic curve; at or above `width` the identity
/// function applies. Sign preserved.
fn beach_flatten(raw: f32, width: f32) -> f32 {
    let a = raw.abs();
    let t = (a / width).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    raw * smooth * smooth
}

impl HeightSampler for Terrain {
    fn height(&self, x: f32, z: f32) -> f32 {
        // Two-layer composition: regional + fine. Offset 100 on fine
        // layer's octave index prevents hash collisions with regional
        // layer — fine octave 0 ≠ regional octave 0.
        let regional = self.config.regional_amplitude_m
            * fbm_2d(
                self.seed,
                0,
                x,
                z,
                self.config.regional_spacing_m,
                self.config.regional_octaves,
            );
        let fine = self.config.fine_amplitude_m
            * fbm_2d(
                self.seed,
                100,
                x,
                z,
                self.config.fine_spacing_m,
                self.config.fine_octaves,
            );
        let raw = self.amplitude_scale * (regional + fine);
        beach_flatten(raw, BEACH_WIDTH_M) + self.base_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_height() {
        let a = Terrain::from_seed(42);
        let b = Terrain::from_seed(42);
        let samples: [(f32, f32); 4] = [(0.0, 0.0), (10.0, -5.0), (-25.0, 25.0), (100.0, -100.0)];
        for (x, z) in samples {
            assert!((a.height(x, z) - b.height(x, z)).abs() < 1e-6);
        }
    }

    #[test]
    fn different_seed_different_height() {
        let a = Terrain::from_seed(1);
        let b = Terrain::from_seed(2);
        // Perlin noise is exactly 0 at all integer grid intersections
        // (including the origin) by construction, so we sample at an
        // interior point where gradients fully contribute.
        let (x, z) = (543.2, -789.1);
        assert!((a.height(x, z) - b.height(x, z)).abs() > 1e-6);
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn max_height_bounds_samples() {
        let t = Terrain::from_seed(0xA1B2_C3D4);
        let max = t.max_height();
        for ix in -10_i32..=10 {
            for iz in -10_i32..=10 {
                let h = t.height(ix as f32 * 50.0, iz as f32 * 50.0);
                assert!(h.abs() <= max + 1e-5, "h={h} max={max} at ({ix},{iz})");
            }
        }
    }

    #[test]
    fn seed_accessor_round_trips() {
        let t = Terrain::from_seed(0xDEAD);
        assert_eq!(t.seed(), 0xDEAD);
    }

    #[test]
    fn flat_water_sits_below_y_zero() {
        let t = Terrain::flat();
        let heights = [(0.0, 0.0), (500.0, -500.0), (-100.0, 100.0)];
        for (x, z) in heights {
            let h = t.height(x, z);
            assert!(
                (h + 10.0).abs() < f32::EPSILON,
                "flat water terrain should be y=-10 everywhere; got {h} at ({x}, {z})",
            );
        }
    }

    #[test]
    fn flat_land_sits_at_y_zero() {
        let t = Terrain::flat_land();
        let heights = [(0.0, 0.0), (500.0, -500.0), (-100.0, 100.0)];
        for (x, z) in heights {
            let h = t.height(x, z);
            assert!(
                h.abs() < f32::EPSILON,
                "flat land terrain should be y=0 everywhere; got {h} at ({x}, {z})",
            );
        }
    }

    // Note: Perlin-periodicity tests moved to `crate::noise::tests`.
}
