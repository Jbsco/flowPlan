//! L3 feature-delta + surface classification visual-bench (Phase 3.6.3a).
//!
//! Emits PNG artifacts comparing L1 (Phase 3.6.1) to L3 (Phase 3.6.3a)
//! heightfields + a surface-class map + a hub-foundation effect zoom.
//!
//! # Running
//!
//! `cargo test -p flowplan-world --release --test l3_feature_visual_bench -- --ignored`
//!
//! Bake is ~30 s (all four bakes compose); per-artifact render is
//! ~10-20 s with feature-assignment caching.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::expect_used)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::useless_let_if_seq)]
#![allow(let_underscore_drop)]

use std::collections::HashMap;
use std::path::Path;

use flowplan_world::composed::WorldRefs;
use flowplan_world::feature_delta::HubFoundationMap;
use flowplan_world::feature_groups::FeatureGroupBundle;
use flowplan_world::hub_catalog::HubCatalog;
use flowplan_world::hydrology::HydrologyGraph;
use flowplan_world::surface_class::SurfaceClass;
use flowplan_world::terrain::{HeightSampler, Terrain};
use flowplan_world::voronoi_cells::SubBiomeCellCatalog;
use image::{ImageBuffer, Rgb};

use bincode as _;
use flowplan_core as _;
use glam as _;
use libm as _;
use rayon as _;
use ron as _;
use serde as _;
use tracing as _;

const FULL_EXTENT_M: f32 = 500_000.0;
const CONTINENTAL_EXTENT_M: f32 = 2_000_000.0;
const HUB_ZOOM_EXTENT_M: f32 = 3_000.0; // 3 km zoom on a major hub
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
        .join("world-gen-plots/l3")
        .join(format!("{CANONICAL_SEED:x}"))
}

fn write_png(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, dir: &Path, filename: &str) {
    std::fs::create_dir_all(dir).expect("create output dir");
    let path = dir.join(filename);
    img.save(&path)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

fn shared_bake() -> (
    Terrain,
    HydrologyGraph,
    SubBiomeCellCatalog,
    HubCatalog,
    HubFoundationMap,
    FeatureGroupBundle,
) {
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let hydrology = HydrologyGraph::bake(CANONICAL_SEED, &terrain);
    let sub_biome = SubBiomeCellCatalog::bake(CANONICAL_SEED);
    let hubs = HubCatalog::bake(CANONICAL_SEED, &terrain, &hydrology);
    let foundations = HubFoundationMap::build(&hubs, &sub_biome);
    let groups = FeatureGroupBundle::build_road_network(&hubs, &sub_biome);
    (terrain, hydrology, sub_biome, hubs, foundations, groups)
}

// ═══════════════════════════════════════════════════════════════════════════
// Color ramps
// ═══════════════════════════════════════════════════════════════════════════

/// Topo ramp matching the hydrology / hub visual-bench convention.
fn topo_rgb(alt_m: f32) -> [u8; 3] {
    if alt_m < 0.0 {
        let t = ((alt_m + 100.0) / 100.0).clamp(0.0, 1.0);
        [
            10 + (t * 30.0) as u8,
            30 + (t * 60.0) as u8,
            70 + (t * 90.0) as u8,
        ]
    } else if alt_m < 50.0 {
        [194, 178, 128]
    } else if alt_m < 600.0 {
        let t = (alt_m - 50.0) / 550.0;
        [
            (124.0 - t * 64.0) as u8,
            (160.0 - t * 40.0) as u8,
            (70.0 - t * 20.0) as u8,
        ]
    } else if alt_m < 1800.0 {
        let t = (alt_m - 600.0) / 1200.0;
        [
            (110.0 + t * 50.0) as u8,
            (100.0 + t * 50.0) as u8,
            (70.0 + t * 30.0) as u8,
        ]
    } else if alt_m < 3000.0 {
        let t = (alt_m - 1800.0) / 1200.0;
        [
            (160.0 + t * 70.0) as u8,
            (150.0 + t * 80.0) as u8,
            (100.0 + t * 100.0) as u8,
        ]
    } else {
        let t = ((alt_m - 3000.0) / 1500.0).clamp(0.0, 1.0);
        let v = (230.0 + t * 25.0) as u8;
        [v, v, v]
    }
}

fn hillshade(dh_dx: f32, dh_dz: f32) -> f32 {
    let n_len = libm::sqrtf(dh_dx * dh_dx + 1.0 + dh_dz * dh_dz);
    let dot = (-dh_dx * -0.577 + 1.0 * 0.577 + -dh_dz * 0.577) / n_len;
    (0.6 + 0.65 * dot.max(0.0)).clamp(0.4, 1.3)
}

/// Diverging ramp for |L3 - L1| delta visualization.
/// Blue = negative (carved), neutral = no delta, red = positive (raised).
fn delta_rgb(delta_m: f32, max_abs: f32) -> [u8; 3] {
    let t = (delta_m / max_abs).clamp(-1.0, 1.0);
    if t < 0.0 {
        let a = -t;
        [
            (240.0 - a * 220.0) as u8,
            (240.0 - a * 140.0) as u8,
            (255.0 - a * 50.0) as u8,
        ]
    } else if t > 0.0 {
        [
            (255.0 - t * 50.0) as u8,
            (240.0 - t * 200.0) as u8,
            (240.0 - t * 220.0) as u8,
        ]
    } else {
        [220, 230, 220] // neutral — no delta
    }
}

/// Surface class → distinct color.
fn surface_rgb(c: SurfaceClass) -> [u8; 3] {
    match c {
        SurfaceClass::Water => [30, 60, 140],
        SurfaceClass::Sand => [220, 200, 140],
        SurfaceClass::Gravel => [150, 140, 120],
        SurfaceClass::Rock => [110, 100, 95],
        SurfaceClass::Cliff => [60, 55, 50],
        SurfaceClass::Mudflat => [130, 110, 80],
        SurfaceClass::Delta => [180, 160, 130],
        SurfaceClass::IceEdge => [230, 240, 250],
        SurfaceClass::Grass => [100, 160, 80],
        SurfaceClass::Paved => [90, 90, 95],
        SurfaceClass::Dirt => [145, 120, 90],
        SurfaceClass::Snow => [245, 250, 255],
        SurfaceClass::PowderSnow => [235, 240, 250],
        SurfaceClass::DryLake => [210, 200, 180],
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn l3_bake_smoke() {
    let (_, _, _, _, f, _) = shared_bake();
    assert!(f.count() > 0, "hub foundation map should have overrides");
    let _ = output_dir();
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_l3_topo_artifacts() {
    // L3-composed topo with hillshade — the "what does the world
    // look like now" artifact post feature-delta composition.
    let (t, h, s, hc, f, fg) = shared_bake();
    let refs = WorldRefs::new(&t, &h, &s, &hc, &f, &fg, None);
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let pixel_m = crop.extent_m / RESOLUTION as f32;
        let min_spacing_m = pixel_m * 0.5;

        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let altitude = refs.height_at_scale(x, z, min_spacing_m);
                let altitude_e = refs.height_at_scale(x + pixel_m, z, min_spacing_m);
                let altitude_s = refs.height_at_scale(x, z + pixel_m, min_spacing_m);
                let dh_dx = (altitude_e - altitude) / pixel_m;
                let dh_dz = (altitude_s - altitude) / pixel_m;
                let shade = hillshade(dh_dx, dh_dz);
                let [r, g, b] = topo_rgb(altitude);
                img.put_pixel(
                    px,
                    py,
                    Rgb([
                        (f32::from(r) * shade).clamp(0.0, 255.0) as u8,
                        (f32::from(g) * shade).clamp(0.0, 255.0) as u8,
                        (f32::from(b) * shade).clamp(0.0, 255.0) as u8,
                    ]),
                );
            }
        }
        write_png(&img, &dir, &format!("l3_topo_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_l3_delta_diff_artifacts() {
    // Diverging red/blue diff map showing where L3 differs from L1.
    // Blue = carved (negative delta), red = raised (positive delta).
    let (t, h, s, hc, f, fg) = shared_bake();
    let refs = WorldRefs::new(&t, &h, &s, &hc, &f, &fg, None);
    let dir = output_dir();
    let max_abs_delta = 200.0_f32; // visualization scale — clamps wilder deltas

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let delta = refs.delta_at(x, z);
                img.put_pixel(px, py, Rgb(delta_rgb(delta, max_abs_delta)));
            }
        }
        write_png(&img, &dir, &format!("l3_delta_diff_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_surface_classification_artifacts() {
    // Surface class map — distinct color per SurfaceClass.
    let (t, h, s, hc, f, fg) = shared_bake();
    let refs = WorldRefs::new(&t, &h, &s, &hc, &f, &fg, None);
    let dir = output_dir();

    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let class = refs.surface_at(x, z);
                img.put_pixel(px, py, Rgb(surface_rgb(class)));
            }
        }
        write_png(&img, &dir, &format!("surface_class_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_hub_foundation_zoom_artifact() {
    // Zoom on a Major hub: side-by-side L1 vs L3 at ~3 km crop.
    // Shows the foundation flatten + skirt in action.
    let (t, h, s, hc, f, fg) = shared_bake();
    let refs = WorldRefs::new(&t, &h, &s, &hc, &f, &fg, None);
    let dir = output_dir();

    // Find a Major hub to center on.
    let sample_hub = hc
        .hubs()
        .iter()
        .find(|hu| matches!(hu.tier, flowplan_world::hub_catalog::HubTier::Major))
        .expect("at least one Major hub in canonical bake");
    let crop = Crop {
        name: "hub_zoom",
        extent_m: HUB_ZOOM_EXTENT_M,
        center_x_m: sample_hub.approx_world_pos.0,
        center_z_m: sample_hub.approx_world_pos.1,
    };

    // Left half = L1 base; right half = L3 composed. Common topo
    // ramp + hillshade so the difference is clear.
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    let pixel_m = crop.extent_m / RESOLUTION as f32;

    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let u = (px as f32 + 0.5) / RESOLUTION as f32;
            let v = (py as f32 + 0.5) / RESOLUTION as f32;
            let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
            let z = crop.center_z_m + (0.5 - v) * crop.extent_m;

            // Split: left half shows L1, right half shows L3.
            let (altitude, ae, as_) = if u < 0.5 {
                (
                    t.height(x, z),
                    t.height(x + pixel_m, z),
                    t.height(x, z + pixel_m),
                )
            } else {
                (
                    refs.height_at(x, z),
                    refs.height_at(x + pixel_m, z),
                    refs.height_at(x, z + pixel_m),
                )
            };

            let dh_dx = (ae - altitude) / pixel_m;
            let dh_dz = (as_ - altitude) / pixel_m;
            let shade = hillshade(dh_dx, dh_dz);
            let [r, g, b] = topo_rgb(altitude);
            let mut rgb = [
                (f32::from(r) * shade).clamp(0.0, 255.0) as u8,
                (f32::from(g) * shade).clamp(0.0, 255.0) as u8,
                (f32::from(b) * shade).clamp(0.0, 255.0) as u8,
            ];

            // Thin divider line at x = 0.5.
            if (u - 0.5).abs() < 0.001 {
                rgb = [255, 60, 60];
            }
            // Mark hub center with a small red cross.
            let center_px = RESOLUTION / 2;
            let center_py = RESOLUTION / 2;
            if (px == center_px && (py as i32 - center_py as i32).abs() <= 8)
                || (py == center_py && (px as i32 - center_px as i32).abs() <= 8)
            {
                rgb = [255, 60, 60];
            }
            img.put_pixel(px, py, Rgb(rgb));
        }
    }
    write_png(&img, &dir, "hub_foundation_zoom_major.png");
    eprintln!(
        "hub zoom: Major hub {} at ({}, {}) altitude ~{:.0} m",
        sample_hub.id,
        sample_hub.approx_world_pos.0,
        sample_hub.approx_world_pos.1,
        t.height(sample_hub.approx_world_pos.0, sample_hub.approx_world_pos.1),
    );
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_delta_stats_diagnostic() {
    // Non-rendering diagnostic — counts cells with non-zero L3 delta
    // + reports surface-class distribution across a wide sample grid.
    let (t, h, s, hc, f, fg) = shared_bake();
    let refs = WorldRefs::new(&t, &h, &s, &hc, &f, &fg, None);
    let sample_n = 200_u32;
    let mut zero_delta = 0_u32;
    let mut pos_delta = 0_u32;
    let mut neg_delta = 0_u32;
    let mut max_pos = 0.0_f32;
    let mut max_neg = 0.0_f32;
    let mut class_counts: HashMap<SurfaceClass, u32> = HashMap::new();
    for ix in 0..sample_n {
        for iz in 0..sample_n {
            let x = (ix as f32 / sample_n as f32 - 0.5) * 2_000_000.0;
            let z = (iz as f32 / sample_n as f32 - 0.5) * 2_000_000.0;
            let delta = refs.delta_at(x, z);
            if delta.abs() < 0.1 {
                zero_delta += 1;
            } else if delta > 0.0 {
                pos_delta += 1;
                if delta > max_pos {
                    max_pos = delta;
                }
            } else {
                neg_delta += 1;
                if delta < max_neg {
                    max_neg = delta;
                }
            }
            *class_counts.entry(refs.surface_at(x, z)).or_insert(0) += 1;
        }
    }
    let total = sample_n * sample_n;
    eprintln!("L3 delta stats (sample n={total}):");
    eprintln!(
        "  zero (|delta| < 0.1 m): {} ({:.1}%)",
        zero_delta,
        100.0 * f32::from(zero_delta > 0) * zero_delta as f32 / total as f32
    );
    eprintln!(
        "  positive delta: {} ({:.1}%), max +{:.1} m",
        pos_delta,
        100.0 * pos_delta as f32 / total as f32,
        max_pos
    );
    eprintln!(
        "  negative delta: {} ({:.1}%), max {:.1} m",
        neg_delta,
        100.0 * neg_delta as f32 / total as f32,
        max_neg
    );
    eprintln!("surface class distribution:");
    let mut sorted: Vec<(SurfaceClass, u32)> = class_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    for (class, count) in sorted {
        eprintln!(
            "  {:?}: {} ({:.1}%)",
            class,
            count,
            100.0 * count as f32 / total as f32
        );
    }
}
