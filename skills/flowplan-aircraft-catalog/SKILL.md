---
name: flowplan-aircraft-catalog
description: Adding or editing an aircraft in flowPlan's catalog. Use when promoting an AIRCRAFT.md sketch to an implemented `data/aircraft/<name>.ron`, authoring scenario coverage for a new aircraft, or reconciling POH figures to the simulator's SI units and coefficient model. Covers the end-to-end path from real-world reference → spec sheet → glTF → scenarios → bench validation.
---

# flowPlan Aircraft Catalog

Adding an aircraft is cheap on paper and structured in code. This skill covers **how to take an aircraft from the catalog to the simulator** without drifting from the specs that govern it.

Authority: [AIRCRAFT.md](../../AIRCRAFT.md) is the living catalog; [`data/aircraft/<name>.ron`](../../data/aircraft) is the authored spec; [`crates/sim/src/aircraft_spec.rs`](../../crates/sim/src/aircraft_spec.rs) defines the schema; [`crates/sim/src/flight_bench.rs`](../../crates/sim/src/flight_bench.rs) is where scenarios attest it flies.

## Core Rules

1. **Catalog first, spec second, glTF third, scenarios fourth.** An entry in AIRCRAFT.md (status `[ready]`) precedes any `.ron` file. A `.ron` file precedes any scenario. Don't invent a spec sheet before the catalog entry lands — that's spec drift.
2. **Every figure cites a source.** `(verify)` tags stay on uncertain entries until a primary source confirms them. A scenario built against a `(verify)` figure gets a wider tolerance.
3. **SI inside the sim, POH units outside.** `AircraftSpec.speeds.*_mps` are SI. Mass is kg. Power stays in hp (recognizability). Fuel stays in US gal (recognizability, since US POHs dominate). See [references/si-conversions.md](references/si-conversions.md).
4. **Sanitized names, no trademarks.** The in-game name is always a homage, never the real name. "Northlander" for the Beaver; "Birchling" for the Super Cub. See [AIRCRAFT.md § MVP Reference](../../AIRCRAFT.md#mvp-reference).
5. **Don't add coefficients outside of `flight.rs` yet.** Per-aircraft moment coefficients, engine curves, and flap tables are currently hardcoded Beaver values. Migrating them into `AircraftSpec` is a planned 2.4.x follow-up that hasn't landed. Until it lands, a new aircraft *reuses* Beaver coefficients with the new spec's mass/area/speeds. Scenarios for the new aircraft will therefore mis-predict — that's fine; they'll tighten when the migration ships.
6. **A new aircraft is not a new flight model.** If the aircraft needs a new physics effect (leading-edge slats, auto-deploying slats, gull-wing coupling, V-tail cross-coupling), that is a flight-model amendment (new milestone in ROADMAP), not a catalog entry.
7. **Characteristic handling notes are non-normative.** The `Characteristic handling:` bullet in AIRCRAFT.md is a design hint. It does not directly set coefficients. See [references/handling-to-coefficients.md](references/handling-to-coefficients.md) for the translation heuristics.

## The `AircraftSpec` Schema

From [`crates/sim/src/aircraft_spec.rs`](../../crates/sim/src/aircraft_spec.rs):

| Field | Unit | Source |
|-------|------|--------|
| `name: String` | - | Sanitized homage; must not be a trademark. Short enough to fit HUD. |
| `inspiration: String` | - | Real-world basis + year range. Documentation only. |
| `empty_mass_kg: f32` | kg | POH empty weight, lb × 0.4536 |
| `max_mass_wheels_kg: f32` | kg | MTOW wheels, lb × 0.4536 |
| `max_mass_floats_kg: f32` | kg | MTOW floats, lb × 0.4536 (often ~5% more than wheels) |
| `wing_area_m2: f32` | m² | sq ft × 0.0929 |
| `aspect_ratio: f32` | - | `wingspan² / wing_area`; same dimensionless value in POH |
| `airfoil_root: String` | - | NACA designation or section name; free-form |
| `airfoil_tip: String` | - | NACA designation or section name; free-form |
| `engine.name: String` | - | Real engine name (documentation; sanitize only if licensing risk is real) |
| `engine.power_hp: f32` | hp | POH rated SL power |
| `engine.cruise_fuel_gph: f32` | US gal/hr | POH cruise fuel burn |
| `fuel_capacity_gal: f32` | US gal | POH usable fuel |
| `speeds.vne_mps` | m/s | Vne, from mph or kt |
| `speeds.cruise_mps` | m/s | Published cruise (mph or kt) |
| `speeds.vs_clean_mps` | m/s | Vs (flaps up) |
| `speeds.vso_mps` | m/s | Vso (flaps + floats) |
| `speeds.vx_mps` | m/s | Best-angle climb |
| `speeds.vy_mps` | m/s | Best-rate climb |

**Missing from the schema (and deliberately so, for now):** moment coefficients, drag polar, Cl(α) table, prop efficiency curve, flap deltas. These are planned to land per-aircraft in a 2.4.x follow-up. Current sim uses hardcoded Beaver placeholders regardless of which `AircraftSpec` is loaded.

**Practical implication:** until the migration lands, authoring a `<aircraft>.ron` gets the game to *recognize* the aircraft for its speeds and mass. It does not get a distinct flight *feel*. A Super Cub will fly like a heavier-than-expected Beaver. Scenarios for the Super Cub will consequently not match POH — tolerate this as a known limitation until per-aircraft coefficients land.

## End-to-End Cold-Start Checklist

For an agent starting from "please add the Super Cub":

1. **Find the catalog entry.** Exact section name in [AIRCRAFT.md](../../AIRCRAFT.md). If the aircraft is in the catalog but `[sketch]`, promote to `[ready]` only after a ROADMAP slot exists.
2. **Gather primary sources.** POH, TCDS, NACA TNs for the airfoil, manufacturer specs. Cite them in the RON header comments.
3. **Convert POH figures to SI** using the formulas in [references/si-conversions.md](references/si-conversions.md). Do the arithmetic on authoring, not at runtime.
4. **Author `data/aircraft/<name>.ron`.** Copy `northlander.ron` as a template. Keep the header comments that cite sources; adjust the imperial→SI conversion comments so reviewers can verify.
5. **Build a glTF** at `data/aircraft/<name>.glb`. Blender export with +Y up, -Z forward. Mesh origin at CG (not at float bottom). Low-poly, cel-shading-friendly (strong silhouette, fewer than ~20k triangles). If the mesh isn't ready, ship the `.ron` alone — the game falls back to the placeholder cube on missing-file errors.
6. **Write scenarios for the aircraft.** Copy the Beaver `FlightScenario` pattern. Replace POH figures with the new aircraft's numbers. See [flowplan-flight-bench/SKILL.md](../flowplan-flight-bench/SKILL.md) for the authoring rules. Wide tolerances at first; tighten when per-aircraft coefficients land.
7. **Run the bench.** `cargo test -p flowplan-sim --test flight_bench --release`. Expect wider tolerance margins than the Beaver; the aircraft is sharing Beaver coefficients.
8. **Cite `(verify)` tags in scenario descriptions.** If a POH figure was `(verify)` in AIRCRAFT.md, say so in the scenario, and use a wider tolerance.
9. **Update PROGRESS.md.** New aircraft → a phase note under the current milestone.
10. **Do not update ROADMAP.md** unless the aircraft is being promoted through a phase gate (e.g., as part of Phase 5 Milestone 5.1 content pass). The milestone structure is load-bearing — adding a single aircraft is not its own milestone.

## Anti-Patterns

1. **Authoring a spec from another flight sim.** X-Plane / MSFS / FSX figures encode someone else's tuning, not the real aircraft. Find the POH instead.
2. **Fuzzing the numbers to make scenarios pass.** If `beaver_vy_climb` fails for the new aircraft, the per-aircraft coefficient migration isn't ready — not a reason to invent mass or power values.
3. **Inventing `(verify)` entries as "close enough".** `(verify)` means *we haven't sourced this*. Leaving a scenario depending on a made-up figure silently invalidates the POH citation discipline.
4. **Using the real aircraft's trademark name.** "DHC-2 Beaver" is the real aircraft; "Northlander" is ours. Never ship "Beaver" in anything user-visible.
5. **Adding `AircraftSpec` fields without an amendment.** The schema is narrow on purpose. New fields (moment coefficients, flap tables) are gated behind the per-aircraft migration that hasn't landed.
6. **Skipping the catalog.** AIRCRAFT.md isn't a summary of what's implemented — it's the design surface. An aircraft that's in the simulator but not in the catalog is a spec violation, even if the code works.

## Configuration Variants

Each aircraft can ship in multiple configurations (wheels, floats, amphibs, skis). Today the sim does not distinguish these at the `AircraftSpec` level — the Beaver's `max_mass_wheels_kg` and `max_mass_floats_kg` are separate fields, but the active mass used by the flight model is `max_mass_wheels_kg` (Phase 2.4 assumption).

When per-variant configuration support lands (expected with Milestone 2.5 contact physics), the pattern will be:

- One spec file per aircraft, one configuration enum, game selects the variant at flight start.
- Variant affects mass (floats add ~120 kg for the Beaver), drag (floats add ~8% Cd0), and stall speed (Vso vs Vs).

Until then, scenarios that want to test a floats-config (`beaver_vso_floats`) simply spawn with `max_mass_floats_kg` and a flap-down Vso target.

## Quick Commands

```sh
# Parse-test a RON file (parses in `AircraftSpec::from_ron`)
cargo test -p flowplan-sim aircraft_spec

# Verify the game loads the spec at startup
timeout 3 env RUST_LOG=info cargo run -p flowplan-app --release

# Run the bench suite that references the new spec (once scenarios exist)
cargo test -p flowplan-sim --test flight_bench --release
```

## References

- [references/si-conversions.md](references/si-conversions.md) — exact conversion factors for POH → SI with worked Beaver examples.
- [references/handling-to-coefficients.md](references/handling-to-coefficients.md) — translating AIRCRAFT.md `Characteristic handling` notes into coefficient intent (docile → conservative damping; STOL → higher Cl_max; etc.), and what's deferred to the coefficient-migration milestone.
- [references/cold-start-checklist.md](references/cold-start-checklist.md) — the long-form cold-start walkthrough with Beaver as the worked example.
- [AIRCRAFT.md](../../AIRCRAFT.md) — the living catalog.
- [FEATURES.md § Aircraft Catalog](../../FEATURES.md#aircraft-catalog) — design direction, per-aircraft data contract.
- [`data/aircraft/northlander.ron`](../../data/aircraft/northlander.ron) — template RON, with SI-conversion comments in the header.
- [`crates/sim/src/aircraft_spec.rs`](../../crates/sim/src/aircraft_spec.rs) — schema + parser.
- [flowplan-flight-bench/SKILL.md](../flowplan-flight-bench/SKILL.md) — how to write scenarios once the aircraft exists.
