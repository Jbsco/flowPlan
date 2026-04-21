//! Flight-test-bench integration test (Phase 2.4.8).
//!
//! Runs every `FlightScenario` in `BEAVER_SCENARIOS` headlessly, checks assertions,
//! and writes per-scenario telemetry CSVs to `target/flight-bench/`. Fails the test
//! if any assertion is violated; otherwise the CSVs are available for offline
//! analysis via `tools/plot_telemetry.py`.
//!
//! Run:
//! ```sh
//! cargo test -p flowplan-sim --test flight_bench
//! ```
//! or via the orchestrator `tools/run_flight_bench.sh` (see `TEST_PLAN.md`).
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
use proptest as _;
use ron as _;
use serde as _;
use tempfile as _;
use thiserror as _;

use std::fs;
use std::path::PathBuf;

use flowplan_sim::aircraft_spec::AircraftSpec;
use flowplan_sim::flight_bench::{
    check_assertions, log_to_csv, run_scenario_headless, BEAVER_SCENARIOS,
};

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at the crate dir; workspace root is two levels up
    // (crates/sim -> crates -> flowplan).
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates
    p.pop(); // flowplan
    p
}

fn load_spec(relative_path: &str) -> AircraftSpec {
    let full = workspace_root().join(relative_path);
    let ron = fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()));
    AircraftSpec::from_ron(&ron).expect("parse aircraft spec")
}

fn output_dir() -> PathBuf {
    let dir = workspace_root().join("target").join("flight-bench");
    fs::create_dir_all(&dir).expect("create flight-bench output dir");
    dir
}

#[test]
fn beaver_scenarios_all_pass() {
    let out_dir = output_dir();
    let mut failures: Vec<String> = Vec::new();

    for scenario in BEAVER_SCENARIOS {
        let spec = load_spec(scenario.aircraft_spec_path);
        let log = run_scenario_headless(scenario, spec);

        // Always write the CSV so the plotting script has fresh data.
        let csv = log_to_csv(&log);
        let csv_path = out_dir.join(format!("{}.csv", scenario.name));
        fs::write(&csv_path, csv).expect("write csv");

        // Evaluate assertions.
        let results = check_assertions(scenario, &log);
        for r in &results {
            if !r.passed {
                failures.push(format!(
                    "  [{}] {}: {}",
                    scenario.name,
                    format_assertion(&r.assertion),
                    r.message,
                ));
            }
        }

        eprintln!(
            "flight-bench: {} -> {} ticks captured, {} assertions ({} passed)",
            scenario.name,
            log.len(),
            results.len(),
            results.iter().filter(|r| r.passed).count(),
        );
    }

    if !failures.is_empty() {
        panic!(
            "flight-bench assertions failed ({} total):\n{}",
            failures.len(),
            failures.join("\n"),
        );
    }
}

fn format_assertion(a: &flowplan_sim::flight_bench::Assertion) -> String {
    format!("{a:?}")
}
