//! Performance regression tests (Phase 3.6.3c.4 perf discipline).
//!
//! Each test runs a representative workload, measures wall-clock,
//! asserts elapsed-time stays under a documented budget. Failures are
//! REGRESSIONS — investigate and fix, don't relax the budget.
//!
//! # When a budget fails
//!
//! 1. Reproduce locally on the same hardware (timings are hardware-
//!    dependent — a CI machine with different cores will produce
//!    different numbers; budgets target the user's dev machine plus
//!    a 2× headroom).
//! 2. Profile the hot path. `cargo flamegraph` or `perf record` are
//!    the canonical tools.
//! 3. Find the regression (recent change that pushed cost over the
//!    budget) and either revert / optimize / re-architect.
//! 4. ONLY relax the budget if the new cost is genuinely irreducible
//!    AND the new ceiling is acceptable to the project. Document the
//!    change in PROGRESS.md.
//!
//! # Hardware envelope
//!
//! Budgets are tuned for the user's dev machine (Manjaro, multi-core).
//! Each budget includes a 2× headroom factor — slower hardware will
//! still pass; faster hardware passes trivially. CI on much weaker
//! hardware may need adjustment; cross that bridge when it lands.
//!
//! # Why integration tests, not microbenches
//!
//! Criterion microbenches are excellent for fine-grained tuning but
//! require explicit running and result interpretation. Integration
//! tests with timing-assertions catch regressions automatically on
//! every `cargo test` run. Use criterion for tuning sessions; use
//! these tests as the always-on regression net.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::expect_used)]
#![allow(clippy::print_stderr)]

use std::time::Instant;

use flowplan_world::baked_assignments::BakedAssignments;
use flowplan_world::composed::WorldRefsArc;
use flowplan_world::feature_delta::HubFoundationMap;
use flowplan_world::feature_groups::FeatureGroupBundle;
use flowplan_world::hub_catalog::HubCatalog;
use flowplan_world::hydrology::HydrologyGraph;
use flowplan_world::terrain::{HeightSampler, Terrain};
use flowplan_world::voronoi_cells::SubBiomeCellCatalog;

// Silence unused-crate-dependencies for transitive deps the integration
// test target can see.
use bincode as _;
use flowplan_core as _;
use glam as _;
use image as _;
use libm as _;
use rayon as _;
use ron as _;
use serde as _;
use tracing as _;

// ═══════════════════════════════════════════════════════════════════════
// Per-call hot-path budgets
// ═══════════════════════════════════════════════════════════════════════

/// L1 height (pure terrain fbm, no L3) per-call ceiling.
///
/// `Terrain::height` is called per-vertex during chunk mesh build.
/// Multi-octave fbm dominates; ceiling captures regression in noise
/// primitives (e.g. removed perlin pre-wrap).
#[test]
fn perf_budget_terrain_height_l1() {
    let terrain = Terrain::from_seed(0x1337_CAFE_B0A7_5EAF);
    let n = 100_000;
    let start = Instant::now();
    for i in 0..n {
        let x = (i as f32 * 17.3) % 1_000_000.0;
        let z = (i as f32 * 23.7) % 1_000_000.0;
        std::hint::black_box(terrain.height(x, z));
    }
    let elapsed = start.elapsed();
    let per_call_ns = elapsed.as_nanos() / n as u128;
    eprintln!(
        "terrain.height: {} ns/call ({:.2} ms total for {n} samples)",
        per_call_ns,
        elapsed.as_secs_f64() * 1000.0
    );
    // Budget: 1 s for 100K samples = 10 µs/call. Multi-octave fbm
    // typically clocks 3-5 µs; 10 µs gives 2× headroom.
    assert!(
        elapsed.as_millis() < 1000,
        "terrain.height regressed: 100K samples took {} ms (budget 1000 ms)",
        elapsed.as_millis()
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Per-chunk mesh build perf — moved to app crate (assets dep)
// ═══════════════════════════════════════════════════════════════════════

// `terrain_voronoi_chunk` lives in `flowplan-assets`, which world doesn't
// depend on. The chunk mesh-build perf test lives in
// `crates/app/tests/perf_budgets.rs` so it has access to the full
// chunked-terrain stack.

// ═══════════════════════════════════════════════════════════════════════
// Simulated-rebuild budget — represents a worst-case streaming move
// ═══════════════════════════════════════════════════════════════════════

/// Simulated streaming-rebuild equivalent: 100K composed-height
/// samples (a small chunk's height eval pass).
///
/// Without `BakedAssignments`, `compute_delta_at` re-runs `assign_feature`
/// per query (~100 µs each). With baked, lookup is O(1). This test
/// guards the baked-fast-path stays fast (3.6.3c.3c.3 fix).
#[test]
#[ignore = "requires bake (~30 s); opt-in via --ignored"]
fn perf_budget_composed_height_with_baked() {
    let bundle = setup_minimal_world();
    let n = 100_000;
    let start = Instant::now();
    for i in 0..n {
        let x = (i as f32 * 17.3) % 1_000_000.0;
        let z = (i as f32 * 23.7) % 1_000_000.0;
        std::hint::black_box(bundle.world_refs.height_at(x, z));
    }
    let elapsed = start.elapsed();
    let per_call_us = elapsed.as_micros() / n as u128;
    eprintln!(
        "world_refs.height_at (with baked): {} µs/call ({:.2} s total for {n} samples)",
        per_call_us,
        elapsed.as_secs_f64()
    );
    // Budget: 5 s for 100K samples = 50 µs/call. Composed L1+L3
    // typically clocks 5-10 µs with baked feature lookup; 50 µs
    // allows 5-10× headroom for hardware variation + future
    // additions (water surface, etc.). If this fails the most
    // likely cause is `BakedAssignments` not being threaded into
    // WorldRefs (regression of 3.6.3c.3c.3 fix).
    assert!(
        elapsed.as_millis() < 5000,
        "world_refs.height_at regressed: 100K samples took {} ms (budget 5000 ms). \
         Most likely cause: BakedAssignments not threaded → assign_feature recompute per query.",
        elapsed.as_millis()
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Streaming-cell-set query budget
// ═══════════════════════════════════════════════════════════════════════

/// `cells_within_radius` is called per-frame by the streaming layer.
/// Must stay sub-millisecond for headroom.
#[test]
#[ignore = "requires sub-biome catalog bake (~10 s); opt-in via --ignored"]
fn perf_budget_cells_within_radius() {
    let cells = SubBiomeCellCatalog::bake(0x1337_CAFE_B0A7_5EAF);
    let n = 100;
    let start = Instant::now();
    for i in 0..n {
        let x = (i as f32 * 1_000.0) % 1_000_000.0;
        let z = (i as f32 * 1_300.0) % 1_000_000.0;
        std::hint::black_box(cells.cells_within_radius(x, z, 22_000.0));
    }
    let elapsed = start.elapsed();
    let per_call_ms = elapsed.as_secs_f64() * 1000.0 / n as f64;
    eprintln!("cells_within_radius: {per_call_ms:.2} ms/call ({n} calls @ 22 km)");
    // Budget: 10 ms per call (very generous; typical < 1 ms).
    assert!(
        per_call_ms < 10.0,
        "cells_within_radius regressed: {per_call_ms:.2} ms/call (budget 10 ms)"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Test scaffolding
// ═══════════════════════════════════════════════════════════════════════

struct TestWorld {
    world_refs: WorldRefsArc,
}

/// Minimal canonical-seed world setup. Bake takes ~30 s (hydrology +
/// hub catalog + sub-biome catalog + assignments + road network).
/// Tests using this should be `#[ignore]` so CI doesn't pay it on
/// every run.
fn setup_minimal_world() -> TestWorld {
    let seed = 0x1337_CAFE_B0A7_5EAF;
    let terrain = Terrain::from_seed(seed);
    let hydrology = HydrologyGraph::bake(seed, &terrain);
    let hub_catalog = HubCatalog::bake(seed, &terrain, &hydrology);
    let cells = SubBiomeCellCatalog::bake(seed);
    let foundations = HubFoundationMap::build(&hub_catalog, &cells);
    let groups = FeatureGroupBundle::build_road_network(&hub_catalog, &cells);
    let baked = BakedAssignments::bake(&cells, &terrain, &hydrology, &hub_catalog, &groups);

    let world_refs = WorldRefsArc::new(
        std::sync::Arc::new(terrain),
        std::sync::Arc::new(hydrology),
        std::sync::Arc::new(cells),
        std::sync::Arc::new(hub_catalog),
        std::sync::Arc::new(foundations),
        std::sync::Arc::new(groups),
        std::sync::Arc::new(baked),
    );

    TestWorld { world_refs }
}
