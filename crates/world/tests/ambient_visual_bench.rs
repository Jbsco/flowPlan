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
use flowplan_world::hydrology::{HydrologyGraph, SEA_LEVEL_M};
use flowplan_world::sub_biome::SubBiomeLibraries;
use flowplan_world::terrain::{HeightSampler, Terrain};
use flowplan_world::voronoi_cells::SubBiomeCellCatalog;
use image::{ImageBuffer, Rgb};

// Silence unused-crate-dependencies for transitive deps the integration
// test target can see.
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
/// spawns). Used for the noise + spawn-weight artifacts.
const PLAYER_EXTENT_M: f32 = 10_000.0;
/// 30 km wider crop matching the streaming radius — used by the
/// real-bake bench to show the FULL coverage area.
const STREAMING_EXTENT_M: f32 = 30_000.0;
const RESOLUTION: u32 = 1024;

/// Per-instance multi-zoom (small scale): each ambient instance
/// renders as a 1px dot. Feasible up to ~30 km because per-cell
/// generation cost is bounded.
const INSTANCE_ZOOM_LEVELS_M: &[(f32, &str)] = &[
    (1_000.0, "01km-near"),
    (3_000.0, "03km-close"),
    (10_000.0, "10km-mid"),
    (30_000.0, "30km-wide"),
];

/// Wide-scale per-cell-status zooms: each pixel asks `nearest_cell`
/// then colors by populated / zero / water. Skip per-instance dots
/// (would be billions at this scale). Mirrors the existing terrain /
/// hub / hydrology bench crop set (player / quadrant / full /
/// continental).
const WIDE_ZOOM_LEVELS_M: &[(f32, &str)] = &[
    (100_000.0, "100km"),
    (250_000.0, "250km-quadrant"),
    (500_000.0, "500km-full"),
    (2_000_000.0, "2000km-continental"),
];

/// Read crop-center override from env vars `FLOWPLAN_BENCH_CENTER_X` and
/// `_Z`. Defaults to (0, 0). Lets the user point the bench at any
/// in-game position to compare against a screenshot.
fn crop_center() -> (f32, f32) {
    let x: f32 = std::env::var("FLOWPLAN_BENCH_CENTER_X")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let z: f32 = std::env::var("FLOWPLAN_BENCH_CENTER_Z")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    (x, z)
}

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

/// Background showing biome class + actual-underwater hydrology
/// overlay for the player crop. "Actually underwater" = terrain
/// height below water-surface elevation, matching the generator's
/// new water filter. Cells classified as water by hydrology but with
/// terrain ABOVE sea level (visible green coastal flats) render as
/// land and ARE valid ambient placement targets.
fn render_underlay(
    terrain: &Terrain,
    _hydrology: &HydrologyGraph,
) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
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
            let truly_underwater = HeightSampler::height(terrain, x, z) < SEA_LEVEL_M;
            if truly_underwater {
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
    let mut img = render_underlay(terrain, hydrology);

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
    let is_water = |x: f32, z: f32| HeightSampler::height(terrain, x, z) < SEA_LEVEL_M;

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
    let is_water = |x: f32, z: f32| HeightSampler::height(&terrain, x, z) < SEA_LEVEL_M;

    // Multi-zoom artifact emission.
    let (cx, cz) = crop_center();
    eprintln!("crop center: ({cx:.0}, {cz:.0})");
    eprintln!("=== per-instance zooms (small scale) ===");
    for &(extent_m, label) in INSTANCE_ZOOM_LEVELS_M {
        eprintln!("--- zoom level: {label} ({extent_m:.0} m extent) ---");
        render_zoom_level(
            label,
            (cx, cz),
            extent_m,
            &terrain,
            &hydrology,
            &catalog,
            &baked,
            libs,
            &height_at,
            &is_water,
        );
        // Coverage-diff: per-pixel "would-emit?" vs is_water, colored
        // to surface ANY misalignment between the dot layer and the
        // water layer (orange = instances on water, magenta = land
        // without coverage). Replaces the user's earlier "I think
        // there's a shift" observation with a direct visual answer.
        render_coverage_diff(
            label,
            (cx, cz),
            extent_m,
            &terrain,
            &catalog,
            &baked,
            libs,
            &is_water,
        );
    }
    eprintln!("=== per-cell-status zooms (wide scale) ===");
    for &(extent_m, label) in WIDE_ZOOM_LEVELS_M {
        eprintln!("--- zoom level: {label} ({extent_m:.0} m extent) ---");
        render_wide_status(
            label,
            (cx, cz),
            extent_m,
            &terrain,
            &catalog,
            &baked,
            libs,
            &is_water,
        );
    }
}

/// Coverage-diff bench (5c.2.c.12o). Per pixel: compute "would the
/// generator emit ANY ambient instance at this world coord?" by
/// re-evaluating the same filter chain (water + altitude + slope +
/// per-entry constraints) the generator uses, and compare against
/// the underlay's water classification. ALSO overlays the actual
/// generated dots (yellow) so the user can see directly where each
/// instance lands relative to the would-emit map. If any dot lands
/// on blue (true water) or magenta (uncovered land), the generator
/// is emitting somewhere the static filter says it shouldn't.
///
/// Color key:
/// - GREEN: would-emit AND not-water (correct land coverage)
/// - BLUE: !would-emit AND water (correct water exclusion)
/// - **ORANGE**: would-emit AND water (BUG — generator would emit on water)
/// - **MAGENTA**: !would-emit AND not-water (uncovered land — bug or
///   by-design coverage gap because no entry's altitude/slope range
///   accepts this pixel)
///
/// Probabilistic noise + bake-dependent feature-overlap / clearing are
/// SKIPPED — we want the static "could ANY entry emit here under any
/// noise" answer.
#[allow(clippy::too_many_arguments)]
fn render_coverage_diff(
    label: &str,
    center: (f32, f32),
    extent_m: f32,
    terrain: &Terrain,
    catalog: &SubBiomeCellCatalog,
    baked: &BakedAssignments,
    libs: &SubBiomeLibraries,
    is_water: &dyn Fn(f32, f32) -> bool,
) {
    // Pass 1: per-pixel coverage-diff classification.
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(RESOLUTION, RESOLUTION);
    let mut counts = [0_u64; 4]; // green, blue, orange, magenta
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = pixel_to_world_at(center, extent_m, px, py);
            let water = is_water(x, z);
            let would_emit = !water && pixel_could_emit(x, z, terrain, catalog, baked, libs);
            let color = match (would_emit, water) {
                (true, false) => {
                    counts[0] += 1;
                    [60, 180, 80]
                },
                (false, true) => {
                    counts[1] += 1;
                    [30, 60, 120]
                },
                (true, true) => {
                    counts[2] += 1;
                    [255, 140, 0]
                },
                (false, false) => {
                    counts[3] += 1;
                    [220, 0, 200]
                },
            };
            img.put_pixel(px, py, Rgb(color));
        }
    }
    let total = u64::from(RESOLUTION) * u64::from(RESOLUTION);
    eprintln!(
        "  coverage-diff {label}: green(land+covered)={} ({:.1}%), blue(water)={} ({:.1}%), ORANGE(water+covered=BUG)={} ({:.1}%), MAGENTA(land+uncovered)={} ({:.1}%)",
        counts[0],
        counts[0] as f64 / total as f64 * 100.0,
        counts[1],
        counts[1] as f64 / total as f64 * 100.0,
        counts[2],
        counts[2] as f64 / total as f64 * 100.0,
        counts[3],
        counts[3] as f64 / total as f64 * 100.0,
    );
    write_png(&format!("coverage-{label}"), &img);

    // Pass 2: same crop, with actual generator-emitted dots OVERLAID
    // as bright yellow. If any yellow dot falls on blue (water) or
    // magenta (uncovered), the generator is emitting somewhere the
    // static filter says it shouldn't — that's the user's "shift"
    // observation made literal.
    let probe_radius = extent_m * core::f32::consts::SQRT_2 / 2.0 + 4_000.0;
    let cells_in_crop = catalog.cells_within_radius(center.0, center.1, probe_radius);
    let height_at = |x: f32, z: f32| HeightSampler::height(terrain, x, z);
    let mut dot_total = 0_u64;
    let mut dot_on_green = 0_u64;
    let mut dot_on_blue = 0_u64;
    let mut dot_on_orange = 0_u64;
    let mut dot_on_magenta = 0_u64;
    let mut real_water_bug = 0_u64;
    let mut dot_on_blue_examples: Vec<(f32, f32, f32, u32)> = Vec::new(); // (x, z, terrain_h, cell_id)
    for &cell_id in &cells_in_crop {
        let seed_pos = catalog.seeds()[cell_id as usize].position;
        let host_biome = biome_at(GAME_SEED, seed_pos.0, seed_pos.1);
        let entries = libs.ambient_for_biome(host_biome);
        if entries.is_empty() {
            continue;
        }
        let assignment = baked.feature_at_cell(cell_id);
        let feat_insts = baked.instances_at_cell(cell_id);
        let feature = libs
            .find(
                assignment.source.feature_source_biome(),
                &assignment.feature_id,
            )
            .or_else(|| libs.oceanic().iter().find(|f| f.id == assignment.feature_id));
        let ambient_control = feature.and_then(|f| f.ambient_control);
        let target = cell_id;
        let is_in_cell = |x: f32, z: f32| catalog.nearest_cell(x, z) == target;
        let insts = generate_ambient_for_cell(
            GAME_SEED,
            cell_id,
            seed_pos,
            host_biome,
            entries,
            feat_insts,
            ambient_control,
            &height_at,
            is_water,
            &is_in_cell,
        );
        for inst in &insts {
            if let Some((px, py)) =
                world_to_pixel_at(center, extent_m, inst.position.x, inst.position.z)
            {
                dot_total += 1;
                let bg = img.get_pixel(px, py).0;
                let on_blue = bg == [30, 60, 120];
                if bg == [60, 180, 80] {
                    dot_on_green += 1;
                } else if on_blue {
                    dot_on_blue += 1;
                    if dot_on_blue_examples.len() < 20 {
                        let h = HeightSampler::height(terrain, inst.position.x, inst.position.z);
                        dot_on_blue_examples.push((
                            inst.position.x,
                            inst.position.z,
                            h,
                            cell_id,
                        ));
                    }
                } else if bg == [255, 140, 0] {
                    dot_on_orange += 1;
                } else if bg == [220, 0, 200] {
                    dot_on_magenta += 1;
                }
                let direct_water = is_water(inst.position.x, inst.position.z);
                if direct_water {
                    real_water_bug += 1;
                }
                img.put_pixel(px, py, Rgb([255, 230, 0]));
            }
        }
    }
    eprintln!(
        "  dot-overlay {label}: total={} on_green={} ({:.1}%) on_blue={} ({:.1}%) on_orange={} on_magenta={} REAL_water_bug={} ({:.3}%)",
        dot_total,
        dot_on_green,
        if dot_total > 0 { dot_on_green as f64 / dot_total as f64 * 100.0 } else { 0.0 },
        dot_on_blue,
        if dot_total > 0 { dot_on_blue as f64 / dot_total as f64 * 100.0 } else { 0.0 },
        dot_on_orange,
        dot_on_magenta,
        real_water_bug,
        if dot_total > 0 { real_water_bug as f64 / dot_total as f64 * 100.0 } else { 0.0 },
    );
    if !dot_on_blue_examples.is_empty() {
        eprintln!(
            "  dot-on-blue examples (x, z, terrain_h, cell_id), pixel-center is water but dot's exact terrain_h >= SEA_LEVEL_M:"
        );
        for (x, z, h, cid) in dot_on_blue_examples.iter().take(10) {
            // Sample pixel-center neighbors to see how far the
            // nearest water-pixel is.
            let pixel_size_m = extent_m / RESOLUTION as f32;
            let mut nearest_water_m = f32::INFINITY;
            for dx_steps in -3..=3 {
                for dz_steps in -3..=3 {
                    let nx = x + dx_steps as f32 * pixel_size_m;
                    let nz = z + dz_steps as f32 * pixel_size_m;
                    if HeightSampler::height(terrain, nx, nz) < SEA_LEVEL_M {
                        let d = (dx_steps as f32 * pixel_size_m)
                            .hypot(dz_steps as f32 * pixel_size_m);
                        if d < nearest_water_m {
                            nearest_water_m = d;
                        }
                    }
                }
            }
            eprintln!(
                "    cell={cid} dot=({x:.1}, {z:.1}) terrain_h={h:.2} nearest_water_within_3px={nearest_water_m:.1}m (pixel_size={pixel_size_m:.1}m)"
            );
        }
    }
    write_png(&format!("coverage-with-dots-{label}"), &img);
}

/// Per-pixel "would ANY entry emit here?" check. Mirrors the static
/// (non-probabilistic, non-bake-dependent) part of the generator's
/// filter chain.
fn pixel_could_emit(
    x: f32,
    z: f32,
    terrain: &Terrain,
    catalog: &SubBiomeCellCatalog,
    _baked: &BakedAssignments,
    libs: &SubBiomeLibraries,
) -> bool {
    // Polygon clip is implicit — the pixel IS in some cell, so
    // nearest_cell(x,z) returns its cell_id. The cell's biome is
    // determined by its seed position.
    let cell_id = catalog.nearest_cell(x, z);
    let seed_pos = catalog.seeds()[cell_id as usize].position;
    let host_biome = biome_at(GAME_SEED, seed_pos.0, seed_pos.1);
    let entries = libs.ambient_for_biome(host_biome);
    if entries.is_empty() {
        return false;
    }
    let altitude = HeightSampler::height(terrain, x, z);
    // Cheap slope: 4-sample finite-difference gradient like the generator.
    let off = 5.0_f32;
    let h_e = HeightSampler::height(terrain, x + off, z);
    let h_w = HeightSampler::height(terrain, x - off, z);
    let h_n = HeightSampler::height(terrain, x, z + off);
    let h_s = HeightSampler::height(terrain, x, z - off);
    let dh_dx = (h_e - h_w) / (2.0 * off);
    let dh_dz = (h_n - h_s) / (2.0 * off);
    let slope_deg = dh_dx.hypot(dh_dz).atan().to_degrees();
    // Could ANY entry accept this pixel?
    entries.iter().any(|e| {
        altitude >= e.altitude_range_m.0
            && altitude <= e.altitude_range_m.1
            && slope_deg <= e.slope_max_deg
    })
}

/// Wide-scale per-cell status artifact. At 100km–2000km extents,
/// per-instance dots are intractable (millions to billions of
/// instances). Instead, color each pixel by its containing cell's
/// status: green = populated, red = zero (no ambient), blue = water.
/// Quick to compute (one nearest_cell + one cheap zero-check per pixel).
#[allow(clippy::too_many_arguments)]
fn render_wide_status(
    label: &str,
    center: (f32, f32),
    extent_m: f32,
    terrain: &Terrain,
    catalog: &SubBiomeCellCatalog,
    baked: &BakedAssignments,
    libs: &SubBiomeLibraries,
    is_water: &dyn Fn(f32, f32) -> bool,
) {
    use std::collections::HashMap;
    // Cache per-cell status so we don't re-evaluate the same cell
    // for every pixel that falls inside it. Wide crops have large
    // cells covering many pixels.
    let mut cell_status: HashMap<u32, CellStatus> = HashMap::new();
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(RESOLUTION, RESOLUTION);
    let mut pixel_count = [0_u64; 4]; // populated, zero, water, no-biome
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = pixel_to_world_at(center, extent_m, px, py);
            // Water pixel — sea-level cut.
            if is_water(x, z) {
                img.put_pixel(px, py, Rgb([30, 60, 120]));
                pixel_count[2] += 1;
                continue;
            }
            let cid = catalog.nearest_cell(x, z);
            let status = *cell_status
                .entry(cid)
                .or_insert_with(|| evaluate_cell_status(cid, catalog, baked, libs));
            let rgb = match status {
                CellStatus::Populated => [60, 180, 80], // green
                CellStatus::Zero => [200, 60, 60],      // red
                CellStatus::NoBiomeEntries => [120, 120, 120], // grey
            };
            pixel_count[match status {
                CellStatus::Populated => 0,
                CellStatus::Zero => 1,
                CellStatus::NoBiomeEntries => 3,
            }] += 1;
            img.put_pixel(px, py, Rgb(rgb));
        }
    }
    let total_pixels = u64::from(RESOLUTION) * u64::from(RESOLUTION);
    eprintln!(
        "  {label}: pixels populated={} ({:.1}%), zero={} ({:.1}%), water={} ({:.1}%), no-biome-entries={} ({:.1}%)",
        pixel_count[0],
        pixel_count[0] as f64 / total_pixels as f64 * 100.0,
        pixel_count[1],
        pixel_count[1] as f64 / total_pixels as f64 * 100.0,
        pixel_count[2],
        pixel_count[2] as f64 / total_pixels as f64 * 100.0,
        pixel_count[3],
        pixel_count[3] as f64 / total_pixels as f64 * 100.0,
    );
    eprintln!("  {} unique cells evaluated", cell_status.len());
    let _ = terrain; // only is_water uses terrain
    write_png(&format!("status-{label}"), &img);
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
enum CellStatus {
    Populated,
    Zero,
    NoBiomeEntries,
}

/// Quick eval — does this cell produce ANY ambient instances at its
/// seed position? Skips the polygon clip for speed (just answers
/// "would the generator emit > 0 here"). Wide-zoom heuristic; at
/// continental scale we don't need exact counts.
fn evaluate_cell_status(
    cell_id: u32,
    catalog: &SubBiomeCellCatalog,
    baked: &BakedAssignments,
    libs: &SubBiomeLibraries,
) -> CellStatus {
    let seed_pos = catalog.seeds()[cell_id as usize].position;
    let host_biome = biome_at(GAME_SEED, seed_pos.0, seed_pos.1);
    let entries = libs.ambient_for_biome(host_biome);
    if entries.is_empty() {
        return CellStatus::NoBiomeEntries;
    }
    let assignment = baked.feature_at_cell(cell_id);
    let feat = libs
        .find(
            assignment.source.feature_source_biome(),
            &assignment.feature_id,
        )
        .or_else(|| libs.oceanic().iter().find(|f| f.id == assignment.feature_id));
    // If the feature is oceanic-themed (open_ocean, kelp_forest, etc.)
    // we know the cell will likely produce zero from our earlier bench.
    // Use the actual zero list discovered: open_ocean / kelp_forest /
    // shipping_lane / fishing_grounds / sailing_regatta_zone /
    // crab_pot_patch / fjord_entrance.
    let zero_features = [
        "open_ocean",
        "kelp_forest",
        "shipping_lane",
        "fishing_grounds",
        "sailing_regatta_zone",
        "crab_pot_patch",
        "fjord_entrance",
    ];
    if zero_features.iter().any(|z| *z == assignment.feature_id) {
        return CellStatus::Zero;
    }
    let _ = feat;
    CellStatus::Populated
}

/// Render the placement-dots + voronoi-mosaic artifact pair for a
/// single zoom level. Caller owns the bake; we just sample.
#[allow(clippy::too_many_arguments)]
fn render_zoom_level(
    label: &str,
    center: (f32, f32),
    extent_m: f32,
    terrain: &Terrain,
    _hydrology: &HydrologyGraph,
    catalog: &SubBiomeCellCatalog,
    baked: &BakedAssignments,
    libs: &SubBiomeLibraries,
    height_at: &dyn Fn(f32, f32) -> f32,
    is_water: &dyn Fn(f32, f32) -> bool,
) {
    // Find cells whose seeds could contribute to the crop. Square crop
    // → corners at extent×√2/2 from center; add cell-extent margin.
    let probe_radius = extent_m * core::f32::consts::SQRT_2 / 2.0 + 4_000.0;
    let cells_in_crop = catalog.cells_within_radius(center.0, center.1, probe_radius);

    // Underlay: keep biome colors for orientation (user prefers them
    // to grey + colored dots).
    let mut placements: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = pixel_to_world_at(center, extent_m, px, py);
            let biome = biome_at(GAME_SEED, x, z);
            let bg = match biome {
                BiomeClass::CoastalBoreal => [70, 130, 90],
                BiomeClass::HighDesert => [200, 170, 120],
                BiomeClass::RockyAlpine => [150, 150, 160],
                BiomeClass::MediterraneanArchipelago => [180, 200, 140],
            };
            let underwater = HeightSampler::height(terrain, x, z) < SEA_LEVEL_M;
            placements.put_pixel(
                px,
                py,
                Rgb(if underwater { [30, 60, 120] } else { bg }),
            );
        }
    }

    let mut populated = 0_u32;
    let mut zero = 0_u32;
    let mut total_insts = 0_u64;
    let mut zero_by_feat: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut biome_stats: std::collections::BTreeMap<String, (u32, u32, u64)> =
        std::collections::BTreeMap::new();

    for &cell_id in &cells_in_crop {
        let seed_pos = catalog.seeds()[cell_id as usize].position;
        let host_biome = biome_at(GAME_SEED, seed_pos.0, seed_pos.1);
        let entries = libs.ambient_for_biome(host_biome);
        if entries.is_empty() {
            continue;
        }
        let assignment = baked.feature_at_cell(cell_id);
        let feat_insts = baked.instances_at_cell(cell_id);
        let feature = libs
            .find(
                assignment.source.feature_source_biome(),
                &assignment.feature_id,
            )
            .or_else(|| libs.oceanic().iter().find(|f| f.id == assignment.feature_id));
        let ambient_control = feature.and_then(|f| f.ambient_control);
        let target = cell_id;
        let is_in_cell = |x: f32, z: f32| catalog.nearest_cell(x, z) == target;
        let insts = generate_ambient_for_cell(
            GAME_SEED,
            cell_id,
            seed_pos,
            host_biome,
            entries,
            feat_insts,
            ambient_control,
            height_at,
            is_water,
            &is_in_cell,
        );
        if insts.is_empty() {
            zero += 1;
            *zero_by_feat
                .entry(assignment.feature_id.clone())
                .or_insert(0) += 1;
        } else {
            populated += 1;
            total_insts += insts.len() as u64;
        }
        let bs = biome_stats
            .entry(format!("{host_biome:?}"))
            .or_insert((0, 0, 0));
        bs.0 += 1;
        if insts.is_empty() {
            bs.1 += 1;
        }
        bs.2 += insts.len() as u64;

        // 1px black dots — sharp, no coastline overshoot.
        let mut visible_in_crop = 0_u32;
        for inst in &insts {
            if let Some((px, py)) =
                world_to_pixel_at(center, extent_m, inst.position.x, inst.position.z)
            {
                visible_in_crop += 1;
                placements.put_pixel(px, py, Rgb([0, 0, 0]));
            }
        }
        if !insts.is_empty() {
            let visible_frac = f64::from(visible_in_crop) / insts.len() as f64;
            if visible_frac < 0.05 && insts.len() > 100 {
                eprintln!(
                    "    cell {cell_id} seed=({:.0}, {:.0}) host={host_biome:?} insts={} visible={} ({:.1}%)",
                    seed_pos.0,
                    seed_pos.1,
                    insts.len(),
                    visible_in_crop,
                    visible_frac * 100.0
                );
            }
        }
    }
    eprintln!(
        "  {label}: {populated} populated / {zero} zero / {} total insts (avg {:.0}/populated)",
        total_insts,
        if populated > 0 {
            total_insts as f64 / f64::from(populated)
        } else {
            0.0
        },
    );
    for (k, (c, z, i)) in &biome_stats {
        let nonzero = c - z;
        let avg = if nonzero > 0 {
            *i as f64 / f64::from(nonzero)
        } else {
            0.0
        };
        eprintln!("    {k}: {c} cells, {z} zero, {i} insts, avg {avg:.0}/non-zero");
    }
    if zero > 0 {
        let mut v: Vec<_> = zero_by_feat.iter().collect();
        v.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        eprintln!("    zero by feature_id (top 8):");
        for (fid, c) in v.into_iter().take(8) {
            eprintln!("      {fid}: {c}");
        }
    }
    write_png(&format!("placements-{label}"), &placements);

    // Voronoi mosaic at the same crop.
    let mut mosaic: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::new(RESOLUTION, RESOLUTION);
    for py in 0..RESOLUTION {
        for px in 0..RESOLUTION {
            let (x, z) = pixel_to_world_at(center, extent_m, px, py);
            let cid = catalog.nearest_cell(x, z);
            let rgb = cell_color(cid);
            let underwater = HeightSampler::height(terrain, x, z) < SEA_LEVEL_M;
            let final_rgb = if underwater {
                [
                    (u32::from(rgb[0]) * 30 / 100) as u8,
                    (u32::from(rgb[1]) * 40 / 100) as u8,
                    (u32::from(rgb[2]) * 70 / 100 + 60) as u8,
                ]
            } else {
                rgb
            };
            mosaic.put_pixel(px, py, Rgb(final_rgb));
        }
    }
    write_png(&format!("mosaic-{label}"), &mosaic);
}

fn pixel_to_world_at(center: (f32, f32), extent_m: f32, px: u32, py: u32) -> (f32, f32) {
    let u = (f32::from(px as u16) + 0.5) / RESOLUTION as f32;
    let v = (f32::from(py as u16) + 0.5) / RESOLUTION as f32;
    let x = center.0 + (u - 0.5) * extent_m;
    let z = center.1 + (0.5 - v) * extent_m;
    (x, z)
}

fn world_to_pixel_at(
    center: (f32, f32),
    extent_m: f32,
    x: f32,
    z: f32,
) -> Option<(u32, u32)> {
    // Toroidal-wrap delta: cells via cells_within_radius can have
    // literal seed coords near WORLD_EXTENT_M when their wrap-distance
    // to center is small. Normalize delta to [-EXTENT/2, +EXTENT/2].
    let half_world = flowplan_core::WORLD_EXTENT_M * 0.5;
    let mut dx = x - center.0;
    let mut dz = z - center.1;
    if dx > half_world {
        dx -= flowplan_core::WORLD_EXTENT_M;
    } else if dx < -half_world {
        dx += flowplan_core::WORLD_EXTENT_M;
    }
    if dz > half_world {
        dz -= flowplan_core::WORLD_EXTENT_M;
    } else if dz < -half_world {
        dz += flowplan_core::WORLD_EXTENT_M;
    }
    let u = dx / extent_m + 0.5;
    let v = 0.5 - dz / extent_m;
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    let px = (u * RESOLUTION as f32) as u32;
    let py = (v * RESOLUTION as f32) as u32;
    Some((px.min(RESOLUTION - 1), py.min(RESOLUTION - 1)))
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
