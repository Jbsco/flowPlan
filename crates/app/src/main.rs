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

mod ambient_worker;
mod chunked_terrain;
mod chunked_water;
mod chunked_weather;
mod debug_camera;
mod debug_gizmo;
mod debug_hud;
mod game_app;
mod input_map;
mod snapshot_buf;
mod streaming;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

/// Runtime options parsed from CLI flags. Simple flag parser -- `clap` would be
/// overkill for this many flags and adds a dependency we don't need yet.
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
    /// `--aircraft <id>`: select aircraft by stable catalog id (matches the RON
    /// filename, e.g. `northlander` for `data/aircraft/northlander.ron`). Falls
    /// back to `FLOWPLAN_AIRCRAFT` env var's `<id>` segment, then to the
    /// catalog's first entry.
    pub aircraft_id: Option<String>,
    /// `--variant <id>`: select the variant within the chosen aircraft
    /// (`floats` / `wheels` / future). Falls back to `FLOWPLAN_AIRCRAFT` env
    /// var's `:variant` segment, then to the aircraft's `default_variant`.
    pub variant_id: Option<String>,
    /// `--list-aircraft`: print the catalog and exit. Useful for discovering
    /// valid `--aircraft` / `--variant` ids without reading the RON files.
    pub list_aircraft: bool,
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
            "--aircraft" => {
                let id = args
                    .next()
                    .context("--aircraft requires an aircraft id argument")?;
                opts.aircraft_id = Some(id);
            },
            "--variant" => {
                let id = args
                    .next()
                    .context("--variant requires a variant id argument")?;
                opts.variant_id = Some(id);
            },
            "--list-aircraft" => {
                opts.list_aircraft = true;
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
    // Env var fallback: FLOWPLAN_AIRCRAFT=<aircraft_id>[:<variant_id>]. Only
    // applied when the matching CLI flag is absent — CLI wins.
    if opts.aircraft_id.is_none() || opts.variant_id.is_none() {
        if let Ok(raw) = std::env::var("FLOWPLAN_AIRCRAFT") {
            let (a, v) = raw
                .split_once(':')
                .map_or((raw.as_str(), ""), |(a, v)| (a, v));
            if opts.aircraft_id.is_none() && !a.is_empty() {
                opts.aircraft_id = Some(a.to_string());
            }
            if opts.variant_id.is_none() && !v.is_empty() {
                opts.variant_id = Some(v.to_string());
            }
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
    --aircraft <id>              Select aircraft by catalog id (e.g. 'northlander',
                                 'catalina'). Defaults to the first entry in
                                 data/aircraft/ sorted by id. Env fallback:
                                 FLOWPLAN_AIRCRAFT=<id>[:<variant>].
    --variant <id>               Select variant within the chosen aircraft
                                 (e.g. 'floats', 'wheels'). Defaults to the
                                 aircraft's declared default_variant.
    --list-aircraft              Print the catalog and exit.
    -h, --help                   Print this message and exit.

EXAMPLES:
    flowplan
    flowplan --aircraft northlander --variant wheels
    flowplan --list-aircraft
    FLOWPLAN_AIRCRAFT=catalina:floats flowplan
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
