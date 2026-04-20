# Cold-Start Checklist — Adding an Aircraft (Worked Example)

This is the long form of the 10-step list in SKILL.md, with the Beaver as the already-landed example. Use it as a template when promoting a catalog entry to an implementation.

## Step 0: Precondition

Confirm the catalog entry exists in [AIRCRAFT.md](../../../AIRCRAFT.md) with status `[ready]` or `[mvp]`. If it's `[sketch]`, stop — sketch status means "captured, not designed." Promoting sketch → ready is a catalog edit (AIRCRAFT.md), not code.

Confirm the aircraft has a ROADMAP slot. Phase 5 Milestone 5.1 is the per-aircraft content pass; during Phase 2-4 only the MVP reference (Beaver) is authorized.

## Step 1: Gather primary sources

What you need:

- **POH** or equivalent (AOM, Flight Manual). Canonical figures.
- **TCDS** where available. Structural limits (Vne, MTOW, CG range).
- **Airfoil reference** — NACA TN for the series, or manufacturer's airfoil data.
- **Engine data** — power curve, fuel burn. Often in engine OEM docs, not POH.
- **Photos** — three angles minimum (side, front, top). Needed for the glTF modeler.

For the Beaver, this was: DHC-2 AOM (publicly shared PDFs), Viking Air's DHC-2T performance data, NACA TN-1945 for 64A-series, operator accounts.

Cite each source in the RON header comment. Reviewers re-trace your figures.

## Step 2: Convert POH figures to SI

See [references/si-conversions.md](si-conversions.md) for the full factor table. Apply once, manually, per field. Comment the original POH figure beside each SI value.

Worked example:

```
POH figure           SI                Field
------------         ----              -----
5,100 lb wheels MTOW 2313.3 kg         max_mass_wheels_kg
250 sq ft            23.23 m²          wing_area_m2
160 mph Vne          71.53 m/s         speeds.vne_mps
125 kt cruise        64.31 m/s         speeds.cruise_mps
52 mph Vs            23.25 m/s         speeds.vs_clean_mps
```

Rounding to 4 sig figs. More is false precision.

## Step 3: Author the RON file

Copy [`data/aircraft/northlander.ron`](../../../data/aircraft/northlander.ron) to `data/aircraft/<name>.ron`. Replace:

1. **Header comments** — cite your sources, note any `(verify)` figures from AIRCRAFT.md.
2. **`name`** — sanitized in-game name from the catalog (e.g., `"Birchling"` for Super Cub).
3. **`inspiration`** — real-world basis + years (e.g., `"Piper PA-18 Super Cub (1949-1994)"`).
4. **Every field** — conversion-comment-then-SI pattern from Step 2.

Parse-test:

```sh
cargo test -p flowplan-sim aircraft_spec
```

The test suite includes a `parses_northlander_minimal` — once the new spec is loaded by the app, runtime parse happens automatically.

## Step 4: Author the glTF

In Blender:

- Scale: meters. Beaver is ~9 m long, ~14.6 m span.
- Orientation: +Y up, -Z forward. **Check glTF export settings**: it's `+Y Up` under `Transform`. Apply All Transforms before export (`Object → Apply → All Transforms`) or Blender's pivot-offset will follow the mesh into-game.
- **Origin at CG**, not at float bottom. The app computes `vehicle_float_offset = (-bbox_min_y).max(0.0)` to lift spawn altitude so floats clear the surface. If origin is at float bottom, rotation pivots at the floats and the feel is wrong.
- Low-poly: target <20k triangles. Cel-shading is forgiving of blocky silhouettes; strong outlines benefit from larger flat faces.
- Merge by Distance + Shade Auto Smooth + Recalculate Normals Outside before glTF export. STL imports create disconnected per-triangle soup; fix in Blender first.
- Export to `data/aircraft/<name>.glb`. Embedded-buffer `.gltf` also works; `.glb` is preferred for single-file delivery.

Optional: commit a `beaver_preview.snap.png`-style visual snapshot to verify it loads.

## Step 5: Update the app loader (if needed)

The app's `game_app.rs` loads `data/aircraft/northlander.glb` + `.ron` at startup. If the aircraft is a *swap* for the MVP reference (unusual during Phase 2), edit that path. For additional aircraft (expected Phase 5), the loader needs to gain a selection mechanism — that is a design amendment, not a catalog change. Flag and stop until DESIGN.md amendment lands.

## Step 6: Write scenarios

See [flowplan-flight-bench/SKILL.md](../../flowplan-flight-bench/SKILL.md) for authoring rules. Minimum coverage per aircraft:

- Cruise hold-altitude at published cruise speed.
- Vy climb (full throttle, hold attitude).
- Clean stall induction (decel from cruise to Vs).

Add to a per-aircraft scenario array (`BEAVER_SCENARIOS` for the Beaver; `SUPERCUB_SCENARIOS` would be next). The integration test iterates each aircraft's array; no wiring beyond adding the array and referencing it from the test.

Until per-aircraft coefficients migrate, start tolerances at 2× the Beaver's, because the shared-coefficient state guarantees worse fidelity than a Beaver-on-Beaver-coefficients test.

## Step 7: Run the bench

```sh
cargo test -p flowplan-sim --test flight_bench --release
```

Expect the new aircraft's scenarios to pass with wide tolerances. Failures indicate either:

- A figure is wrong (re-check Step 2 conversions).
- The aircraft genuinely can't be approximated with Beaver aero — you're hitting the per-aircraft-coefficient limit.

Don't narrow tolerances now. Leave the suite permissive until migration.

## Step 8: Update docs

- **PROGRESS.md** — add a note under the current milestone: `- [shipped: Phase X] <Name> spec + glTF + initial scenarios. Scenarios pass with wide tolerances pending per-aircraft coefficient migration.`
- **Do not update ROADMAP.md** unless the aircraft is the subject of a phase gate (Phase 5 content pass).
- **AIRCRAFT.md** — promote status from `[ready]` to `[shipped]` for that entry.

## Step 9: Commit

One commit, imperative subject, explains *why* this aircraft now (not *what* was added). Example: `Phase 5: Add Birchling (Super Cub) catalog entry + spec for light-taildragger coverage`.

## Step 10: Quality gate

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo bench --bench frame_budget -- --test
```

All four green, or the aircraft isn't merged.

## Deferred: per-aircraft coefficient migration

When this lands (expected 2.4.x follow-up), the catalog gains the ability to make each aircraft *feel* distinct. The migration is:

1. Extend `AircraftSpec` with `FlightCoefficients` (moment coefficients, drag polar, Cl table, prop curve, flap deltas).
2. Move Beaver values from `flight.rs` consts into `northlander.ron`.
3. Tune other aircraft's coefficients (or accept Beaver defaults until tuned).
4. Re-run every aircraft's bench suite; narrow tolerances where coefficients now match POH.

Until then, the catalog is "aircraft *recognize* themselves via speeds and mass, but fly with Beaver feel." Document this limitation prominently on any aircraft shipped during this window.
