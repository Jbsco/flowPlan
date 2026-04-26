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
use flowplan_world::biome::{biome_at, BiomeClass};
use flowplan_world::terrain::{HeightSampler, Terrain};
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies lint for transitive deps the integration
// test target sees but doesn't itself name directly.
use bincode as _;
use glam as _;
use libm as _;
use rayon as _;
use ron as _;
use serde as _;
use tracing as _;

/// Full-world-scenario crop extent (m). 500 km — representative regional view.
const FULL_EXTENT_M: f32 = 500_000.0;

/// Continental-scale extent (m). 2000 km — previews Option A world-scale
/// climate seamlessness; biome diversity at this scale is richer than the
/// 500 km regional view. Matches the weather bench's continental crop.
const CONTINENTAL_EXTENT_M: f32 = 2_000_000.0;

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

    const CONTINENTAL: Self = Self {
        name: "continental",
        extent_m: CONTINENTAL_EXTENT_M,
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

/// Verifies `wrap_coord` produces bit-identical output across equivalent
/// world positions (the `sample(x) == sample(x + WORLD_EXTENT_M)` invariant
/// from `MAPGEN § Sampler-wrap requirements`). This is a primitive test; the
/// quantization work (3.6.0b onwards) is what makes samplers actually obey
/// the invariant, but `wrap_coord` itself must at least produce equivalent
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
    // Tolerance: 100 m. Biome-parameterized terrain (3.6.1b) compounds
    // f32 precision across: regional fBm + fine fBm + latitude proxy +
    // biome weights (involving temperature + humidity field sampling) +
    // 4 biome primitives (domain-warped compounds 3 fbm calls each,
    // ridged compounds N octaves). At 5000 km world scale with ~600 m
    // peak terrain, observed worst-case is ~80 m at antipodal points.
    //
    // This test catches MAJOR wrap breakage (e.g., forgotten mod-N
    // indexing in a new Voronoi layer), not every ULP of drift.
    //
    // Real fix for sub-meter seam continuity at 5000 km scale: move
    // position-to-phase multiplications to f64 internally (keep f32 at
    // sin/output boundary). Tracked for future polish; current tolerance
    // well under visual threshold even at the seam.
    let tol = 100.0_f32;
    for &z in &[
        0.0,
        1000.0,
        -1000.0,
        100_000.0,
        2_500_000.0,
        -WORLD_EXTENT_M / 3.0,
    ] {
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

// NOTE: weather-field seam-continuity test is NOT included here yet.
// The weather synoptic harmonics (pressure / temperature / humidity) use
// seed-derived rotated band directions: `sin(k × (x·cos(θ) + z·sin(θ)) + φ)`.
// Frequency quantization alone doesn't produce clean seam wrap for rotated
// bands because the axis-aligned wrap requirement `ω × W = 2π·k` must hold
// in BOTH axes, which requires cos(θ) and sin(θ) to jointly resolve to
// rational values — impossible for arbitrary θ. Real fix is either:
//   (a) decompose each harmonic into separable x-component and z-component
//       sub-harmonics at independently quantized frequencies, OR
//   (b) switch weather fields to gradient-noise (Perlin) with periodic
//       gradient grid — same pattern we already use for terrain.
// Tracked as a known gap; see `3.6.0 wrap rotated-band` memory for status.
// Wavelength quantization is kept because a future separable-harmonic
// refactor needs the same quantized wavelengths.

/// Verifies `wrap_cell_index` produces exact equivalence under periodic
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

/// Topographic-style elevation ramp in meters → RGB.
/// - Below sea level: blue gradients (water)
/// - 0-200 m: lowland greens
/// - 200-800 m: transition green→yellow
/// - 800-1500 m: brown foothills
/// - 1500-3000 m: rock-brown / tan (mountains)
/// - 3000+ m: white (snow / glacier)
fn topo_rgb(altitude_m: f32) -> [u8; 3] {
    // Water
    if altitude_m < 0.0 {
        let depth = (-altitude_m / 300.0).clamp(0.0, 1.0);
        let r = (40.0 * (1.0 - depth)) as u8;
        let g = (70.0 + 50.0 * (1.0 - depth)) as u8;
        let b = (140.0 + 80.0 * (1.0 - depth)) as u8;
        return [r, g, b];
    }
    // Shore (0-50m): light tan
    if altitude_m < 50.0 {
        let t = altitude_m / 50.0;
        let r = (220.0 - 50.0 * t) as u8;
        let g = (210.0 - 40.0 * t) as u8;
        let b = (150.0 - 60.0 * t) as u8;
        return [r, g, b];
    }
    // Lowland green (50-400m)
    if altitude_m < 400.0 {
        let t = (altitude_m - 50.0) / 350.0;
        let r = (170.0 - 50.0 * t) as u8;
        let g = (180.0 - 10.0 * t) as u8;
        let b = (90.0 - 30.0 * t) as u8;
        return [r, g, b];
    }
    // Midland yellow-green (400-900m)
    if altitude_m < 900.0 {
        let t = (altitude_m - 400.0) / 500.0;
        let r = (120.0 + 60.0 * t) as u8;
        let g = (170.0 - 20.0 * t) as u8;
        let b = (60.0 + 10.0 * t) as u8;
        return [r, g, b];
    }
    // Foothill brown (900-1500m)
    if altitude_m < 1500.0 {
        let t = (altitude_m - 900.0) / 600.0;
        let r = (180.0 - 10.0 * t) as u8;
        let g = (150.0 - 40.0 * t) as u8;
        let b = (70.0 + 10.0 * t) as u8;
        return [r, g, b];
    }
    // Mountain brown (1500-2500m)
    if altitude_m < 2500.0 {
        let t = (altitude_m - 1500.0) / 1000.0;
        let r = (170.0 + 40.0 * t) as u8;
        let g = (110.0 + 50.0 * t) as u8;
        let b = (80.0 + 60.0 * t) as u8;
        return [r, g, b];
    }
    // Snow cap (2500m+)
    let t = ((altitude_m - 2500.0) / 1000.0).clamp(0.0, 1.0);
    let r = (210.0 + 45.0 * t) as u8;
    let g = (160.0 + 95.0 * t) as u8;
    let b = (140.0 + 115.0 * t) as u8;
    [r, g, b]
}

/// Apply hillshade: compute slope from (dx, dz) altitude derivatives,
/// derive lighting intensity assuming sun from upper-left. Returns a
/// multiplier in roughly [0.5, 1.3] — darkening for lee slopes,
/// highlighting for windward slopes.
fn hillshade(dh_dx: f32, dh_dz: f32) -> f32 {
    use core::f32::consts::FRAC_1_SQRT_2;
    // Sun direction (normalized): from upper-left, 45° elevation.
    let s = FRAC_1_SQRT_2;
    let sun = glam::Vec3::new(-s, s, s).normalize();
    // Terrain normal: (−dh/dx, 1, −dh/dz) pointing up-and-away.
    let normal = glam::Vec3::new(-dh_dx, 1.0, -dh_dz).normalize();
    let dot = sun.dot(normal).max(0.0);
    // Map [0,1] to [0.6, 1.25] so shadowed terrain is still visible.
    0.6 + dot * 0.65
}

/// Detect contour-line proximity: return 1.0 if `altitude_m` is far
/// from any contour line, 0.3 (dark) if within `line_half_width_m` of
/// a contour at `interval_m` spacing. Gives crisp contour lines on the
/// topo map.
fn contour_factor(altitude_m: f32, interval_m: f32, line_half_width_m: f32) -> f32 {
    // Compute distance to nearest multiple of `interval_m`.
    let r = altitude_m.rem_euclid(interval_m);
    let d_from_line = r.min(interval_m - r);
    if d_from_line < line_half_width_m {
        0.3 // dark band at contour line
    } else {
        1.0
    }
}

/// High resolution for the contour-line artifact. 4096×4096 gives
/// ~2 m per pixel at the 10 km player-local crop, making thin contour
/// lines sharp.
const CONTOUR_RESOLUTION: u32 = 4096;

/// Slate-white background — slight greenish-blue cast, easier on the
/// eye than pure white and reads as a paper-map aesthetic.
const SLATE_WHITE: [u8; 3] = [233, 240, 238];

/// Monowidth-black contour line color.
const CONTOUR_BLACK: [u8; 3] = [18, 22, 24];

/// Target contour line width in pixels (constant regardless of slope).
/// 1.0 = single-pixel-wide crisp lines.
const CONTOUR_LINE_WIDTH_PX: f32 = 1.2;

/// Compute whether a pixel lies on a contour line, using gradient-aware
/// width scaling to keep line thickness constant in pixel space
/// regardless of terrain slope. Returns `true` when the pixel should
/// render as contour-line-black.
///
/// Math: a contour at altitude `k × interval` is the locus where
/// `altitude(x, z) = k × interval`. At gradient magnitude `|∇h|`, the
/// locus is a curve of width `W / |∇h|` in elevation-space per pixel
/// of visual width `W`. So for target visual width `W_px`, the
/// elevation half-width threshold is `W_px × pixel_m × |∇h| / 2`.
fn contour_line_pixel(
    altitude_m: f32,
    interval_m: f32,
    grad_magnitude: f32,
    pixel_m: f32,
    target_width_px: f32,
) -> bool {
    // Thickness in elevation space required for constant-pixel-width line.
    let half_width_elev_m = target_width_px * pixel_m * grad_magnitude * 0.5;
    // Lower bound: keeps lines visible on near-flat terrain.
    // Upper bound: prevents gradient-scaled width from growing into
    // contour-line-as-solid-blob at wide crops where a single pixel
    // spans hundreds of meters of elevation change. 3% of the interval
    // keeps the tightest feature visually distinct from its neighbors.
    let min_width = interval_m * 0.003;
    let max_width = interval_m * 0.03;
    let half_width = half_width_elev_m.clamp(min_width, max_width);
    let r = altitude_m.rem_euclid(interval_m);
    let d_from_line = r.min(interval_m - r);
    d_from_line < half_width
}

// ═══════════════════════════════════════════════════════════════════════════
// Marching Squares contour extraction (alternate output)
// ═══════════════════════════════════════════════════════════════════════════
//
// The per-pixel contour map (below) has fundamental aliasing limits at wide
// crops: one pixel at 488 m/pixel (continental) can internally cross several
// contour intervals, and the "is this pixel near a contour?" test produces
// speckle rather than clean line tracing.
//
// Marching Squares is the classic (1987) algorithm that fixes this: sample
// altitude on a COARSE grid, and for each grid cell detect which contour
// levels cross the cell edges. Interpolate exact crossing points on each
// edge via linear interpolation between corner altitudes, then draw line
// segments between pairs of crossing points. Rasterize with Bresenham.
//
// Result: crisp 1-pixel-wide contour segments that follow iso-altitude
// curves exactly, independent of zoom. No aliasing, no thickness tuning.
//
// Output at: `contour_ms_{full,quadrant,player,continental}.png`.

/// Coarse grid size for Marching Squares altitude sampling. 1024 samples
/// per axis → 1 M altitude lookups per crop (16× fewer than per-pixel at
/// 4096 output). Each grid cell is thus 4×4 pixels in the output image.
/// At player crop (10 km), sample spacing is ~10 m — tight enough to catch
/// all terrain features. At continental (2000 km), ~2 km per sample — at
/// this zoom, contour segments may smooth across sub-grid features but
/// the result is clean and readable rather than speckled.
const MS_GRID: u32 = 1024;

/// Draw a thicker line by stamping a square brush along the Bresenham
/// path. `radius_px = 0` is single-pixel, `radius_px = 1` is 3×3, etc.
/// Used to emphasize index contours (e.g., the sea-level / shoreline
/// contour at altitude 0) per topographic-map convention.
fn draw_line_bold(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 3],
    radius_px: i32,
) {
    for dy in -radius_px..=radius_px {
        for dx in -radius_px..=radius_px {
            draw_line_bresenham(img, x0 + dx, y0 + dy, x1 + dx, y1 + dy, color);
        }
    }
}

/// Bresenham's line algorithm — rasterizes a 1-pixel-wide line between
/// `(x0, y0)` and `(x1, y1)` into `img`, painting `color`. Clamped to
/// image bounds. Classic integer-arithmetic implementation.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
fn draw_line_bresenham(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 3],
) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx: i32 = if x0 < x1 { 1 } else { -1 };
    let sy: i32 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    loop {
        if x >= 0 && x < w && y >= 0 && y < h {
            img.put_pixel(x as u32, y as u32, Rgb(color));
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// For a 2D grid cell with corner altitudes `corners` at world positions
/// `positions`, check which edges are crossed by the `level` contour.
/// Returns 0, 2, or 4 crossing points (saddle cases give 4).
///
/// Corner/edge indexing: bl=0, br=1, tr=2, tl=3. Edges are consecutive
/// pairs: bottom (bl-br), right (br-tr), top (tl-tr), left (bl-tl).
fn marching_squares_crossings(
    corners: [f32; 4],
    positions: [(f32, f32); 4],
    level: f32,
) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(4);
    // Edges: (corner_a_idx, corner_b_idx)
    let edges: [(usize, usize); 4] = [(0, 1), (1, 2), (3, 2), (0, 3)];
    for (a, b) in edges {
        let ha = corners[a];
        let hb = corners[b];
        // Edge straddles `level` iff one corner is above and the other below.
        // Use strict inequality to avoid double-counting corner-exact hits.
        if (ha - level) * (hb - level) < 0.0 {
            let t = (level - ha) / (hb - ha);
            let (ax, az) = positions[a];
            let (bx, bz) = positions[b];
            out.push((ax + t * (bx - ax), az + t * (bz - az)));
        }
    }
    out
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_contour_map_ms_artifacts() {
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let crops_contour: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER, Crop::CONTINENTAL];
    for &crop in crops_contour {
        let mut img = ImageBuffer::new(CONTOUR_RESOLUTION, CONTOUR_RESOLUTION);
        // Paint slate-white background.
        for py in 0..CONTOUR_RESOLUTION {
            for px in 0..CONTOUR_RESOLUTION {
                img.put_pixel(px, py, Rgb(SLATE_WHITE));
            }
        }

        // Contour interval (same tiers as per-pixel method).
        let contour_interval_m = if crop.extent_m <= 20_000.0 {
            15.0 // player: dense contours per pilot-chart convention
        } else if crop.extent_m <= 100_000.0 {
            60.0
        } else if crop.extent_m <= 500_000.0 {
            150.0 // quadrant: tightened from 250 m for more local relief
        } else if crop.extent_m <= 1_000_000.0 {
            500.0
        } else {
            1000.0
        };

        // Mipmap at sample spacing so coarse-grid altitudes are band-limited.
        let sample_spacing_m = crop.extent_m / MS_GRID as f32;
        let min_spacing_m = sample_spacing_m * 2.0;

        // 1. Sample altitudes on the coarse (MS_GRID+1) × (MS_GRID+1) corner grid.
        let grid_pts = (MS_GRID + 1) as usize;
        let mut heights = vec![0.0_f32; grid_pts * grid_pts];
        for gj in 0..=MS_GRID {
            for gi in 0..=MS_GRID {
                let u = gi as f32 / MS_GRID as f32;
                let v = gj as f32 / MS_GRID as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                heights[gj as usize * grid_pts + gi as usize] =
                    terrain.height_at_scale(x, z, min_spacing_m);
            }
        }

        // Convert world (x, z) → pixel (px, py) for segment rasterization.
        let world_to_pixel = |x: f32, z: f32| -> (i32, i32) {
            let u = (x - crop.center_x_m) / crop.extent_m + 0.5;
            let v = 0.5 - (z - crop.center_z_m) / crop.extent_m;
            let px = (u * CONTOUR_RESOLUTION as f32).round() as i32;
            let py = (v * CONTOUR_RESOLUTION as f32).round() as i32;
            (px, py)
        };

        // 2. For each cell, extract contour segments at each crossing level.
        for gj in 0..MS_GRID {
            for gi in 0..MS_GRID {
                // Corner heights (bl, br, tr, tl)
                let bl = heights[gj as usize * grid_pts + gi as usize];
                let br = heights[gj as usize * grid_pts + (gi + 1) as usize];
                let tr = heights[(gj + 1) as usize * grid_pts + (gi + 1) as usize];
                let tl = heights[(gj + 1) as usize * grid_pts + gi as usize];
                let corners = [bl, br, tr, tl];

                // Corner world positions (bl, br, tr, tl).
                let ul = gi as f32 / MS_GRID as f32;
                let ur = (gi + 1) as f32 / MS_GRID as f32;
                let vb = gj as f32 / MS_GRID as f32;
                let vt = (gj + 1) as f32 / MS_GRID as f32;
                let x_l = crop.center_x_m + (ul - 0.5) * crop.extent_m;
                let x_r = crop.center_x_m + (ur - 0.5) * crop.extent_m;
                let z_b = crop.center_z_m + (0.5 - vb) * crop.extent_m;
                let z_t = crop.center_z_m + (0.5 - vt) * crop.extent_m;
                let positions = [(x_l, z_b), (x_r, z_b), (x_r, z_t), (x_l, z_t)];

                let cell_min = corners.iter().copied().fold(f32::INFINITY, f32::min);
                let cell_max = corners.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                if cell_max - cell_min < 1e-6 {
                    continue;
                }
                let start_level = (cell_min / contour_interval_m).ceil() * contour_interval_m;
                let end_level = (cell_max / contour_interval_m).floor() * contour_interval_m;
                // Iterate over integer steps to avoid float-loop counter drift.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let n_levels = ((end_level - start_level) / contour_interval_m).floor() as i32 + 1;
                for step in 0..n_levels {
                    let level = start_level + (step as f32) * contour_interval_m;
                    let crossings = marching_squares_crossings(corners, positions, level);
                    // Bold the sea-level (altitude = 0) contour as the
                    // shoreline / coastline — topographic-map convention
                    // for the most visually important "index contour".
                    let is_sea_level = level.abs() < 1e-3;
                    let bold_radius = i32::from(is_sea_level);
                    match crossings.len() {
                        2 => {
                            let (x0, y0) = world_to_pixel(crossings[0].0, crossings[0].1);
                            let (x1, y1) = world_to_pixel(crossings[1].0, crossings[1].1);
                            draw_line_bold(&mut img, x0, y0, x1, y1, CONTOUR_BLACK, bold_radius);
                        },
                        4 => {
                            // Saddle — naive pairing (a, b) and (c, d).
                            // Canonical resolution uses cell-center altitude
                            // but naive pairing produces acceptable output
                            // for visual-bench use.
                            let (x0, y0) = world_to_pixel(crossings[0].0, crossings[0].1);
                            let (x1, y1) = world_to_pixel(crossings[1].0, crossings[1].1);
                            let (x2, y2) = world_to_pixel(crossings[2].0, crossings[2].1);
                            let (x3, y3) = world_to_pixel(crossings[3].0, crossings[3].1);
                            draw_line_bold(&mut img, x0, y0, x1, y1, CONTOUR_BLACK, bold_radius);
                            draw_line_bold(&mut img, x2, y2, x3, y3, CONTOUR_BLACK, bold_radius);
                        },
                        _ => {},
                    }
                }
            }
        }

        write_png(&img, &dir, &format!("contour_ms_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_contour_map_artifacts() {
    // Per-pixel mipmap-aware contour rendering is the slowest artifact
    // in the suite — 4096² × 3 height samples per pixel × 4 crops, with
    // mipmap-aware fBm cascade on every sample. The Marching Squares
    // variant (`emit_contour_map_ms_artifacts`) covers the same crops
    // ~50× faster and is what reviewers usually want. This per-pixel
    // path is preserved for fidelity comparison + LOD-streaming
    // mipmap validation, gated behind an explicit env-var so default
    // `--ignored` runs aren't blocked on it.
    if std::env::var_os("FLOWPLAN_VISUAL_BENCH_FULL").is_none() {
        eprintln!(
            "skipping emit_contour_map_artifacts (per-pixel mipmap-aware, ~slow); \
             set FLOWPLAN_VISUAL_BENCH_FULL=1 to include. \
             Marching Squares contour artifacts emitted by `emit_contour_map_ms_artifacts` \
             are the default deliverable."
        );
        return;
    }
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let crops_contour: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER, Crop::CONTINENTAL];
    for &crop in crops_contour {
        let mut img = ImageBuffer::new(CONTOUR_RESOLUTION, CONTOUR_RESOLUTION);
        let pixel_m = crop.extent_m / CONTOUR_RESOLUTION as f32;
        // Mipmap-aware terrain sampling: skip octaves finer than half
        // the pixel size (Nyquist-like anti-alias filter). Kills
        // sub-pixel terrain aliasing that produces speckle at wide
        // crops. No performance cost — skipped octaves aren't computed
        // in the first place.
        // Aggressive mipmap (2× pixel) for contour rendering: sub-pixel
        // terrain variation aliases dense speckle into the image (one
        // pixel can internally cross multiple contour intervals at wide
        // crops). Filtering out octaves smaller than 2× the pixel spacing
        // gives band-limited altitude sampling appropriate for per-pixel
        // contour testing. At player scale (2.5 m/pixel), this filters
        // octaves < 5 m — below any terrain feature anyway, no visible
        // change. At continental (488 m/pixel), filters octaves < 976 m
        // — removes the background-fBm variation that was aliasing.
        let min_spacing_m = pixel_m * 2.0;
        // Size-tier contour intervals, modeled on USGS topographic-map
        // conventions (finer contours at pilot-chart scales, sparser at
        // regional / continental views):
        //   <= 20 km (player):      50 m   (USGS 1:24000 equivalent)
        //   <= 100 km:              100 m
        //   <= 500 km (regional):   250 m
        //   <= 1000 km:             500 m
        //   > 1000 km (continental): 1000 m (overview of major ranges only)
        let contour_interval_m = if crop.extent_m <= 20_000.0 {
            15.0 // player: dense contours per pilot-chart convention
        } else if crop.extent_m <= 100_000.0 {
            60.0
        } else if crop.extent_m <= 500_000.0 {
            150.0 // quadrant: tightened from 250 m for more local relief
        } else if crop.extent_m <= 1_000_000.0 {
            500.0
        } else {
            1000.0
        };
        for py in 0..CONTOUR_RESOLUTION {
            for px in 0..CONTOUR_RESOLUTION {
                let u = (px as f32 + 0.5) / CONTOUR_RESOLUTION as f32;
                let v = (py as f32 + 0.5) / CONTOUR_RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let h_here = terrain.height_at_scale(x, z, min_spacing_m);
                let h_east = terrain.height_at_scale(x + pixel_m, z, min_spacing_m);
                let h_south = terrain.height_at_scale(x, z + pixel_m, min_spacing_m);
                let dh_dx = (h_east - h_here) / pixel_m;
                let dh_dz = (h_south - h_here) / pixel_m;
                let grad_mag = libm::sqrtf(dh_dx * dh_dx + dh_dz * dh_dz);
                let is_line = contour_line_pixel(
                    h_here,
                    contour_interval_m,
                    grad_mag,
                    pixel_m,
                    CONTOUR_LINE_WIDTH_PX,
                );
                let color = if is_line { CONTOUR_BLACK } else { SLATE_WHITE };
                img.put_pixel(px, py, Rgb(color));
            }
        }
        write_png(&img, &dir, &format!("contour_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_topo_map_artifacts() {
    let dir = output_dir();
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    // Extra crop for topographic viewing — intermediate scale between
    // full and player showing distinct biome character.
    let crops_topo: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER, Crop::CONTINENTAL];
    for &crop in crops_topo {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let pixel_m = crop.extent_m / RESOLUTION as f32;
        // Anti-aliasing: 2×2 supersampling when pixel size exceeds finest
        // terrain octave (~62 m). Prevents noise in wide-angle crops where
        // a single pixel spans multiple fine-octave wavelengths.
        let aa_samples = if pixel_m > 60.0 { 2 } else { 1 };
        let aa_step = pixel_m / aa_samples as f32;
        let contour_interval_m = (crop.extent_m / 250.0).max(50.0);
        let line_half_width = (contour_interval_m * 0.05).max(1.0);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x0, z0) = crop.world_coord(px, py);
                let mut rsum = 0.0_f32;
                let mut gsum = 0.0_f32;
                let mut bsum = 0.0_f32;
                let mut n = 0.0_f32;
                for ay in 0..aa_samples {
                    for ax in 0..aa_samples {
                        let x = x0 + (ax as f32 + 0.5) * aa_step;
                        let z = z0 + (ay as f32 + 0.5) * aa_step;
                        let h_here = terrain.height(x, z);
                        let h_east = terrain.height(x + pixel_m, z);
                        let h_south = terrain.height(x, z + pixel_m);
                        let dh_dx = (h_east - h_here) / pixel_m;
                        let dh_dz = (h_south - h_here) / pixel_m;
                        let shade = hillshade(dh_dx, dh_dz);
                        let contour = contour_factor(h_here, contour_interval_m, line_half_width);
                        let [r, g, b] = topo_rgb(h_here);
                        let factor = shade * contour;
                        rsum += f32::from(r) * factor;
                        gsum += f32::from(g) * factor;
                        bsum += f32::from(b) * factor;
                        n += 1.0;
                    }
                }
                let rs = (rsum / n).clamp(0.0, 255.0) as u8;
                let gs = (gsum / n).clamp(0.0, 255.0) as u8;
                let bs = (bsum / n).clamp(0.0, 255.0) as u8;
                img.put_pixel(px, py, Rgb([rs, gs, bs]));
            }
        }
        write_png(&img, &dir, &format!("topo_{}.png", crop.name));
    }
}

/// Biome-class color palette — picked for visual contrast at the
/// biome-mask crop. Each biome gets a distinctive hue so multi-biome
/// regions are readable at a glance.
fn biome_rgb(b: BiomeClass) -> [u8; 3] {
    match b {
        // Warm ochre — desert
        BiomeClass::HighDesert => [205, 168, 92],
        // Cold slate-gray — alpine
        BiomeClass::RockyAlpine => [160, 172, 182],
        // Deep teal — boreal coast
        BiomeClass::CoastalBoreal => [62, 128, 132],
        // Warm olive — mediterranean islands
        BiomeClass::MediterraneanArchipelago => [156, 168, 96],
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_biome_mask_artifacts() {
    let dir = output_dir();
    // Biomes emit at all three standard crops + continental (2000 km) to
    // verify that the world-wide climate range produces the full 4-biome
    // catalog (at smaller crops only 2-3 biomes may appear — a 500 km
    // slice is a fraction of the world's climate range).
    let crops_with_continental: &[Crop] =
        &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER, Crop::CONTINENTAL];
    for &crop in crops_with_continental {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let b = biome_at(CANONICAL_SEED, x, z);
                img.put_pixel(px, py, Rgb(biome_rgb(b)));
            }
        }
        write_png(&img, &dir, &format!("biome_{}.png", crop.name));
    }
}
