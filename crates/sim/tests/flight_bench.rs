//! Flight-test-bench integration test (Phase 2.4.8).
//!
//! Iterates the (scenario × variant) matrix: each `FlightScenario` in the catalog
//! runs once per declared variant, with independent telemetry CSVs so each
//! (aircraft, variant, scenario) triple can be reviewed separately. Shared-flight
//! scenarios (cruise, climb, stall, etc.) run for every flyable variant of the
//! target aircraft; contact-physics scenarios narrow to the applicable variant
//! (floats-only for water, wheels-only for ground when 2.5b.2+ lands).
//!
//! Run:
//! ```sh
//! cargo test -p flowplan-sim --test flight_bench
//! ```
//! or via the orchestrator `tools/run_flight_bench.sh` (see `TEST_PLAN.md`).
//!
//! CSV output lands at `target/flight-bench/<aircraft>/<variant>/<scenario>.csv`
//! — flat-listed pre-matrix layouts won't match; `plot_telemetry.py` needs the
//! new path structure.
//!
//! Tolerances start deliberately wide; 2.4.8 coefficient tuning tightens them as
//! POH targets are met.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::manual_assert)]

// Integration-test target pulls in sim's dev-deps via the package graph; suppress
// the unused-crate-dependency warnings for dev-deps this particular test doesn't
// exercise (the bench uses only std + flowplan_sim's public API).
use bytemuck as _;
use flowplan_core as _;
use flowplan_world as _;
use glam as _;
use libm as _;
use proptest as _;
use ron as _;
use serde as _;
use tempfile as _;
use thiserror as _;

use std::fs;
use std::path::PathBuf;

use flowplan_sim::flight_bench::{check_assertions, log_to_csv, BEAVER_SCENARIOS};
use flowplan_sim::{AircraftCatalog, VehicleConfig};

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at the crate dir; workspace root is two levels up
    // (crates/sim -> crates -> flowplan).
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates
    p.pop(); // flowplan
    p
}

fn output_dir() -> PathBuf {
    let dir = workspace_root().join("target").join("flight-bench");
    fs::create_dir_all(&dir).expect("create flight-bench output dir");
    dir
}

#[test]
fn flight_bench_all_pass() {
    let out_base = output_dir();
    let mut failures: Vec<String> = Vec::new();

    // Load the aircraft catalog once; every scenario's aircraft_id is resolved
    // against it. A missing entry is a scenario authoring error, not a runtime
    // failure — panic so the developer sees it immediately.
    let catalog_dir = workspace_root().join("data").join("aircraft");
    let (catalog, errs) = AircraftCatalog::scan(&catalog_dir);
    for e in &errs {
        eprintln!("catalog scan warning: {e}");
    }
    assert!(
        !catalog.is_empty(),
        "catalog is empty at {}; benches require at least one aircraft",
        catalog_dir.display(),
    );

    for scenario in BEAVER_SCENARIOS {
        let entry = catalog.find(scenario.aircraft_id).unwrap_or_else(|| {
            panic!(
                "scenario {} references unknown aircraft_id {:?}",
                scenario.name, scenario.aircraft_id,
            )
        });

        for &variant_id in scenario.variants {
            let variant = entry.find_variant(variant_id).unwrap_or_else(|| {
                panic!(
                    "scenario {} references unknown variant {:?} on aircraft {}",
                    scenario.name, variant_id, scenario.aircraft_id,
                )
            });
            let log =
                run_scenario_headless_with_config(scenario, entry.spec.clone(), variant.config);

            // CSV path: target/flight-bench/<aircraft>/<variant>/<scenario>.csv
            // Nested structure lets plotting scripts walk per-variant folders
            // and keeps shared-flight scenarios' outputs distinguishable.
            let variant_dir = out_base.join(scenario.aircraft_id).join(variant_id);
            fs::create_dir_all(&variant_dir).expect("create variant output dir");
            let csv_path = variant_dir.join(format!("{}.csv", scenario.name));
            fs::write(&csv_path, log_to_csv(&log)).expect("write csv");

            let results = check_assertions(scenario, &log);
            for r in &results {
                if !r.passed {
                    failures.push(format!(
                        "  [{} / {} / {}] {}: {}",
                        scenario.aircraft_id,
                        variant_id,
                        scenario.name,
                        format_assertion(&r.assertion),
                        r.message,
                    ));
                }
            }

            eprintln!(
                "flight-bench: {}/{}/{} -> {} ticks captured, {} assertions ({} passed)",
                scenario.aircraft_id,
                variant_id,
                scenario.name,
                log.len(),
                results.len(),
                results.iter().filter(|r| r.passed).count(),
            );
        }
    }

    if !failures.is_empty() {
        panic!(
            "flight-bench assertions failed ({} total):\n{}",
            failures.len(),
            failures.join("\n"),
        );
    }
}

/// Thin wrapper around `run_scenario_headless` that sets `vehicle.config` so the
/// harness can exercise each variant. Delegated to via the integration test so
/// the library API stays variant-agnostic; the catalog provides the config.
fn run_scenario_headless_with_config(
    scenario: &flowplan_sim::flight_bench::FlightScenario,
    spec: flowplan_sim::AircraftSpec,
    config: VehicleConfig,
) -> Vec<flowplan_sim::flight_bench::BenchSample> {
    flowplan_sim::flight_bench::run_scenario_headless_cfg(scenario, spec, config)
}

fn format_assertion(a: &flowplan_sim::flight_bench::Assertion) -> String {
    format!("{a:?}")
}
