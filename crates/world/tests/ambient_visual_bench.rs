//! Ambient visual-bench (5c.2.c.12g+).
//!
//! Emits PNGs that visualize the per-biome ambient pipeline so we
//! don't have to guess from in-game screenshots. Per the visual-bench
//! pattern (PINNED) — deterministic per `(seed, crop)`, artifact files
//! committed to `docs/world-gen-plots/ambient/<seed>/` once accepted.
//!
//! Three artifacts per biome at the spawn-area player crop (10 km):
//!
//! 1. **density-noise** — raw fBm `density_noise()` output normalized
//!    `[0, 1]` → cool→warm ramp. Reveals the underlying noise field
//!    (peaks, valleys, scale).
//! 2. **spawn-weight** — `noise_floor + (1-floor) × smoothstep(...)`,
//!    the actual probability a candidate at this `(x, z)` spawns.
//!    Reveals what authored thresholds + floor produce.
//! 3. **placement** — actual `FeatureInstance` positions emitted by
//!    `generate_ambient_for_cell` for every cell whose center falls in
//!    the crop. White dots on a hydrology-water + biome-class
//!    background. Reveals what the player *actually sees*.
//!
//! Default-ignored — opt in via:
//!
//!   `cargo test -p flowplan-world --test ambient_visual_bench -- --ignored`

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::imprecise_flops)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::ignore_without_reason)]

use std::path::Path;

use flowplan_world::ambient::generate_ambient_for_cell;
use flowplan_world::baked_assignments::BakedAssignments;
use flowplan_world::biome::{biome_at, BiomeClass};
use flowplan_world::feature_instances::AmbientEntry;
use flowplan_world::hub_catalog::HubCatalog;
use flowplan_world::hydrology::{HydrologyGraph, WaterClass};
use flowplan_world::sub_biome::SubBiomeLibraries;
use flowplan_world::terrain::{HeightSampler, Terrain};
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

/// The actual game world seed (matches `crates/app/src/game_app.rs`
/// WORLD_SEED). Diagnostic must use the same seed as the in-game spawn
/// or the noise field shape won't match what the player sees.
const GAME_SEED: u64 = 0x1337_CAFE_B0A7_5EAF;

/// 10 km player-local crop centered on world origin (where the player
/// spawns).
const PLAYER_EXTENT_M: f32 = 10_000.0;
/// 30 km wider crop matching the streaming radius — used by
/// `ambient_visual_bench_real_bake` to show the FULL coverage area
/// (not just what's directly under the camera).
const STREAMING_EXTENT_M: f32 = 30_000.0;
const RESOLUTION: u32 = 1024;

fn world_coord_at(extent_m: f32, px: u32, py: u32) -> (f32, f32) {
    let u = (f32::from(px as u16) + 0.5) / RESOLUTION as f32;
    let v = (f32::from(py as u16) + 0.5) / RESOLUTION as f32;
    let x = (u - 0.5) * extent_m;
    let z = (0.5 - v) * extent_m;
    (x, z)
}

fn world_coord(px: u32, py: u32) -> (f32, f32) {
    world_coord_at(PLAYER_EXTENT_M, px, py)
}

fn pixel_coord_at(extent_m: f32, x: f32, z: f32) -> Option<(u32, u32)> {
    let u = x / extent_m + 0.5;
    let v = 0.5 - z / extent_m;
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    let px = (u * RESOLUTION as f32) as u32;
    let py = (v * RESOLUTION as f32) as u32;
    Some((px.min(RESOLUTION - 1), py.min(RESOLUTION - 1)))
}

/// Reverse — world (x, z) → pixel (px, py) within the player crop.
/// Returns `None` if outside the crop.
fn pixel_coord(x: f32, z: f32) -> Option<(u32, u32)> {
    pixel_coord_at(PLAYER_EXTENT_M, x, z)
}

/// Cool→warm perceptual ramp.
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

/// Replicate the private `density_noise` from `crates/world/src/ambient.rs`
/// so the bench can sample it without violating module privacy.
fn density_noise(x: f32, z: f32, scale_m: f32, entry_tag: u64) -> f32 {
    let raw = flowplan_world::noise::fbm_2d(entry_tag, 0, x, z, scale_m, 4);
    ((raw / 2.8) + 0.5).clamp(0.0, 1.0)
}

/// Mirror the per-entry tag derivation from the generator.
fn entry_tag(entry_idx: usize) -> u64 {
    const SEED_CONST: u64 = 0x4a47_45be_4143_4f56;
    SEED_CONST.wrapping_add(entry_idx as u64)
}

/// Mirror generator's spawn-weight formula.
fn spawn_weight(noise: f32, entry: &AmbientEntry) -> f32 {
    let taper = entry.noise_taper.max(1e-4);
    let t_raw = (noise - (entry.noise_threshold - taper * 0.5)) / taper;
    let smooth = t_raw.clamp(0.0, 1.0);
    let smooth = smooth * smooth * (3.0 - 2.0 * smooth);
    let floor = entry.noise_floor.clamp(0.0, 1.0);
    floor + (1.0 - floor) * smooth
}

fn out_dir() -> &'static Path {
    Path::new("target/world-gen-plots/ambient")
}

fn write_png(name: &str, buf: &ImageBuffer<Rgb<u8>, Vec<u8>>) {
    let dir = out_dir().join(format!("{GAME_SEED:016x}"));
    std::fs::create_dir_all(&dir).expect("create artifact dir");
    let path = dir.join(format!("{name}.png"));
    buf.save(&path).expect("write png");
    eprintln!("wrote {}", path.display());
}

/// Sample the density-noise field for a biome's first ambient entry
/// (typically the primary canopy). Higher-resolution view of the noise
/// itself, before threshold/floor.
fn render_density_noise(libs: &SubBiomeLibraries, biome: BiomeClass) {
    let Some(entry) = libs.ambient_for_biome(biome).first() else {
        return;
    };
    let tag = entry_tag(0);
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = world_coord(px, py);
            let n = density_noise(x, z, entry.noise_scale_m, tag);
            img.put_pixel(px, py, Rgb(ramp_rgb(n)));
        }
    }
    write_png(&format!("density-noise-{biome:?}"), &img);
}

/// Same crop, with the spawn-weight formula applied (factoring
/// noise_floor + threshold + taper). This is what the generator
/// effectively rolls against `rand` per candidate.
fn render_spawn_weight(libs: &SubBiomeLibraries, biome: BiomeClass) {
    let Some(entry) = libs.ambient_for_biome(biome).first() else {
        return;
    };
    let tag = entry_tag(0);
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = world_coord(px, py);
            let n = density_noise(x, z, entry.noise_scale_m, tag);
            let w = spawn_weight(n, entry);
            img.put_pixel(px, py, Rgb(ramp_rgb(w)));
        }
    }
    write_png(&format!("spawn-weight-{biome:?}"), &img);
}

/// Background showing biome class (HSV-ish color per biome) +
/// hydrology water (dark blue overlay) for the player crop. Used
/// as the underlay for the placement-dot artifact.
fn render_underlay(hydrology: &HydrologyGraph) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = world_coord(px, py);
            let biome = biome_at(GAME_SEED, x, z);
            let bg = match biome {
                BiomeClass::CoastalBoreal => [70, 130, 90],
                BiomeClass::HighDesert => [200, 170, 120],
                BiomeClass::RockyAlpine => [150, 150, 160],
                BiomeClass::MediterraneanArchipelago => [180, 200, 140],
            };
            let water = hydrology
                .water_surface_at(x, z)
                .is_some_and(|s| !matches!(s.class, WaterClass::DryChannel));
            if water {
                img.put_pixel(px, py, Rgb([30, 60, 120]));
            } else {
                img.put_pixel(px, py, Rgb(bg));
            }
        }
    }
    img
}

/// Render every ambient instance the generator emits for cells whose
/// center falls in the crop, as white dots on the underlay. This is
/// the diagnostic that tells us what the player actually sees.
fn render_placements(
    libs: &SubBiomeLibraries,
    terrain: &Terrain,
    hydrology: &HydrologyGraph,
) {
    let mut img = render_underlay(hydrology);

    // Sample cells on a coarse grid; the catalog isn't easily
    // accessible from the test crate, so we synthesize cell-center
    // positions on a 1.5 km grid and just call the generator at each.
    // Good enough for visual-bench — captures the spatial density.
    let cell_radius_m = 1000.0;
    let step_m = 1500.0;
    let half_extent = PLAYER_EXTENT_M / 2.0 + cell_radius_m;
    let half_extent_i = half_extent as i32;
    let step_i = step_m as i32;

    let height_at = |x: f32, z: f32| HeightSampler::height(terrain, x, z);
    let is_water = |x: f32, z: f32| {
        hydrology
            .water_surface_at(x, z)
            .is_some_and(|s| !matches!(s.class, WaterClass::DryChannel))
    };

    let mut per_biome: std::collections::BTreeMap<String, (u64, u64)> =
        std::collections::BTreeMap::new();
    let mut total = 0_u64;
    let mut empty_cells = 0_u64;
    let mut populated_cells = 0_u64;
    let mut z = -half_extent_i;
    while z <= half_extent_i {
        let mut x = -half_extent_i;
        while x <= half_extent_i {
            let cell_pos = (x as f32, z as f32);
            let biome = biome_at(GAME_SEED, cell_pos.0, cell_pos.1);
            let entries = libs.ambient_for_biome(biome);
            // Synthetic cell_id from coordinates so the generator's
            // RNG keying is stable per (x, z).
            let cell_id = ((x as u32) ^ (z as u32).wrapping_shl(16)) as u32;
            let insts = generate_ambient_for_cell(
                GAME_SEED,
                cell_id,
                cell_pos,
                biome,
                entries,
                &[],
                None,
                &height_at,
                &is_water,
                &|_, _| true, // synthetic-cell bench: no polygon clip
            );
            let key = format!("{biome:?}");
            let entry = per_biome.entry(key).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += insts.len() as u64;
            if insts.is_empty() {
                empty_cells += 1;
            } else {
                populated_cells += 1;
            }
            for inst in &insts {
                total += 1;
                if let Some((px, py)) = pixel_coord(inst.position.x, inst.position.z) {
                    // Bright white dot regardless of size — readable
                    // diagnostic. Per-instance scale is in the data,
                    // not the visualization.
                    img.put_pixel(px, py, Rgb([255, 255, 255]));
                }
            }
            x += step_i;
        }
        z += step_i;
    }
    eprintln!(
        "placement bench: {populated_cells} populated / {empty_cells} empty cells, {total} total ambient instances in 10 km crop"
    );
    eprintln!("per-biome cells (cells, total instances):");
    for (k, (cells, insts)) in &per_biome {
        let avg = if *cells > 0 {
            *insts as f64 / *cells as f64
        } else {
            0.0
        };
        eprintln!("  {k}: {cells} cells, {insts} insts, avg {avg:.0}/cell");
    }
    write_png("placements-all-biomes", &img);
}

#[test]
#[ignore]
fn ambient_visual_bench() {
    let libs = SubBiomeLibraries::shared();
    let terrain = Terrain::from_seed(GAME_SEED);
    let hydrology = HydrologyGraph::bake(GAME_SEED, &terrain);

    for biome in [
        BiomeClass::CoastalBoreal,
        BiomeClass::HighDesert,
        BiomeClass::RockyAlpine,
        BiomeClass::MediterraneanArchipelago,
    ] {
        render_density_noise(&libs, biome);
        render_spawn_weight(&libs, biome);
    }
    render_placements(&libs, &terrain, &hydrology);
}

/// Mirror the game's full pipeline: bake the sub-biome catalog,
/// hub catalog, and `BakedAssignments`, then for every cell whose
/// center falls in the player crop, run `generate_ambient_for_cell`
/// with the REAL feature instances + ambient_control. Renders the
/// placements as white dots over the biome+water underlay AND
/// emits stats showing how many cells produced zero ambient and why.
///
/// This is the diagnostic for "why does the game show zero ambient
/// where the synthetic-cell bench shows ~1000 instances per cell".
#[test]
#[ignore]
fn ambient_visual_bench_real_bake() {
    eprintln!("baking world (this takes 30-60s)…");
    let libs = SubBiomeLibraries::shared();
    let terrain = Terrain::from_seed(GAME_SEED);
    let t0 = std::time::Instant::now();
    let hydrology = HydrologyGraph::bake(GAME_SEED, &terrain);
    eprintln!("  hydrology baked in {:.1}s", t0.elapsed().as_secs_f32());
    let catalog = SubBiomeCellCatalog::bake(GAME_SEED);
    let t1 = std::time::Instant::now();
    let hub_catalog = HubCatalog::bake(GAME_SEED, &terrain, &hydrology);
    eprintln!("  hub catalog baked in {:.1}s", t1.elapsed().as_secs_f32());
    let t2 = std::time::Instant::now();
    let baked = BakedAssignments::bake(&catalog, &terrain, &hydrology, &hub_catalog);
    eprintln!(
        "  baked assignments in {:.1}s — {} populated cells",
        t2.elapsed().as_secs_f32(),
        baked.populated_cell_count(),
    );

    // Find all cells whose polygons could contribute to the wider
    // 30 km streaming crop. The crop is a SQUARE so the corners are
    // at `extent × sqrt(2) / 2` from origin; probe needs to cover
    // that plus a cell-extent margin. This same geometry issue
    // applies to the game's circular streaming radius — cells in
    // corner-of-frustum positions won't be enqueued without this fix.
    let probe_radius =
        STREAMING_EXTENT_M * core::f32::consts::SQRT_2 / 2.0 + 4_000.0;
    let cells_in_crop = catalog.cells_within_radius(0.0, 0.0, probe_radius);
    eprintln!(
        "{} cells whose seed is within {} m of origin",
        cells_in_crop.len(),
        probe_radius as i32
    );

    // Per-cell nearest-neighbor distance (seed-to-seed) so we can
    // see how big the cells are in this region.
    if cells_in_crop.len() >= 2 {
        let mut min_neighbor: Vec<f32> = Vec::new();
        for &cid in &cells_in_crop {
            let (x, z) = catalog.seeds()[cid as usize].position;
            let mut best = f32::INFINITY;
            for &other in &cells_in_crop {
                if other == cid {
                    continue;
                }
                let (ox, oz) = catalog.seeds()[other as usize].position;
                let d = ((x - ox).powi(2) + (z - oz).powi(2)).sqrt();
                if d < best {
                    best = d;
                }
            }
            min_neighbor.push(best);
        }
        let avg = min_neighbor.iter().sum::<f32>() / min_neighbor.len() as f32;
        let max = min_neighbor.iter().copied().fold(0.0_f32, f32::max);
        let min = min_neighbor
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        eprintln!(
            "  nearest-neighbor seed distance: avg={avg:.0} m, min={min:.0} m, max={max:.0} m"
        );
    }

    let height_at = |x: f32, z: f32| HeightSampler::height(&terrain, x, z);
    let is_water = |x: f32, z: f32| {
        hydrology
            .water_surface_at(x, z)
            .is_some_and(|s| !matches!(s.class, WaterClass::DryChannel))
    };

    // Render at the wider STREAMING_EXTENT_M crop so the full set of
    // cells visible from spawn is captured.
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = world_coord_at(STREAMING_EXTENT_M, px, py);
            let biome = biome_at(GAME_SEED, x, z);
            let bg = match biome {
                BiomeClass::CoastalBoreal => [70, 130, 90],
                BiomeClass::HighDesert => [200, 170, 120],
                BiomeClass::RockyAlpine => [150, 150, 160],
                BiomeClass::MediterraneanArchipelago => [180, 200, 140],
            };
            let water = hydrology
                .water_surface_at(x, z)
                .is_some_and(|s| !matches!(s.class, WaterClass::DryChannel));
            if water {
                img.put_pixel(px, py, Rgb([30, 60, 120]));
            } else {
                img.put_pixel(px, py, Rgb(bg));
            }
        }
    }
    let mut zero_count = 0_u32;
    let mut populated_count = 0_u32;
    let mut total_insts = 0_u64;
    let mut feature_id_zero_counts: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut feat_inst_count_when_zero: Vec<usize> = Vec::new();

    for cell_id in &cells_in_crop {
        let seed_pos = catalog.seeds()[*cell_id as usize].position;
        let host_biome = biome_at(GAME_SEED, seed_pos.0, seed_pos.1);
        let entries = libs.ambient_for_biome(host_biome);
        if entries.is_empty() {
            continue;
        }
        let assignment = baked.feature_at_cell(*cell_id);
        let feat_insts = baked.instances_at_cell(*cell_id);
        let feature = libs
            .find(
                assignment.source.feature_source_biome(),
                &assignment.feature_id,
            )
            .or_else(|| libs.oceanic().iter().find(|f| f.id == assignment.feature_id));
        let ambient_control = feature.and_then(|f| f.ambient_control);
        let target_id = *cell_id;
        let is_in_cell = |x: f32, z: f32| catalog.nearest_cell(x, z) == target_id;
        let insts = generate_ambient_for_cell(
            GAME_SEED,
            *cell_id,
            seed_pos,
            host_biome,
            entries,
            feat_insts,
            ambient_control,
            &height_at,
            &is_water,
            &is_in_cell,
        );
        if insts.is_empty() {
            zero_count += 1;
            *feature_id_zero_counts
                .entry(assignment.feature_id.clone())
                .or_insert(0) += 1;
            feat_inst_count_when_zero.push(feat_insts.len());
        } else {
            populated_count += 1;
            total_insts += insts.len() as u64;
        }
        // Color by biome so we can see who's placing what:
        // white = CoastalBoreal, yellow = Mediterranean, cyan = alpine,
        // magenta = desert.
        let dot_color: [u8; 3] = match host_biome {
            BiomeClass::CoastalBoreal => [255, 255, 255],
            BiomeClass::MediterraneanArchipelago => [255, 255, 0],
            BiomeClass::RockyAlpine => [0, 255, 255],
            BiomeClass::HighDesert => [255, 0, 255],
        };
        for inst in &insts {
            if let Some((px, py)) =
                pixel_coord_at(STREAMING_EXTENT_M, inst.position.x, inst.position.z)
            {
                img.put_pixel(px, py, Rgb(dot_color));
            }
        }
    }

    eprintln!(
        "real-bake placement: {populated_count} populated / {zero_count} zero / {} total insts (avg {:.0}/populated cell)",
        total_insts,
        if populated_count > 0 {
            total_insts as f64 / f64::from(populated_count)
        } else { 0.0 }
    );

    // Per-biome breakdown so we can see which biomes are
    // under-producing in the spawn area.
    let mut biome_stats: std::collections::BTreeMap<String, (u32, u32, u64)> =
        std::collections::BTreeMap::new();
    for cell_id in &cells_in_crop {
        let seed_pos = catalog.seeds()[*cell_id as usize].position;
        let biome = biome_at(GAME_SEED, seed_pos.0, seed_pos.1);
        let entries = libs.ambient_for_biome(biome);
        if entries.is_empty() {
            continue;
        }
        let assignment = baked.feature_at_cell(*cell_id);
        let feat_insts = baked.instances_at_cell(*cell_id);
        let feature = libs
            .find(
                assignment.source.feature_source_biome(),
                &assignment.feature_id,
            )
            .or_else(|| libs.oceanic().iter().find(|f| f.id == assignment.feature_id));
        let ambient_control = feature.and_then(|f| f.ambient_control);
        let target_id = *cell_id;
        let is_in_cell = |x: f32, z: f32| catalog.nearest_cell(x, z) == target_id;
        let insts = generate_ambient_for_cell(
            GAME_SEED,
            *cell_id,
            seed_pos,
            biome,
            entries,
            feat_insts,
            ambient_control,
            &height_at,
            &is_water,
            &is_in_cell,
        );
        let key = format!("{biome:?}");
        let entry = biome_stats.entry(key).or_insert((0, 0, 0));
        entry.0 += 1;
        if insts.is_empty() {
            entry.1 += 1; // zero
        }
        entry.2 += insts.len() as u64;
    }
    eprintln!("per-biome cells (total / zero / total instances / avg per non-zero):");
    for (k, (cells, zeros, insts)) in &biome_stats {
        let nonzero = cells - zeros;
        let avg = if nonzero > 0 {
            *insts as f64 / f64::from(nonzero)
        } else {
            0.0
        };
        eprintln!("  {k}: {cells} cells, {zeros} zero, {insts} insts, avg {avg:.0}/non-zero cell");
    }
    eprintln!("zero-ambient cells by feature_id (top 15):");
    let mut v: Vec<_> = feature_id_zero_counts.iter().collect();
    v.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
    for (fid, c) in v.into_iter().take(15) {
        eprintln!("  {fid}: {c}");
    }
    if !feat_inst_count_when_zero.is_empty() {
        let avg_feat = feat_inst_count_when_zero.iter().sum::<usize>() as f64
            / feat_inst_count_when_zero.len() as f64;
        let max_feat = feat_inst_count_when_zero.iter().copied().max().unwrap_or(0);
        eprintln!(
            "  zero-ambient cells avg feat_inst count: {avg_feat:.1}, max: {max_feat}"
        );
    }
    write_png("real-bake-placements", &img);

    // 5c.2.c.12i: Voronoi-polygon mosaic. Color each pixel by its
    // nearest cell_id (hash → RGB). Confirms Voronoi cells tile the
    // crop with no gaps; visually matches the white-dot placements so
    // we can see "this cell got X dots, that cell got 0".
    let mut mosaic: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = world_coord_at(STREAMING_EXTENT_M, px, py);
            let cid = catalog.nearest_cell(x, z);
            let rgb = cell_color(cid);
            mosaic.put_pixel(px, py, Rgb(rgb));
        }
    }
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = world_coord_at(STREAMING_EXTENT_M, px, py);
            let water = hydrology
                .water_surface_at(x, z)
                .is_some_and(|s| !matches!(s.class, WaterClass::DryChannel));
            if water {
                let cur = mosaic.get_pixel(px, py).0;
                let dark = [
                    (u32::from(cur[0]) * 30 / 100) as u8,
                    (u32::from(cur[1]) * 40 / 100) as u8,
                    (u32::from(cur[2]) * 70 / 100 + 60) as u8,
                ];
                mosaic.put_pixel(px, py, Rgb(dark));
            }
        }
    }
    write_png("voronoi-cell-mosaic", &mosaic);
}

/// Hash a cell_id into a stable RGB triple. Bright distinct colors so
/// adjacent cells contrast.
fn cell_color(cid: u32) -> [u8; 3] {
    let h = cid.wrapping_mul(2_654_435_761);
    let r = ((h >> 16) & 0xFF) as u8;
    let g = ((h >> 8) & 0xFF) as u8;
    let b = (h & 0xFF) as u8;
    // Boost saturation a bit so all colors are readable.
    [r.saturating_add(40), g.saturating_add(40), b.saturating_add(40)]
}
