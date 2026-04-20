#!/usr/bin/env bash
#
# flowPlan flight-test-bench orchestrator.
#
# Runs the headless bench (writes CSVs, validates assertions), then generates
# PNGs via the plot script. Optionally also runs the visual playback of each
# scenario in sequence (`--visual`), which opens the game window and plays the
# scripted scenario inputs through the normal renderer + HUD + widgets.
#
# Usage:
#     tools/run_flight_bench.sh                 # headless + plots
#     tools/run_flight_bench.sh --visual        # + watch each scenario in sequence
#     tools/run_flight_bench.sh --visual --pause  # + pause between each for review
#
# Outputs:
#     target/flight-bench/<scenario>.csv        (per-tick telemetry, 120 Hz)
#     target/flight-bench/<scenario>.png        (2x4 panel plot)
#
# The visual runs use the release build + --exit-on-scenario-end so the window
# closes automatically once each scenario's duration elapses, and the shell can
# chain to the next one. Each visual run rewrites its CSV (deterministic, so
# the bytes are identical to the headless run).

set -euo pipefail

cd "$(dirname "$0")/.."

OUT_DIR="target/flight-bench"
VISUAL=0
PAUSE=0

for arg in "$@"; do
    case "$arg" in
        --visual)
            VISUAL=1
            ;;
        --pause)
            PAUSE=1
            ;;
        --help|-h)
            sed -n '3,22p' "$0"
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

# Color helpers (no-op if terminal doesn't support color).
if [ -t 1 ] && command -v tput >/dev/null 2>&1 && [ "$(tput colors 2>/dev/null || echo 0)" -ge 8 ]; then
    BOLD=$(tput bold); GREEN=$(tput setaf 2); CYAN=$(tput setaf 6); YELLOW=$(tput setaf 3); DIM=$(tput dim); RESET=$(tput sgr0)
else
    BOLD=""; GREEN=""; CYAN=""; YELLOW=""; DIM=""; RESET=""
fi

section() { echo "${BOLD}${CYAN}==> $*${RESET}"; }
ok()      { echo "${GREEN}  ✓ $*${RESET}"; }
warn()    { echo "${YELLOW}  ! $*${RESET}"; }

# ---- Step 1: headless bench --------------------------------------------------

# Clean stale outputs from scenarios that have been renamed or deleted. The
# headless bench writes the current set fresh; without a clean start, the
# visual orchestrator (below) would discover stale CSVs from deleted scenarios
# and try to run them through the app, hitting "unknown scenario" errors.
if [[ -d "$OUT_DIR" ]]; then
    rm -f "$OUT_DIR"/*.csv "$OUT_DIR"/*.png
fi

section "headless bench (all scenarios, deterministic, runs assertions)"
if cargo test -p flowplan-sim --test flight_bench --release 2>&1 | tail -5; then
    ok "assertions passed"
else
    warn "assertions failed (continuing so you can still inspect plots)"
fi

# ---- Step 2: plots -----------------------------------------------------------

section "plots"
if command -v python3 >/dev/null 2>&1 && python3 -c "import matplotlib, pandas" 2>/dev/null; then
    python3 tools/plot_telemetry.py --outdir "$OUT_DIR" "$OUT_DIR"/*.csv
    ok "wrote $(ls "$OUT_DIR"/*.png 2>/dev/null | wc -l) plot(s) to $OUT_DIR"
else
    warn "skipping plots: python3 + matplotlib + pandas not available"
fi

# ---- Step 3: visual playback (optional) --------------------------------------

if [[ "$VISUAL" -eq 1 ]]; then
    # Discover scenarios from CSV outputs. Names match the const &[FlightScenario]
    # array in crates/sim/src/flight_bench.rs.
    mapfile -t CSVS < <(ls "$OUT_DIR"/*.csv 2>/dev/null | sort)
    TOTAL=${#CSVS[@]}

    section "visual playback — $TOTAL scenario(s)"
    echo "${DIM}  tip: each window auto-closes when the scenario duration elapses.${RESET}"
    if [[ "$PAUSE" -eq 1 ]]; then
        echo "${DIM}  --pause set: press Enter between scenarios to continue.${RESET}"
    fi
    echo

    IDX=0
    for csv in "${CSVS[@]}"; do
        IDX=$((IDX + 1))
        name=$(basename "$csv" .csv)

        # Infer duration from CSV's last time_s column so the user knows what
        # they're watching.
        duration=$(tail -1 "$csv" | cut -d, -f2 | awk '{printf "%.0f", $1}')

        section "[$IDX/$TOTAL] $name  (${duration}s)"
        cargo run -p flowplan-app --release -- \
            --scenario "$name" \
            --log-dir "$OUT_DIR" \
            --exit-on-scenario-end

        ok "$name complete"

        if [[ "$PAUSE" -eq 1 ]] && [[ "$IDX" -lt "$TOTAL" ]]; then
            echo
            read -r -p "    press Enter for next scenario..." _
        fi
    done
fi

# ---- Summary -----------------------------------------------------------------

echo
section "done"
echo "  outputs:  ${BOLD}$OUT_DIR/${RESET}"
if ls "$OUT_DIR"/*.csv >/dev/null 2>&1; then
    for csv in "$OUT_DIR"/*.csv; do
        name=$(basename "$csv" .csv)
        png="$OUT_DIR/$name.png"
        if [ -f "$png" ]; then
            echo "    $name  (csv + png)"
        else
            echo "    $name  (csv only)"
        fi
    done
fi
