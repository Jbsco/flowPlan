// Cloud-noise primitives — WGSL port of `crates/world/src/noise.rs`
// Worley + features_per_cell. Bit-identical to the Rust implementation
// when both are compiled with u32 wrap-on-overflow arithmetic (Rust
// `wrapping_*` methods; WGSL native `*` / `+` on `u32`).
//
// Per `docs/WEATHER.md` § Worley noise primitive: this is the GPU-side
// raymarcher's source of cellular sub-cell detail. The WGSL output
// MUST match the Rust output bit-for-bit for the cloud-density
// determinism contract to hold (CODING_RULES.md § Determinism
// Discipline → CPU-GPU shared samplers).
//
// # Usage
//
// Include via `include_str!("cloud_noise.wgsl")` in the parent shader,
// or via wgsl `#import` if the project switches to a preprocessor.
// The cloud-raymarch shader (Phase 3.6.3c.6 / 3.2x) calls `worley_2d`
// or `worley_3d` per ray-step.
//
// # CPU/GPU bit-identity gate
//
// Test rig: `crates/render/tests/cloud_noise_bit_identity.rs` (Phase
// B4 deliverable). Samples N reference points on both sides, hashes
// outputs, asserts identical. Failure blocks the milestone.

// World extent (m). Mirrors `flowplan_core::WORLD_EXTENT_M`.
const WORLD_EXTENT_M: f32 = 5000000.0;

// PCG hash multiplier — matches Rust `1_664_525`.
const PCG_MULT: u32 = 1664525u;
const PCG_INC: u32 = 1013904223u;

// ──────────────────────────────────────────────────────────────────
// Wrap helpers — match `flowplan_core::wrap_coord` / `wrap_cell_index`
// ──────────────────────────────────────────────────────────────────

fn wrap_coord(x: f32) -> f32 {
    let m = x - floor(x / WORLD_EXTENT_M) * WORLD_EXTENT_M;
    return m;
}

fn wrap_cell_index_u32(i: i32, n: i32) -> u32 {
    // Same semantics as Rust wrap_cell_index for non-negative `n`.
    // Returns value in [0, n) regardless of sign of `i`.
    let m = i % n;
    if (m < 0) {
        return u32(m + n);
    } else {
        return u32(m);
    }
}

// ──────────────────────────────────────────────────────────────────
// PCG hash — pure u32, bit-identical to Rust pcg_2d_hash / pcg_3d_hash
// ──────────────────────────────────────────────────────────────────

fn pcg_2d_hash(v_in: vec2<u32>) -> vec2<u32> {
    var v = v_in;
    v.x = v.x * PCG_MULT + v.y + PCG_INC;
    v.y = v.y * PCG_MULT + v.x + PCG_INC;
    v.x = v.x ^ (v.x >> 16u);
    v.y = v.y ^ (v.y >> 16u);
    v.x = v.x * PCG_MULT + v.y;
    v.y = v.y * PCG_MULT + v.x;
    v.x = v.x ^ (v.x >> 16u);
    v.y = v.y ^ (v.y >> 16u);
    return v;
}

fn pcg_3d_hash(v_in: vec3<u32>) -> vec3<u32> {
    var v = v_in;
    v.x = v.x * PCG_MULT + v.y + PCG_INC;
    v.y = v.y * PCG_MULT + v.z + PCG_INC;
    v.z = v.z * PCG_MULT + v.x + PCG_INC;
    v.x = v.x ^ (v.x >> 16u);
    v.y = v.y ^ (v.y >> 16u);
    v.z = v.z ^ (v.z >> 16u);
    v.x = v.x * PCG_MULT + v.y;
    v.y = v.y * PCG_MULT + v.z;
    v.z = v.z * PCG_MULT + v.x;
    v.x = v.x ^ (v.x >> 16u);
    v.y = v.y ^ (v.y >> 16u);
    v.z = v.z ^ (v.z >> 16u);
    return v;
}

// Compress 64-bit master seed (passed in as two u32s: high, low) into
// a single u32 via xor-fold. Matches Rust `seed_to_u32`.
fn seed_to_u32(seed_hi: u32, seed_lo: u32) -> u32 {
    return seed_hi ^ seed_lo;
}

// Rotate-left u32. WGSL doesn't have a native rotation operator;
// implement via shift + or. Matches Rust `u32::rotate_left`.
fn rotl_u32(x: u32, n: u32) -> u32 {
    let n_clamped = n & 31u;
    return (x << n_clamped) | (x >> ((32u - n_clamped) & 31u));
}

// ──────────────────────────────────────────────────────────────────
// Feature jitter — matches Rust feature_jitter / feature_jitter_3d
// ──────────────────────────────────────────────────────────────────

struct WorleyResult {
    f1: f32,
    f2: f32,
    f_blended: f32,
}

fn feature_jitter_2d(
    seed_hi: u32, seed_lo: u32,
    octave: u32,
    ix: i32, iz: i32, n_cells: i32,
    feature_idx: u32,
) -> vec2<f32> {
    let ix_w = wrap_cell_index_u32(ix, n_cells);
    let iz_w = wrap_cell_index_u32(iz, n_cells);
    let s = seed_to_u32(seed_hi, seed_lo);
    let s0 = s ^ (octave * 0x517CC1B7u) ^ (feature_idx * 0xD24367E1u) ^ ix_w;
    let s1 = rotl_u32(s, 13u) ^ (octave * 0x9E3779B9u) ^ (feature_idx * 0xBF58476Du) ^ iz_w;
    let h = pcg_2d_hash(vec2<u32>(s0, s1));
    let scale = 1.0 / f32(1u << 24u);
    return vec2<f32>(f32(h.x >> 8u) * scale, f32(h.y >> 8u) * scale);
}

fn feature_jitter_3d(
    seed_hi: u32, seed_lo: u32,
    octave: u32,
    ix: i32, iy: i32, iz: i32, n_cells: i32,
    feature_idx: u32,
) -> vec3<f32> {
    let ix_w = wrap_cell_index_u32(ix, n_cells);
    let iz_w = wrap_cell_index_u32(iz, n_cells);
    // Y axis hashes via direct cast; negative altitudes produce
    // different bit patterns from positives (no collision).
    let iy_h = u32(iy) * 0xC2B2AE35u;
    let s = seed_to_u32(seed_hi, seed_lo);
    let s0 = s ^ (octave * 0x517CC1B7u) ^ (feature_idx * 0xD24367E1u) ^ ix_w;
    let s1 = rotl_u32(s, 11u) ^ (octave * 0xC4CEB9FEu) ^ iy_h;
    let s2 = rotl_u32(s, 23u) ^ (octave * 0x9E3779B9u) ^ (feature_idx * 0xBF58476Du) ^ iz_w;
    let h = pcg_3d_hash(vec3<u32>(s0, s1, s2));
    let scale = 1.0 / f32(1u << 24u);
    return vec3<f32>(
        f32(h.x >> 8u) * scale,
        f32(h.y >> 8u) * scale,
        f32(h.z >> 8u) * scale,
    );
}

// ──────────────────────────────────────────────────────────────────
// F1+F2 smooth-blend — matches Rust smooth_blend
// ──────────────────────────────────────────────────────────────────

fn smooth_blend(f1: f32, f2: f32) -> f32 {
    var raw_t: f32;
    if (f2 > 1e-6) {
        raw_t = clamp((f2 - f1) / f2, 0.0, 1.0);
    } else {
        raw_t = 1.0;
    }
    let smoothed = raw_t * raw_t * (3.0 - 2.0 * raw_t);
    return f1 + (f2 - f1) * (1.0 - smoothed) * 0.5;
}

// Insertion-sort top-2 — match Rust insert_sorted_top2.
// Updates `arr` in-place. WGSL doesn't have stable Box<[f32]>; pass
// scratch as `ptr<function, array<f32, 4>>`.
fn insert_sorted_top2(arr: ptr<function, array<f32, 4>>, value: f32) {
    if (value < (*arr)[0]) {
        (*arr)[3] = (*arr)[2];
        (*arr)[2] = (*arr)[1];
        (*arr)[1] = (*arr)[0];
        (*arr)[0] = value;
    } else if (value < (*arr)[1]) {
        (*arr)[3] = (*arr)[2];
        (*arr)[2] = (*arr)[1];
        (*arr)[1] = value;
    } else if (value < (*arr)[2]) {
        (*arr)[3] = (*arr)[2];
        (*arr)[2] = value;
    } else if (value < (*arr)[3]) {
        (*arr)[3] = value;
    }
}

// ──────────────────────────────────────────────────────────────────
// Worley 2D / 3D — matches Rust worley_2d / worley_3d
// ──────────────────────────────────────────────────────────────────

fn worley_2d(
    seed_hi: u32, seed_lo: u32,
    octave: u32,
    x: f32, z: f32,
    spacing_m: f32,
    features_per_cell: u32,
) -> WorleyResult {
    let xw = wrap_coord(x);
    let zw = wrap_coord(z);
    let n_cells = i32(round(WORLD_EXTENT_M / spacing_m));
    let effective_spacing = WORLD_EXTENT_M / f32(n_cells);
    let ix0 = i32(floor(xw / effective_spacing));
    let iz0 = i32(floor(zw / effective_spacing));
    // Precision-preserving fractional cell offset (PINNED 2026-04-26).
    // `q = x/spacing - ix` loses precision at high `x` because the
    // divide produces a high-magnitude value with limited f32 ULPs.
    // Subtracting `ix * spacing` first cancels the high-magnitude bits,
    // leaving a small residual (always < spacing) at full f32 precision.
    // Sub-millimeter resolution at world-extent (5 Mm) instead of 0.6 m.
    // See WEATHER.md § Precision discipline.
    let qx = (xw - f32(ix0) * effective_spacing) / effective_spacing;
    let qz = (zw - f32(iz0) * effective_spacing) / effective_spacing;

    var top2: array<f32, 4> = array<f32, 4>(1.0e30, 1.0e30, 1.0e30, 1.0e30);
    for (var dx = -1; dx <= 1; dx = dx + 1) {
        for (var dz = -1; dz <= 1; dz = dz + 1) {
            for (var k = 0u; k < features_per_cell; k = k + 1u) {
                let j = feature_jitter_2d(
                    seed_hi, seed_lo, octave,
                    ix0 + dx, iz0 + dz, n_cells, k,
                );
                let fx = f32(dx) + j.x - qx;
                let fz = f32(dz) + j.y - qz;
                let d2 = fx * fx + fz * fz;
                insert_sorted_top2(&top2, d2);
            }
        }
    }

    var result: WorleyResult;
    result.f1 = sqrt(top2[0]);
    result.f2 = sqrt(top2[1]);
    result.f_blended = smooth_blend(result.f1, result.f2);
    return result;
}

// 3D Perlin noise — single octave. Companion to worley_3d for the
// Perlin-Worley blend (WEATHER.md § sub-cell detail noise). Cheaper
// than Worley (~3-5×). Same wrap discipline (X+Z wrap, Y unbounded).
//
// Bit-identical to Rust `cloud_perlin_3d` when both use u32 wrap-on-
// overflow arithmetic.

fn cloud_perlin_gradient_3d(
    seed_hi: u32, seed_lo: u32,
    octave: u32,
    ix: i32, iy: i32, iz: i32, n_cells: i32,
) -> vec3<f32> {
    let ix_w = wrap_cell_index_u32(ix, n_cells);
    let iz_w = wrap_cell_index_u32(iz, n_cells);
    // Distinct multiplier from feature_jitter_3d's 0xC2B2AE35u to
    // avoid collisions with Worley feature jitter.
    let iy_h = u32(iy) * 0x9E3779B1u;
    let s = seed_to_u32(seed_hi, seed_lo);
    let s0 = s ^ (octave * 0xCC9E2D51u) ^ ix_w;
    let s1 = rotl_u32(s, 7u) ^ (octave * 0x1B873593u) ^ iy_h;
    let s2 = rotl_u32(s, 19u) ^ (octave * 0xCAFEBABEu) ^ iz_w;
    let h = pcg_3d_hash(vec3<u32>(s0, s1, s2));
    // Map u32 → f32 in [-1, 1]. Same precision behavior as Rust's
    // `(h as f32) * (2.0 / u32::MAX as f32) - 1.0`.
    let scale = 2.0 / 4294967295.0;
    let gx = f32(h.x) * scale - 1.0;
    let gy = f32(h.y) * scale - 1.0;
    let gz = f32(h.z) * scale - 1.0;
    let len_sq = gx * gx + gy * gy + gz * gz;
    var inv_len = 1.0;
    if (len_sq > 1e-6) {
        // Two-step sqrt-then-divide pattern. Compilers occasionally
        // fuse `1.0 / sqrt(x)` into a `rsqrt` instruction with reduced
        // precision (~22-bit vs IEEE-correctly-rounded 24-bit). The
        // intermediate var prevents the fusion, keeping WGSL within
        // 1 ULP of Rust's `1.0 / libm::sqrtf(x)` per CPU-GPU bit-identity
        // contract (WEATHER.md § Worley noise primitive).
        let len = sqrt(len_sq);
        inv_len = 1.0 / len;
    }
    return vec3<f32>(gx, gy, gz) * inv_len;
}

fn cloud_perlin_fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn cloud_perlin_lerp(a: f32, b: f32, t: f32) -> f32 {
    return a + t * (b - a);
}

fn cloud_perlin_3d(
    seed_hi: u32, seed_lo: u32,
    octave: u32,
    x: f32, y: f32, z: f32,
    spacing_m: f32,
) -> f32 {
    let xw = wrap_coord(x);
    let zw = wrap_coord(z);
    let n_cells = i32(round(WORLD_EXTENT_M / spacing_m));
    let effective_spacing = WORLD_EXTENT_M / f32(n_cells);
    let ix = i32(floor(xw / effective_spacing));
    let iy = i32(floor(y / effective_spacing));
    let iz = i32(floor(zw / effective_spacing));
    // Precision-preserving fractional cell offset — see worley_2d for
    // the math. PINNED 2026-04-26.
    let fx = (xw - f32(ix) * effective_spacing) / effective_spacing;
    let fy = (y - f32(iy) * effective_spacing) / effective_spacing;
    let fz = (zw - f32(iz) * effective_spacing) / effective_spacing;

    var dots: array<f32, 8>;
    for (var k = 0; k < 8; k = k + 1) {
        let dx = k & 1;
        let dy = (k >> 1) & 1;
        let dz = (k >> 2) & 1;
        let g = cloud_perlin_gradient_3d(
            seed_hi, seed_lo, octave,
            ix + dx, iy + dy, iz + dz, n_cells,
        );
        let ofx = fx - f32(dx);
        let ofy = fy - f32(dy);
        let ofz = fz - f32(dz);
        dots[k] = g.x * ofx + g.y * ofy + g.z * ofz;
    }

    let u = cloud_perlin_fade(fx);
    let v = cloud_perlin_fade(fy);
    let w = cloud_perlin_fade(fz);

    let nx00 = cloud_perlin_lerp(dots[0], dots[1], u);
    let nx10 = cloud_perlin_lerp(dots[2], dots[3], u);
    let nx01 = cloud_perlin_lerp(dots[4], dots[5], u);
    let nx11 = cloud_perlin_lerp(dots[6], dots[7], u);
    let nxy0 = cloud_perlin_lerp(nx00, nx10, v);
    let nxy1 = cloud_perlin_lerp(nx01, nx11, v);
    return cloud_perlin_lerp(nxy0, nxy1, w);
}

fn worley_3d(
    seed_hi: u32, seed_lo: u32,
    octave: u32,
    x: f32, y: f32, z: f32,
    spacing_m: f32,
    features_per_cell: u32,
) -> WorleyResult {
    let xw = wrap_coord(x);
    let zw = wrap_coord(z);
    // Y unwrapped (altitude is unbounded by topology).
    let n_cells = i32(round(WORLD_EXTENT_M / spacing_m));
    let effective_spacing = WORLD_EXTENT_M / f32(n_cells);
    let ix0 = i32(floor(xw / effective_spacing));
    let iy0 = i32(floor(y / effective_spacing));
    let iz0 = i32(floor(zw / effective_spacing));
    // Precision-preserving fractional cell offset — see worley_2d for
    // the math. PINNED 2026-04-26.
    let qx = (xw - f32(ix0) * effective_spacing) / effective_spacing;
    let qy = (y - f32(iy0) * effective_spacing) / effective_spacing;
    let qz = (zw - f32(iz0) * effective_spacing) / effective_spacing;

    var top2: array<f32, 4> = array<f32, 4>(1.0e30, 1.0e30, 1.0e30, 1.0e30);
    for (var dx = -1; dx <= 1; dx = dx + 1) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            for (var dz = -1; dz <= 1; dz = dz + 1) {
                for (var k = 0u; k < features_per_cell; k = k + 1u) {
                    let j = feature_jitter_3d(
                        seed_hi, seed_lo, octave,
                        ix0 + dx, iy0 + dy, iz0 + dz,
                        n_cells, k,
                    );
                    let fx = f32(dx) + j.x - qx;
                    let fy = f32(dy) + j.y - qy;
                    let fz = f32(dz) + j.z - qz;
                    let d2 = fx * fx + fy * fy + fz * fz;
                    insert_sorted_top2(&top2, d2);
                }
            }
        }
    }

    var result: WorleyResult;
    result.f1 = sqrt(top2[0]);
    result.f2 = sqrt(top2[1]);
    result.f_blended = smooth_blend(result.f1, result.f2);
    return result;
}
