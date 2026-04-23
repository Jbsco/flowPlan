//! Sub-biome Voronoi partition visual-bench (Phase 3.6.2).
//!
//! Emits PNG artifacts for L2 Voronoi cells + feature-type assignment
//! + bleed-over identification + composite view overlaying cells on
//! the hydrology / topo base.
//!
//! # Running
//!
//! Default: `#[ignore]` — opt-in via
//! `cargo test -p flowplan-world --release --test sub_biome_visual_bench -- --ignored`.
//! Bake is ~25-30 s (Bridson Poisson-disk dominates); feature
//! assignment is memoized per crop to keep rendering fast.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::expect_used)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::branches_sharing_code)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(let_underscore_drop)]

use std::collections::HashMap;
use std::path::Path;

use flowplan_core::WORLD_EXTENT_M;
use flowplan_world::baked_assignments::BakedAssignments;
use flowplan_world::hub_catalog::HubCatalog;
use flowplan_world::hydrology::HydrologyGraph;
use flowplan_world::sub_biome::FeatureAssignment;
use flowplan_world::terrain::Terrain;
use flowplan_world::voronoi_cells::SubBiomeCellCatalog;
use image::{ImageBuffer, Rgb};

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
        .join("world-gen-plots/sub_biome")
        .join(format!("{CANONICAL_SEED:x}"))
}

fn write_png(img: &ImageBuffer<Rgb<u8>, Vec<u8>>, dir: &Path, filename: &str) {
    std::fs::create_dir_all(dir).expect("create output dir");
    let path = dir.join(filename);
    img.save(&path)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    eprintln!("wrote {}", path.display());
}

fn shared_bake() -> (Terrain, HydrologyGraph, SubBiomeCellCatalog) {
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let hydrology = HydrologyGraph::bake(CANONICAL_SEED, &terrain);
    let cells = SubBiomeCellCatalog::bake(CANONICAL_SEED);
    (terrain, hydrology, cells)
}

/// Per-crop feature cache — memoize feature_at_cell across pixels so
/// each cell's feature is computed once, not `pixels_per_cell` times.
type FeatureCache = HashMap<u32, FeatureAssignment>;

fn cached_feature<'a>(
    cache: &'a mut FeatureCache,
    cells: &SubBiomeCellCatalog,
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
    cell_id: u32,
) -> &'a FeatureAssignment {
    cache
        .entry(cell_id)
        .or_insert_with(|| cells.feature_at_cell(cell_id, terrain, hydrology))
}

/// Convert world (x, z) → pixel coord in crop (toroidal wrap aware).
/// Currently unused by the per-pixel rendering passes but kept for
/// later overlay work (e.g. drawing region-boundary polylines).
#[allow(dead_code)]
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

/// Topo elevation ramp (shared with other visual-bench files).
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

/// Hash a string to an RGB color (distinct-but-repeatable per id).
fn hash_string_rgb(s: &str) -> [u8; 3] {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    [
        (h & 0xFF) as u8,
        ((h >> 8) & 0xFF) as u8,
        ((h >> 16) & 0xFF) as u8,
    ]
}

/// Map cell_id to distinct pleasant color (HSV-wheel).
fn cell_id_rgb(cell_id: u32) -> [u8; 3] {
    let hue = (cell_id.wrapping_mul(2_654_435_761)) >> 24;
    [
        (hue & 0xFF) as u8,
        ((hue.wrapping_mul(17)) & 0xFF) as u8,
        ((hue.wrapping_mul(31)) & 0xFF) as u8,
    ]
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sub_biome_bake_smoke() {
    let (_, _, c) = shared_bake();
    assert!(c.cell_count() > 3_000_000);
    let _ = output_dir();
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_cell_partition_artifacts() {
    let (_, _, cells) = shared_bake();
    let dir = output_dir();
    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let cell_id = cells.nearest_cell(x, z);
                let base = cell_id_rgb(cell_id);
                // Brighten slightly so adjacent cells have contrast.
                let brightened = [
                    (base[0] as u16 + 60).min(255) as u8,
                    (base[1] as u16 + 60).min(255) as u8,
                    (base[2] as u16 + 60).min(255) as u8,
                ];
                img.put_pixel(px, py, Rgb(brightened));
            }
        }
        write_png(&img, &dir, &format!("cell_partition_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_feature_scatter_artifacts() {
    let (terrain, hydrology, cells) = shared_bake();
    let dir = output_dir();
    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let mut cache: FeatureCache = FeatureCache::new();
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let cell_id = cells.nearest_cell(x, z);
                let feature = cached_feature(&mut cache, &cells, &terrain, &hydrology, cell_id);
                let rgb = hash_string_rgb(&feature.feature_id);
                img.put_pixel(px, py, Rgb(rgb));
            }
        }
        write_png(&img, &dir, &format!("feature_scatter_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_bleed_over_artifacts() {
    let (terrain, hydrology, cells) = shared_bake();
    let dir = output_dir();
    // Color scheme:
    //   deep blue = Primary (most cells)
    //   orange = BleedOver cell (cross-biome feature sneaking in)
    //   red = Fallback (constraint rejection - transient)
    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let mut cache: FeatureCache = FeatureCache::new();
        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;
                let cell_id = cells.nearest_cell(x, z);
                let feature = cached_feature(&mut cache, &cells, &terrain, &hydrology, cell_id);
                let rgb = if feature.is_fallback {
                    [220, 40, 40] // red: fallback
                } else if feature.source.is_bleed_over() {
                    [240, 150, 40] // orange: bleed-over
                } else {
                    [30, 60, 140] // deep blue: primary
                };
                img.put_pixel(px, py, Rgb(rgb));
            }
        }
        write_png(&img, &dir, &format!("bleed_over_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_composite_artifacts() {
    let (terrain, hydrology, cells) = shared_bake();
    let dir = output_dir();
    for &crop in CROPS {
        let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
        let pixel_m = crop.extent_m / RESOLUTION as f32;
        let min_spacing_m = pixel_m * 0.5;
        let mut cache: FeatureCache = FeatureCache::new();

        for py in 0..RESOLUTION {
            for px in 0..RESOLUTION {
                let u = (px as f32 + 0.5) / RESOLUTION as f32;
                let v = (py as f32 + 0.5) / RESOLUTION as f32;
                let x = crop.center_x_m + (u - 0.5) * crop.extent_m;
                let z = crop.center_z_m + (0.5 - v) * crop.extent_m;

                // Topo base + hillshade.
                let h = terrain.height_at_scale(x, z, min_spacing_m);
                let h_e = terrain.height_at_scale(x + pixel_m, z, min_spacing_m);
                let h_s = terrain.height_at_scale(x, z + pixel_m, min_spacing_m);
                let dh_dx = (h_e - h) / pixel_m;
                let dh_dz = (h_s - h) / pixel_m;
                let shade = hillshade(dh_dx, dh_dz);
                let topo = topo_rgb(h);

                // Feature tint: blend 40% of feature-hash color with
                // topo. Bleed-over cells get extra orange boost.
                let cell_id = cells.nearest_cell(x, z);
                let feature = cached_feature(&mut cache, &cells, &terrain, &hydrology, cell_id);
                let feat_rgb = hash_string_rgb(&feature.feature_id);
                let alpha = if feature.source.is_bleed_over() {
                    0.55_f32 // stronger tint for bleed-over
                } else {
                    0.35
                };
                let r = (topo[0] as f32 * (1.0 - alpha) + feat_rgb[0] as f32 * alpha) * shade;
                let g = (topo[1] as f32 * (1.0 - alpha) + feat_rgb[1] as f32 * alpha) * shade;
                let b = (topo[2] as f32 * (1.0 - alpha) + feat_rgb[2] as f32 * alpha) * shade;
                img.put_pixel(
                    px,
                    py,
                    Rgb([
                        r.clamp(0.0, 255.0) as u8,
                        g.clamp(0.0, 255.0) as u8,
                        b.clamp(0.0, 255.0) as u8,
                    ]),
                );
            }
        }
        write_png(&img, &dir, &format!("composite_{}.png", crop.name));
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_catalog_stats() {
    // Non-rendering diagnostic. Walks a sample of cells + reports
    // feature-type distribution + bleed-over rate + fallback rate.
    let (terrain, hydrology, cells) = shared_bake();
    let sample_size = 20_000_u32;
    let stride = (cells.cell_count() as u32 / sample_size).max(1);
    let mut feature_counts: HashMap<String, u32> = HashMap::new();
    let mut bleed_count = 0_u32;
    let mut fallback_count = 0_u32;
    let mut n = 0_u32;
    for cell_id in (0..cells.cell_count() as u32).step_by(stride as usize) {
        let f = cells.feature_at_cell(cell_id, &terrain, &hydrology);
        *feature_counts.entry(f.feature_id.clone()).or_insert(0) += 1;
        if f.source.is_bleed_over() {
            bleed_count += 1;
        }
        if f.is_fallback {
            fallback_count += 1;
        }
        n += 1;
    }
    eprintln!("sub-biome catalog stats (sample n={n}):");
    eprintln!(
        "  bleed-over rate: {:.1}% ({bleed_count}/{n})",
        100.0 * bleed_count as f32 / n as f32
    );
    eprintln!(
        "  fallback rate: {:.1}% ({fallback_count}/{n})",
        100.0 * fallback_count as f32 / n as f32
    );
    // Top 15 features.
    let mut sorted: Vec<(String, u32)> = feature_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("  top features:");
    for (id, count) in sorted.iter().take(15) {
        eprintln!(
            "    {id}: {count} ({:.1}%)",
            100.0 * *count as f32 / n as f32
        );
    }
}

#[test]
#[ignore = "visual-bench: opt-in via `cargo test -- --ignored`"]
fn emit_placement_rule_rejection_stats() {
    // 3.6.3c.1b calibration signal: compare feature frequencies across
    // a cell sample with and without PlacementRules evaluation. The
    // delta shows rules ARE firing + their net effect on the catalog.
    //
    // Columns: feature_id | without_rules | with_rules | delta
    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let hydrology = HydrologyGraph::bake(CANONICAL_SEED, &terrain);
    let cells = SubBiomeCellCatalog::bake(CANONICAL_SEED);
    let hub_catalog = HubCatalog::bake(CANONICAL_SEED, &terrain, &hydrology);

    let sample_size = 10_000_u32;
    let stride = (cells.cell_count() as u32 / sample_size).max(1);
    let mut without: HashMap<String, u32> = HashMap::new();
    let mut with_rules: HashMap<String, u32> = HashMap::new();
    let mut differ = 0_u32;
    let mut n = 0_u32;
    for cell_id in (0..cells.cell_count() as u32).step_by(stride as usize) {
        let a = cells.feature_at_cell(cell_id, &terrain, &hydrology);
        let b = cells.feature_at_cell_with_rules(cell_id, &terrain, &hydrology, &hub_catalog);
        *without.entry(a.feature_id.clone()).or_insert(0) += 1;
        *with_rules.entry(b.feature_id.clone()).or_insert(0) += 1;
        if a.feature_id != b.feature_id {
            differ += 1;
        }
        n += 1;
    }

    eprintln!(
        "placement-rule rejection stats (sample n={n}):\n  cells reassigned by rules: {differ} ({:.2}%)",
        100.0 * differ as f32 / n as f32
    );

    // Sorted diff table — features whose count moved the most.
    let mut keys: std::collections::BTreeSet<String> = without.keys().cloned().collect();
    keys.extend(with_rules.keys().cloned());
    let mut rows: Vec<(String, i32, u32, u32)> = keys
        .into_iter()
        .map(|k| {
            let a = i32::try_from(*without.get(&k).unwrap_or(&0)).unwrap_or(0);
            let b = i32::try_from(*with_rules.get(&k).unwrap_or(&0)).unwrap_or(0);
            (k, b - a, a as u32, b as u32)
        })
        .collect();
    rows.sort_by_key(|(_, d, _, _)| -d.abs());

    eprintln!(
        "  {:24}  {:>8} {:>8} {:>8}",
        "feature_id", "without", "with", "delta"
    );
    for (id, delta, a, b) in rows.iter().take(25) {
        let sign = if *delta >= 0 { "+" } else { "" };
        eprintln!("    {id:24}  {a:>8} {b:>8} {sign}{delta:>7}");
    }

    // Sanity assertion: rules fired on at least *some* cells. With 18
    // features authored with rules + 10k samples, we'd expect >0.1 %
    // reassignment even on a conservative canonical seed.
    assert!(
        differ > 0,
        "expected PlacementRules to reassign at least some cells; sample shows 0 — did wiring regress?"
    );
}

#[test]
#[ignore = "expensive bake (~30-90 s); opt-in via --ignored"]
fn emit_baked_assignments_calibration() {
    // 3.6.3c.1c calibration: full eager bake of all ~3.9 M cells with
    // self-spacing rule enforcement + attractor weight_mult biasing.
    // Reports wall-clock + per-feature counts + self-spacing reject
    // count + bleed-over rate. Compares baked output (with self-
    // spacing) against the lazy with-rules path (no self-spacing) so
    // the spacing-enforced delta is visible.

    let terrain = Terrain::from_seed(CANONICAL_SEED);
    let hydrology = HydrologyGraph::bake(CANONICAL_SEED, &terrain);
    let hub_catalog = HubCatalog::bake(CANONICAL_SEED, &terrain, &hydrology);
    let cells = SubBiomeCellCatalog::bake(CANONICAL_SEED);

    eprintln!("baking BakedAssignments — eager pass with self-spacing + attractor biasing...");
    let baked = BakedAssignments::bake(&cells, &terrain, &hydrology, &hub_catalog);

    let stats = baked.stats();
    eprintln!("=== BakedAssignments stats ===");
    eprintln!("  cell_count:                {}", stats.cell_count);
    eprintln!("  unique features in catalog: {}", baked.unique_feature_count());
    eprintln!("  fallback (`plain`) count:  {} ({:.2}%)",
        stats.fallback_count,
        100.0 * stats.fallback_count as f32 / stats.cell_count as f32);
    eprintln!("  bleed-over count:          {} ({:.2}%)",
        stats.bleed_over_count,
        100.0 * stats.bleed_over_count as f32 / stats.cell_count as f32);
    eprintln!("  self-spacing rejections:   {}", stats.self_spacing_reject_count);
    eprintln!("  bake wall-clock:           {} ms ({:.2} s)",
        stats.bake_wallclock_ms,
        stats.bake_wallclock_ms as f64 / 1000.0);

    // Per-feature baked counts (top 25 by frequency).
    let mut counts: HashMap<String, u32> = HashMap::new();
    for cell_id in 0..stats.cell_count {
        let assn = baked.feature_at_cell(cell_id);
        *counts.entry(assn.feature_id).or_insert(0) += 1;
    }
    let mut sorted: Vec<(String, u32)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("=== Top 25 baked features ===");
    for (id, c) in sorted.iter().take(25) {
        let pct = 100.0 * *c as f32 / stats.cell_count as f32;
        eprintln!("  {id:30} {c:>8} ({pct:.2}%)");
    }

    // Per-feature spatial-hash sizes (proves self-spacing tracked the
    // 18 authored-with-self_spacing features).
    eprintln!("=== Per-feature self-spacing placement counts ===");
    let tracked_feature_ids = [
        "ranch", "remote_compound", "homestead_clearing", "farmland_parcel",
        "dwelling_cluster", "natural_meadow", "alpine_meadow", "creek_valley",
        "scenic_overlook", "river_bend_viewpoint", "hot_springs",
        "abandoned_homestead", "fishing_camp", "tackle_shop", "bait_shop",
        "ferry_dock", "gas_station", "general_store",
    ];
    let hashes = baked.spatial_hashes();
    eprintln!("  tracked feature types: {}", hashes.feature_count());
    for id in tracked_feature_ids {
        eprintln!("  {id:30} {} placements", hashes.placement_count(id));
    }

    // Sanity: bake produced results.
    assert!(stats.cell_count > 3_000_000, "expected >3M cells in canonical seed");
    assert!(stats.bake_wallclock_ms > 0, "wall-clock should be measured");
}
