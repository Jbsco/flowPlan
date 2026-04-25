//! Scan the world for one good spawn point per biome. Reports
//! coords + altitude + distance from origin for each.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::expect_used)]
#![allow(clippy::print_stderr)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::while_float)]

use flowplan_world::biome::biome_at;
use flowplan_world::hydrology::{HydrologyGraph, SEA_LEVEL_M, WaterClass};
use flowplan_world::terrain::{HeightSampler, Terrain};

use flowplan_core as _;
use glam as _;
use libm as _;
use rayon as _;
use ron as _;
use serde as _;
use image as _;

const GAME_SEED: u64 = 0x1337_CAFE_B0A7_5EAF;

#[test]
#[ignore]
fn locate_one_spawn_per_biome() {
    let terrain = Terrain::from_seed(GAME_SEED);
    let hydrology = HydrologyGraph::bake(GAME_SEED, &terrain);

    // Scan the world on a 20km grid (250 × 250 = 62500 samples).
    // For each biome, find the candidate with terrain altitude
    // most representative of the biome and well above sea level.
    let scan_step_m = 20_000.0_f32;
    let half_extent = 2_500_000.0_f32; // half world

    let mut best: std::collections::HashMap<String, (f32, f32, f32)> =
        std::collections::HashMap::new(); // biome → (x, z, alt)

    let mut i = 0;
    let mut x = -half_extent;
    while x < half_extent {
        let mut z = -half_extent;
        while z < half_extent {
            let alt = HeightSampler::height(&terrain, x, z);
            // Need above-sea land. Cap at 3500 m so alpine cells
            // (1200 - 2800 m typical) are included.
            if !(10.0..=3500.0).contains(&alt) {
                z += scan_step_m;
                continue;
            }
            // Skip water cells per hydrology so we don't spawn on
            // an underwater bench.
            let underwater = hydrology
                .water_surface_at(x, z)
                .is_some_and(|s| !matches!(s.class, WaterClass::DryChannel) && alt < s.elevation_m);
            if underwater {
                z += scan_step_m;
                continue;
            }
            let biome = biome_at(GAME_SEED, x, z);
            let key = format!("{biome:?}");
            // Pick the FIRST good candidate near origin (smaller |x|+|z|
            // = closer to spawn for shorter ferry to verify).
            let dist = (x.abs() + z.abs()).max(1.0);
            let entry = best
                .entry(key.clone())
                .or_insert((f32::INFINITY, f32::INFINITY, 0.0));
            if dist < (entry.0.abs() + entry.1.abs()).max(1.0) {
                *entry = (x, z, alt);
            }
            i += 1;
            z += scan_step_m;
        }
        x += scan_step_m;
    }

    eprintln!("=== One spawn point per biome (closest-to-origin land 10-1500m altitude) ===");
    eprintln!("scanned {i} land samples");
    for biome in [
        "CoastalBoreal",
        "MediterraneanArchipelago",
        "HighDesert",
        "RockyAlpine",
    ] {
        if let Some((x, z, alt)) = best.get(biome).copied() {
            eprintln!(
                "  {biome}: ({x:>+9.0}, {z:>+9.0})  altitude {alt:>+5.0} m  (dist {:.0} km)",
                (x.hypot(z) / 1000.0)
            );
        } else {
            eprintln!("  {biome}: NOT FOUND in scan");
        }
    }
    let _ = SEA_LEVEL_M;
}
