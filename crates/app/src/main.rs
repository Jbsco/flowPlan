//! flowPlan application entry point.
//!
//! Phase 1 scope: fixed-timestep sim loop, input queue, camera view switching, procedural
//! terrain + placeholder cube rendered with a flat-shaded pipeline, double-buffered sim
//! snapshot with interpolation into the render.
//!
//! See [`ROADMAP.md § Phase 1`](../../../ROADMAP.md#phase-1-core-loop).

#![deny(unsafe_code)]
// Binary crates do not expose APIs externally; `unreachable_pub` warnings are noise.
#![allow(unreachable_pub)]

mod debug_gizmo;
mod debug_hud;
mod game_app;
mod input_map;
mod snapshot_buf;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

/// Runtime options parsed from CLI flags. Simple flag parser -- `clap` would be
/// overkill for three flags and adds a dependency we don't need yet.
#[derive(Debug, Default, Clone)]
pub struct CliOptions {
    /// `--scenario <name>`: play back a named `FlightScenario` (see
    /// `flowplan_sim::flight_bench::BEAVER_SCENARIOS`) instead of taking keyboard
    /// input. Primary 2.4.8 feature for visual flight-bench inspection.
    pub scenario_name: Option<String>,
    /// `--log-dir <path>`: capture per-tick telemetry to
    /// `<path>/<scenario>.csv`. Only meaningful with `--scenario`.
    pub log_dir: Option<std::path::PathBuf>,
    /// `--exit-on-scenario-end`: automatically exit once the scenario's duration
    /// elapses. Useful for shell-driven batch runs.
    pub exit_on_scenario_end: bool,
}

fn main() -> Result<()> {
    install_tracing();
    let opts = parse_cli().context("parse CLI")?;
    tracing::info!("flowPlan starting");
    game_app::run(opts)
}

fn parse_cli() -> Result<CliOptions> {
    let mut opts = CliOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" => {
                let name = args
                    .next()
                    .context("--scenario requires a scenario name argument")?;
                opts.scenario_name = Some(name);
            },
            "--log-dir" => {
                let path = args.next().context("--log-dir requires a path argument")?;
                opts.log_dir = Some(path.into());
            },
            "--exit-on-scenario-end" => {
                opts.exit_on_scenario_end = true;
            },
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            },
            other => {
                anyhow::bail!("unknown argument: {other}. Run with --help for usage.");
            },
        }
    }
    Ok(opts)
}

fn print_help() {
    println!(
        "flowPlan

USAGE:
    flowplan [FLAGS]

FLAGS:
    --scenario <name>            Play back a named FlightScenario
                                 (use scenario name from
                                 flowplan_sim::flight_bench::BEAVER_SCENARIOS).
    --log-dir <path>             Write per-tick telemetry CSV to this directory
                                 (used with --scenario).
    --exit-on-scenario-end       Close the window automatically when the scenario's
                                 duration elapses (useful in batch runs).
    -h, --help                   Print this message and exit.

EXAMPLES:
    flowplan
    flowplan --scenario beaver_cruise_sl
    flowplan --scenario beaver_vy_climb --log-dir target/flight-bench --exit-on-scenario-end
"
    );
}

fn install_tracing() {
    // `calloop=error` silences a known benign warning from winit 0.30's Wayland
    // backend ("Received an event for non-existence source") that fires when calloop
    // dispatches a queued event whose source was removed between queueing and
    // dispatch -- typically during resize, scale change, or focus transitions.
    // Upstream race in winit/calloop; no downstream fix. Drop the `calloop=error`
    // segment to re-surface genuine calloop errors if we ever hit one.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,calloop=error")
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
