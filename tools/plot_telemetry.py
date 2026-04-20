#!/usr/bin/env python3
"""
Plot flowPlan flight-test-bench telemetry CSVs.

Reads one or more CSVs produced by the bench (`target/flight-bench/*.csv` or the
visual-playback `--log-dir`) and emits one PNG per scenario with a 2x3 panel layout
of the most informative time series.

Usage:
    python3 tools/plot_telemetry.py target/flight-bench/*.csv
    python3 tools/plot_telemetry.py --outdir docs/plots target/flight-bench/*.csv

Dependencies: matplotlib, pandas (both via pip / apt). The script is deliberately
small and readable -- no CLI library, no config files. Extend the panels as you
need more views; a Beaver performance plot for in-game aircraft descriptions is a
straightforward extension.
"""

import argparse
import os
import sys
from pathlib import Path

try:
    import matplotlib.pyplot as plt
    import pandas as pd
except ImportError as e:
    print(
        f"missing dependency: {e}. install with `pip install matplotlib pandas`.",
        file=sys.stderr,
    )
    sys.exit(1)


def plot_scenario(csv_path: Path, out_dir: Path) -> Path:
    df = pd.read_csv(csv_path)
    scenario = csv_path.stem

    # 2x3 grid of the fields we care about the most. Add more panels as needed.
    fig, axes = plt.subplots(2, 3, figsize=(15, 8), sharex=True)
    fig.suptitle(f"{scenario}", fontsize=14)

    t = df["time_s"]

    # Altitude
    ax = axes[0, 0]
    ax.plot(t, df["pos_y"], color="tab:blue")
    ax.set_ylabel("altitude (m, MSL)")
    ax.set_title("altitude")
    ax.grid(True, alpha=0.3)

    # Airspeed + vertical speed
    ax = axes[0, 1]
    ax.plot(t, df["airspeed_mps"], label="airspeed", color="tab:orange")
    ax.plot(t, df["vs_mps"], label="vertical speed", color="tab:green", alpha=0.7)
    ax.axhline(0, color="grey", linewidth=0.5)
    ax.set_ylabel("m/s")
    ax.set_title("airspeed + VS")
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    # Pitch / roll / yaw angles (attitude)
    ax = axes[0, 2]
    ax.plot(t, df["pitch_deg"], label="pitch", color="tab:red")
    ax.plot(t, df["roll_deg"], label="roll", color="tab:green", alpha=0.7)
    ax.plot(t, df["fpa_deg"], label="γ (FPA)", color="tab:blue", linestyle="--", alpha=0.7)
    ax.axhline(0, color="grey", linewidth=0.5)
    ax.set_ylabel("degrees")
    ax.set_title("attitude + flight path")
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    # Alpha + stall-state threshold
    ax = axes[1, 0]
    ax.plot(t, df["alpha_deg"], color="tab:red", label="α")
    ax.plot(t, df["beta_deg"], color="tab:purple", alpha=0.7, label="β")
    ax.axhline(16.0, color="crimson", linestyle=":", alpha=0.5, label="α_stall")
    ax.axhline(-16.0, color="crimson", linestyle=":", alpha=0.5)
    ax.set_ylabel("degrees")
    ax.set_title("α / β + stall threshold")
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    # Controls
    ax = axes[1, 1]
    ax.plot(t, df["throttle"], label="throttle", color="tab:blue")
    ax.plot(t, df["pitch_in"], label="pitch in", color="tab:red", alpha=0.7)
    ax.plot(t, df["roll_in"], label="roll in", color="tab:green", alpha=0.7)
    ax.plot(t, df["yaw_in"], label="yaw in", color="tab:orange", alpha=0.7)
    ax.set_ylabel("input")
    ax.set_title("controls")
    ax.set_ylim(-1.1, 1.1)
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    # Engine
    ax = axes[1, 2]
    ax.plot(t, df["engine_kw"], label="engine (kW)", color="tab:blue")
    ax.plot(t, df["fuel_gph"], label="fuel (gph)", color="tab:orange", alpha=0.7)
    ax.set_ylabel("")
    ax.set_title("engine + fuel flow")
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    axes[1, 0].set_xlabel("time (s)")
    axes[1, 1].set_xlabel("time (s)")
    axes[1, 2].set_xlabel("time (s)")

    fig.tight_layout(rect=[0, 0, 1, 0.96])

    out_path = out_dir / f"{scenario}.png"
    fig.savefig(out_path, dpi=120, bbox_inches="tight")
    plt.close(fig)
    return out_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv", nargs="+", help="telemetry CSV(s) produced by the bench")
    parser.add_argument(
        "--outdir",
        default="target/flight-bench",
        help="output directory for PNGs (default: %(default)s)",
    )
    args = parser.parse_args()

    out_dir = Path(args.outdir)
    out_dir.mkdir(parents=True, exist_ok=True)

    for csv in args.csv:
        path = Path(csv)
        if not path.exists():
            print(f"skipping {csv}: not found", file=sys.stderr)
            continue
        out = plot_scenario(path, out_dir)
        print(f"{csv} -> {out}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
