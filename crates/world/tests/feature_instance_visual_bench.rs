//! Feature-instance visual bench (5c.2.c.12t — surface visibility regression).
//!
//! Purpose: prove that every feature with a RON-authored composition
//! produces visible primitives — and that those primitives have the
//! intended footprint, primitive vocabulary, and palette. Closes the
//! "I only see ambient objects" diagnostic gap surfaced when the
//! `AUTHORED_FEATURE_IDS` whitelist was discovered to filter all but
//! `dwelling_cluster`.
//!
//! Per-feature artifact: top-down 2D rendering of composed instances at
//! a representative cell-center for the feature's home biome. One PNG
//! per feature, sized to comfortably contain its widest component.
//!
//! Summary artifact: 7×8 grid of all 49 authored features at uniform
//! scale for at-a-glance visibility scan.
//!
//! Default-ignored — opt in via:
//!
//!     cargo test -p flowplan-world --test feature_instance_visual_bench -- --ignored
//!
//! Artifacts land in `target/world-gen-plots/feature-instances/<seed>/`.

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

use std::path::{Path, PathBuf};

use flowplan_world::baked_assignments::BakedAssignments;
use flowplan_world::biome::BiomeClass;
use flowplan_world::feature_instances::{
    compose_instances_for_feature, FeatureInstance, PrimitiveKind,
};
use flowplan_world::hub_catalog::HubCatalog;
use flowplan_world::hydrology::HydrologyGraph;
use flowplan_world::sub_biome::{BiomeSource, SubBiomeFeature, SubBiomeLibraries};
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

/// Game world seed (matches `crates/app/src/game_app.rs::WORLD_SEED`).
const GAME_SEED: u64 = 0x1337_CAFE_B0A7_5EAF;

/// Row in the populated-cells diagnostic — `(cell_id, wrapped_pos,
/// feature_id, instance_count, dist_to_spawn_m)`.
type PopulatedRow = (u32, (f32, f32), String, usize, f32);

/// Per-feature crop is sized so the largest authored composition (the
/// 1500 m parallel-runways `airport_major`) fits with margin. Also drives
/// the summary-grid tile dimensions.
const PER_FEATURE_EXTENT_M: f32 = 1_800.0;

/// Per-feature image resolution. ~3.5 m/px — coarse but enough to read
/// primitive footprints at a glance.
const PER_FEATURE_RESOLUTION: u32 = 512;

/// Summary-grid tile resolution. 7 columns × 8 rows = 56 tiles ≥ 49
/// authored features, so the grid has comfortable padding.
const SUMMARY_TILE_RESOLUTION: u32 = 256;
const SUMMARY_GRID_COLS: u32 = 7;
const SUMMARY_GRID_ROWS: u32 = 8;

/// Background — soft slate-gray so any palette color is readable.
const BG_RGB: [u8; 3] = [40, 44, 50];
/// Per-cell hairline border — distinguishes the cell footprint from
/// background space.
const CELL_BORDER_RGB: [u8; 3] = [70, 76, 88];
/// Origin crosshair — locates cell-center reference. Subtle (won't
/// dominate small features).
const CROSSHAIR_RGB: [u8; 3] = [120, 130, 145];

#[test]
#[ignore]
fn render_per_feature_artifacts() {
    let libs = SubBiomeLibraries::shared();
    let cases = enumerate_authored_features(libs);

    let out_dir = artifact_dir("per-feature");
    std::fs::create_dir_all(&out_dir).expect("create artifact dir");

    let mut report: Vec<String> = Vec::new();
    let mut total_instances = 0_usize;

    for (biome, feature) in &cases {
        let instances = compose_instances_for_feature(
            GAME_SEED,
            42,
            (0.0, 0.0),
            feature,
            BiomeSource::Primary(*biome),
            libs,
            &|_, _| 0.0,
        );
        total_instances += instances.len();

        let img = render_feature_top_down(&instances, PER_FEATURE_RESOLUTION, PER_FEATURE_EXTENT_M);
        let path = out_dir.join(format!("{:?}__{}.png", biome, feature.id));
        img.save(&path).expect("save per-feature png");

        let counts = count_by_kind(&instances);
        report.push(format!(
            "{:>30} ({:?}): {:>3} instances [Box={} Cone={} Cyl={} Sph={} HC={} FQ={}]",
            feature.id,
            biome,
            instances.len(),
            counts[0],
            counts[1],
            counts[2],
            counts[3],
            counts[4],
            counts[5],
        ));
    }

    eprintln!("=== feature-instance bench: per-feature artifacts ===");
    eprintln!("seed: 0x{GAME_SEED:016X}");
    eprintln!("authored features rendered: {}", cases.len());
    eprintln!("total instances composed: {total_instances}");
    eprintln!("artifact dir: {}", out_dir.display());
    for line in &report {
        eprintln!("  {line}");
    }

    assert!(
        !cases.is_empty(),
        "no authored compositions found — RON load broken?"
    );
    for (biome, feature) in &cases {
        let instances = compose_instances_for_feature(
            GAME_SEED,
            42,
            (0.0, 0.0),
            feature,
            BiomeSource::Primary(*biome),
            libs,
            &|_, _| 0.0,
        );
        assert!(
            !instances.is_empty(),
            "feature {} ({:?}) composed zero instances despite having a composition — \
             a feature listed as authored is invisible. Check the RON.",
            feature.id,
            biome,
        );
    }
}

#[test]
#[ignore]
fn render_summary_grid_artifact() {
    let libs = SubBiomeLibraries::shared();
    let cases = enumerate_authored_features(libs);

    let out_dir = artifact_dir("summary");
    std::fs::create_dir_all(&out_dir).expect("create artifact dir");

    let label_band = 24_u32;
    let tile_total_h = SUMMARY_TILE_RESOLUTION + label_band;
    let grid_w = SUMMARY_TILE_RESOLUTION * SUMMARY_GRID_COLS;
    let grid_h = tile_total_h * SUMMARY_GRID_ROWS;
    let mut grid: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(grid_w, grid_h, Rgb([16, 18, 22]));

    for (idx, (biome, feature)) in cases.iter().enumerate() {
        let col = (idx as u32) % SUMMARY_GRID_COLS;
        let row = (idx as u32) / SUMMARY_GRID_COLS;
        if row >= SUMMARY_GRID_ROWS {
            // Should never trigger given the sizing, but guard so we
            // don't panic if authored count exceeds 56.
            break;
        }

        let instances = compose_instances_for_feature(
            GAME_SEED,
            42,
            (0.0, 0.0),
            feature,
            BiomeSource::Primary(*biome),
            libs,
            &|_, _| 0.0,
        );
        let tile =
            render_feature_top_down(&instances, SUMMARY_TILE_RESOLUTION, PER_FEATURE_EXTENT_M);

        let x0 = col * SUMMARY_TILE_RESOLUTION;
        let y0 = row * tile_total_h;
        for ty in 0..SUMMARY_TILE_RESOLUTION {
            for tx in 0..SUMMARY_TILE_RESOLUTION {
                let p = tile.get_pixel(tx, ty);
                grid.put_pixel(x0 + tx, y0 + ty, *p);
            }
        }
        // Label band — short feature id rendered as 5×7 bitmap text
        // (uppercase + digits + underscore). Fits the available 24 px
        // height comfortably.
        let label = short_label(&feature.id);
        draw_label(&mut grid, x0 + 4, y0 + SUMMARY_TILE_RESOLUTION + 6, &label);
    }

    let path = out_dir.join(format!("authored-features-grid-{GAME_SEED:016X}.png"));
    grid.save(&path).expect("save grid png");

    eprintln!("=== feature-instance bench: summary grid ===");
    eprintln!("artifact: {}", path.display());
    eprintln!(
        "tiles: {}/{} (grid {}×{}, {}×{} px each + {}px label)",
        cases.len(),
        SUMMARY_GRID_COLS * SUMMARY_GRID_ROWS,
        SUMMARY_GRID_COLS,
        SUMMARY_GRID_ROWS,
        SUMMARY_TILE_RESOLUTION,
        SUMMARY_TILE_RESOLUTION,
        label_band,
    );
}

/// Diagnostic: bake the canonical world and report populated cells
/// within the in-game streaming radius (22 km) of player spawn. Closes
/// the "I don't see any feature instances at spawn" gap by surfacing
/// whether it's a data gap (no populated cells nearby) or a render gap
/// (data exists but doesn't reach the screen).
///
/// Override the spawn position via `FLOWPLAN_BENCH_SPAWN_X` /
/// `FLOWPLAN_BENCH_SPAWN_Z` (defaults to canonical origin (0, 0)).
///
/// Slow — bakes the full 3.9 M-cell world (~30-60 s). `#[ignore]` so it
/// doesn't run by default.
#[test]
#[ignore]
fn report_populated_cells_near_spawn() {
    let spawn_x: f32 = std::env::var("FLOWPLAN_BENCH_SPAWN_X")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let spawn_z: f32 = std::env::var("FLOWPLAN_BENCH_SPAWN_Z")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let radius_m: f32 = 22_000.0; // matches in-game STREAM_RADIUS_M

    eprintln!("=== feature-instance bench: spawn-area populated cells ===");
    eprintln!("seed:   0x{GAME_SEED:016X}");
    eprintln!("spawn:  ({spawn_x:.0}, {spawn_z:.0})");
    eprintln!("radius: {radius_m:.0} m  (in-game STREAM_RADIUS_M)");
    eprintln!("baking world (~30-60 s)...");

    let bake_start = std::time::Instant::now();
    let terrain = Terrain::from_seed(GAME_SEED);
    let hydrology = HydrologyGraph::bake(GAME_SEED, &terrain);
    let hub_catalog = HubCatalog::bake(GAME_SEED, &terrain, &hydrology);
    let cells = SubBiomeCellCatalog::bake(GAME_SEED);
    let baked = BakedAssignments::bake(&cells, &terrain, &hydrology, &hub_catalog);
    eprintln!(
        "bake done in {:.1} s; total cells = {}",
        bake_start.elapsed().as_secs_f64(),
        baked.stats().cell_count
    );

    let active = cells.cells_within_radius(spawn_x, spawn_z, radius_m);
    eprintln!("active cells within {radius_m:.0} m: {}", active.len());

    // Mirror the in-game wrap fix (game_app.rs upload loop) — cells
    // returned via wrap-distance get their seed coords wrapped to be
    // within ±EXTENT/2 of the player so the camera-relative shader can
    // project them. Without this, "near" cells via wrap appear at +5 Mm
    // and render off-screen.
    let half_world = flowplan_core::WORLD_EXTENT_M * 0.5;
    let wrap_axis = |v: f32, cam: f32| -> f32 {
        let d = v - cam;
        if d > half_world {
            v - flowplan_core::WORLD_EXTENT_M
        } else if d < -half_world {
            v + flowplan_core::WORLD_EXTENT_M
        } else {
            v
        }
    };

    let mut populated: Vec<PopulatedRow> = Vec::new();
    let mut empty_count = 0_usize;
    let mut total_inst = 0_usize;
    for cell_id in &active {
        let insts = baked.instances_at_cell(*cell_id);
        let raw_pos = cells.seeds()[*cell_id as usize].position;
        let pos = (wrap_axis(raw_pos.0, spawn_x), wrap_axis(raw_pos.1, spawn_z));
        let assn = baked.feature_at_cell(*cell_id);
        let dist = (pos.0 - spawn_x).hypot(pos.1 - spawn_z);
        if insts.is_empty() {
            empty_count += 1;
        } else {
            total_inst += insts.len();
            populated.push((*cell_id, pos, assn.feature_id, insts.len(), dist));
        }
    }
    populated.sort_by(|a, b| a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal));

    eprintln!(
        "populated cells (with feature instances): {} / {} ({:.1} %)",
        populated.len(),
        active.len(),
        100.0 * populated.len() as f32 / active.len().max(1) as f32,
    );
    eprintln!("empty cells (composition: None / fallback): {empty_count}");
    eprintln!("total feature instances in radius: {total_inst}");
    eprintln!();
    eprintln!(
        "=== nearest populated cells to spawn ({}/closest 25) ===",
        populated.len().min(25)
    );
    eprintln!(
        "{:<6} {:>10} {:>10} {:>8} {:>5}  feature_id",
        "cell#", "x", "z", "dist_m", "n"
    );
    for (cell_id, pos, fid, n, dist) in populated.iter().take(25) {
        eprintln!(
            "{cell_id:<6} {:>10.0} {:>10.0} {:>8.0} {:>5}  {fid}",
            pos.0, pos.1, dist, n
        );
    }

    // Per-feature breakdown.
    let mut by_feature: std::collections::HashMap<String, (u32, u32)> =
        std::collections::HashMap::new();
    for (_, _, fid, n, _) in &populated {
        let entry = by_feature.entry(fid.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += *n as u32;
    }
    let mut sorted: Vec<(String, (u32, u32))> = by_feature.into_iter().collect();
    sorted.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    eprintln!();
    eprintln!("=== feature breakdown in spawn radius ===");
    eprintln!("{:<30} {:>8} {:>8}", "feature_id", "cells", "instances");
    for (fid, (cells, total)) in sorted.iter().take(40) {
        eprintln!("{fid:<30} {cells:>8} {total:>8}");
    }

    // Render a 22 km × 22 km top-down PNG showing populated cell
    // positions as colored dots, all cells (populated + empty) as faint
    // background dots. Surfaces spatial distribution at a glance.
    let out_dir = artifact_dir("spawn-area");
    std::fs::create_dir_all(&out_dir).expect("create artifact dir");
    let resolution: u32 = 1024;
    let extent_m = 2.0 * radius_m;
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(resolution, resolution, Rgb([16, 18, 22]));

    // Faint dot per active cell.
    for cell_id in &active {
        let pos = cells.seeds()[*cell_id as usize].position;
        if let Some((px, pz)) = world_to_pixel_centered(pos, spawn_x, spawn_z, extent_m, resolution)
        {
            put_safe(&mut img, px as i32, pz as i32, [50, 56, 68]);
        }
    }
    // Brighter dot per populated cell (color by primitive count tier).
    for (_, pos, _, n, _) in &populated {
        let color = if *n >= 20 {
            [255, 220, 100] // hub-grade
        } else if *n >= 10 {
            [200, 180, 140]
        } else if *n >= 5 {
            [140, 200, 140]
        } else {
            [100, 160, 200]
        };
        if let Some((px, pz)) =
            world_to_pixel_centered(*pos, spawn_x, spawn_z, extent_m, resolution)
        {
            for dy in -2_i32..=2 {
                for dx in -2_i32..=2 {
                    if dx * dx + dy * dy <= 4 {
                        put_safe(&mut img, px as i32 + dx, pz as i32 + dy, color);
                    }
                }
            }
        }
    }
    // Crosshair at spawn.
    let center = (resolution / 2) as i32;
    for d in -8_i32..=8 {
        if d.unsigned_abs() % 2 == 0 {
            put_safe(&mut img, center + d, center, [240, 80, 80]);
            put_safe(&mut img, center, center + d, [240, 80, 80]);
        }
    }
    let path = out_dir.join(format!("spawn-area-{spawn_x:+010.0}_{spawn_z:+010.0}.png"));
    img.save(&path).expect("save spawn-area png");
    eprintln!();
    eprintln!("artifact: {}", path.display());

    assert!(
        !populated.is_empty(),
        "ZERO populated cells within {radius_m} m of spawn ({spawn_x}, {spawn_z}) — \
         feature-instance pipeline appears broken or this spawn is in \
         a hard-empty area. Check by changing FLOWPLAN_BENCH_SPAWN_X/Z."
    );
}

fn world_to_pixel_centered(
    pos: (f32, f32),
    cx: f32,
    cz: f32,
    extent_m: f32,
    resolution: u32,
) -> Option<(u32, u32)> {
    let dx = pos.0 - cx;
    let dz = pos.1 - cz;
    let u = dx / extent_m + 0.5;
    let v = 0.5 - dz / extent_m;
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    let px = (u * resolution as f32) as u32;
    let py = (v * resolution as f32) as u32;
    Some((px.min(resolution - 1), py.min(resolution - 1)))
}

// ═══════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════

fn artifact_dir(stem: &str) -> PathBuf {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    workspace_root
        .join("target")
        .join("world-gen-plots")
        .join("feature-instances")
        .join(format!("{GAME_SEED:016x}"))
        .join(stem)
}

/// Walk all biomes (4 home biomes + oceanic) and return every feature
/// with a `composition: Some([...])`. Each feature appears exactly once
/// at its home biome — bleed-over rendering is a render-side concern not
/// exercised here.
fn enumerate_authored_features(
    libs: &'static SubBiomeLibraries,
) -> Vec<(BiomeClass, &'static SubBiomeFeature)> {
    let mut out = Vec::new();
    for biome in [
        BiomeClass::CoastalBoreal,
        BiomeClass::HighDesert,
        BiomeClass::RockyAlpine,
        BiomeClass::MediterraneanArchipelago,
    ] {
        for f in libs.for_biome(biome) {
            if f.composition.is_some() {
                out.push((biome, f));
            }
        }
    }
    for f in libs.oceanic() {
        if f.composition.is_some() {
            out.push((BiomeClass::CoastalBoreal, f));
        }
    }
    out
}

fn count_by_kind(instances: &[FeatureInstance]) -> [u32; 6] {
    let mut counts = [0_u32; 6];
    for inst in instances {
        let idx = match inst.primitive {
            PrimitiveKind::Box => 0,
            PrimitiveKind::Cone => 1,
            PrimitiveKind::Cylinder => 2,
            PrimitiveKind::Sphere => 3,
            PrimitiveKind::HalfCylinder => 4,
            PrimitiveKind::FlatQuad => 5,
        };
        counts[idx] += 1;
    }
    counts
}

/// Top-down render of `instances` into a square `resolution × resolution`
/// image covering `extent_m × extent_m` of world centered on origin.
/// Each primitive draws its XZ footprint as a filled marker:
///
/// - `Box`, `HalfCylinder` — rotated rectangle
/// - `Cone`, `Cylinder`, `Sphere` — circle (radius = max(scale.x, scale.z) / 2)
/// - `FlatQuad` — rotated rectangle, drawn first as an underlay
fn render_feature_top_down(
    instances: &[FeatureInstance],
    resolution: u32,
    extent_m: f32,
) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(resolution, resolution, Rgb(BG_RGB));

    // Cell-edge hairline (very faint — just to ground the eye).
    let edge = (resolution / 32) as i32;
    for px in 0..resolution as i32 {
        for d in 0..edge {
            put_safe(&mut img, px, d, CELL_BORDER_RGB);
            put_safe(&mut img, px, resolution as i32 - 1 - d, CELL_BORDER_RGB);
            put_safe(&mut img, d, px, CELL_BORDER_RGB);
            put_safe(&mut img, resolution as i32 - 1 - d, px, CELL_BORDER_RGB);
        }
    }

    let m_per_px = extent_m / resolution as f32;
    let center = resolution as i32 / 2;

    // Origin crosshair — short dashed lines, easy to ignore but useful.
    for d in -10_i32..=10_i32 {
        if d.unsigned_abs() % 3 != 0 {
            put_safe(&mut img, center + d, center, CROSSHAIR_RGB);
            put_safe(&mut img, center, center + d, CROSSHAIR_RGB);
        }
    }

    // Draw FlatQuad first (underlay — runways, tarmac, road surfaces).
    for inst in instances {
        if inst.primitive == PrimitiveKind::FlatQuad {
            draw_rect_rotated(&mut img, inst, resolution, m_per_px);
        }
    }
    // Then everything else over the top.
    for inst in instances {
        match inst.primitive {
            PrimitiveKind::FlatQuad => {},
            PrimitiveKind::Box | PrimitiveKind::HalfCylinder => {
                draw_rect_rotated(&mut img, inst, resolution, m_per_px);
            },
            PrimitiveKind::Cone | PrimitiveKind::Cylinder | PrimitiveKind::Sphere => {
                draw_circle(&mut img, inst, resolution, m_per_px);
            },
        }
    }
    img
}

fn world_to_pixel(x: f32, z: f32, resolution: u32, m_per_px: f32) -> (i32, i32) {
    let center = resolution as i32 / 2;
    let px = center + (x / m_per_px).round() as i32;
    let pz = center - (z / m_per_px).round() as i32;
    (px, pz)
}

fn draw_rect_rotated(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    inst: &FeatureInstance,
    resolution: u32,
    m_per_px: f32,
) {
    let half_w = inst.scale.x * 0.5;
    let half_d = inst.scale.z * 0.5;
    let theta = inst.rotation_y_rad;
    let (sin_t, cos_t) = (theta.sin(), theta.cos());

    let max_dim = (inst.scale.x.max(inst.scale.z)).abs();
    let radius_px = ((max_dim / m_per_px) * 0.75).ceil().max(1.0) as i32;
    let (cx, cz) = world_to_pixel(inst.position.x, inst.position.z, resolution, m_per_px);

    for dpz in -radius_px..=radius_px {
        for dpx in -radius_px..=radius_px {
            // Inverse-rotate the pixel offset back to local box coords.
            let world_dx = dpx as f32 * m_per_px;
            let world_dz = -dpz as f32 * m_per_px; // image y → -world z
            let local_x = world_dx * cos_t + world_dz * sin_t;
            let local_z = -world_dx * sin_t + world_dz * cos_t;
            if local_x.abs() <= half_w && local_z.abs() <= half_d {
                let rgb = [inst.color[0], inst.color[1], inst.color[2]];
                put_safe(img, cx + dpx, cz + dpz, rgb);
            }
        }
    }
}

fn draw_circle(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    inst: &FeatureInstance,
    resolution: u32,
    m_per_px: f32,
) {
    // Circle radius = avg of x/z scales (cones, cylinders, spheres are
    // built from unit-radius meshes — actual world radius is scale.x).
    let radius_m = (inst.scale.x.max(inst.scale.z)).abs() * 0.5;
    let radius_px = (radius_m / m_per_px).ceil().max(1.0) as i32;
    let (cx, cz) = world_to_pixel(inst.position.x, inst.position.z, resolution, m_per_px);
    let r2 = radius_px * radius_px;

    for dpz in -radius_px..=radius_px {
        for dpx in -radius_px..=radius_px {
            if dpx * dpx + dpz * dpz <= r2 {
                let rgb = [inst.color[0], inst.color[1], inst.color[2]];
                put_safe(img, cx + dpx, cz + dpz, rgb);
            }
        }
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

// ═══════════════════════════════════════════════════════════════════════
// Tiny bitmap text — 5×7 glyphs for label band on the summary grid.
// ═══════════════════════════════════════════════════════════════════════

fn short_label(id: &str) -> String {
    // Trim to the 18 most-significant chars uppercased, replace
    // underscores with spaces to fit the bitmap font's repertoire.
    id.to_uppercase()
        .chars()
        .take(18)
        .map(|c| if c == '_' { ' ' } else { c })
        .collect()
}

fn draw_label(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x: u32, y: u32, text: &str) {
    let mut cur_x = x;
    for ch in text.chars() {
        if let Some(glyph) = glyph_5x7(ch) {
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..5 {
                    if (bits >> (4 - col)) & 1 == 1 {
                        put_safe(
                            img,
                            (cur_x + col as u32) as i32,
                            (y + row as u32) as i32,
                            [220, 220, 230],
                        );
                    }
                }
            }
        }
        cur_x += 6;
    }
}

/// Pixel-perfect 5×7 bitmap for ASCII uppercase + digits + space.
/// Each glyph is 7 rows of 5 bits (MSB → leftmost pixel). Returns
/// `None` for unsupported chars.
#[allow(clippy::unreadable_literal)]
fn glyph_5x7(ch: char) -> Option<[u8; 7]> {
    Some(match ch {
        ' ' => [0b00000; 7],
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        _ => return None,
    })
}
