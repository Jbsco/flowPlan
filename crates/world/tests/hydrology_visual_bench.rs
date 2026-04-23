//! Hydrology visual-bench (Phase 3.6.1d).
//!
//! Emits PNG artifacts for the L1 hydrology bake: flow direction,
//! accumulation, wetness (per-basin fill ratio), and a composite view
//! overlaying the hydrology graph on the topo base.
//!
//! # Running
//!
//! Default: tests here are `#[ignore]` — opt-in via
//! `cargo test -p flowplan-world --release --test hydrology_visual_bench -- --ignored`.
//! The bake is cached across tests via a lazy static.
//!
//! # Output
//!
//! PNGs written to `target/world-gen-plots/hydrology/<seed>/`.
//! Reference artifacts land at `docs/world-gen-plots/hydrology/<seed>/`
//! once accepted.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::expect_used)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::erasing_op)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::explicit_iter_loop)]
#![allow(clippy::let_underscore_must_use)]
#![allow(let_underscore_drop)]

use std::path::Path;

use flowplan_core::WORLD_EXTENT_M;
use flowplan_world::hydrology::{HydrologyGraph, WaterClass, BAKE_GRID_N, BAKE_RESOLUTION_M};
use flowplan_world::terrain::Terrain;
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies for transitive test deps.
use glam as _;
use libm as _;
use rayon as _;
use ron as _;
use serde as _;

const FULL_EXTENT_M: f32 = 500_000.0;
const CONTINENTAL_EXTENT_M: f32 = 2_000_000.0;
const PLAYER_EXTENT_M: f32 = 10_000.0;
const RESOLUTION: u32 = 1024;

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
    // Player crop moved from world origin (which straddles the
    // toroidal seam + lands in a river-sparse coastal region for the
    // canonical seed). The position here targets a known-river-rich
    // interior zone observable in the quadrant artifact (central-left
    // of the quadrant crop, where accumulation converges). Tuned to
    // this canonical seed; different seeds may want a different center.
    const PLAYER: Self = Self {
        name: "player",
        extent_m: PLAYER_EXTENT_M,
        center_x_m: 50_000.0,
        center_z_m: -125_000.0,
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

const CROPS: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::PLAYER, Crop::CONTINENTAL];

// ═══════════════════════════════════════════════════════════════════════════
// Bake cache — shared across tests via OnceLock
// ═══════════════════════════════════════════════════════════════════════════

fn shared_bake() -> (Terrain, HydrologyGraph) {
    // Each test builds its own; re-baking is ~2 s but lets tests run in
    // parallel. If bake becomes a hotspot we can switch to a OnceLock
    // shared handle.
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let graph = HydrologyGraph::bake(CANONICAL_SEED, &terrain);
    (terrain, graph)
}

fn output_dir() -> std::path::PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| std::path::PathBuf::from("target"), Into::into);
    target
        .join("world-gen-plots/hydrology")
        .join(format!("{CANONICAL_SEED:x}"))
}

fn write_png(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, dir: &Path, filename: &str) {
    std::fs::create_dir_all(dir).expect("create output dir");
    let path = dir.join(filename);
    img.save(&path)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

// ═══════════════════════════════════════════════════════════════════════════
// Hydrology grid → pixel coord mapping
// ═══════════════════════════════════════════════════════════════════════════

/// Sample a hydrology grid at crop pixel (px, py) via nearest-cell lookup.
/// Wraps toroidally.
fn sample_cell_at_pixel<T: Copy>(grid: &[T], crop: Crop, px: u32, py: u32) -> T {
    let (x, z) = crop.world_coord(px, py);
    let x_wrapped = flowplan_core::wrap_coord(x);
    let z_wrapped = flowplan_core::wrap_coord(z);
    let ix = ((x_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
    let iz = ((z_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
    grid[iz * BAKE_GRID_N + ix]
}

// ═══════════════════════════════════════════════════════════════════════════
// Color ramps (shared with terrain visual-bench conventions)
// ═══════════════════════════════════════════════════════════════════════════

/// Topo elevation ramp: water-blue → shore-tan → lowland-green → foothill-
/// brown → mountain-tan → snow-white. Matches `terrain_visual_bench`.
fn topo_rgb(alt_m: f32) -> [u8; 3] {
    if alt_m < 0.0 {
        // Water — darker deep blue, lighter towards coast.
        let t = ((alt_m + 100.0) / 100.0).clamp(0.0, 1.0);
        let r = 10 + (t * 30.0) as u8;
        let g = 30 + (t * 60.0) as u8;
        let b = 70 + (t * 90.0) as u8;
        [r, g, b]
    } else if alt_m < 50.0 {
        [194, 178, 128] // sand / shore
    } else if alt_m < 600.0 {
        let t = (alt_m - 50.0) / 550.0;
        let r = (124.0 - t * 64.0) as u8;
        let g = (160.0 - t * 40.0) as u8;
        let b = (70.0 - t * 20.0) as u8;
        [r, g, b]
    } else if alt_m < 1800.0 {
        let t = (alt_m - 600.0) / 1200.0;
        let r = (110.0 + t * 50.0) as u8;
        let g = (100.0 + t * 50.0) as u8;
        let b = (70.0 + t * 30.0) as u8;
        [r, g, b]
    } else if alt_m < 3000.0 {
        let t = (alt_m - 1800.0) / 1200.0;
        let r = (160.0 + t * 70.0) as u8;
        let g = (150.0 + t * 80.0) as u8;
        let b = (100.0 + t * 100.0) as u8;
        [r, g, b]
    } else {
        let t = ((alt_m - 3000.0) / 1500.0).clamp(0.0, 1.0);
        let v = (230.0 + t * 25.0) as u8;
        [v, v, v]
    }
}

/// Log-ramp for flow accumulation. Maps log1p(accum) to blue→cyan→green→
/// yellow→red.
fn accum_rgb(accum: f32, max_accum: f32) -> [u8; 3] {
    let lo = 0.0_f32.ln_1p();
    let hi = max_accum.ln_1p().max(lo + 1.0);
    let t = ((accum.ln_1p() - lo) / (hi - lo)).clamp(0.0, 1.0);
    if t < 0.25 {
        let u = t / 0.25;
        [10, (u * 160.0) as u8, (150.0 + u * 100.0) as u8]
    } else if t < 0.5 {
        let u = (t - 0.25) / 0.25;
        [
            (u * 80.0) as u8,
            (160.0 + u * 95.0) as u8,
            (250.0 - u * 250.0) as u8,
        ]
    } else if t < 0.75 {
        let u = (t - 0.5) / 0.25;
        [(80.0 + u * 175.0) as u8, 255, 0]
    } else {
        let u = (t - 0.75) / 0.25;
        [255, (255.0 - u * 200.0) as u8, 0]
    }
}

/// 8-way directional color for D8 flow direction. Each direction gets a
/// distinct hue (wrap around color wheel). `u32::MAX` (sea sink) = dark.
fn d8_dir_rgb(upstream_cell_idx: usize, down_cell_idx: u32) -> [u8; 3] {
    if down_cell_idx == u32::MAX {
        return [20, 20, 40]; // sea sink
    }
    let d = down_cell_idx as i64 - upstream_cell_idx as i64;
    let n = BAKE_GRID_N as i64;
    let (dx, dz) = if d == -n - 1 || d == n - 1 - n * n * 0 {
        (-1, -1)
    } else if d == -n {
        (0, -1)
    } else if d == -n + 1 {
        (1, -1)
    } else if d == -1 {
        (-1, 0)
    } else if d == 1 {
        (1, 0)
    } else if d == n - 1 {
        (-1, 1)
    } else if d == n {
        (0, 1)
    } else if d == n + 1 {
        (1, 1)
    } else {
        // Wrapped across seam — fallback.
        (0, 0)
    };
    // Map (dx, dz) to hue.
    let angle = libm::atan2f(dz as f32, dx as f32); // -π..π
    let t = (angle + core::f32::consts::PI) / (2.0 * core::f32::consts::PI); // 0..1
    let hue = t * 6.0;
    let sector = hue.floor() as i32;
    let f = hue - sector as f32;
    let v = 200.0_f32;
    let p = 50.0_f32;
    let q = v - (v - p) * f;
    let w = v - (v - p) * (1.0 - f);
    let (r, g, b) = match sector {
        0 => (v, w, p),
        1 => (q, v, p),
        2 => (p, v, w),
        3 => (p, q, v),
        4 => (w, p, v),
        _ => (v, p, q),
    };
    [r as u8, g as u8, b as u8]
}

fn hillshade(dh_dx: f32, dh_dz: f32) -> f32 {
    let normal_x = -dh_dx;
    let normal_z = -dh_dz;
    let normal_y = 1.0_f32;
    let n_len = libm::sqrtf(normal_x * normal_x + normal_y * normal_y + normal_z * normal_z);
    // Sun from upper-left.
    let sun_x = -0.577_f32;
    let sun_y = 0.577;
    let sun_z = 0.577;
    let dot = (normal_x * sun_x + normal_y * sun_y + normal_z * sun_z) / n_len;
    (0.6 + 0.65 * dot.max(0.0)).clamp(0.4, 1.3)
}

// ═══════════════════════════════════════════════════════════════════════════
// Visual-bench tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hydrology_bake_writes_artifacts_dir() {
    // Smoke test that also ensures the canonical bake runs inside test
    // mode. Runs in normal `cargo test` (non-ignored) so CI gets a
    // determinism touchpoint even without --ignored.
    let (_, g) = shared_bake();
    assert_eq!(g.seed(), CANONICAL_SEED);
    // Dir exists for subsequent --ignored artifact emits.
    let _ = output_dir();
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_accumulation_artifacts() {
    let (_, g) = shared_bake();
    let accum = g.flow_accumulation_grid();
    let max_accum = accum.iter().copied().fold(0.0_f32, f32::max);
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let a = sample_cell_at_pixel(accum, crop, px, py);
                let rgb = accum_rgb(a, max_accum);
                img.put_pixel(px, py, Rgb(rgb));
            }
        }
        write_png(&img, &dir, &format!("accumulation_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_flow_direction_artifacts() {
    let (_, g) = shared_bake();
    let dirs = g.d8_downstream_grid();
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let x_wrapped = flowplan_core::wrap_coord(x);
                let z_wrapped = flowplan_core::wrap_coord(z);
                let ix = ((x_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
                let iz = ((z_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
                let cell_idx = iz * BAKE_GRID_N + ix;
                let rgb = d8_dir_rgb(cell_idx, dirs[cell_idx]);
                img.put_pixel(px, py, Rgb(rgb));
            }
        }
        write_png(&img, &dir, &format!("flow_direction_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_wetness_artifacts() {
    let (_, g) = shared_bake();
    let basins = g.basin_grid();
    let raw = g.raw_elevation_grid();
    let dir = output_dir();

    // Per-basin fill ratio lookup table.
    let lakes = g.lakes();
    let mut fill_ratio: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
    for b in lakes {
        let ratio = if b.area_cells > 0 {
            b.water_area_cells as f32 / b.area_cells as f32
        } else {
            0.0
        };
        fill_ratio.insert(b.id, ratio);
    }

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let x_wrapped = flowplan_core::wrap_coord(x);
                let z_wrapped = flowplan_core::wrap_coord(z);
                let ix = ((x_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
                let iz = ((z_wrapped / BAKE_RESOLUTION_M) as usize).min(BAKE_GRID_N - 1);
                let cell_idx = iz * BAKE_GRID_N + ix;
                let basin = basins[cell_idx];
                let alt = raw[cell_idx];

                let rgb = if alt <= 0.0 {
                    [18, 36, 78] // sea — dark blue
                } else if let Some(&r) = fill_ratio.get(&basin) {
                    // Basin fill color: red (dry) → yellow (partial) → green (brim).
                    if r >= 0.99 {
                        [50, 180, 50] // brim / overflow
                    } else if r >= 0.5 {
                        [210, 200, 40] // partial
                    } else if r > 0.0 {
                        [220, 110, 40] // small fraction
                    } else {
                        [130, 80, 40] // near-dry
                    }
                } else {
                    // Dry land colored by elevation topo ramp, muted.
                    let base = topo_rgb(alt);
                    [
                        (base[0] as f32 * 0.6) as u8,
                        (base[1] as f32 * 0.6) as u8,
                        (base[2] as f32 * 0.6) as u8,
                    ]
                };
                img.put_pixel(px, py, Rgb(rgb));
            }
        }
        write_png(&img, &dir, &format!("wetness_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_hydrology_composite_artifacts() {
    let (terrain, g) = shared_bake();
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let pixel_m = crop.extent_m / RESOLUTION as f32;
        let min_spacing_m = pixel_m * 0.5;

        // Pass 1: per-pixel topo + lakes (pixel-accurate sea + lake
        // surfaces via terrain height comparison).
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let (x, z) = crop.world_coord(px, py);
                let h = terrain.height_at_scale(x, z, min_spacing_m);
                let h_east = terrain.height_at_scale(x + pixel_m, z, min_spacing_m);
                let h_south = terrain.height_at_scale(x, z + pixel_m, min_spacing_m);
                let dh_dx = (h_east - h) / pixel_m;
                let dh_dz = (h_south - h) / pixel_m;
                let shade = hillshade(dh_dx, dh_dz);

                let rgb = if h <= 0.0 {
                    // Sea: per-pixel h test → smooth coastline.
                    let topo = topo_rgb(h);
                    [
                        (topo[0] as f32 * shade).clamp(0.0, 255.0) as u8,
                        (topo[1] as f32 * shade).clamp(0.0, 255.0) as u8,
                        (topo[2] as f32 * shade).clamp(0.0, 255.0) as u8,
                    ]
                } else {
                    // Above sea: topo + lakes (per-pixel inside-basin test).
                    let topo = topo_rgb(h);
                    let mut r = topo[0] as f32 * shade;
                    let mut gc = topo[1] as f32 * shade;
                    let mut b = topo[2] as f32 * shade;
                    if let Some(s) = g.water_surface_at(x, z) {
                        if matches!(
                            s.class,
                            WaterClass::LakeLarge
                                | WaterClass::LakeSmall
                                | WaterClass::AlpineTarn
                                | WaterClass::ClosedBasinLake
                        ) && h < s.elevation_m
                        {
                            let depth = (s.elevation_m - h).clamp(0.0, 50.0);
                            let t = depth / 50.0;
                            r = 80.0 - t * 30.0;
                            gc = 140.0 - t * 40.0;
                            b = 200.0 - t * 40.0;
                        } else if matches!(s.class, WaterClass::DeltaFan) {
                            // Delta fan — warm-tan sediment tint over the
                            // topo base, visible even where water is absent.
                            r = 200.0;
                            gc = 180.0;
                            b = 130.0;
                        } else if matches!(s.class, WaterClass::DryChannel) {
                            // Dry channels are rendered via the river-polyline
                            // overlay pass below; keep topo here.
                        }
                    }
                    [
                        r.clamp(0.0, 255.0) as u8,
                        gc.clamp(0.0, 255.0) as u8,
                        b.clamp(0.0, 255.0) as u8,
                    ]
                };
                img.put_pixel(px, py, Rgb(rgb));
            }
        }

        // Pass 2: vector river polylines. Renders 1-pixel-wide at wide
        // crops, thicker for the player view so rivers stay visible at
        // close zoom where they're often the only hydrology content.
        let river_color_river = [80_u8, 160, 220];
        let river_color_creek = [140_u8, 190, 220];
        // Thicker at player scale (~10 m/pixel × 3 ≈ 30 m wide);
        // 1-pixel-wide at wide crops.
        let radius_px = i32::from(crop.extent_m <= PLAYER_EXTENT_M);

        for river in g.rivers() {
            let color = match river.class {
                WaterClass::Creek => river_color_creek,
                _ => river_color_river,
            };
            for w in river.waypoints.windows(2) {
                let (x0, z0, _) = w[0];
                let (x1, z1, _) = w[1];
                if let (Some((px0, py0)), Some((px1, py1))) =
                    (world_to_pixel(crop, x0, z0), world_to_pixel(crop, x1, z1))
                {
                    draw_line_bold(&mut img, px0, py0, px1, py1, color, radius_px);
                }
            }
        }

        write_png(&img, &dir, &format!("composite_{}.png", crop.name));
    }
}

// Convert world (x, z) to pixel coords within the crop. Respects
// toroidal wrap: the crop can straddle the seam (e.g. player crop at
// origin (0,0) with [-5k, +5k] bounds overlaps [W-5k, W) + [0, 5k)).
// For each candidate (x, z) we try the nearest equivalent position
// (shifted by ±WORLD_EXTENT_M per axis) and accept the first that
// falls inside the crop.
fn world_to_pixel(crop: Crop, x: f32, z: f32) -> Option<(i32, i32)> {
    for dx in [-WORLD_EXTENT_M, 0.0, WORLD_EXTENT_M] {
        for dz in [-WORLD_EXTENT_M, 0.0, WORLD_EXTENT_M] {
            let xc = x + dx;
            let zc = z + dz;
            let u = (xc - crop.center_x_m) / crop.extent_m + 0.5;
            let v = 0.5 - (zc - crop.center_z_m) / crop.extent_m;
            if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
                let px = (u * RESOLUTION as f32) as i32;
                let py = (v * RESOLUTION as f32) as i32;
                return Some((px, py));
            }
        }
    }
    None
}

// Bresenham line drawer with radius (draws `radius_px` copies with 1-px
// offsets for bolded lines). Copy of the pattern from terrain_visual_bench.
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

fn draw_line_bresenham(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 3],
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;
    let w = RESOLUTION as i32;
    let h = RESOLUTION as i32;
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

#[test]
fn bake_stats_sanity() {
    // Prints bake stats at test time for tuning feedback. Doesn't assert
    // bounds (those belong in the main unit tests) — just reports so
    // `cargo test -- --nocapture` lets us see how the wetness priors
    // are trending.
    let (_, g) = shared_bake();
    let total = BAKE_GRID_N * BAKE_GRID_N;
    // Count cells by water class via public API. basin_grid's SEA_BASIN_ID
    // means "drains to sea" (every land cell), NOT "is sea" — so we
    // count water classes instead.
    let mut n_sea = 0_usize;
    let mut n_lake = 0_usize;
    let mut n_river = 0_usize;
    let mut n_creek = 0_usize;
    let mut n_delta = 0_usize;
    let mut n_dry_channel = 0_usize;
    let mut n_dry_land = 0_usize;
    for iz in 0..BAKE_GRID_N {
        for ix in 0..BAKE_GRID_N {
            let x = (ix as f32 + 0.5) * BAKE_RESOLUTION_M;
            let z = (iz as f32 + 0.5) * BAKE_RESOLUTION_M;
            match g.water_surface_at(x, z).map(|s| s.class) {
                Some(
                    WaterClass::OpenOcean | WaterClass::OpenCoastal | WaterClass::ProtectedBay,
                ) => {
                    n_sea += 1;
                },
                Some(
                    WaterClass::LakeLarge
                    | WaterClass::LakeSmall
                    | WaterClass::AlpineTarn
                    | WaterClass::ClosedBasinLake,
                ) => n_lake += 1,
                Some(WaterClass::River) => n_river += 1,
                Some(WaterClass::Creek) => n_creek += 1,
                Some(WaterClass::DeltaFan) => n_delta += 1,
                Some(WaterClass::DryChannel) => n_dry_channel += 1,
                None => n_dry_land += 1,
            }
        }
    }
    eprintln!(
        "hydrology bake stats: seed={:x} lakes={} rivers={} coastline_segments={}",
        CANONICAL_SEED,
        g.lake_count(),
        g.river_count(),
        g.coastline_segments().len()
    );
    eprintln!("  cells ({total} total):");
    eprintln!(
        "  sea: {} ({:.1}%)",
        n_sea,
        100.0 * n_sea as f32 / total as f32
    );
    eprintln!(
        "  lake: {} ({:.1}%)",
        n_lake,
        100.0 * n_lake as f32 / total as f32
    );
    eprintln!(
        "  river: {} ({:.1}%)",
        n_river,
        100.0 * n_river as f32 / total as f32
    );
    eprintln!(
        "  creek: {} ({:.1}%)",
        n_creek,
        100.0 * n_creek as f32 / total as f32
    );
    eprintln!(
        "  delta_fan: {} ({:.1}%)",
        n_delta,
        100.0 * n_delta as f32 / total as f32
    );
    eprintln!(
        "  dry_channel: {} ({:.1}%)",
        n_dry_channel,
        100.0 * n_dry_channel as f32 / total as f32
    );
    eprintln!(
        "  dry_land: {} ({:.1}%)",
        n_dry_land,
        100.0 * n_dry_land as f32 / total as f32
    );

    // Terrain altitude distribution from the raw coarse grid. Tells us
    // whether "too much sea" is a terrain issue (altitudes skew low) or
    // a hydrology classification issue (terrain is fine but bake is
    // misreading it).
    let raw = g.raw_elevation_grid();
    let mut buckets = [0_u32; 9]; // < -100, -100..0, 0..100, 100..500, 500..1000, 1000..1500, 1500..2500, 2500..3500, >= 3500
    let mut sum = 0.0_f64;
    let mut mn = f32::INFINITY;
    let mut mx = f32::NEG_INFINITY;
    for &h in raw.iter() {
        sum += h as f64;
        if h < mn {
            mn = h;
        }
        if h > mx {
            mx = h;
        }
        let idx = if h < -100.0 {
            0
        } else if h < 0.0 {
            1
        } else if h < 100.0 {
            2
        } else if h < 500.0 {
            3
        } else if h < 1000.0 {
            4
        } else if h < 1500.0 {
            5
        } else if h < 2500.0 {
            6
        } else if h < 3500.0 {
            7
        } else {
            8
        };
        buckets[idx] += 1;
    }
    let mean = sum / raw.len() as f64;
    eprintln!("terrain altitude: min={mn:.1} max={mx:.1} mean={mean:.1}");
    let labels = [
        "<-100",
        "-100..0",
        "0..100",
        "100..500",
        "500..1000",
        "1000..1500",
        "1500..2500",
        "2500..3500",
        ">=3500",
    ];
    for (i, &c) in buckets.iter().enumerate() {
        eprintln!(
            "  {}: {} ({:.1}%)",
            labels[i],
            c,
            100.0 * c as f32 / raw.len() as f32
        );
    }

    // Biome distribution via cell-center sampling.
    let mut bn_desert = 0_u32;
    let mut bn_alpine = 0_u32;
    let mut bn_coastal = 0_u32;
    let mut bn_med = 0_u32;
    for iz in 0..BAKE_GRID_N {
        for ix in 0..BAKE_GRID_N {
            let x = (ix as f32 + 0.5) * BAKE_RESOLUTION_M;
            let z = (iz as f32 + 0.5) * BAKE_RESOLUTION_M;
            match flowplan_world::biome::biome_at(CANONICAL_SEED, x, z) {
                flowplan_world::biome::BiomeClass::HighDesert => bn_desert += 1,
                flowplan_world::biome::BiomeClass::RockyAlpine => bn_alpine += 1,
                flowplan_world::biome::BiomeClass::CoastalBoreal => bn_coastal += 1,
                flowplan_world::biome::BiomeClass::MediterraneanArchipelago => bn_med += 1,
            }
        }
    }
    let bt = (BAKE_GRID_N * BAKE_GRID_N) as f32;
    eprintln!(
        "biome distribution: HighDesert={:.1}% RockyAlpine={:.1}% CoastalBoreal={:.1}% Mediterranean={:.1}%",
        100.0 * bn_desert as f32 / bt,
        100.0 * bn_alpine as f32 / bt,
        100.0 * bn_coastal as f32 / bt,
        100.0 * bn_med as f32 / bt,
    );
    let _ = WORLD_EXTENT_M;
}
