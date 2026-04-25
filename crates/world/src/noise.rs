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

use flowplan_core::{splitmix64, wrap_cell_index, wrap_coord, WORLD_EXTENT_M};

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

/// Single octave of 2D Perlin noise.
///
/// Grid spacing `spacing_m` controls the scale: at spacing 1000 m,
/// features are ~1 km across. Output bounded in `[−√0.5, √0.5] ≈
/// [−0.707, 0.707]` by the 2D Perlin theorem.
///
/// **Wrap invariant**: `WORLD_EXTENT_M / spacing_m` must be an integer
/// (callers enforce via `spacing_m` divisibility). Output at `x` and
/// `x + WORLD_EXTENT_M` is identical by construction.
#[allow(clippy::cast_possible_truncation)]
pub fn perlin_2d(seed: u64, octave: u32, x: f32, z: f32, spacing_m: f32) -> f32 {
    // Pre-wrap to canonical [0, WORLD_EXTENT_M) range. Without this,
    // canonically-equivalent inputs (e.g., literal -1.0 and
    // WORLD_EXTENT_M - 1.0) sample at different fractional cell
    // positions because `fx = xs - floor(xs)` is computed BEFORE the
    // cell-index wrap. Result: noise discontinuity at the wrap seam,
    // visible to consumers passing literal player-relative coords.
    // 5c.2.c.12r — discovered via the ambient bench's 5×5 hi-res grid.
    let x = wrap_coord(x);
    let z = wrap_coord(z);
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
    fbm_2d_at_scale(seed, octave_offset, x, z, base_spacing_m, octaves, 0.0)
}

/// Mipmap-aware fBm — same as [`fbm_2d`] but skips octaves whose spacing
/// falls below `min_spacing_m`. Equivalent to `fbm_2d` when
/// `min_spacing_m = 0.0`.
///
/// **Use case**: rendering at `pixel_size_m` world-meters per pixel,
/// passing `min_spacing_m = pixel_size_m × 0.5` removes octaves that
/// would alias (sub-pixel feature wavelength). Mirrors GPU mipmap
/// filtering but at the procedural level. Saves 2× compute per octave
/// skipped and produces properly band-limited output.
///
/// Used by terrain contour bench (wide-angle crops), hydrology bake
/// (100 m sampling doesn't need 60 m octaves), Phase 3.6.2 streaming
/// chunks (LOD-based octave selection), and in-engine map view
/// (pan/zoom-appropriate detail).
#[allow(clippy::module_name_repetitions)]
pub fn fbm_2d_at_scale(
    seed: u64,
    octave_offset: u32,
    x: f32,
    z: f32,
    base_spacing_m: f32,
    octaves: u32,
    min_spacing_m: f32,
) -> f32 {
    let mut total = 0.0_f32;
    let mut amplitude = 1.0_f32;
    let mut spacing = base_spacing_m;
    for i in 0..octaves {
        // Stop when this octave's spacing is below the mipmap floor;
        // sub-pixel-scale detail would alias rather than contribute.
        if spacing < min_spacing_m {
            break;
        }
        total += amplitude * perlin_2d(seed, octave_offset + i, x, z, spacing);
        amplitude *= 0.5;
        spacing *= 0.5;
    }
    total
}

/// Ridged multifractal noise.
///
/// fBm variant where each octave's Perlin output is inverted-absolute-
/// valued and raised to a power, producing sharp crested ridges with
/// smooth valleys. Per `MAPGEN.md § Layer 1 noise primitives`:
///
/// ```text
/// ridged(x, z) = Σ_{i=0..N−1} amp × persist^i × (offset − |perlin_i|)^power
/// ```
///
/// - `offset` (typical 1.0) shifts the absolute-valued Perlin so the
///   ridge-lines (where |perlin| = 0) map to the maximum output. Lower
///   offset = narrower ridges; higher offset = broader peaks.
/// - `power` (typical 1.5–2.5) sharpens the ridges. Power 1.0 = triangle-
///   wave valleys; power 2.0 = spiky crests; power 3.0+ = cliff-like.
///
/// Output is **always non-negative** (returns 0 at valleys, max at
/// ridge-lines). Typical range `[0, 2.0]` with defaults.
///
/// **Use case**: alpine mountain ridges (high power, moderate offset),
/// mesa/canyon landscapes (moderate power, moderate offset).
#[allow(clippy::module_name_repetitions)]
pub fn ridged_multifractal_2d(
    seed: u64,
    octave_offset: u32,
    x: f32,
    z: f32,
    base_spacing_m: f32,
    octaves: u32,
    offset: f32,
    power: f32,
) -> f32 {
    ridged_multifractal_2d_at_scale(
        seed,
        octave_offset,
        x,
        z,
        base_spacing_m,
        octaves,
        offset,
        power,
        0.0,
    )
}

/// Mipmap-aware ridged multifractal — see [`fbm_2d_at_scale`] for the
/// mipmap-floor concept. Skips octaves finer than `min_spacing_m`.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::module_name_repetitions)]
pub fn ridged_multifractal_2d_at_scale(
    seed: u64,
    octave_offset: u32,
    x: f32,
    z: f32,
    base_spacing_m: f32,
    octaves: u32,
    offset: f32,
    power: f32,
    min_spacing_m: f32,
) -> f32 {
    let mut total = 0.0_f32;
    let mut amplitude = 1.0_f32;
    let mut spacing = base_spacing_m;
    for i in 0..octaves {
        if spacing < min_spacing_m {
            break;
        }
        let p = perlin_2d(seed, octave_offset + i, x, z, spacing);
        let inverted = (offset - libm::fabsf(p)).max(0.0);
        let ridged = libm::powf(inverted, power);
        total += amplitude * ridged;
        amplitude *= 0.5;
        spacing *= 0.5;
    }
    total
}

/// Domain-warped fBm — feeds the fBm output back as a coordinate
/// perturbation before sampling a second fBm. Per `MAPGEN.md § Layer 1
/// noise primitives`:
///
/// ```text
/// warp_x = warp_amp × fbm_warp(x, z)
/// warp_z = warp_amp × fbm_warp(x + decorrelate, z + decorrelate)
/// output = fbm_base(x + warp_x, z + warp_z)
/// ```
///
/// Breaks up straight lines and grid alignment artifacts. Produces
/// organic wrinkled distortion — suited to fjord-like coastlines,
/// varied island shapes, and general "terrain that looks geographic".
///
/// `warp_amplitude_m` controls distortion strength (500–5000 m typical).
/// `warp_spacing_m` sets the warp field's coarsest scale (usually
/// ~2× `base_spacing_m` so warp is smoother than the base signal).
///
/// Octave-offset convention: uses `octave_offset + 0..octaves` for the
/// base fBm, `octave_offset + 100..100+warp_octaves` for warp x,
/// `octave_offset + 200..200+warp_octaves` for warp z. Caller must leave
/// a 300-range gap in their offset assignments.
#[allow(clippy::module_name_repetitions)]
#[allow(clippy::too_many_arguments)]
pub fn domain_warped_fbm_2d(
    seed: u64,
    octave_offset: u32,
    x: f32,
    z: f32,
    base_spacing_m: f32,
    octaves: u32,
    warp_amplitude_m: f32,
    warp_spacing_m: f32,
    warp_octaves: u32,
) -> f32 {
    domain_warped_fbm_2d_at_scale(
        seed,
        octave_offset,
        x,
        z,
        base_spacing_m,
        octaves,
        warp_amplitude_m,
        warp_spacing_m,
        warp_octaves,
        0.0,
    )
}

/// Mipmap-aware domain-warped fBm.
///
/// Both warp and base layers skip octaves finer than `min_spacing_m`.
/// Warp octaves are typically much coarser than base, so may survive a
/// mipmap cap that kills most base octaves — that's fine, wide-zoom
/// contours still benefit from the warp's large-scale distortion even
/// when fine detail is filtered.
#[allow(clippy::module_name_repetitions)]
#[allow(clippy::too_many_arguments)]
pub fn domain_warped_fbm_2d_at_scale(
    seed: u64,
    octave_offset: u32,
    x: f32,
    z: f32,
    base_spacing_m: f32,
    octaves: u32,
    warp_amplitude_m: f32,
    warp_spacing_m: f32,
    warp_octaves: u32,
    min_spacing_m: f32,
) -> f32 {
    let warp_x = warp_amplitude_m
        * fbm_2d_at_scale(
            seed,
            octave_offset + 100,
            x,
            z,
            warp_spacing_m,
            warp_octaves,
            min_spacing_m,
        );
    let warp_z = warp_amplitude_m
        * fbm_2d_at_scale(
            seed,
            octave_offset + 200,
            x + 31_415.9,
            z + 27_182.8,
            warp_spacing_m,
            warp_octaves,
            min_spacing_m,
        );
    fbm_2d_at_scale(
        seed,
        octave_offset,
        x + warp_x,
        z + warp_z,
        base_spacing_m,
        octaves,
        min_spacing_m,
    )
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)] // test grid iteration: i32→f32 cast is idiomatic
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

    /// Wrap-seam regression test (5c.2.c.12r). The original
    /// `perlin_wraps_toroidally` used spacing=2000m which divides
    /// `WORLD_EXTENT_M` exactly (`5_000_000 / 2000 = 2500`). Ambient + many
    /// other consumers use spacings that DON'T divide evenly:
    /// `30m → 166666.67`, `60m → 83333.33`, `100m → 50000.0` (✓), `130m →
    /// 38461.54`. Any spacing whose `WORLD_EXTENT_M / spacing` is
    /// non-integer caused noise discontinuity at the wrap seam — at
    /// literal `x = -ε` vs canonical `x = WORLD_EXTENT_M - ε`, perlin
    /// sampled at different fractional cell positions and returned
    /// different values.
    ///
    /// Bug discovered 2026-04-24 via the ambient bench's 5×5 hi-res
    /// grid showing 90.8% dot-on-blue in tiles near the seam. Fix
    /// applied at every ambient consumer site (pre-wrap with
    /// `wrap_coord` before noise call). Long-term fix moves the
    /// pre-wrap inside `perlin_2d` itself; this test asserts the
    /// invariant either way.
    #[test]
    fn perlin_seam_continuous_at_all_authored_wavelengths() {
        // Wavelengths actually used across the project. The 30/60/130
        // values are the smoking-gun cases — they don't divide
        // WORLD_EXTENT_M evenly and surfaced the wrap-seam bug.
        let wavelengths = [
            30.0_f32, 50.0, 60.0, 100.0, 120.0, 130.0, 140.0, 150.0, 180.0, 200.0,
            250.0, 280.0, 350.0, 400.0, 420.0, 460.0, 500.0, 550.0, 600.0, 700.0,
            800.0, 900.0, 1000.0, 2000.0, 5000.0,
        ];
        let probe_points = [(0.0_f32, 0.0_f32), (1234.5, -567.8), (-1.0, 1.0), (12_345.0, -54_321.0)];

        let mut fail_count = 0_u32;
        let mut max_diff = 0.0_f32;
        let mut worst: (f32, (f32, f32), f32, f32) = (0.0, (0.0, 0.0), 0.0, 0.0);

        for &spacing in &wavelengths {
            for &(x, z) in &probe_points {
                let a = perlin_2d(42, 0, x, z, spacing);
                let b = perlin_2d(42, 0, x + WORLD_EXTENT_M, z, spacing);
                let diff = (a - b).abs();
                if diff > max_diff {
                    max_diff = diff;
                    worst = (spacing, (x, z), a, b);
                }
                if diff >= 1e-3 {
                    fail_count += 1;
                }
                // Same for Z direction.
                let c = perlin_2d(42, 0, x, z + WORLD_EXTENT_M, spacing);
                let diff_z = (a - c).abs();
                if diff_z > max_diff {
                    max_diff = diff_z;
                    worst = (spacing, (x, z), a, c);
                }
                if diff_z >= 1e-3 {
                    fail_count += 1;
                }
            }
        }
        assert_eq!(
            fail_count, 0,
            "wrap-seam discontinuity at {} probes; worst: spacing={} m at ({},{}): {} vs {} (Δ={})",
            fail_count,
            worst.0,
            worst.1 .0,
            worst.1 .1,
            worst.2,
            worst.3,
            max_diff,
        );
    }

    /// fBm-level wrap-seam regression. fBm sums multiple octaves at
    /// halving spacings, so even if the BASE spacing is integer-
    /// divisor of `WORLD_EXTENT_M`, finer octaves may not be. Validates
    /// the full multi-octave stack actually used by terrain / weather /
    /// ambient density.
    #[test]
    fn fbm_seam_continuous_at_authored_octaves() {
        let cases = [
            // (base_spacing_m, octaves, label)
            (60.0_f32, 4_u32, "ambient density coarse"),
            (130.0, 4, "ambient maquis"),
            (400.0, 4, "ambient olive"),
            (500.0, 4, "ambient juniper"),
            (1000.0, 4, "terrain fine"),
            (2000.0, 6, "terrain regional"),
            (5000.0, 4, "weather synoptic"),
        ];
        let probe_points = [(0.0_f32, 0.0_f32), (1234.5, -567.8), (12_345.0, -54_321.0)];

        for &(spacing, octaves, label) in &cases {
            for &(x, z) in &probe_points {
                let a = fbm_2d(42, 0, x, z, spacing, octaves);
                let b = fbm_2d(42, 0, x + WORLD_EXTENT_M, z, spacing, octaves);
                let diff = (a - b).abs();
                assert!(
                    diff < 1e-3,
                    "fbm seam discontinuous: {label} (spacing={spacing}m, octaves={octaves}) at ({x},{z}): {a} vs {b} (Δ={diff})"
                );
            }
        }
    }

    #[test]
    fn fbm_different_offsets_decorrelated() {
        // Two fbm layers at different octave_offset should differ at a fixed
        // point (decorrelated via independent gradient hashes).
        let a = fbm_2d(42, 0, 500.0, 500.0, 2000.0, 4);
        let b = fbm_2d(42, 100, 500.0, 500.0, 2000.0, 4);
        assert!(
            (a - b).abs() > 1e-3,
            "same seed, different offset produced same value"
        );
    }

    #[test]
    fn ridged_multifractal_always_nonnegative() {
        // Ridged output is (offset - |perlin|)^power at each octave, always ≥0.
        for ix in -5..=5 {
            for iz in -5..=5 {
                let x = ix as f32 * 3000.0;
                let z = iz as f32 * 3000.0;
                let r = ridged_multifractal_2d(42, 0, x, z, 2000.0, 4, 1.0, 2.0);
                assert!(r >= 0.0, "ridged returned negative: {r} at ({x}, {z})");
            }
        }
    }

    #[test]
    fn ridged_multifractal_wraps_toroidally() {
        let a = ridged_multifractal_2d(42, 0, 123.0, 456.0, 2000.0, 4, 1.0, 2.0);
        let b = ridged_multifractal_2d(42, 0, 123.0 + WORLD_EXTENT_M, 456.0, 2000.0, 4, 1.0, 2.0);
        assert!((a - b).abs() < 1e-2, "ridged seam: {a} vs {b}");
    }

    #[test]
    fn fbm_at_scale_zero_min_spacing_matches_unfiltered() {
        // min_spacing_m = 0.0 must exactly equal fbm_2d for all callers
        // depending on default behavior to pass through.
        let a = fbm_2d(42, 0, 100.0, 200.0, 2000.0, 6);
        let b = fbm_2d_at_scale(42, 0, 100.0, 200.0, 2000.0, 6, 0.0);
        assert_eq!(a.to_bits(), b.to_bits(), "a={a} b={b}");
    }

    #[test]
    fn fbm_at_scale_caps_at_min_spacing() {
        // With 6 octaves at base 2000m: octave spacings = {2000, 1000,
        // 500, 250, 125, 62}. min_spacing_m = 400 should keep octaves
        // {2000, 1000, 500} (3 octaves), skip the rest. Result should
        // differ from unfiltered.
        let a = fbm_2d(42, 0, 100.0, 200.0, 2000.0, 6);
        let b = fbm_2d_at_scale(42, 0, 100.0, 200.0, 2000.0, 6, 400.0);
        assert!((a - b).abs() > 1e-3, "mipmap cap produced unchanged output");
        // Matches direct call with 3 octaves.
        let c = fbm_2d(42, 0, 100.0, 200.0, 2000.0, 3);
        assert_eq!(
            b.to_bits(),
            c.to_bits(),
            "capped (b={b}) should equal 3-octave direct (c={c})"
        );
    }

    #[test]
    fn fbm_at_scale_wraps_toroidally() {
        let a = fbm_2d_at_scale(42, 0, 100.0, 200.0, 2000.0, 6, 250.0);
        let b = fbm_2d_at_scale(42, 0, 100.0 + WORLD_EXTENT_M, 200.0, 2000.0, 6, 250.0);
        assert!((a - b).abs() < 1e-3, "mipmap seam: a={a} b={b}");
    }

    #[test]
    fn domain_warped_fbm_wraps_toroidally() {
        let a = domain_warped_fbm_2d(42, 0, 100.0, 200.0, 2000.0, 4, 500.0, 4000.0, 3);
        let b = domain_warped_fbm_2d(
            42,
            0,
            100.0 + WORLD_EXTENT_M,
            200.0,
            2000.0,
            4,
            500.0,
            4000.0,
            3,
        );
        // Domain warp compounds f32 precision loss; tolerance is higher.
        assert!((a - b).abs() < 0.1, "warped fbm seam: {a} vs {b}");
    }
}
