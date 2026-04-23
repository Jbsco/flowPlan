//! Hub catalog + magnetic variation visual-bench (Phase 3.6.1e).
//!
//! Emits PNG artifacts for hub distribution, region Voronoi partition,
//! magnetic variation field, and a composite view overlaying hubs on
//! the hydrology / topo base.
//!
//! # Running
//!
//! Default: `#[ignore]` — opt-in via
//! `cargo test -p flowplan-world --release --test hub_catalog_visual_bench -- --ignored`.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::expect_used)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::explicit_iter_loop)]
#![allow(let_underscore_drop)]

use std::path::Path;

use flowplan_core::WORLD_EXTENT_M;
use flowplan_world::hub_catalog::{HubCatalog, HubTier};
use flowplan_world::hydrology::HydrologyGraph;
use flowplan_world::magnetic_variation::{magnetic_variation, MAG_VAR_AMPLITUDE_DEG};
use flowplan_world::terrain::Terrain;
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies for transitive test deps.
use glam as _;
use libm as _;
use ron as _;
use serde as _;

const FULL_EXTENT_M: f32 = 500_000.0;
const CONTINENTAL_EXTENT_M: f32 = 2_000_000.0;
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
    const CONTINENTAL: Self = Self {
        name: "continental",
        extent_m: CONTINENTAL_EXTENT_M,
        center_x_m: 0.0,
        center_z_m: 0.0,
    };
}

const CROPS: &[Crop] = &[Crop::FULL, Crop::QUADRANT, Crop::CONTINENTAL];

fn output_dir() -> std::path::PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| std::path::PathBuf::from("target"), Into::into);
    target
        .join("world-gen-plots/hub_catalog")
        .join(format!("{CANONICAL_SEED:x}"))
}

fn write_png(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, dir: &Path, filename: &str) {
    std::fs::create_dir_all(dir).expect("create output dir");
    let path = dir.join(filename);
    img.save(&path)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

fn shared_bake() -> (Terrain, HydrologyGraph, HubCatalog) {
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let hydrology = HydrologyGraph::bake(CANONICAL_SEED, &terrain);
    let catalog = HubCatalog::bake(CANONICAL_SEED, &terrain, &hydrology);
    (terrain, hydrology, catalog)
}

/// Topo elevation ramp (matches hydrology visual-bench).
fn topo_rgb(alt_m: f32) -> [u8; 3] {
    if alt_m < 0.0 {
        let t = ((alt_m + 100.0) / 100.0).clamp(0.0, 1.0);
        let r = 10 + (t * 30.0) as u8;
        let g = 30 + (t * 60.0) as u8;
        let b = 70 + (t * 90.0) as u8;
        [r, g, b]
    } else if alt_m < 50.0 {
        [194, 178, 128]
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

fn hillshade(dh_dx: f32, dh_dz: f32) -> f32 {
    let normal_x = -dh_dx;
    let normal_z = -dh_dz;
    let normal_y = 1.0_f32;
    let n_len = libm::sqrtf(normal_x * normal_x + normal_y * normal_y + normal_z * normal_z);
    let sun_x = -0.577_f32;
    let sun_y = 0.577;
    let sun_z = 0.577;
    let dot = (normal_x * sun_x + normal_y * sun_y + normal_z * sun_z) / n_len;
    (0.6 + 0.65 * dot.max(0.0)).clamp(0.4, 1.3)
}

/// Convert world (x, z) to pixel coords within a crop, respecting
/// toroidal wrap.
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

/// Draw a filled disc of `radius_px` pixels centered at (cx, cy).
fn draw_disc(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    cx: i32,
    cy: i32,
    radius_px: i32,
    color: [u8; 3],
) {
    let r2 = radius_px * radius_px;
    let w = RESOLUTION as i32;
    let h = RESOLUTION as i32;
    for dy in -radius_px..=radius_px {
        for dx in -radius_px..=radius_px {
            if dx * dx + dy * dy > r2 {
                continue;
            }
            let x = cx + dx;
            let y = cy + dy;
            if x >= 0 && x < w && y >= 0 && y < h {
                img.put_pixel(x as u32, y as u32, Rgb(color));
            }
        }
    }
}

/// Per-tier pixel radius + color.
fn tier_glyph(tier: HubTier) -> (i32, [u8; 3]) {
    match tier {
        HubTier::Major => (7, [255, 80, 80]),      // large red dot
        HubTier::Regional => (5, [255, 160, 40]),  // medium orange
        HubTier::Local => (3, [255, 240, 80]),     // small yellow
        HubTier::Remote => (2, [130, 255, 130]),   // tiny green
        HubTier::Isolated => (1, [200, 200, 255]), // single pixel light blue
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests — determinism smoke (runs in default cargo test)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hub_catalog_bake_smoke() {
    let (_, _, c) = shared_bake();
    assert_eq!(c.seed(), CANONICAL_SEED);
    assert!(c.hub_count() > 1000);
    let _ = output_dir();
}

// ═══════════════════════════════════════════════════════════════════════════
// Visual-bench artifacts
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_hub_scatter_artifacts() {
    let (_, _, c) = shared_bake();
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        // Dark background so bright hub dots pop.
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                img.put_pixel(px, py, Rgb([24, 28, 40]));
            }
        }
        // Place hubs largest-tier first so smaller-tier dots draw on top
        // and are visible. Actually reverse — we want MAJOR prominent so
        // draw MAJOR last (on top of smaller tiers).
        for tier in [
            HubTier::Isolated,
            HubTier::Remote,
            HubTier::Local,
            HubTier::Regional,
            HubTier::Major,
        ] {
            let (r, color) = tier_glyph(tier);
            for h in c.hubs().iter().filter(|h| h.tier == tier) {
                if let Some((px, py)) =
                    world_to_pixel(crop, h.approx_world_pos.0, h.approx_world_pos.1)
                {
                    draw_disc(&mut img, px, py, r, color);
                }
            }
        }
        write_png(&img, &dir, &format!("hub_scatter_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_region_voronoi_artifacts() {
    let (_, _, c) = shared_bake();
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let region_id = c.region_id_at(x, z);
                // Hash region id → distinct pleasant color (HSV-like).
                let hue = region_id.wrapping_mul(2_654_435_761) >> 24;
                let color = [
                    (hue & 0xFF) as u8,
                    ((hue.wrapping_mul(17)) & 0xFF) as u8,
                    ((hue.wrapping_mul(31)) & 0xFF) as u8,
                ];
                // Blend with topo-like base.
                let r = (color[0] as u16 + 80).min(255) as u8;
                let g = (color[1] as u16 + 80).min(255) as u8;
                let b = (color[2] as u16 + 80).min(255) as u8;
                img.put_pixel(px, py, Rgb([r, g, b]));
            }
        }
        // Overlay region seed centers as small white dots.
        for seed in c.region_seeds() {
            if let Some((px, py)) = world_to_pixel(crop, seed.position.0, seed.position.1) {
                draw_disc(&mut img, px, py, 3, [255, 255, 255]);
            }
        }
        write_png(&img, &dir, &format!("region_voronoi_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_magnetic_variation_artifacts() {
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let amplitude = MAG_VAR_AMPLITUDE_DEG;
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let var_deg = magnetic_variation(CANONICAL_SEED, x, z);
                // Diverging color ramp: negative = blue (west), zero =
                // white, positive = red (east). Matches nautical-chart
                // convention for magnetic-variation display.
                let t = (var_deg / amplitude).clamp(-1.0, 1.0);
                let rgb = if t < 0.0 {
                    // Blue gradient.
                    let a = (-t).clamp(0.0, 1.0);
                    [
                        (240.0 - a * 180.0) as u8,
                        (240.0 - a * 180.0) as u8,
                        (255.0 - a * 50.0) as u8,
                    ]
                } else {
                    // Red gradient.
                    [
                        (255.0 - t * 50.0) as u8,
                        (240.0 - t * 180.0) as u8,
                        (240.0 - t * 180.0) as u8,
                    ]
                };
                img.put_pixel(px, py, Rgb(rgb));
            }
        }
        write_png(&img, &dir, &format!("magnetic_variation_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_hub_composite_artifacts() {
    let (terrain, _hydrology, c) = shared_bake();
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let pixel_m = crop.extent_m / RESOLUTION as f32;
        let min_spacing_m = pixel_m * 0.5;

        // Pass 1: topo base with hillshade.
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let h = terrain.height_at_scale(x, z, min_spacing_m);
                let h_east = terrain.height_at_scale(x + pixel_m, z, min_spacing_m);
                let h_south = terrain.height_at_scale(x, z + pixel_m, min_spacing_m);
                let dh_dx = (h_east - h) / pixel_m;
                let dh_dz = (h_south - h) / pixel_m;
                let shade = hillshade(dh_dx, dh_dz);
                let topo = topo_rgb(h);
                img.put_pixel(
                    px,
                    py,
                    Rgb([
                        (topo[0] as f32 * shade).clamp(0.0, 255.0) as u8,
                        (topo[1] as f32 * shade).clamp(0.0, 255.0) as u8,
                        (topo[2] as f32 * shade).clamp(0.0, 255.0) as u8,
                    ]),
                );
            }
        }

        // Pass 2: hubs overlaid (smallest tier first; bigger tiers on top).
        for tier in [
            HubTier::Isolated,
            HubTier::Remote,
            HubTier::Local,
            HubTier::Regional,
            HubTier::Major,
        ] {
            let (r, color) = tier_glyph(tier);
            for h in c.hubs().iter().filter(|h| h.tier == tier) {
                if let Some((px, py)) =
                    world_to_pixel(crop, h.approx_world_pos.0, h.approx_world_pos.1)
                {
                    draw_disc(&mut img, px, py, r, color);
                }
            }
        }
        write_png(&img, &dir, &format!("composite_{}.png", crop.name));
    }
}
