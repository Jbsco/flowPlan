#!/usr/bin/env bash
#
# flowPlan flight-test-bench orchestrator.
#
# Runs the headless bench (writes CSVs), then generates PNGs via the plot script.
# Optionally runs the visual playback of each scenario in sequence (`--visual`).
#
# Usage:
#     tools/run_flight_bench.sh                # headless + plots
#     tools/run_flight_bench.sh --visual       # additionally watch each scenario
#
# Outputs:
#     target/flight-bench/<scenario>.csv
#     target/flight-bench/<scenario>.png
#
# The visual runs use the release build + --exit-on-scenario-end so the window
# closes automatically once each scenario's duration elapses, and the shell can
# chain to the next one.

set -euo pipefail

cd "$(dirname "$0")/.."

OUT_DIR="target/flight-bench"
VISUAL=0

for arg in "$@"; do
    case "$arg" in
        --visual)
            VISUAL=1
            ;;
        --help|-h)
            sed -n '3,17p' "$0"
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

echo "==> headless bench"
cargo test -p flowplan-sim --test flight_bench --release -- --nocapture

if command -v python3 >/dev/null 2>&1; then
    echo "==> generating plots"
    if python3 -c "import matplotlib, pandas" 2>/dev/null; then
        python3 tools/plot_telemetry.py --outdir "$OUT_DIR" "$OUT_DIR"/*.csv
    else
        echo "(skipping plots: matplotlib + pandas not installed)" >&2
    fi
else
    echo "(skipping plots: python3 not found)" >&2
fi

if [[ "$VISUAL" -eq 1 ]]; then
    # Discover scenario names from the headless CSV outputs.
    for csv in "$OUT_DIR"/*.csv; do
        name=$(basename "$csv" .csv)
        echo "==> visual playback: $name"
        cargo run -p flowplan-app --release -- \
            --scenario "$name" \
            --log-dir "$OUT_DIR" \
            --exit-on-scenario-end
    done
fi

echo "==> done. outputs in $OUT_DIR/"
