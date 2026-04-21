#!/usr/bin/env python3
"""
Plot flowPlan flight-test-bench telemetry CSVs.

Reads one or more CSVs produced by the bench (`target/flight-bench/*.csv` or the
visual-playback `--log-dir`) and emits one PNG per scenario with a 2x4 panel
layout of the most informative time series.

Panels (reading left-right, top-bottom):
    altitude | airspeed + VS | attitude (pitch/roll/yaw) | α / β + stall
    ground-track (x-z) | moments (M, L, N) | controls | engine + fuel

Usage:
    python3 tools/plot_telemetry.py target/flight-bench/*.csv
    python3 tools/plot_telemetry.py --outdir docs/plots target/flight-bench/*.csv

Dependencies: matplotlib, pandas.
"""

import argparse
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

    fig, axes = plt.subplots(2, 4, figsize=(20, 9))
    fig.suptitle(f"{scenario}", fontsize=14)

    t = df["time_s"]

    # (0,0) Altitude
    ax = axes[0, 0]
    ax.plot(t, df["pos_y"], color="tab:blue")
    ax.axhline(df["pos_y"].iloc[0], color="grey", linewidth=0.5, linestyle=":")
    ax.set_ylabel("altitude (m, MSL)")
    ax.set_xlabel("time (s)")
    ax.set_title("altitude")
    ax.grid(True, alpha=0.3)

    # (0,1) Airspeed (left y) + VS (right y)
    ax = axes[0, 1]
    ax.plot(t, df["airspeed_mps"], label="airspeed", color="tab:orange")
    ax.set_ylabel("airspeed (m/s)", color="tab:orange")
    ax.tick_params(axis="y", labelcolor="tab:orange")
    ax2 = ax.twinx()
    ax2.plot(t, df["vs_mps"], label="VS", color="tab:green", alpha=0.8)
    ax2.axhline(0, color="grey", linewidth=0.5)
    ax2.set_ylabel("VS (m/s)", color="tab:green")
    ax2.tick_params(axis="y", labelcolor="tab:green")
    ax.set_xlabel("time (s)")
    ax.set_title("airspeed + VS")
    ax.grid(True, alpha=0.3)

    # (0,2) Attitude: pitch + roll + yaw (degrees)
    ax = axes[0, 2]
    ax.plot(t, df["pitch_deg"], label="pitch", color="tab:red")
    ax.plot(t, df["roll_deg"], label="roll", color="tab:green", alpha=0.8)
    ax.plot(t, df["yaw_deg"], label="yaw (heading Δ)", color="tab:blue", alpha=0.8)
    ax.plot(t, df["fpa_deg"], label="γ (FPA)", color="tab:purple", linestyle="--", alpha=0.7)
    ax.axhline(0, color="grey", linewidth=0.5)
    ax.set_ylabel("degrees")
    ax.set_xlabel("time (s)")
    ax.set_title("attitude + flight path")
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    # (0,3) α / β + stall threshold
    ax = axes[0, 3]
    ax.plot(t, df["alpha_deg"], color="tab:red", label="α")
    ax.plot(t, df["beta_deg"], color="tab:purple", alpha=0.7, label="β")
    ax.axhline(16.0, color="crimson", linestyle=":", alpha=0.5, label="α_stall (16°)")
    ax.axhline(12.8, color="orange", linestyle=":", alpha=0.5, label="α_approach (12.8°)")
    ax.axhline(-16.0, color="crimson", linestyle=":", alpha=0.5)
    ax.set_ylabel("degrees")
    ax.set_xlabel("time (s)")
    ax.set_title("α / β + stall thresholds")
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    # (1,0) Ground track (world XZ, equal aspect). flowPlan convention: nose -Z
    # forward, +X to the right. Show path, start marker, end marker. Note that
    # "pos_z" grows more negative as the plane flies forward, so we plot -pos_z
    # as "distance ahead" to read naturally.
    ax = axes[1, 0]
    ax.plot(df["pos_x"], -df["pos_z"], color="tab:blue", linewidth=1)
    ax.scatter([df["pos_x"].iloc[0]], [-df["pos_z"].iloc[0]],
               color="tab:green", s=60, zorder=3, label="start")
    ax.scatter([df["pos_x"].iloc[-1]], [-df["pos_z"].iloc[-1]],
               color="tab:red", s=60, zorder=3, marker="X", label="end")
    ax.set_xlabel("x (m, +right)")
    ax.set_ylabel("distance ahead (m, -z)")
    ax.set_title("ground track (world)")
    ax.set_aspect("equal", adjustable="datalim")
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)

    # (1,1) Moment breakdown (N·m). pitch_M, roll_L, yaw_N are the TOTAL per-axis
    # aerodynamic moments after all contributions (control / stability / damping /
    # coupling). The dashed lines show the COUPLING component of roll and yaw —
    # roll-coupling is dominated by torque-roll (engine reaction); yaw-coupling
    # sums adverse-yaw + P-factor + spiral-slipstream. Plotting them lets you see
    # at a glance whether the aircraft is fighting torque vs. slipstream vs. aero
    # restoring moments.
    ax = axes[1, 1]
    ax.plot(t, df["pitch_M"], label="pitch M", color="tab:red", alpha=0.8)
    ax.plot(t, df["roll_L"], label="roll L (total)", color="tab:green", alpha=0.8)
    ax.plot(t, df["yaw_N"], label="yaw N (total)", color="tab:blue", alpha=0.8)
    if "roll_L_coupling" in df.columns:
        ax.plot(
            t,
            df["roll_L_coupling"],
            label="roll coupling (torque)",
            color="tab:green",
            alpha=0.5,
            linestyle="--",
        )
    if "yaw_N_coupling" in df.columns:
        ax.plot(
            t,
            df["yaw_N_coupling"],
            label="yaw coupling (P-factor + slipstream)",
            color="tab:blue",
            alpha=0.5,
            linestyle="--",
        )
    ax.axhline(0, color="grey", linewidth=0.5)
    ax.set_ylabel("moment (N·m)")
    ax.set_xlabel("time (s)")
    ax.set_title("moments (totals + coupling components)")
    ax.legend(loc="best", fontsize=7)
    ax.grid(True, alpha=0.3)

    # (1,2) Controls
    ax = axes[1, 2]
    ax.plot(t, df["throttle"], label="throttle", color="tab:blue")
    ax.plot(t, df["pitch_in"], label="pitch in", color="tab:red", alpha=0.8)
    ax.plot(t, df["roll_in"], label="roll in", color="tab:green", alpha=0.8)
    ax.plot(t, df["yaw_in"], label="yaw in", color="tab:orange", alpha=0.8)
    ax.set_ylabel("input [-1,1]")
    ax.set_xlabel("time (s)")
    ax.set_title("controls")
    ax.set_ylim(-0.1, 1.1)
    ax.legend(loc="best", fontsize=8)
    ax.grid(True, alpha=0.3)
    # Overlay flap position on secondary axis if any non-zero
    if (df["flap_deg"] != 0).any():
        ax2 = ax.twinx()
        ax2.plot(t, df["flap_deg"], label="flap (°)", color="tab:purple", alpha=0.5,
                 linestyle=":")
        ax2.set_ylabel("flap (°)", color="tab:purple")
        ax2.tick_params(axis="y", labelcolor="tab:purple")

    # (1,3) Engine + fuel (dual axis)
    ax = axes[1, 3]
    ax.plot(t, df["engine_kw"], label="engine (kW)", color="tab:blue")
    ax.set_ylabel("engine power (kW)", color="tab:blue")
    ax.tick_params(axis="y", labelcolor="tab:blue")
    ax2 = ax.twinx()
    ax2.plot(t, df["fuel_gph"], label="fuel (gph)", color="tab:orange", alpha=0.7)
    ax2.set_ylabel("fuel flow (gph)", color="tab:orange")
    ax2.tick_params(axis="y", labelcolor="tab:orange")
    ax.set_xlabel("time (s)")
    ax.set_title("engine + fuel flow")
    ax.grid(True, alpha=0.3)

    fig.tight_layout(rect=[0, 0, 1, 0.96])

    out_path = out_dir / f"{scenario}.png"
    fig.savefig(out_path, dpi=110)
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
