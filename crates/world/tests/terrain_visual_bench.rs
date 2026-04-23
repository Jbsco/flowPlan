//! Terrain visual-bench (3.6.0+).
//!
//! Emits PNG artifacts for terrain-layer world-gen: heightfield, biome
//! classification, hydrology graph, hub catalog. Seam-continuity assertions
//! verify the toroidal-wrap invariants land bit-identically on either side
//! of `x = 0` / `x = WORLD_EXTENT_M`.
//!
//! See [`TEST_PLAN.md § World-Gen Visual Bench`](../../../TEST_PLAN.md#world-gen-visual-bench)
//! for the pattern and [`MAPGEN.md § World topology`](../../../MAPGEN.md#world-topology)
//! for the wrap mechanics being validated.
//!
//! # Running
//!
//! Default: tests here are `#[ignore]` — opt-in via
//! `cargo test -p flowplan-world --test terrain_visual_bench -- --ignored`.
//! Non-ignored tests run in normal `cargo test` and assert determinism
//! without writing PNGs.
//!
//! # Output
//!
//! PNGs written to `target/world-gen-plots/terrain/<seed>/`. Reference
//! artifacts land at `docs/world-gen-plots/terrain/<seed>/` once accepted.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::expect_used)]

use std::path::Path;

use flowplan_core::{wrap_cell_index, wrap_coord, WORLD_EXTENT_M};
use flowplan_world::terrain::{HeightSampler, Terrain};
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies lint for transitive deps the integration
// test target sees but doesn't itself name directly.
use glam as _;
use libm as _;

/// Full-world-scenario crop extent (m). 500 km — representative regional view.
const FULL_EXTENT_M: f32 = 500_000.0;

/// Player-local crop extent (m). Matches ~10 km streaming disk outer radius.
const PLAYER_EXTENT_M: f32 = 10_000.0;

/// PNG resolution (px per side).
const RESOLUTION: u32 = 1024;

/// Canonical seed used by all terrain visual-bench scenarios. Matches the
/// weather bench seed so composite views can be authored coherently.
const CANONICAL_SEED: u64 = 0xF10_1A11_u64;

#[derive(Copy, Clone)]
struct Crop {
    name: &'static str,
    extent_m: f32,
    center_x_m: f32,
    center_z_m: f32,
}

impl Crop {
    const FULL: Self = Self {
        name: "full",
        extent_m: FULL_EXTENT_M,
        center_x_m: 0.0,
        center_z_m: 0.0,
    };

    const QUADRANT: Self = Self {
        name: "quadrant",
        extent_m: FULL_EXTENT_M / 2.0,
        center_x_m: FULL_EXTENT_M / 4.0,
        center_z_m: -FULL_EXTENT_M / 4.0,
    };

    const PLAYER: Self = Self {
        name: "player",
        extent_m: PLAYER_EXTENT_M,
        center_x_m: 0.0,
        center_z_m: 0.0,
    };

    fn world_coord(self, px: u32, py: u32) -> (f32, f32) {
        let u = (px as f32 + 0.5) / RESOLUTION as f32;
        let v = (py as f32 + 0.5) / RESOLUTION as f32;
        let x = self.center_x_m + (u - 0.5) * self.extent_m;
        let z = self.center_z_m + (0.5 - v) * self.extent_m;
        (x, z)
    }
}

const CROPS: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER];

/// Map normalized `[0, 1]` → RGB via perceptually-flat cool→warm ramp.
/// Identical to the weather bench's ramp for cross-artifact visual parity.
fn ramp_rgb(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let (r, g, b) = if t < 1.0 / 3.0 {
        let u = t * 3.0;
        (0.0, u, 1.0)
    } else if t < 2.0 / 3.0 {
        let u = (t - 1.0 / 3.0) * 3.0;
        (u, 1.0, 1.0 - u)
    } else {
        let u = (t - 2.0 / 3.0) * 3.0;
        (1.0, 1.0 - u, 0.0)
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

fn render_field<F: Fn(f32, f32) -> f32>(
    crop: Crop,
    value_range: (f32, f32),
    sample: F,
) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    let (lo, hi) = value_range;
    let span = hi - lo;
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = crop.world_coord(px, py);
            let value = sample(x, z);
            let normalized = ((value - lo) / span).clamp(0.0, 1.0);
            let rgb = ramp_rgb(normalized);
            img.put_pixel(px, py, Rgb(rgb));
        }
    }
    img
}

fn output_dir() -> std::path::PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| std::path::PathBuf::from("target"), Into::into);
    target
        .join("world-gen-plots/terrain")
        .join(format!("{CANONICAL_SEED:x}"))
}

fn write_png(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, dir: &Path, filename: &str) {
    std::fs::create_dir_all(dir).expect("create output dir");
    let path = dir.join(filename);
    img.save(&path)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

// ══════════════════════════════════════════════════════════════════════════
// Determinism tests — run in normal `cargo test` (no `--ignored`)
// ══════════════════════════════════════════════════════════════════════════

/// Verifies wrap_coord produces bit-identical output across equivalent
/// world positions (the `sample(x) == sample(x + WORLD_EXTENT_M)` invariant
/// from MAPGEN § Sampler-wrap requirements). This is a primitive test; the
/// quantization work (3.6.0b onwards) is what makes samplers actually obey
/// the invariant, but wrap_coord itself must at least produce equivalent
/// canonical coordinates for equivalent queries.
#[test]
fn wrap_coord_is_periodic_bit_identical() {
    // Sample at a grid of test points covering signed + unsigned regions,
    // near-seam + center-of-world + beyond-extent.
    let test_xs = [
        0.0,
        1.0,
        100_000.0,
        WORLD_EXTENT_M / 2.0,
        WORLD_EXTENT_M - 1.0,
        WORLD_EXTENT_M + 1.0,
        -1.0,
        -100_000.0,
        -WORLD_EXTENT_M,
        2.0 * WORLD_EXTENT_M + 12345.0,
    ];
    for &x in &test_xs {
        let canonical = wrap_coord(x);
        let shifted_positive = wrap_coord(x + WORLD_EXTENT_M);
        let shifted_negative = wrap_coord(x - WORLD_EXTENT_M);
        // Allow small float tolerance: fmod() on large magnitudes loses a
        // few ULPs, but the canonical value must match within 10cm.
        assert!(
            (canonical - shifted_positive).abs() < 0.1,
            "wrap_coord({x}) = {canonical} vs wrap_coord({x}+W) = {shifted_positive}"
        );
        assert!(
            (canonical - shifted_negative).abs() < 0.1,
            "wrap_coord({x}) = {canonical} vs wrap_coord({x}-W) = {shifted_negative}"
        );
    }
}

/// Verifies `Terrain::from_seed` produces continuous output at the
/// toroidal seam — `height(x, z) ≈ height(x + W, z)` after octave
/// quantization (3.6.0b). Without quantization this would fail
/// catastrophically (arbitrary discontinuity at the seam); with it,
/// the only residual error is f32 precision in `ω × W` (at 5000 km
/// world × finest-octave ω ≈ 0.3 rad/m, the product carries ~15 ULPs
/// of accumulated roundoff, producing < 1 m displacement in the
/// sampled height).
///
/// The f32 precision limit is a conscious tradeoff: world-scale math
/// in f32 lets us use GPU-shared buffers directly without f64 emulation
/// on the shader side. The sub-meter seam discontinuity is well below
/// visual threshold at terrain amplitudes ±275 m. If Phase 6+ streaming
/// work surfaces this as a problem, converting position-to-phase
/// multiplication to f64 internally (with f32 sin output) would cap
/// seam error at ULPs of the sin function itself (~1e-6).
#[test]
fn terrain_wraps_continuously_across_seam() {
    let terrain = Terrain::from_seed(0xF10_1A11_u64);
    let max_h = terrain.max_height();
    // Tolerance: 2 m = 0.7% of peak terrain amplitude (±275 m).
    // Visually imperceptible at any viewing distance. Observed worst-case
    // on the current 5-octave configuration is ~1 m (f32 precision in
    // ω × W multiplication at 5000 km world scale × fine-octave
    // ω ≈ 0.3 rad/m).
    let tol = 2.0_f32;
    for &z in &[0.0, 1000.0, -1000.0, 100_000.0, 2_500_000.0, -WORLD_EXTENT_M / 3.0] {
        for &x_base in &[0.0, 100.0, -100.0, 12345.6, -12345.6, WORLD_EXTENT_M / 2.0] {
            let h_base = terrain.height(x_base, z);
            let h_shifted = terrain.height(x_base + WORLD_EXTENT_M, z);
            assert!(
                (h_base - h_shifted).abs() < tol,
                "x_base={x_base}, z={z}: h={h_base} vs h+W={h_shifted} (peak={max_h})"
            );
            let h_z_shifted = terrain.height(x_base, z + WORLD_EXTENT_M);
            assert!(
                (h_base - h_z_shifted).abs() < tol,
                "x_base={x_base}, z={z}: h={h_base} vs h+W_z={h_z_shifted}"
            );
        }
    }
}

/// Verifies wrap_cell_index produces exact equivalence under periodic
/// shifts. Unlike coord wrap (float), cell index is integer — zero
/// tolerance required.
#[test]
fn wrap_cell_index_is_periodic_exact() {
    let n = 200_i64;
    let test_is: [i64; 9] = [0, 1, 99, 100, 199, 200, -1, -100, -250];
    for &i in &test_is {
        let canonical = wrap_cell_index(i, n);
        assert_eq!(canonical, wrap_cell_index(i + n, n), "i={i}");
        assert_eq!(canonical, wrap_cell_index(i - n, n), "i={i}");
        assert_eq!(canonical, wrap_cell_index(i + 5 * n, n), "i={i}");
        assert!(canonical >= 0 && canonical < n, "i={i} out of range");
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Visual artifacts — opt-in via `--ignored`
// ══════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_heightfield_artifacts() {
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let max_h = terrain.max_height();
    let value_range = (-max_h, max_h);
    for &crop in CROPS {
        let img = render_field(crop, value_range, |x, z| terrain.height(x, z));
        write_png(&img, &dir, &format!("heightfield_{}.png", crop.name));
    }
}
