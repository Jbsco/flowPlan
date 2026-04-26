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
    // cell-index wrap. 5c.2.c.12r — discovered via the ambient
    // bench's 5×5 hi-res grid.
    let x = wrap_coord(x);
    let z = wrap_coord(z);
    #[allow(clippy::cast_possible_truncation)]
    let n_cells = (WORLD_EXTENT_M / spacing_m).round() as i64;
    // 5c.2.c.12s — wrap-seam continuity. The cell grid must tile
    // WORLD_EXTENT_M exactly. With requested spacing = 60 m, naive
    // `n_cells * spacing = 4_999_980 m` ≠ 5_000_000 m → 20 m gap that
    // exposes a noise jump at the seam (literal x = 0 / W). Use the
    // EFFECTIVE spacing `W / n_cells` so the grid tiles W exactly,
    // then divide by that effective spacing when computing cell
    // coords. The change in spacing is < 0.0001× from requested
    // values for our authored wavelengths — invisible visually but
    // produces a perfectly continuous seam.
    #[allow(clippy::cast_precision_loss)]
    let effective_spacing = WORLD_EXTENT_M / n_cells as f32;
    let xs = x / effective_spacing;
    let zs = z / effective_spacing;
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

/// Result of a Worley (cellular) noise sample.
///
/// Returned by [`worley_2d`] and [`worley_3d`]. All three values are
/// always populated — the caller picks whichever matches the use case.
/// Distances are normalized by `spacing_m`, so each value is in
/// approximately `[0, ~1.5)` — `f1 < f2` always.
///
/// - `f1`: distance to nearest feature point. Sharp cellular structure.
///   Use for hard cellular look (rare; Perlin dominates the cloud blend).
/// - `f2`: distance to second-nearest feature point. Always `>= f1`.
///   Use raw for "wide" cellular bands.
/// - `f_blended`: F1+F2 smoothstep blend at feature boundary. Soft
///   cellular structure with no hard edges. Default cloud-noise output
///   for cumulus / nimbus / squall (per WEATHER.md cell-type table).
/// - `f2_minus_f1`: derived `f2 - f1`. Wispy striated patterns; use for
///   secondary-octave detail on squall / frontal cells.
#[derive(Copy, Clone, Debug)]
pub struct WorleyResult {
    pub f1: f32,
    pub f2: f32,
    pub f_blended: f32,
}

impl WorleyResult {
    /// `f2 - f1`. Wispy striated patterns. Always non-negative.
    #[must_use]
    pub fn f2_minus_f1(self) -> f32 {
        self.f2 - self.f1
    }
}

/// Maximum supported features per cell at any single octave.
///
/// Multi-feature Worley (per [`docs/WEATHER.md`](../../../../docs/WEATHER.md#worley-noise-primitive))
/// places `k` deterministically-jittered feature points per grid cell,
/// where `k ∈ [1, 3]` per region (varies via a coarse Perlin density
/// field — see [`features_per_cell_at`]). Stack-allocated `[f32; 4]`
/// scratch buffer is sized for `k_max = 3` plus one slot of slack so
/// insertion-sort doesn't index past the end.
const WORLEY_MAX_FEATURES_PER_CELL: usize = 3;

/// 2D PCG hash mixing 4 u32 inputs into 2 u32 outputs.
///
/// Pure u32 arithmetic with natural wrap-on-overflow — bit-identical
/// CPU↔GPU when both use u32 wrapping multiply (Rust `wrapping_mul`,
/// WGSL native `*` on `u32`). This is the foundation of cloud-noise
/// determinism per [`docs/WEATHER.md`](../../../../docs/WEATHER.md#cpu-gpu-bit-identity).
///
/// The hash is a derivation of the well-known PCG-2D family adapted
/// for bias-free distribution across all 32 bits of each output word.
/// Reference: <https://www.shadertoy.com/view/XlGcRh>.
fn pcg_2d_hash(v_in: [u32; 2]) -> [u32; 2] {
    let mut v = v_in;
    v[0] = v[0]
        .wrapping_mul(1_664_525)
        .wrapping_add(v[1])
        .wrapping_add(1_013_904_223);
    v[1] = v[1]
        .wrapping_mul(1_664_525)
        .wrapping_add(v[0])
        .wrapping_add(1_013_904_223);
    v[0] ^= v[0] >> 16;
    v[1] ^= v[1] >> 16;
    v[0] = v[0].wrapping_mul(1_664_525).wrapping_add(v[1]);
    v[1] = v[1].wrapping_mul(1_664_525).wrapping_add(v[0]);
    v[0] ^= v[0] >> 16;
    v[1] ^= v[1] >> 16;
    v
}

/// 3D PCG hash — 6 u32 inputs (the per-axis hash inputs) into 3 u32
/// outputs. Same family as [`pcg_2d_hash`] extended to 3 channels.
fn pcg_3d_hash(v_in: [u32; 3]) -> [u32; 3] {
    let mut v = v_in;
    v[0] = v[0]
        .wrapping_mul(1_664_525)
        .wrapping_add(v[1])
        .wrapping_add(1_013_904_223);
    v[1] = v[1]
        .wrapping_mul(1_664_525)
        .wrapping_add(v[2])
        .wrapping_add(1_013_904_223);
    v[2] = v[2]
        .wrapping_mul(1_664_525)
        .wrapping_add(v[0])
        .wrapping_add(1_013_904_223);
    v[0] ^= v[0] >> 16;
    v[1] ^= v[1] >> 16;
    v[2] ^= v[2] >> 16;
    v[0] = v[0].wrapping_mul(1_664_525).wrapping_add(v[1]);
    v[1] = v[1].wrapping_mul(1_664_525).wrapping_add(v[2]);
    v[2] = v[2].wrapping_mul(1_664_525).wrapping_add(v[0]);
    v[0] ^= v[0] >> 16;
    v[1] ^= v[1] >> 16;
    v[2] ^= v[2] >> 16;
    v
}

/// Compress a 64-bit master seed into 32 bits for the u32 hash chain.
///
/// PCG-family hashes need u32-sized state. The full 64-bit `master_seed`
/// must distinctly identify a world even after this compression — using
/// xor-fold (high half ⊕ low half) preserves bit entropy from both
/// halves into the 32-bit output. Combined with the per-call mixing in
/// [`pcg_2d_hash`] / [`pcg_3d_hash`], the effective entropy after
/// hashing is full 32 bits.
///
/// Note: this means seeds differing ONLY in their high 32 bits AND
/// having a coincidentally-identical xor-fold will hash to the same
/// cloud-noise output. The probability is 2⁻³² ≈ 0 for our use case;
/// world-distinct seeds are guaranteed by the `master_seed` being mixed
/// with octave + cell coords + `feature_idx` in the hash chain.
#[allow(clippy::cast_possible_truncation)]
fn seed_to_u32(seed: u64) -> u32 {
    ((seed >> 32) ^ seed) as u32
}

/// 2D feature-jitter offset for Worley noise.
///
/// Returns `(jx, jz) ∈ [0, 1)²` for the `feature_idx`-th feature in
/// grid cell `(ix, iz)`. Deterministic per `(seed, octave, ix, iz,
/// feature_idx)`. Uses the u32 PCG hash chain for CPU↔GPU bit-identity
/// per WEATHER.md.
///
/// Hash inputs include `wrap_cell_index` on `(ix, iz)` so canonical
/// and wrapped-equivalent grid cells produce identical jitter, the
/// same wrap discipline as Perlin's `gradient`.
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_possible_truncation)] // intentional truncation: hash inputs only need low 32 bits
fn feature_jitter(
    seed: u64,
    octave: u32,
    ix: i64,
    iz: i64,
    n_cells: i64,
    feature_idx: u32,
) -> (f32, f32) {
    let ix_w = wrap_cell_index(ix, n_cells) as u32;
    let iz_w = wrap_cell_index(iz, n_cells) as u32;
    // Mix seed + octave + feature_idx into the 2 input slots so the
    // PCG hash diffuses all 5 inputs across both output u32s.
    let s0 = seed_to_u32(seed)
        ^ octave.wrapping_mul(0x517C_C1B7)
        ^ feature_idx.wrapping_mul(0xD243_67E1)
        ^ ix_w;
    let s1 = seed_to_u32(seed).rotate_left(13)
        ^ octave.wrapping_mul(0x9E37_79B9)
        ^ feature_idx.wrapping_mul(0xBF58_476D)
        ^ iz_w;
    let h = pcg_2d_hash([s0, s1]);
    // Two independent unit fractions from the hash. Top 24 bits per
    // word for [0, 1) range with 24-bit precision (matches f32 mantissa).
    #[allow(clippy::cast_precision_loss)]
    let jx = ((h[0] >> 8) as f32) / ((1u32 << 24) as f32);
    #[allow(clippy::cast_precision_loss)]
    let jz = ((h[1] >> 8) as f32) / ((1u32 << 24) as f32);
    (jx, jz)
}

/// 3D feature-jitter offset for Worley noise. Y axis is unbounded
/// (altitude) — does NOT wrap. Hash uses [`pcg_3d_hash`] for the same
/// CPU↔GPU bit-identity discipline as the 2D variant.
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_possible_truncation)] // intentional truncation: hash inputs only need low 32 bits
fn feature_jitter_3d(
    seed: u64,
    octave: u32,
    ix: i64,
    iy: i64,
    iz: i64,
    n_cells: i64,
    feature_idx: u32,
) -> (f32, f32, f32) {
    let ix_w = wrap_cell_index(ix, n_cells) as u32;
    let iz_w = wrap_cell_index(iz, n_cells) as u32;
    // Y axis hashes via direct cast of the (potentially negative) i64
    // into u32 with its own distinguishing constant. Negative altitudes
    // produce different bit patterns from positives, no collision risk.
    let iy_h = (iy as u32).wrapping_mul(0xC2B2_AE35);
    let s0 = seed_to_u32(seed)
        ^ octave.wrapping_mul(0x517C_C1B7)
        ^ feature_idx.wrapping_mul(0xD243_67E1)
        ^ ix_w;
    let s1 = seed_to_u32(seed).rotate_left(11) ^ octave.wrapping_mul(0xC4CE_B9FE) ^ iy_h;
    let s2 = seed_to_u32(seed).rotate_left(23)
        ^ octave.wrapping_mul(0x9E37_79B9)
        ^ feature_idx.wrapping_mul(0xBF58_476D)
        ^ iz_w;
    let h = pcg_3d_hash([s0, s1, s2]);
    #[allow(clippy::cast_precision_loss)]
    let jx = ((h[0] >> 8) as f32) / ((1u32 << 24) as f32);
    #[allow(clippy::cast_precision_loss)]
    let jy = ((h[1] >> 8) as f32) / ((1u32 << 24) as f32);
    #[allow(clippy::cast_precision_loss)]
    let jz = ((h[2] >> 8) as f32) / ((1u32 << 24) as f32);
    (jx, jy, jz)
}

/// Insert `value` into a sorted-ascending array, dropping the largest
/// element to make room. `arr` must be initialized with `f32::INFINITY`
/// for the "empty" slots so the first inserts naturally fall into place.
///
/// Used by Worley to track the two smallest distance² values across
/// the 9 (2D) or 27 (3D) feature-cell neighborhood without sorting the
/// full list.
fn insert_sorted_top2(arr: &mut [f32; 4], value: f32) {
    if value < arr[0] {
        arr[3] = arr[2];
        arr[2] = arr[1];
        arr[1] = arr[0];
        arr[0] = value;
    } else if value < arr[1] {
        arr[3] = arr[2];
        arr[2] = arr[1];
        arr[1] = value;
    } else if value < arr[2] {
        arr[3] = arr[2];
        arr[2] = value;
    } else if value < arr[3] {
        arr[3] = value;
    }
}

/// Smoothstep at feature boundaries: produces F1+F2 blend with soft
/// cellular structure and no hard edges. Pattern parallels biome
/// edge-blending — when one feature is a clear winner (`f1 ≪ f2`),
/// returns near-`f1`; in boundary regions (`f1 ≈ f2`), returns weighted
/// blend.
///
/// Per [`docs/WEATHER.md`](../../../../docs/WEATHER.md#multi-feature-parallel-to-biome-blending):
/// this is the v1 default, not a future enhancement. Pure-feature-per-cell
/// hard F1 boundaries are visually too uniform.
fn smooth_blend(f1: f32, f2: f32) -> f32 {
    // (f2 - f1) / f2 ∈ [0, 1]. Near 1 = clear winner; near 0 = boundary.
    // Smoothstep produces C¹ continuous blend weight.
    let raw_t = if f2 > 1e-6 {
        ((f2 - f1) / f2).clamp(0.0, 1.0)
    } else {
        // f2 ≈ 0 means we're sitting on a feature point (impossible
        // unless query coincides with a jittered feature) — return f1
        // unblended.
        1.0
    };
    let smoothed = raw_t * raw_t * (3.0 - 2.0 * raw_t);
    // f1 + (f2 - f1) × (1 - smoothed) × 0.5
    // When raw_t = 1 (clear winner): smoothed = 1, blend = f1.
    // When raw_t = 0 (boundary): smoothed = 0, blend = (f1 + f2) × 0.5.
    f1 + (f2 - f1) * (1.0 - smoothed) * 0.5
}

/// Single-octave 2D Worley (cellular) noise with multi-feature
/// per-cell + F1+F2 smooth-blend.
///
/// Places `features_per_cell` jittered feature points per grid cell.
/// Returns the smallest two distances in the 3×3 grid neighborhood,
/// normalized by `spacing_m`, plus a smoothstep-blended composite.
/// Toroidal-wrap-aware via [`wrap_cell_index`] — same wrap discipline
/// as [`perlin_2d`].
///
/// Cost: `9 × features_per_cell` distance computes per sample. At
/// `features_per_cell = 2` that's 18 distance computes per sample;
/// at `features_per_cell = 3` it's 27.
///
/// **Wrap invariant**: `WORLD_EXTENT_M / spacing_m` must be integer
/// (callers enforce via spacing divisibility). Output at `x` and
/// `x + WORLD_EXTENT_M` is identical by construction.
///
/// **Determinism**: pure function of inputs. libm-discipline
/// (uses [`libm::sqrtf`], [`libm::floorf`]). Within-platform bit-
/// identical; cross-platform deferred to Phase 6.1 hardening.
#[allow(clippy::cast_possible_truncation)]
pub fn worley_2d(
    seed: u64,
    octave: u32,
    x: f32,
    z: f32,
    spacing_m: f32,
    features_per_cell: u32,
) -> WorleyResult {
    debug_assert!(
        features_per_cell >= 1 && features_per_cell <= WORLEY_MAX_FEATURES_PER_CELL as u32,
        "features_per_cell must be in [1, 3]"
    );
    let x = wrap_coord(x);
    let z = wrap_coord(z);
    #[allow(clippy::cast_possible_truncation)]
    let n_cells = (WORLD_EXTENT_M / spacing_m).round() as i64;
    #[allow(clippy::cast_precision_loss)]
    let effective_spacing = WORLD_EXTENT_M / n_cells as f32;
    let xs = x / effective_spacing;
    let zs = z / effective_spacing;
    #[allow(clippy::cast_possible_truncation)]
    let ix0 = libm::floorf(xs) as i64;
    #[allow(clippy::cast_possible_truncation)]
    let iz0 = libm::floorf(zs) as i64;
    #[allow(clippy::cast_precision_loss)]
    let qx = xs - ix0 as f32;
    #[allow(clippy::cast_precision_loss)]
    let qz = zs - iz0 as f32;

    let mut top2 = [f32::INFINITY; 4];
    for dx in -1..=1 {
        for dz in -1..=1 {
            for k in 0..features_per_cell {
                let (jx, jz) = feature_jitter(seed, octave, ix0 + dx, iz0 + dz, n_cells, k);
                #[allow(clippy::cast_precision_loss)]
                let fx = dx as f32 + jx - qx;
                #[allow(clippy::cast_precision_loss)]
                let fz = dz as f32 + jz - qz;
                let d2 = fx * fx + fz * fz;
                insert_sorted_top2(&mut top2, d2);
            }
        }
    }

    let f1 = libm::sqrtf(top2[0]);
    let f2 = libm::sqrtf(top2[1]);
    WorleyResult {
        f1,
        f2,
        f_blended: smooth_blend(f1, f2),
    }
}

/// Single-octave 3D Worley with multi-feature + F1+F2 smooth-blend.
///
/// Y axis (altitude) does NOT wrap; it's unbounded by world topology.
/// X and Z axes wrap toroidally per the Worley 2D variant.
///
/// Cost: `27 × features_per_cell` distance computes per sample. At
/// `features_per_cell = 2` that's 54; at `features_per_cell = 3` it's
/// 81. GPU bench cost ~15-25 ns at `k = 2` on modern hardware (per
/// WEATHER.md performance budget — fits inside the cloud-pass budget).
///
/// **Use case**: cloud raymarcher samples a 3D Worley field per ray
/// step for sub-cell cloud detail. Octave slot `[240, 280)` per the
/// octave-offset convention.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::too_many_arguments)]
pub fn worley_3d(
    seed: u64,
    octave: u32,
    x: f32,
    y: f32,
    z: f32,
    spacing_m: f32,
    features_per_cell: u32,
) -> WorleyResult {
    debug_assert!(
        features_per_cell >= 1 && features_per_cell <= WORLEY_MAX_FEATURES_PER_CELL as u32,
        "features_per_cell must be in [1, 3]"
    );
    let x = wrap_coord(x);
    let z = wrap_coord(z);
    // Y unwrapped — altitude is bounded by physics, not topology.
    #[allow(clippy::cast_possible_truncation)]
    let n_cells = (WORLD_EXTENT_M / spacing_m).round() as i64;
    #[allow(clippy::cast_precision_loss)]
    let effective_spacing = WORLD_EXTENT_M / n_cells as f32;
    let xs = x / effective_spacing;
    let ys = y / effective_spacing;
    let zs = z / effective_spacing;
    #[allow(clippy::cast_possible_truncation)]
    let ix0 = libm::floorf(xs) as i64;
    #[allow(clippy::cast_possible_truncation)]
    let iy0 = libm::floorf(ys) as i64;
    #[allow(clippy::cast_possible_truncation)]
    let iz0 = libm::floorf(zs) as i64;
    #[allow(clippy::cast_precision_loss)]
    let qx = xs - ix0 as f32;
    #[allow(clippy::cast_precision_loss)]
    let qy = ys - iy0 as f32;
    #[allow(clippy::cast_precision_loss)]
    let qz = zs - iz0 as f32;

    let mut top2 = [f32::INFINITY; 4];
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                for k in 0..features_per_cell {
                    let (jx, jy, jz) =
                        feature_jitter_3d(seed, octave, ix0 + dx, iy0 + dy, iz0 + dz, n_cells, k);
                    #[allow(clippy::cast_precision_loss)]
                    let fx = dx as f32 + jx - qx;
                    #[allow(clippy::cast_precision_loss)]
                    let fy = dy as f32 + jy - qy;
                    #[allow(clippy::cast_precision_loss)]
                    let fz = dz as f32 + jz - qz;
                    let d2 = fx * fx + fy * fy + fz * fz;
                    insert_sorted_top2(&mut top2, d2);
                }
            }
        }
    }

    let f1 = libm::sqrtf(top2[0]);
    let f2 = libm::sqrtf(top2[1]);
    WorleyResult {
        f1,
        f2,
        f_blended: smooth_blend(f1, f2),
    }
}

/// 3D Perlin gradient at integer cell corner. Returns unit vector
/// derived from the PCG hash chain — same pattern as
/// [`feature_jitter_3d`] for CPU↔GPU bit-identity.
///
/// Uses `octave * 0x...EBA1u` constants distinct from Worley's
/// `0xD24367E1u` so the same `(seed, octave, cell)` does not collide
/// between the two noise systems. Caller should also use a Perlin-
/// specific octave offset (Worley reserves 240, Perlin uses 241+).
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_possible_truncation)]
fn cloud_perlin_gradient_3d(
    seed: u64,
    octave: u32,
    ix: i64,
    iy: i64,
    iz: i64,
    n_cells: i64,
) -> [f32; 3] {
    let ix_w = wrap_cell_index(ix, n_cells) as u32;
    let iz_w = wrap_cell_index(iz, n_cells) as u32;
    // Y axis hashes via direct cast; negative altitudes get distinct
    // bit patterns from positives. Different multiplier from
    // feature_jitter_3d's 0xC2B2_AE35 to avoid any collision.
    let iy_h = (iy as u32).wrapping_mul(0x9E37_79B1);
    let s0 = seed_to_u32(seed) ^ octave.wrapping_mul(0xCC9E_2D51) ^ ix_w;
    let s1 = seed_to_u32(seed).rotate_left(7) ^ octave.wrapping_mul(0x1B87_3593) ^ iy_h;
    let s2 = seed_to_u32(seed).rotate_left(19) ^ octave.wrapping_mul(0xCAFE_BABE) ^ iz_w;
    let h = pcg_3d_hash([s0, s1, s2]);
    // Map u32 → f32 in [-1, 1]. 2 / u32::MAX as f32 has tiny precision
    // loss but matches WGSL's `f32(u32) * scale - 1.0` exactly when
    // both compute `2 / 4294967295.0`. Use libm operations only — no
    // platform-specific intrinsics.
    #[allow(clippy::cast_precision_loss)]
    let scale = 2.0_f32 / u32::MAX as f32;
    #[allow(clippy::cast_precision_loss)]
    let gx = (h[0] as f32) * scale - 1.0;
    #[allow(clippy::cast_precision_loss)]
    let gy = (h[1] as f32) * scale - 1.0;
    #[allow(clippy::cast_precision_loss)]
    let gz = (h[2] as f32) * scale - 1.0;
    let len_sq = gx * gx + gy * gy + gz * gz;
    let inv_len = if len_sq > 1e-6 {
        1.0 / libm::sqrtf(len_sq)
    } else {
        1.0
    };
    [gx * inv_len, gy * inv_len, gz * inv_len]
}

/// Single-octave 3D Perlin noise for cloud sub-cell detail.
///
/// Pure u32 PCG hash chain → bit-identical CPU↔GPU when both use
/// wrap-on-overflow arithmetic. Companion to [`worley_3d`] in the
/// Perlin-Worley blend per
/// [`docs/WEATHER.md`](../../../../docs/WEATHER.md#sub-cell-detail-noise).
///
/// **Distinct from existing `perlin_2d`**: this uses the u32 PCG hash
/// (matches WGSL precisely), while the existing 2D Perlin uses the
/// u64 splitmix64 hash (CPU-only, terrain/biome/weather consumer).
/// Naming convention — "cloud_*" prefix marks the u32-hash family
/// reserved for CPU-GPU shared cloud rendering.
///
/// Output range: roughly `[-1.0, 1.0]` (theoretical bound for unit
/// gradients trilinearly interpolated; typical samples in `[-0.7, 0.7]`).
///
/// **Wrap invariant**: X and Z wrap toroidally via [`wrap_cell_index`]
/// at `n_cells = WORLD_EXTENT_M / spacing_m`. Y axis (altitude) is
/// unbounded — does NOT wrap.
///
/// **Cost**: ~8 hash calls + 8 dot products + trilinear interp.
/// ~3-5× cheaper than [`worley_3d`] at `features_per_cell = 1`.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::many_single_char_names)] // (x,y,z,u,v,w,k) match standard Perlin notation
pub fn cloud_perlin_3d(seed: u64, octave: u32, x: f32, y: f32, z: f32, spacing_m: f32) -> f32 {
    let x = wrap_coord(x);
    let z = wrap_coord(z);
    // Y unwrapped — altitude bounded by physics, not topology.
    let n_cells = (WORLD_EXTENT_M / spacing_m).round() as i64;
    #[allow(clippy::cast_precision_loss)]
    let effective_spacing = WORLD_EXTENT_M / n_cells as f32;
    let xs = x / effective_spacing;
    let ys = y / effective_spacing;
    let zs = z / effective_spacing;
    let ix = libm::floorf(xs) as i64;
    let iy = libm::floorf(ys) as i64;
    let iz = libm::floorf(zs) as i64;
    #[allow(clippy::cast_precision_loss)]
    let fx = xs - ix as f32;
    #[allow(clippy::cast_precision_loss)]
    let fy = ys - iy as f32;
    #[allow(clippy::cast_precision_loss)]
    let fz = zs - iz as f32;

    // 8 corners, each contributes (gradient · offset_from_corner).
    let mut dots = [0.0_f32; 8];
    for k in 0..8 {
        let dx = i64::from(k & 1);
        let dy = i64::from((k >> 1) & 1);
        let dz = i64::from((k >> 2) & 1);
        let g = cloud_perlin_gradient_3d(seed, octave, ix + dx, iy + dy, iz + dz, n_cells);
        #[allow(clippy::cast_precision_loss)]
        let ofx = fx - dx as f32;
        #[allow(clippy::cast_precision_loss)]
        let ofy = fy - dy as f32;
        #[allow(clippy::cast_precision_loss)]
        let ofz = fz - dz as f32;
        dots[k as usize] = g[0] * ofx + g[1] * ofy + g[2] * ofz;
    }

    let u = fade(fx);
    let v = fade(fy);
    let w = fade(fz);

    let nx00 = lerp(dots[0], dots[1], u);
    let nx10 = lerp(dots[2], dots[3], u);
    let nx01 = lerp(dots[4], dots[5], u);
    let nx11 = lerp(dots[6], dots[7], u);
    let nxy0 = lerp(nx00, nx10, v);
    let nxy1 = lerp(nx01, nx11, v);
    lerp(nxy0, nxy1, w)
}

/// Per-region feature-count `k ∈ {1, 2, 3}` for multi-feature Worley.
///
/// Sampled from a coarse Perlin density field at `density_spacing_m`
/// (typically 10× the Worley spacing — produces regional clustering of
/// dense vs sparse cellular structure). Per
/// [`docs/WEATHER.md`](../../../../docs/WEATHER.md#per-cell-feature-count-variation):
/// some regions have dense cumulus (k=3, lots of small puffs); others
/// have isolated patterns (k=1, fewer larger clouds).
///
/// `density_octave_offset` should be disjoint from the Worley octave
/// (`[240, 280)` in the cloud-noise slot — pick something in `[280,
/// 290)` for the density field).
#[must_use]
pub fn features_per_cell_at(
    seed: u64,
    density_octave_offset: u32,
    cell_x_world_m: f32,
    cell_z_world_m: f32,
    density_spacing_m: f32,
) -> u32 {
    let raw = perlin_2d(
        seed,
        density_octave_offset,
        cell_x_world_m,
        cell_z_world_m,
        density_spacing_m,
    );
    // Perlin returns ~[-0.707, 0.707]; normalize to [0, 1].
    let normalized = ((raw + 0.707) / 1.414).clamp(0.0, 1.0);
    if normalized > 0.66 {
        3
    } else if normalized > 0.33 {
        2
    } else {
        1
    }
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
            30.0_f32, 50.0, 60.0, 100.0, 120.0, 130.0, 140.0, 150.0, 180.0, 200.0, 250.0, 280.0,
            350.0, 400.0, 420.0, 460.0, 500.0, 550.0, 600.0, 700.0, 800.0, 900.0, 1000.0, 2000.0,
            5000.0,
        ];
        let probe_points = [
            (0.0_f32, 0.0_f32),
            (1234.5, -567.8),
            (-1.0, 1.0),
            (12_345.0, -54_321.0),
        ];

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
            fail_count, worst.0, worst.1 .0, worst.1 .1, worst.2, worst.3, max_diff,
        );
    }

    /// Wrap-seam *continuity* (5c.2.c.12s). The earlier test only
    /// asserted "canonically-equivalent inputs give same output" —
    /// `perlin_2d(x) == perlin_2d(x + W)`. That doesn't catch the
    /// real artifact: a discontinuity *at the seam itself*, between
    /// canonical `W - ε` and canonical `0 + ε` (which are physically
    /// 2ε apart on the torus). For wavelengths where `W / spacing` is
    /// non-integer, the cell grid wraps at a different distance than
    /// `W`, creating a noise jump at the seam. Visible in-game as a
    /// straight line at `x = 0` or `z = 0`.
    #[test]
    fn perlin_seam_continuous_at_torus_boundary() {
        // Compare noise across the torus seam: canonical W - ε vs
        // canonical 0 + ε. With my fix (effective_spacing = W /
        // n_cells), the cell grid tiles W exactly and noise IS
        // continuous in BOTH value AND derivative across the seam.
        //
        // The natural Perlin gradient at the seam is bounded by
        // ~1/spacing per meter (just like everywhere else in the
        // grid). So |noise(-ε) - noise(+ε)| should be at most
        // `2ε / spacing` (the Lipschitz bound), times a small safety
        // factor for fade-curve curvature. This is the SAME bound
        // that applies to any 2ε-apart sample inside a cell.
        let wavelengths = [
            30.0_f32, 50.0, 60.0, 100.0, 130.0, 200.0, 400.0, 500.0, 1000.0,
        ];
        let zs = [0.0_f32, 12_345.0, 1234.5, -54_321.0, 999.0];
        let epsilons = [0.5_f32, 1.0, 5.0, 20.0];

        for &spacing in &wavelengths {
            // Lipschitz bound × safety. Empirical max seen in this
            // test post-fix is ≈ 1.05 × (2ε / spacing).
            let bound_per_2eps = 1.5 / spacing;
            for &z in &zs {
                for &eps in &epsilons {
                    let bound = (bound_per_2eps * 2.0 * eps).max(1e-3);

                    let a = perlin_2d(42, 0, -eps, z, spacing);
                    let b = perlin_2d(42, 0, eps, z, spacing);
                    let diff = (a - b).abs();
                    assert!(
                        diff <= bound,
                        "X-seam discontinuous at spacing={spacing}m z={z} ε={eps}: a={a} b={b} Δ={diff} > bound={bound}"
                    );

                    let c = perlin_2d(42, 0, z, -eps, spacing);
                    let d = perlin_2d(42, 0, z, eps, spacing);
                    let diff_z = (c - d).abs();
                    assert!(
                        diff_z <= bound,
                        "Z-seam discontinuous at spacing={spacing}m x={z} ε={eps}: c={c} d={d} Δ={diff_z} > bound={bound}"
                    );

                    // Also verify NO JUMP at the exact seam: noise at
                    // canonical 0 should be the limit of noise as we
                    // approach from either side. Test by comparing
                    // value at canonical 0 to the average of (-ε, +ε).
                    // For continuous noise, midpoint ≈ average.
                    let mid = perlin_2d(42, 0, 0.0, z, spacing);
                    let avg = (a + b) * 0.5;
                    assert!(
                        (mid - avg).abs() <= bound,
                        "X-seam mid≠avg at spacing={spacing}m z={z} ε={eps}: mid={mid} avg={avg}"
                    );
                }
            }
        }
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

    // ────────────────────────────────────────────────────────────────
    // Worley noise tests (multi-feature, F1+F2 blend, toroidal wrap)
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn worley_2d_deterministic() {
        let a = worley_2d(42, 240, 100.0, 200.0, 1000.0, 2);
        let b = worley_2d(42, 240, 100.0, 200.0, 1000.0, 2);
        assert_eq!(a.f1.to_bits(), b.f1.to_bits());
        assert_eq!(a.f2.to_bits(), b.f2.to_bits());
        assert_eq!(a.f_blended.to_bits(), b.f_blended.to_bits());
    }

    #[test]
    fn worley_3d_deterministic() {
        let a = worley_3d(42, 240, 100.0, 500.0, 200.0, 1000.0, 2);
        let b = worley_3d(42, 240, 100.0, 500.0, 200.0, 1000.0, 2);
        assert_eq!(a.f1.to_bits(), b.f1.to_bits());
        assert_eq!(a.f2.to_bits(), b.f2.to_bits());
        assert_eq!(a.f_blended.to_bits(), b.f_blended.to_bits());
    }

    #[test]
    fn worley_2d_f1_le_f2_invariant() {
        // F1 must always be <= F2 by definition; covers the
        // insertion-sort logic across many sample positions.
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 137.0;
                let z = (iz as f32 - 10.0) * 191.0;
                let r = worley_2d(42, 240, x, z, 1000.0, 2);
                assert!(r.f1 <= r.f2, "f1={} f2={} at ({x}, {z})", r.f1, r.f2);
                assert!(r.f1 >= 0.0, "f1 negative at ({x}, {z})");
                assert!(r.f2 >= 0.0, "f2 negative at ({x}, {z})");
            }
        }
    }

    #[test]
    fn worley_3d_f1_le_f2_invariant() {
        for ix in 0..10 {
            for iy in 0..10 {
                for iz in 0..10 {
                    let x = (ix as f32 - 5.0) * 137.0;
                    let y = (iy as f32 - 5.0) * 113.0;
                    let z = (iz as f32 - 5.0) * 191.0;
                    let r = worley_3d(42, 240, x, y, z, 1000.0, 2);
                    assert!(r.f1 <= r.f2, "f1 > f2 at ({x},{y},{z})");
                    assert!(r.f1 >= 0.0);
                    assert!(r.f2 >= 0.0);
                }
            }
        }
    }

    #[test]
    fn worley_2d_wraps_toroidally() {
        // Same wrap discipline as Perlin — sampling at x and x + W
        // must produce identical results.
        for &(x, z) in &[(0.0, 0.0), (1234.5, -567.8), (250_000.0, -100_000.0)] {
            for k in 1..=3 {
                let a = worley_2d(42, 240, x, z, 1000.0, k);
                let b = worley_2d(42, 240, x + WORLD_EXTENT_M, z, 1000.0, k);
                let c = worley_2d(42, 240, x, z + WORLD_EXTENT_M, 1000.0, k);
                assert!(
                    (a.f1 - b.f1).abs() < 1e-3,
                    "x-wrap f1 at ({x},{z}, k={k}): {} vs {}",
                    a.f1,
                    b.f1
                );
                assert!(
                    (a.f1 - c.f1).abs() < 1e-3,
                    "z-wrap f1 at ({x},{z}, k={k}): {} vs {}",
                    a.f1,
                    c.f1
                );
                assert!((a.f_blended - b.f_blended).abs() < 1e-3, "x-wrap f_blended");
            }
        }
    }

    #[test]
    fn worley_2d_seam_continuous_at_authored_wavelengths() {
        // Mirrors the Perlin seam-continuity test for spacings that
        // don't divide WORLD_EXTENT_M evenly. Catches the same class
        // of wrap-fractional bugs (5c.2.c.12r) at the Worley layer.
        let wavelengths = [
            100.0_f32, 250.0, 500.0, 1000.0, 1500.0, 2000.0, 5000.0, 10_000.0,
        ];
        let probe_points = [(0.0, 0.0), (1234.5, -567.8), (-1.0, 1.0)];

        let mut max_diff = 0.0_f32;
        for &spacing in &wavelengths {
            for &(x, z) in &probe_points {
                let a = worley_2d(42, 240, x, z, spacing, 2);
                let b = worley_2d(42, 240, x + WORLD_EXTENT_M, z, spacing, 2);
                let diff = (a.f_blended - b.f_blended).abs();
                if diff > max_diff {
                    max_diff = diff;
                }
                assert!(
                    diff < 1e-3,
                    "seam f_blended @ spacing={spacing} ({x},{z}): {} vs {}",
                    a.f_blended,
                    b.f_blended
                );
            }
        }
    }

    #[test]
    fn worley_2d_different_seeds_decorrelated() {
        // Different seeds at the same position should produce different
        // distance values across at least some samples in a small grid.
        let mut differs_at_least_once = false;
        for ix in 0..10 {
            for iz in 0..10 {
                let x = ix as f32 * 200.0;
                let z = iz as f32 * 200.0;
                let r1 = worley_2d(1, 240, x, z, 1000.0, 2);
                let r2 = worley_2d(2, 240, x, z, 1000.0, 2);
                if r1.f1.to_bits() != r2.f1.to_bits() {
                    differs_at_least_once = true;
                }
            }
        }
        assert!(
            differs_at_least_once,
            "seeds 1 vs 2 produced identical Worley fields"
        );
    }

    #[test]
    fn worley_2d_feature_count_varies_output() {
        // k=1 (sparse) vs k=3 (dense) must produce visibly different
        // distance distributions. f1 with k=3 should be smaller on
        // average since more candidate features per cell ↓ closest-
        // distance.
        let mut sum_f1_k1 = 0.0_f32;
        let mut sum_f1_k3 = 0.0_f32;
        let mut count = 0_u32;
        for ix in 0..30 {
            for iz in 0..30 {
                let x = ix as f32 * 73.0;
                let z = iz as f32 * 89.0;
                sum_f1_k1 += worley_2d(42, 240, x, z, 1000.0, 1).f1;
                sum_f1_k3 += worley_2d(42, 240, x, z, 1000.0, 3).f1;
                count += 1;
            }
        }
        let avg_k1 = sum_f1_k1 / count as f32;
        let avg_k3 = sum_f1_k3 / count as f32;
        assert!(
            avg_k3 < avg_k1,
            "k=3 should pack features tighter than k=1: avg_f1 k1={avg_k1} k3={avg_k3}"
        );
    }

    #[test]
    fn worley_blended_between_f1_and_f2() {
        // f_blended should always lie within [f1, f2] inclusive — it's
        // a weighted mix, not an extrapolation.
        for ix in 0..20 {
            for iz in 0..20 {
                let x = (ix as f32 - 10.0) * 137.0;
                let z = (iz as f32 - 10.0) * 191.0;
                let r = worley_2d(42, 240, x, z, 1000.0, 2);
                assert!(
                    r.f_blended >= r.f1 - 1e-5 && r.f_blended <= r.f2 + 1e-5,
                    "f_blended {} not in [f1={}, f2={}]",
                    r.f_blended,
                    r.f1,
                    r.f2
                );
            }
        }
    }

    #[test]
    fn worley_at_feature_point_has_near_zero_f1() {
        // Querying near a feature point should give near-zero f1.
        // We synthesize this by computing the feature position
        // explicitly, then sampling at it.
        let spacing = 1000.0_f32;
        #[allow(clippy::cast_possible_truncation)]
        let n_cells = (WORLD_EXTENT_M / spacing).round() as i64;
        let effective_spacing = WORLD_EXTENT_M / n_cells as f32;
        for ix in 0..5 {
            for iz in 0..5 {
                let (jx, jz) = feature_jitter(42, 240, ix, iz, n_cells, 0);
                let fx_world = (ix as f32 + jx) * effective_spacing;
                let fz_world = (iz as f32 + jz) * effective_spacing;
                let r = worley_2d(42, 240, fx_world, fz_world, spacing, 2);
                assert!(
                    r.f1 < 0.05,
                    "f1 at feature point should be near 0; got {} at ({fx_world}, {fz_world})",
                    r.f1
                );
            }
        }
    }

    #[test]
    fn worley_3d_y_axis_does_not_wrap() {
        // Y axis is unbounded altitude, NOT toroidal. Sampling at very
        // different Y values must produce different distance results.
        let r_low = worley_3d(42, 240, 0.0, 100.0, 0.0, 1000.0, 2);
        let r_high = worley_3d(42, 240, 0.0, 5_000.0, 0.0, 1000.0, 2);
        // At different altitudes the closest feature in 3D space differs
        // unless we got astronomically lucky — assert decorrelation.
        assert!(
            (r_low.f1 - r_high.f1).abs() > 0.01,
            "3D Worley should differ across y; got {} vs {}",
            r_low.f1,
            r_high.f1
        );
    }

    #[test]
    fn features_per_cell_in_valid_range() {
        // All grid samples must produce k ∈ [1, 3].
        for ix in 0..30 {
            for iz in 0..30 {
                let x = ix as f32 * 1000.0;
                let z = iz as f32 * 1000.0;
                let k = features_per_cell_at(42, 280, x, z, 10_000.0);
                assert!((1..=3).contains(&k), "k out of range at ({x}, {z}): {k}");
            }
        }
    }

    // ────────────────────────────────────────────────────────────────
    // cloud_perlin_3d tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn cloud_perlin_3d_deterministic() {
        let a = cloud_perlin_3d(42, 241, 100.0, 500.0, 200.0, 1500.0);
        let b = cloud_perlin_3d(42, 241, 100.0, 500.0, 200.0, 1500.0);
        assert_eq!(a.to_bits(), b.to_bits());
    }

    #[test]
    fn cloud_perlin_3d_in_unit_range() {
        // Theoretical bound is ~[-1, 1]; typical samples are tighter.
        // Catches buggy hashes that produce out-of-range output.
        for ix in 0..15 {
            for iy in 0..5 {
                for iz in 0..15 {
                    let x = (ix as f32 - 7.0) * 200.0;
                    let y = (iy as f32 - 2.0) * 300.0;
                    let z = (iz as f32 - 7.0) * 200.0;
                    let v = cloud_perlin_3d(42, 241, x, y, z, 1500.0);
                    assert!(
                        (-1.5..=1.5).contains(&v),
                        "perlin out of range at ({x}, {y}, {z}): {v}"
                    );
                }
            }
        }
    }

    #[test]
    fn cloud_perlin_3d_wraps_toroidally() {
        // X+Z wrap, Y unbounded.
        for &(x, y, z) in &[
            (0.0_f32, 500.0, 0.0_f32),
            (1234.5, 1500.0, -567.8),
            (250_000.0, 2200.0, -100_000.0),
        ] {
            let a = cloud_perlin_3d(42, 241, x, y, z, 1500.0);
            let b = cloud_perlin_3d(42, 241, x + WORLD_EXTENT_M, y, z, 1500.0);
            let c = cloud_perlin_3d(42, 241, x, y, z + WORLD_EXTENT_M, 1500.0);
            assert!((a - b).abs() < 1e-3, "x-wrap at ({x},{y},{z}): {a} vs {b}");
            assert!((a - c).abs() < 1e-3, "z-wrap at ({x},{y},{z}): {a} vs {c}");
        }
    }

    #[test]
    fn cloud_perlin_3d_y_axis_does_not_wrap() {
        // Different altitudes should produce different values.
        let a = cloud_perlin_3d(42, 241, 100.0, 500.0, 100.0, 1500.0);
        let b = cloud_perlin_3d(42, 241, 100.0, 5_000.0, 100.0, 1500.0);
        assert!(
            (a - b).abs() > 1e-3,
            "perlin should differ across y; got {a} vs {b}"
        );
    }

    #[test]
    fn cloud_perlin_3d_different_seeds_decorrelated() {
        let mut differs = false;
        for ix in 0..10 {
            for iz in 0..10 {
                let x = ix as f32 * 200.0;
                let z = iz as f32 * 200.0;
                let a = cloud_perlin_3d(1, 241, x, 1500.0, z, 1500.0);
                let b = cloud_perlin_3d(2, 241, x, 1500.0, z, 1500.0);
                if a.to_bits() != b.to_bits() {
                    differs = true;
                    break;
                }
            }
            if differs {
                break;
            }
        }
        assert!(differs, "perlin seeds 1 vs 2 produced identical output");
    }

    #[test]
    fn cloud_perlin_3d_smooth_continuity() {
        // Adjacent samples should be similar (Perlin is C2-continuous).
        // Catches buggy gradient calc that produces discontinuities.
        let a = cloud_perlin_3d(42, 241, 100.0, 1500.0, 100.0, 1500.0);
        let b = cloud_perlin_3d(42, 241, 100.1, 1500.0, 100.0, 1500.0);
        // ~0.0001% along cell, change should be tiny.
        assert!(
            (a - b).abs() < 0.01,
            "perlin discontinuity at small offset: {a} vs {b}"
        );
    }

    #[test]
    fn features_per_cell_varies_across_world() {
        // Coarse Perlin sampling should produce a mix of 1, 2, 3
        // across a sample grid — tests the density-field
        // distribution isn't degenerate.
        let mut counts = [0_u32; 4];
        for ix in 0..40 {
            for iz in 0..40 {
                let x = ix as f32 * 5000.0;
                let z = iz as f32 * 5000.0;
                let k = features_per_cell_at(42, 280, x, z, 10_000.0);
                counts[k as usize] += 1;
            }
        }
        // At least some of each should appear; 0% of one tier would
        // signal a broken density-field threshold.
        for k in 1..=3 {
            assert!(
                counts[k] > 0,
                "no cells with k={k}; distribution: {counts:?}"
            );
        }
    }
}
