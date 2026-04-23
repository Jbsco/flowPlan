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

use flowplan_core::{splitmix64, wrap_cell_index, WORLD_EXTENT_M};

/// A sampleable height function.
pub trait HeightSampler: Send + Sync {
    /// World-space height at `(x, z)`.
    fn height(&self, x: f32, z: f32) -> f32;
}

// ═══════════════════════════════════════════════════════════════════════════
// Noise primitives (Perlin + fBm, toroidal-wrap)
// ═══════════════════════════════════════════════════════════════════════════

/// Gradient direction at integer grid cell `(ix, iz)` for a given
/// `(seed, octave)`. Grid wraps at `n_cells` so lookups at `ix + n_cells`
/// produce identical output — the mechanism that makes fBm toroidal.
///
/// Hash uses Knuth's multiplicative mixing of the inputs followed by
/// `splitmix64` for thorough bit-diffusion. Adjacent grid cells produce
/// uncorrelated directions.
fn gradient(seed: u64, octave: u32, ix: i64, iz: i64, n_cells: i64) -> (f32, f32) {
    use core::f32::consts::TAU;
    let ix_w = wrap_cell_index(ix, n_cells) as u64;
    let iz_w = wrap_cell_index(iz, n_cells) as u64;
    let combined = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (u64::from(octave)).wrapping_mul(0x517C_C1B7_2722_0A95)
        ^ ix_w.wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ iz_w.wrapping_mul(0x94D0_49BB_1331_11EB);
    let h = splitmix64(combined);
    #[allow(clippy::cast_precision_loss)]
    let theta = (h as f64 / u64::MAX as f64) as f32 * TAU;
    (libm::cosf(theta), libm::sinf(theta))
}

/// Perlin's quintic fade function — smooths interpolation weights to
/// `t³(6t² − 15t + 10)`, zeroing first + second derivatives at endpoints
/// for C² continuity across grid cell boundaries.
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + t * (b - a)
}

/// Single octave of 2D Perlin noise. Grid spacing `spacing_m` controls
/// the scale: at spacing 1000 m, features are ~1 km across. Output is
/// bounded in `[-√0.5, √0.5] ≈ [-0.707, 0.707]` by the 2D Perlin theorem.
///
/// **Wrap invariant**: `WORLD_EXTENT_M / spacing_m` must be an integer
/// (callers enforce via `base_spacing` divisibility). Output at `x` and
/// `x + WORLD_EXTENT_M` is identical by construction.
#[allow(clippy::cast_possible_truncation)]
fn perlin(seed: u64, octave: u32, x: f32, z: f32, spacing_m: f32) -> f32 {
    let n_cells = (WORLD_EXTENT_M / spacing_m).round() as i64;
    let xs = x / spacing_m;
    let zs = z / spacing_m;
    #[allow(clippy::cast_possible_truncation)]
    let ix = libm::floorf(xs) as i64;
    #[allow(clippy::cast_possible_truncation)]
    let iz = libm::floorf(zs) as i64;
    #[allow(clippy::cast_precision_loss)]
    let fx = xs - ix as f32;
    #[allow(clippy::cast_precision_loss)]
    let fz = zs - iz as f32;

    // Four corner gradients
    let g00 = gradient(seed, octave, ix, iz, n_cells);
    let g10 = gradient(seed, octave, ix + 1, iz, n_cells);
    let g01 = gradient(seed, octave, ix, iz + 1, n_cells);
    let g11 = gradient(seed, octave, ix + 1, iz + 1, n_cells);

    // Dot products with distance-to-corner vectors
    let n00 = g00.0 * fx + g00.1 * fz;
    let n10 = g10.0 * (fx - 1.0) + g10.1 * fz;
    let n01 = g01.0 * fx + g01.1 * (fz - 1.0);
    let n11 = g11.0 * (fx - 1.0) + g11.1 * (fz - 1.0);

    // Bilinear with quintic smoothing
    let u = fade(fx);
    let v = fade(fz);
    let nx0 = lerp(n00, n10, u);
    let nx1 = lerp(n01, n11, u);
    lerp(nx0, nx1, v)
}

/// Fractional Brownian motion: sum of Perlin octaves at halving spacing
/// (lacunarity = 2.0) and halving amplitude (persistence = 0.5). Returns
/// raw sum; caller normalizes/scales.
///
/// `octave_offset` shifts the octave indices so independent fBm layers
/// (regional / fine) use different gradient hashes even at the same seed.
fn fbm(
    seed: u64,
    octave_offset: u32,
    x: f32,
    z: f32,
    base_spacing_m: f32,
    octaves: u32,
) -> f32 {
    let mut total = 0.0_f32;
    let mut amplitude = 1.0_f32;
    let mut spacing = base_spacing_m;
    for i in 0..octaves {
        total += amplitude * perlin(seed, octave_offset + i, x, z, spacing);
        amplitude *= 0.5;
        spacing *= 0.5;
    }
    total
}

// ═══════════════════════════════════════════════════════════════════════════
// Terrain
// ═══════════════════════════════════════════════════════════════════════════

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
    fn theoretical_max(self) -> f32 {
        let regional_sum: f32 = (0..self.regional_octaves)
            .map(|i| 0.5_f32.powi(i as i32))
            .sum();
        let fine_sum: f32 = (0..self.fine_octaves)
            .map(|i| 0.5_f32.powi(i as i32))
            .sum();
        // 2D Perlin peak ≈ 0.7071; 1.2× fudge for correlated-octave
        // cases that slightly exceed per-octave bounds.
        (self.regional_amplitude_m * regional_sum + self.fine_amplitude_m * fine_sum)
            * 0.7071
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
            * fbm(
                self.seed,
                0,
                x,
                z,
                self.config.regional_spacing_m,
                self.config.regional_octaves,
            );
        let fine = self.config.fine_amplitude_m
            * fbm(
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

    #[test]
    fn perlin_is_periodic_by_construction() {
        // Perlin at (x, z) and (x + WORLD_EXTENT_M, z) must match closely
        // (integer-grid wrap via wrap_cell_index; residual is f32 precision).
        let seed = 0xCAFE_BEEF;
        let octave = 0;
        let spacing = 2000.0;
        for &x in &[0.0, 1234.5, -1234.5, 1_000_000.0] {
            for &z in &[0.0, 999.9, -999.9] {
                let p1 = perlin(seed, octave, x, z, spacing);
                let p2 = perlin(seed, octave, x + WORLD_EXTENT_M, z, spacing);
                // f32 precision accumulates ~1e-4 at world scale
                assert!((p1 - p2).abs() < 1e-3, "x={x} z={z}: p1={p1} p2={p2}");
            }
        }
    }
}
