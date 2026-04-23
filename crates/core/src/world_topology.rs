//! World topology constants and wrap arithmetic.
//!
//! flowPlan's world is a **flat square torus** of side [`WORLD_EXTENT_M`].
//! All world-gen samplers operate on world coordinates `(x, z)` that
//! conceptually wrap: `sample(x, z) ≡ sample(x + WORLD_EXTENT_M, z)` etc.
//!
//! This module exposes the canonical extent constant plus the small set of
//! wrap primitives (coordinate wrap, mod-N cell index wrap) used across
//! the terrain, weather, hydrology, and Voronoi subsystems. Centralizing
//! these eliminates the risk of mismatched implementations that would
//! desync terrain-wrap from weather-wrap from hydrology-wrap.
//!
//! See [`MAPGEN.md § World topology`](../../../../MAPGEN.md#world-topology)
//! for the full design and [`ROADMAP.md § Milestone 3.6.0`](../../../../ROADMAP.md#milestone-360-toroidal-wrap--sampler-quantization-pre-layer-1-foundation)
//! for the world-bake prerequisites this foundation unlocks.
//!
//! # Quantization requirement
//!
//! For samplers to wrap cleanly at the seam, every noise harmonic wavelength
//! `λ` must satisfy `WORLD_EXTENT_M mod λ == 0` (equivalently, the spatial
//! frequency `1/λ` must be an integer multiple of `1/WORLD_EXTENT_M`).
//! Callers are responsible for quantizing their own frequencies. This module
//! exposes [`WORLD_EXTENT_M_INV`] as the fundamental frequency so callers can
//! quantize directly: `quantized_freq = (raw_freq / WORLD_EXTENT_M_INV).round() * WORLD_EXTENT_M_INV`.

/// World extent per side, meters. The world is a toroidal square of this
/// side length. Strawman value (2026-04-21 world-scope lock) is 5000 km;
/// at ferry speed (~50 m/s) traversal takes ~28 hours.
///
/// Retuning this value is a world-bake-time parameter only — it is part
/// of the seed bundle (see `MAPGEN.md § World-bake parameters`), so
/// changing it produces a different world. Existing saves remain valid
/// only if the original world-extent is persisted and re-applied.
///
/// All noise wavelengths, Voronoi grid spacings, and hydrology-graph
/// coordinates must divide this extent evenly for clean seam-wrap.
pub const WORLD_EXTENT_M: f32 = 5_000_000.0;

/// Reciprocal of [`WORLD_EXTENT_M`] — the fundamental spatial frequency
/// of the toroidal world. Any harmonic in a wrap-clean sampler is an
/// integer multiple of this: `freq_n = n × WORLD_EXTENT_M_INV`.
///
/// Exposed as a separate constant to avoid repeated runtime division.
pub const WORLD_EXTENT_M_INV: f32 = 1.0 / WORLD_EXTENT_M;

/// Wrap a world-coordinate value into the canonical `[0, WORLD_EXTENT_M)`
/// interval. Handles negative input correctly (unlike the `%` operator on
/// signed floats which preserves sign).
///
/// # Examples
///
/// ```
/// use flowplan_core::{wrap_coord, WORLD_EXTENT_M};
/// assert!((wrap_coord(0.0) - 0.0).abs() < 1e-3);
/// assert!((wrap_coord(WORLD_EXTENT_M) - 0.0).abs() < 1e-3);
/// assert!((wrap_coord(-1.0) - (WORLD_EXTENT_M - 1.0)).abs() < 1e-3);
/// assert!((wrap_coord(WORLD_EXTENT_M + 100.0) - 100.0).abs() < 1e-3);
/// ```
#[must_use]
pub fn wrap_coord(x: f32) -> f32 {
    let m = x % WORLD_EXTENT_M;
    if m < 0.0 {
        m + WORLD_EXTENT_M
    } else {
        m
    }
}

/// Wrap an integer cell index into `[0, n)` via modular arithmetic.
/// Handles negative input correctly.
///
/// Used by Voronoi grid lookups where `n = WORLD_EXTENT_M / GRID_SPACING_M`
/// (must be integer — callers enforce grid-spacing evenly divides extent).
///
/// # Examples
///
/// ```
/// use flowplan_core::wrap_cell_index;
/// assert_eq!(wrap_cell_index(0, 200), 0);
/// assert_eq!(wrap_cell_index(200, 200), 0);
/// assert_eq!(wrap_cell_index(-1, 200), 199);
/// assert_eq!(wrap_cell_index(250, 200), 50);
/// ```
#[must_use]
pub const fn wrap_cell_index(i: i64, n: i64) -> i64 {
    let m = i % n;
    if m < 0 {
        m + n
    } else {
        m
    }
}

/// Quantize an **angular frequency** `ω` (radians per meter) so that the
/// wave `sin(ω × x + φ)` wraps exactly at [`WORLD_EXTENT_M`].
///
/// Mathematical requirement: `ω × WORLD_EXTENT_M = 2π × k` for integer `k`
/// (the wave must complete an integer number of cycles per world). Rounding
/// to the nearest valid `k` shifts `ω` by at most `π / WORLD_EXTENT_M` —
/// at 5000 km this is ~0.6 µrad/m, which produces sub-0.1% wavelength
/// error at every scale the terrain + weather systems use.
///
/// Minimum `k = 1` enforced: a wavelength longer than the world extent
/// would collapse to DC (constant) rather than alias, which is almost
/// certainly not what the caller wanted.
///
/// # Examples
///
/// ```
/// use flowplan_core::{quantize_angular_frequency, WORLD_EXTENT_M};
/// use core::f32::consts::TAU;
/// // Input ω = 0.006 rad/m → wavelength ≈ 1047 m at world scale
/// let omega_raw = 0.006;
/// let omega_q = quantize_angular_frequency(omega_raw);
/// // Product with world extent should be an integer multiple of TAU
/// let cycles = omega_q * WORLD_EXTENT_M / TAU;
/// assert!((cycles - cycles.round()).abs() < 1e-4);
/// // Rounding error should be small
/// assert!((omega_q - omega_raw).abs() / omega_raw < 0.001);
/// ```
#[must_use]
pub fn quantize_angular_frequency(omega_raw: f32) -> f32 {
    use core::f32::consts::TAU;
    let cycles_per_world = omega_raw * WORLD_EXTENT_M / TAU;
    let k = cycles_per_world.round().max(1.0);
    k * TAU / WORLD_EXTENT_M
}

/// Quantize a **wavelength** `λ` (meters) so that the world extent is
/// an integer multiple of the wavelength. Convenience wrapper around
/// [`quantize_angular_frequency`] for callers that reason in wavelengths
/// rather than radians.
///
/// Minimum result: `WORLD_EXTENT_M` (single cycle per world). Wavelengths
/// longer than the world are clamped rather than aliased.
///
/// # Examples
///
/// ```
/// use flowplan_core::{quantize_wavelength, WORLD_EXTENT_M};
/// // 350 km input at 5000 km world → k = 14.29 rounds to 14 → 357.14 km.
/// let lambda_q = quantize_wavelength(350_000.0);
/// let cycles = WORLD_EXTENT_M / lambda_q;
/// assert!((cycles - cycles.round()).abs() < 1e-4);
/// ```
#[must_use]
pub fn quantize_wavelength(lambda_raw: f32) -> f32 {
    use core::f32::consts::TAU;
    let omega_raw = TAU / lambda_raw;
    let omega_q = quantize_angular_frequency(omega_raw);
    TAU / omega_q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_coord_zero_is_zero() {
        assert!((wrap_coord(0.0) - 0.0).abs() < 1e-3);
    }

    #[test]
    fn wrap_coord_extent_returns_zero() {
        assert!((wrap_coord(WORLD_EXTENT_M) - 0.0).abs() < 1.0);
    }

    #[test]
    fn wrap_coord_negative() {
        let x = wrap_coord(-1000.0);
        assert!((x - (WORLD_EXTENT_M - 1000.0)).abs() < 1e-3);
    }

    #[test]
    fn wrap_coord_large_positive() {
        let x = wrap_coord(WORLD_EXTENT_M + 12345.0);
        assert!((x - 12345.0).abs() < 1.0);
    }

    #[test]
    fn wrap_cell_index_basic() {
        assert_eq!(wrap_cell_index(0, 200), 0);
        assert_eq!(wrap_cell_index(199, 200), 199);
        assert_eq!(wrap_cell_index(200, 200), 0);
        assert_eq!(wrap_cell_index(-1, 200), 199);
        assert_eq!(wrap_cell_index(-200, 200), 0);
        assert_eq!(wrap_cell_index(-201, 200), 199);
    }

    #[test]
    fn world_extent_inv_reciprocal() {
        assert!((WORLD_EXTENT_M * WORLD_EXTENT_M_INV - 1.0).abs() < 1e-6);
    }
}
