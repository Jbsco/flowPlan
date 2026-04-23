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

use crate::biome::{biome_weights_at, BiomeWeights};
use crate::noise::{
    domain_warped_fbm_2d_at_scale, fbm_2d_at_scale, ridged_multifractal_2d_at_scale,
};

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

// ═══════════════════════════════════════════════════════════════════════════
// Per-biome terrain profiles (Phase 3.6.1b)
// ═══════════════════════════════════════════════════════════════════════════

/// Noise primitive selection per [MAPGEN.md § Layer 1 noise primitives](../../../MAPGEN.md#layer-1-noise-primitives-reference).
#[derive(Copy, Clone, Debug)]
enum PrimitiveType {
    /// Standard Perlin-fBm (smooth rolling terrain).
    Fbm,
    /// Ridged multifractal (sharp crests + valleys). Used for alpine +
    /// mesa/canyon biomes.
    Ridged,
    /// Domain-warped fBm (organic twisted terrain). Used for coastlines
    /// + archipelagos where grid-alignment would be obvious.
    DomainWarped,
}

/// Per-biome terrain parameters. Each biome declares a primary primitive
/// + scale + amplitude + mean-altitude bias + a secondary fBm "background"
/// layer that adds rolling variation even in valleys of the primary
/// primitive (prevents flat-plateau artifacts where ridged multifractal
/// bottoms out to zero). At sample time the terrain module computes this
/// biome's contribution and blends all 4 by [`BiomeWeights`](crate::biome::BiomeWeights).
#[derive(Copy, Clone, Debug)]
struct BiomeProfile {
    /// Noise primitive: fBm, ridged, or domain-warped.
    primitive: PrimitiveType,
    /// Biome-primitive base spacing (m). Must divide `WORLD_EXTENT_M`.
    spacing_m: f32,
    /// Octave count for the primitive.
    octaves: u32,
    /// Primitive amplitude (m) — scales the noise output.
    amplitude_m: f32,
    /// Mean altitude bias (m) — biome's "typical altitude" added to its
    /// noise contribution. Alpine starts ~1500 m; coastal ~-100 m;
    /// desert plateaus ~800 m; mediterranean ~50 m.
    mean_altitude_m: f32,
    /// Octave-offset base (range `[500..700)` reserved for biome primitives
    /// — 20 slots per biome accommodates primary + warp-x + warp-z +
    /// background-fbm hash independence). Disjoint from terrain regional
    /// `[0..100)`, fine `[100..200)`, weather `[200..240)`, voronoi-weight
    /// `[400..500)`.
    octave_offset_base: u32,
    /// Ridged-specific: offset (1.0 typical) — sharpness of ridges.
    ridged_offset: f32,
    /// Ridged-specific: power (1.5-2.5 typical).
    ridged_power: f32,
    /// Domain-warp-specific: displacement amplitude (m).
    warp_amplitude_m: f32,
    /// Domain-warp-specific: warp field spacing (m). Should divide
    /// `WORLD_EXTENT_M`. Typically ~2× `spacing_m` so warp is smoother
    /// than the base signal.
    warp_spacing_m: f32,
    /// Background fBm amplitude (m) — rolling variation within the biome.
    /// Prevents flat-plateau artifacts in ridged biomes where the ridge
    /// function's valleys collapse to zero. For domain-warped biomes it
    /// just adds terrain micro-texture. Typical: 30-80 m.
    background_amplitude_m: f32,
    /// Background fBm spacing (m) — typical ~500-1500 m, below biome's
    /// primary spacing so background varies at smaller feature scale.
    background_spacing_m: f32,
}

impl BiomeProfile {
    /// High Desert / Plateau — ridged multifractal at moderate power
    /// (mesas + canyons, moderate altitude).
    const HIGH_DESERT: Self = Self {
        primitive: PrimitiveType::Ridged,
        spacing_m: 20_000.0, // 20 km mesa scale
        octaves: 5,
        amplitude_m: 400.0, // mesas rise 400 m above mean
        mean_altitude_m: 800.0, // Colorado Plateau baseline
        octave_offset_base: 500,
        ridged_offset: 1.0,
        ridged_power: 1.8, // moderate ridges — mesas not alpine-sharp
        warp_amplitude_m: 0.0,
        warp_spacing_m: 40_000.0,
        background_amplitude_m: 60.0, // rolling desert floor variation
        background_spacing_m: 1_000.0,
    };

    /// Rocky Alpine — ridged multifractal at high power (sharp peaks).
    const ROCKY_ALPINE: Self = Self {
        primitive: PrimitiveType::Ridged,
        spacing_m: 10_000.0, // 10 km individual mountain scale
        octaves: 5,
        amplitude_m: 900.0, // peaks rise 900 m above mean
        mean_altitude_m: 1500.0, // Colorado Rockies baseline
        octave_offset_base: 520,
        ridged_offset: 1.0,
        ridged_power: 2.5, // sharp peaks
        warp_amplitude_m: 0.0,
        warp_spacing_m: 20_000.0,
        background_amplitude_m: 80.0, // alpine valley + ridge flank rolling variation
        background_spacing_m: 800.0,
    };

    /// Coastal Boreal — domain-warped fBm (fjord-like organic coast).
    const COASTAL_BOREAL: Self = Self {
        primitive: PrimitiveType::DomainWarped,
        spacing_m: 10_000.0,
        octaves: 5,
        amplitude_m: 300.0, // moderate relief
        mean_altitude_m: -50.0, // sits near sea level (slight negative so fjords form)
        octave_offset_base: 540,
        ridged_offset: 1.0,
        ridged_power: 1.5,
        warp_amplitude_m: 3_000.0, // organic fjord distortion
        warp_spacing_m: 20_000.0,
        background_amplitude_m: 40.0, // coastal fjord wall texture
        background_spacing_m: 600.0,
    };

    /// Mediterranean Archipelago — strongly domain-warped fBm
    /// (varied island clusters).
    const MEDITERRANEAN: Self = Self {
        primitive: PrimitiveType::DomainWarped,
        spacing_m: 5_000.0, // 5 km island scale
        octaves: 5,
        amplitude_m: 200.0, // low islands
        mean_altitude_m: 20.0, // just above sea level
        octave_offset_base: 560,
        ridged_offset: 1.0,
        ridged_power: 1.5,
        warp_amplitude_m: 5_000.0, // strong warp for varied island shapes
        warp_spacing_m: 15_000.0,
        background_amplitude_m: 30.0, // small-island surface variation
        background_spacing_m: 400.0,
    };
}

/// Compute this biome's terrain contribution (m) at `(x, z)`, optionally
/// capping octaves at `min_spacing_m` (mipmap-aware). `min_spacing_m = 0`
/// means include all octaves.
///
/// Composition: `mean_altitude + amplitude × primary_primitive +
/// background_amplitude × fbm_background`. The background fBm prevents
/// flat-plateau artifacts in ridged biomes (where the ridge function's
/// valleys would otherwise collapse to exactly zero contribution,
/// producing perfectly flat regions at `mean_altitude` between peaks).
fn biome_contribution_at_scale(
    profile: &BiomeProfile,
    seed: u64,
    x: f32,
    z: f32,
    min_spacing_m: f32,
) -> f32 {
    let primitive_value = match profile.primitive {
        PrimitiveType::Fbm => fbm_2d_at_scale(
            seed,
            profile.octave_offset_base,
            x,
            z,
            profile.spacing_m,
            profile.octaves,
            min_spacing_m,
        ),
        PrimitiveType::Ridged => ridged_multifractal_2d_at_scale(
            seed,
            profile.octave_offset_base,
            x,
            z,
            profile.spacing_m,
            profile.octaves,
            profile.ridged_offset,
            profile.ridged_power,
            min_spacing_m,
        ),
        PrimitiveType::DomainWarped => domain_warped_fbm_2d_at_scale(
            seed,
            profile.octave_offset_base,
            x,
            z,
            profile.spacing_m,
            profile.octaves,
            profile.warp_amplitude_m,
            profile.warp_spacing_m,
            3,
            min_spacing_m,
        ),
    };
    // Background fBm adds rolling variation in valleys / flat areas.
    // Octave offset = base + 15 (reserves 15 slots for primary primitive
    // and any domain-warp sub-indices within the 20-slot biome range).
    let background_value = fbm_2d_at_scale(
        seed,
        profile.octave_offset_base + 15,
        x,
        z,
        profile.background_spacing_m,
        4,
        min_spacing_m,
    );
    profile.mean_altitude_m
        + profile.amplitude_m * primitive_value
        + profile.background_amplitude_m * background_value
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

impl Terrain {
    /// Sample height at `(x, z)` with mipmap-aware octave filtering.
    /// `min_spacing_m = 0.0` includes all octaves (matches
    /// [`HeightSampler::height`] exactly). Higher `min_spacing_m` skips
    /// octaves finer than that threshold — use when rendering at
    /// `pixel_size_m` resolution to avoid sub-pixel-frequency aliasing.
    ///
    /// Suggested: `min_spacing_m = pixel_size_m × 0.5` for anti-alias
    /// filtering (Nyquist-inspired). Used by contour-bench wide crops,
    /// Phase 3.6.2 LOD streaming chunks, hydrology bake (100 m grid),
    /// in-engine map view.
    #[must_use]
    pub fn height_at_scale(&self, x: f32, z: f32, min_spacing_m: f32) -> f32 {
        let regional = self.config.regional_amplitude_m
            * fbm_2d_at_scale(
                self.seed,
                0,
                x,
                z,
                self.config.regional_spacing_m,
                self.config.regional_octaves,
                min_spacing_m,
            );
        let fine = self.config.fine_amplitude_m
            * fbm_2d_at_scale(
                self.seed,
                100,
                x,
                z,
                self.config.fine_spacing_m,
                self.config.fine_octaves,
                min_spacing_m,
            );
        let baseline = regional + fine;

        let biome_contribution_total = if self.amplitude_scale > 0.0 {
            biome_blended_contribution_at_scale(self.seed, x, z, min_spacing_m)
        } else {
            0.0
        };

        let raw = self.amplitude_scale * (baseline + biome_contribution_total);
        beach_flatten(raw, BEACH_WIDTH_M) + self.base_height
    }
}

impl HeightSampler for Terrain {
    fn height(&self, x: f32, z: f32) -> f32 {
        // Phase 3.6.1b per-biome terrain composition:
        //
        //   height = baseline_regional_fbm(x, z)              (continental trend)
        //          + baseline_fine_fbm(x, z)                  (small-scale texture)
        //          + Σ biome_weight[i] × biome_contribution[i](x, z)
        //
        // The baseline layers provide biome-independent regional variation
        // so biome transitions don't produce flat-height seams. The
        // biome-weighted contributions shape mountains, mesas, coastlines,
        // and islands per-region. Soft biome weights (BiomeWeights sums to
        // 1.0, smoothstepped across ~2°C temperature transitions) ensure
        // smooth transitions at biome boundaries — no visible seams.
        //
        // Delegates to height_at_scale with min_spacing_m = 0.0 to share
        // the implementation. No extra cost — same loop, just without
        // the early-break.
        self.height_at_scale(x, z, 0.0)
    }
}

/// Compute the biome-weighted terrain contribution at `(x, z)`, with
/// mipmap-aware octave filtering. Queries [`biome_weights_at`] for soft
/// 4-biome membership, then evaluates each biome's primitive (capped at
/// `min_spacing_m`) and blends by weight.
fn biome_blended_contribution_at_scale(
    seed: u64,
    x: f32,
    z: f32,
    min_spacing_m: f32,
) -> f32 {
    let weights = biome_weights_at(seed, x, z);
    let mut total = 0.0_f32;
    if weights.high_desert > 1e-4 {
        total += weights.high_desert
            * biome_contribution_at_scale(&BiomeProfile::HIGH_DESERT, seed, x, z, min_spacing_m);
    }
    if weights.rocky_alpine > 1e-4 {
        total += weights.rocky_alpine
            * biome_contribution_at_scale(&BiomeProfile::ROCKY_ALPINE, seed, x, z, min_spacing_m);
    }
    if weights.coastal_boreal > 1e-4 {
        total += weights.coastal_boreal
            * biome_contribution_at_scale(&BiomeProfile::COASTAL_BOREAL, seed, x, z, min_spacing_m);
    }
    if weights.mediterranean > 1e-4 {
        total += weights.mediterranean
            * biome_contribution_at_scale(&BiomeProfile::MEDITERRANEAN, seed, x, z, min_spacing_m);
    }
    total
}

/// Silences unused-import warnings when the biome module is compiled
/// alone during tests. Real callers use `biome_weights_at` via the
/// `BiomeWeights` return type.
#[allow(dead_code)]
fn _unused_weights_check(w: BiomeWeights) -> f32 {
    w.high_desert + w.rocky_alpine + w.coastal_boreal + w.mediterranean
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

    #[test]
    fn height_at_scale_zero_matches_height() {
        // Default height() must equal height_at_scale(_, _, 0.0) exactly.
        let t = Terrain::from_seed(0xCAFE);
        for &(x, z) in &[
            (0.0, 0.0),
            (1234.5, -6789.0),
            (100_000.0, 50_000.0),
        ] {
            let h = t.height(x, z);
            let h_scale = t.height_at_scale(x, z, 0.0);
            assert_eq!(
                h.to_bits(),
                h_scale.to_bits(),
                "height mismatch at ({x}, {z}): h={h} vs scale={h_scale}"
            );
        }
    }

    #[test]
    fn height_at_scale_wider_caps_more_octaves() {
        // Larger min_spacing_m should produce smoother output (fewer
        // octaves). height_at_scale(0) != height_at_scale(1000) at
        // generic sample points.
        let t = Terrain::from_seed(42);
        let h_full = t.height_at_scale(1234.5, -6789.0, 0.0);
        let h_capped = t.height_at_scale(1234.5, -6789.0, 1000.0);
        // Difference must be non-trivial at a non-grid-aligned point.
        assert!(
            (h_full - h_capped).abs() > 0.1,
            "mipmap cap didn't alter height: h_full={h_full} h_capped={h_capped}"
        );
    }
}
