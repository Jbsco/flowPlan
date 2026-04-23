//! Shared noise primitives for world-gen: Perlin + fBm.
//!
//! All primitives are **toroidal-wrap-continuous by construction**:
//! gradient-grid lookups wrap via `wrap_cell_index` so sampling at
//! `(x, z)` and `(x + WORLD_EXTENT_M, z)` produces identical output.
//!
//! Used by:
//! - `terrain` (Phase 3.6.0) — regional + fine two-layer composition.
//! - `weather::{temperature, humidity, pressure}` (Phase 3.6.1a follow-up)
//!   — climate/synoptic fields replacing the old rotated-band harmonics.
//! - Future `hydrology` + `biome` modules (3.6.1c+).
//!
//! # Design notes
//!
//! Single implementation of `perlin_2d` and `fbm_2d` keeps consistency
//! across subsystems. The `octave_offset` parameter lets independent
//! layers (terrain regional vs fine, temperature vs humidity) use the
//! same `master_seed` without gradient-hash collisions — just pass
//! disjoint offset ranges.
//!
//! # Wrap invariant
//!
//! `WORLD_EXTENT_M / spacing_m` must be integer for exact seam
//! continuity. Callers enforce this by picking `spacing_m` that divides
//! `WORLD_EXTENT_M` evenly (e.g., 100000, 50000, 25000, 10000, 2000 m
//! at the default 5000 km world extent).

use flowplan_core::{splitmix64, wrap_cell_index, WORLD_EXTENT_M};

/// Gradient direction at integer grid cell `(ix, iz)` for `(seed, octave)`.
/// Grid wraps at `n_cells` — lookups at `ix + n_cells` produce identical
/// output, the mechanism that makes fBm toroidal.
///
/// Hash uses Knuth's multiplicative mixing of inputs followed by
/// `splitmix64` for thorough bit-diffusion. Adjacent grid cells produce
/// uncorrelated directions.
#[allow(clippy::cast_sign_loss)] // wrap_cell_index returns non-negative i64 by contract
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
    let h_f64 = h as f64;
    #[allow(clippy::cast_precision_loss)]
    let u64_max_f64 = u64::MAX as f64;
    #[allow(clippy::cast_possible_truncation)]
    let theta = (h_f64 / u64_max_f64) as f32 * TAU;
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
/// the scale: at spacing 1000 m, features are ~1 km across. Output
/// bounded in `[−√0.5, √0.5] ≈ [−0.707, 0.707]` by the 2D Perlin theorem.
///
/// **Wrap invariant**: `WORLD_EXTENT_M / spacing_m` must be an integer
/// (callers enforce via `spacing_m` divisibility). Output at `x` and
/// `x + WORLD_EXTENT_M` is identical by construction.
#[allow(clippy::cast_possible_truncation)]
pub fn perlin_2d(seed: u64, octave: u32, x: f32, z: f32, spacing_m: f32) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
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

    let g00 = gradient(seed, octave, ix, iz, n_cells);
    let g10 = gradient(seed, octave, ix + 1, iz, n_cells);
    let g01 = gradient(seed, octave, ix, iz + 1, n_cells);
    let g11 = gradient(seed, octave, ix + 1, iz + 1, n_cells);

    let n00 = g00.0 * fx + g00.1 * fz;
    let n10 = g10.0 * (fx - 1.0) + g10.1 * fz;
    let n01 = g01.0 * fx + g01.1 * (fz - 1.0);
    let n11 = g11.0 * (fx - 1.0) + g11.1 * (fz - 1.0);

    let u = fade(fx);
    let v = fade(fz);
    let nx0 = lerp(n00, n10, u);
    let nx1 = lerp(n01, n11, u);
    lerp(nx0, nx1, v)
}

/// Fractional Brownian motion: sum of Perlin octaves at halving spacing
/// (lacunarity = 2.0) and halving amplitude (persistence = 0.5).
///
/// `octave_offset` shifts octave indices so independent fBm layers
/// (regional / fine, temperature / humidity) use different gradient
/// hashes even at the same seed. Convention: disjoint ranges per
/// consumer (e.g., terrain regional `[0..100)`, terrain fine
/// `[100..200)`, weather temperature `[200..300)`, etc.).
///
/// Output unnormalized; caller normalizes/scales. Typical raw range
/// (6 octaves, persistence 0.5) is roughly `[−1.4, 1.4]`.
#[allow(clippy::module_name_repetitions)]
pub fn fbm_2d(
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
        total += amplitude * perlin_2d(seed, octave_offset + i, x, z, spacing);
        amplitude *= 0.5;
        spacing *= 0.5;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perlin_deterministic() {
        let a = perlin_2d(42, 0, 100.0, 200.0, 1000.0);
        let b = perlin_2d(42, 0, 100.0, 200.0, 1000.0);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn perlin_wraps_toroidally() {
        for &(x, z) in &[(0.0, 0.0), (1234.5, -567.8), (2_000_000.0, 1_000_000.0)] {
            let a = perlin_2d(42, 0, x, z, 2000.0);
            let b = perlin_2d(42, 0, x + WORLD_EXTENT_M, z, 2000.0);
            assert!((a - b).abs() < 1e-3, "x={x} z={z}: a={a} b={b}");
        }
    }

    #[test]
    fn fbm_different_offsets_decorrelated() {
        // Two fbm layers at different octave_offset should differ at a fixed
        // point (decorrelated via independent gradient hashes).
        let a = fbm_2d(42, 0, 500.0, 500.0, 2000.0, 4);
        let b = fbm_2d(42, 100, 500.0, 500.0, 2000.0, 4);
        assert!((a - b).abs() > 1e-3, "same seed, different offset produced same value");
    }
}
