//! Feature-groups visual bench (3.6.3c.3a).
//!
//! Emits a top-down PNG of the canonical-seed road network so we can
//! eyeball:
//! - Coverage (does every region have road connectivity?)
//! - Length distribution (no absurd 100 km straight runs?)
//! - Wrap-edge handling (roads cleanly cross the toroidal seam?)
//! - Hub-tier filtering (Major / Regional / Local connect; Remote /
//!   Isolated stay unconnected)
//!
//! Default-ignored — opt in via:
//!
//!     cargo test -p flowplan-world --release --test feature_groups_visual_bench -- --ignored --nocapture
//!
//! Artifacts land in `target/world-gen-plots/feature-groups/<seed>/`.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::cast_lossless)]

use std::path::{Path, PathBuf};

use flowplan_world::baked_assignments::BakedAssignments;
use flowplan_world::feature_groups::{FeatureGroupBundle, FeatureGroupKind};
use flowplan_world::hub_catalog::{HubCatalog, HubTier};
use flowplan_world::hydrology::HydrologyGraph;
use flowplan_world::sub_biome::SubBiomeLibraries;
use flowplan_world::terrain::Terrain;
use flowplan_world::voronoi_cells::SubBiomeCellCatalog;
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies for transitive deps the integration
// test target can see.
use flowplan_core as _;
use glam as _;
use libm as _;
use rayon as _;
use ron as _;
use serde as _;

const GAME_SEED: u64 = 0x1337_CAFE_B0A7_5EAF;

/// Continental-scale crop (full world) at low res for the network-shape
/// bench.
const CONTINENTAL_EXTENT_M: f32 = 2_500_000.0; // half-world
const CONTINENTAL_RESOLUTION: u32 = 1024;

/// Player-local 50 km crop for verifying density + alignment near
/// canonical spawn.
const LOCAL_EXTENT_M: f32 = 50_000.0;
const LOCAL_RESOLUTION: u32 = 1024;

#[test]
#[ignore]
fn render_road_network_artifacts() {
    eprintln!("=== feature-groups bench: road network ===");
    eprintln!("seed: 0x{GAME_SEED:016X}");
    eprintln!("baking world (~120 s)...");

    let bake_start = std::time::Instant::now();
    let terrain = Terrain::from_seed(GAME_SEED);
    let hydrology = HydrologyGraph::bake(GAME_SEED, &terrain);
    let hub_catalog = HubCatalog::bake(GAME_SEED, &terrain, &hydrology);
    let cells = SubBiomeCellCatalog::bake(GAME_SEED);
    // Pass 1: hub-to-hub trunk roads.
    let mut bundle = FeatureGroupBundle::build_road_network(&hub_catalog, &cells);
    let pass1_count = bundle.len();
    eprintln!(
        "Pass 1 (hub trunks) done in {:.1} s — {} groups",
        bake_start.elapsed().as_secs_f64(),
        pass1_count,
    );

    // BakedAssignments — required to know which cells emit roads.
    let baked_start = std::time::Instant::now();
    let baked = BakedAssignments::bake(&cells, &terrain, &hydrology, &hub_catalog, &bundle);
    eprintln!(
        "BakedAssignments done in {:.1} s",
        baked_start.elapsed().as_secs_f64()
    );

    // Pass 2: feature-emit access roads (3.6.3c.3c).
    let pass2_start = std::time::Instant::now();
    let libs = SubBiomeLibraries::shared();
    let emit_cells = baked.collect_road_emit_cells(libs);
    let emit_count = emit_cells.len();
    bundle.extend_with_feature_emit_roads(&emit_cells, &cells, &terrain, &hydrology);
    let pass2_count = bundle.len() - pass1_count;
    eprintln!(
        "Pass 2 (feature-emit) done in {:.1} s — {} emit cells, {} new groups",
        pass2_start.elapsed().as_secs_f64(),
        emit_count,
        pass2_count,
    );

    eprintln!("total bake: {:.1} s", bake_start.elapsed().as_secs_f64());

    eprintln!();
    eprintln!("=== bundle stats ===");
    eprintln!("groups (road segments): {}", bundle.len());
    eprintln!("total cell memberships:  {}", bundle.cell_count());

    // Per-kind segment counts.
    let mut by_kind: std::collections::HashMap<FeatureGroupKind, u32> =
        std::collections::HashMap::new();
    for group in bundle.groups() {
        *by_kind.entry(group.kind).or_insert(0) += 1;
    }
    eprintln!();
    eprintln!("=== per-kind segment count ===");
    for kind in [
        FeatureGroupKind::Highway,
        FeatureGroupKind::Paved,
        FeatureGroupKind::Gravel,
        FeatureGroupKind::Dirt,
        FeatureGroupKind::RoughDirt,
        FeatureGroupKind::Rail,
    ] {
        eprintln!(
            "{kind:?}: {} segments",
            by_kind.get(&kind).copied().unwrap_or(0)
        );
    }

    // Length distribution.
    let mut lengths_m: Vec<f32> = bundle
        .groups()
        .iter()
        .map(|g| {
            g.polyline
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                .sum()
        })
        .collect();
    lengths_m.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if !lengths_m.is_empty() {
        let total: f32 = lengths_m.iter().sum();
        let median = lengths_m[lengths_m.len() / 2];
        let p99 = lengths_m[(lengths_m.len() * 99 / 100).min(lengths_m.len() - 1)];
        let max = lengths_m.last().copied().unwrap_or(0.0);
        eprintln!();
        eprintln!("=== road length distribution ===");
        eprintln!("total network: {:.1} km", total / 1000.0);
        eprintln!("median segment: {:.1} km", median / 1000.0);
        eprintln!("99th percentile: {:.1} km", p99 / 1000.0);
        eprintln!("max segment:    {:.1} km", max / 1000.0);
    }

    // Hub-tier coverage.
    let mut connected_per_tier: std::collections::HashMap<HubTier, u32> =
        std::collections::HashMap::new();
    let mut hub_in_road: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for group in bundle.groups() {
        if let Some(start) = group.polyline.first() {
            // Find the closest hub to the start endpoint (within 5 km
            // tolerance; roads start at hub seed positions exactly).
            for hub in hub_catalog.hubs() {
                let d = (hub.approx_world_pos.0 - start.0).hypot(hub.approx_world_pos.1 - start.1);
                if d < 5_000.0 {
                    hub_in_road.insert(hub.id);
                    *connected_per_tier.entry(hub.tier).or_insert(0) += 1;
                    break;
                }
            }
        }
    }
    eprintln!();
    eprintln!("=== hubs participating in road network (by tier) ===");
    for (tier, count) in &connected_per_tier {
        eprintln!("{tier:?}: {count} hub-endpoints touched");
    }
    let road_hub_count = hub_in_road.len();
    let total_hub_count = hub_catalog.hubs().len();
    eprintln!(
        "{} of {} total hubs ({:.1} %) connected via roads",
        road_hub_count,
        total_hub_count,
        100.0 * road_hub_count as f32 / total_hub_count as f32,
    );

    let out_dir = artifact_dir();
    std::fs::create_dir_all(&out_dir).expect("create artifact dir");

    // Continental crop.
    let continental_path = render_network(
        &bundle,
        &hub_catalog,
        &out_dir,
        "continental",
        CONTINENTAL_EXTENT_M,
        CONTINENTAL_RESOLUTION,
        (0.0, 0.0),
    );
    eprintln!();
    eprintln!("artifact (continental): {}", continental_path.display());

    // Player-local crop.
    let local_path = render_network(
        &bundle,
        &hub_catalog,
        &out_dir,
        "spawn-50km",
        LOCAL_EXTENT_M,
        LOCAL_RESOLUTION,
        (0.0, 0.0),
    );
    eprintln!("artifact (local 50 km): {}", local_path.display());

    assert!(
        !bundle.is_empty(),
        "no road groups baked — `build_road_network` produced empty bundle"
    );
    assert!(
        road_hub_count > 0,
        "no hubs participate in road network — endpoint matching broken?"
    );
}

fn render_network(
    bundle: &FeatureGroupBundle,
    hubs: &HubCatalog,
    out_dir: &Path,
    stem: &str,
    extent_m: f32,
    resolution: u32,
    center: (f32, f32),
) -> PathBuf {
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(resolution, resolution, Rgb([18, 22, 28]));

    // Hub markers — small dots colored by tier.
    for hub in hubs.hubs() {
        if let Some((px, py)) = world_to_pixel(hub.approx_world_pos, center, extent_m, resolution) {
            let color = match hub.tier {
                HubTier::Major => [255, 230, 130],
                HubTier::Regional => [220, 190, 140],
                HubTier::Local => [180, 200, 220],
                HubTier::Remote => [120, 140, 160],
                HubTier::Isolated => [80, 92, 110],
            };
            for dy in -2_i32..=2_i32 {
                for dx in -2_i32..=2_i32 {
                    if dx * dx + dy * dy <= 4 {
                        put_safe(&mut img, px as i32 + dx, py as i32 + dy, color);
                    }
                }
            }
        }
    }

    // Road polylines — color-coded by kind. Paved/Highway are visually
    // dominant; RoughDirt/Dirt are dimmer to reflect their smaller
    // physical footprint.
    for group in bundle.groups() {
        let color = match group.kind {
            FeatureGroupKind::Highway => [255, 200, 80],
            FeatureGroupKind::Paved => [220, 220, 220],
            FeatureGroupKind::Gravel => [180, 170, 140],
            FeatureGroupKind::Dirt => [150, 120, 90],
            FeatureGroupKind::RoughDirt => [110, 90, 70],
            FeatureGroupKind::Rail => [180, 130, 200],
        };
        for window in group.polyline.windows(2) {
            let p0 = world_to_pixel_signed(window[0], center, extent_m, resolution);
            let p1 = world_to_pixel_signed(window[1], center, extent_m, resolution);
            draw_line(&mut img, p0, p1, color);
        }
    }

    // Crosshair at center.
    let cx = (resolution / 2) as i32;
    for d in -10_i32..=10_i32 {
        if d.unsigned_abs() % 3 != 0 {
            put_safe(&mut img, cx + d, cx, [255, 100, 100]);
            put_safe(&mut img, cx, cx + d, [255, 100, 100]);
        }
    }

    let path = out_dir.join(format!("road-network-{stem}-{GAME_SEED:016X}.png"));
    img.save(&path).expect("save artifact png");
    path
}

fn world_to_pixel(
    pos: (f32, f32),
    center: (f32, f32),
    extent_m: f32,
    resolution: u32,
) -> Option<(u32, u32)> {
    let dx = pos.0 - center.0;
    let dz = pos.1 - center.1;
    let half = extent_m * 0.5;
    if dx.abs() > half || dz.abs() > half {
        return None;
    }
    let u = dx / extent_m + 0.5;
    let v = 0.5 - dz / extent_m;
    let px = (u * resolution as f32) as u32;
    let py = (v * resolution as f32) as u32;
    Some((px.min(resolution - 1), py.min(resolution - 1)))
}

fn world_to_pixel_signed(
    pos: (f32, f32),
    center: (f32, f32),
    extent_m: f32,
    resolution: u32,
) -> (i32, i32) {
    let dx = pos.0 - center.0;
    let dz = pos.1 - center.1;
    let u = dx / extent_m + 0.5;
    let v = 0.5 - dz / extent_m;
    let px = (u * resolution as f32) as i32;
    let py = (v * resolution as f32) as i32;
    (px, py)
}

fn draw_line(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    a: (i32, i32),
    b: (i32, i32),
    color: [u8; 3],
) {
    // Simple incremental line drawing — acceptable for diagnostic output.
    let dx = (b.0 - a.0).abs();
    let dy = (b.1 - a.1).abs();
    let steps = dx.max(dy).max(1);
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let x = a.0 as f32 + (b.0 - a.0) as f32 * t;
        let y = a.1 as f32 + (b.1 - a.1) as f32 * t;
        put_safe(img, x as i32, y as i32, color);
    }
}

fn put_safe(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x: i32, y: i32, rgb: [u8; 3]) {
    if x < 0 || y < 0 {
        return;
    }
    let (w, h) = (img.width() as i32, img.height() as i32);
    if x >= w || y >= h {
        return;
    }
    img.put_pixel(x as u32, y as u32, Rgb(rgb));
}

fn artifact_dir() -> PathBuf {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    workspace_root
        .join("target")
        .join("world-gen-plots")
        .join("feature-groups")
        .join(format!("{GAME_SEED:016x}"))
}
