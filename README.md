# flowPlan

A small, lightweight, high-FPS 3D floatplane / bushplane game built around the *feel* of low-altitude flight over quiet water, remote strips, and weather that matters. Deliveries, exploration, wildlife, and the aircraft itself carry the game; no engine framework, no photorealistic PBR, no scripted narrative -- emergent systems do the work.

"flowPlan" is a working title, a pun on *float plane*.

---

## Status

**Phase 3 — Atmosphere, in progress.** Phases 0 (Skeleton), 1 (Core Loop), and 2 (Feel Core, including flight model + water/ground contact physics) complete. Phase 3 weather Layer W1 (pressure/temperature/humidity/wind/turbulence/frontal graph) and Layer W2 (weather-cell Voronoi partition + per-cell state with life-cycle evolution) are shipped, with a 37-artifact visual-bench harness per seed. World scope locked (2026-04-21) to one continuous toroidal world per seed (strawman 5000 km). Next: weather-affects-flight wiring (3.2.3), audio spine (3.1), orographic/rain-shadow humidity coupling. See [PROGRESS.md](PROGRESS.md) for the authoritative snapshot — it's kept current tick-by-tick and survives session compaction.

This follows the front-loaded design discipline used in the author's prior projects ([hadlink](https://github.com/Jbsco/hadlink), admt) where specifications are the contract and code traces back to them. See [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md) for the approach.

---

## Common commands

All commands run from the workspace root. Release builds are recommended for running the app; debug builds work but are noticeably slower.

### Running the game

```bash
# Open the window with full keyboard flight controls.
# Defaults to the first catalog entry (data/aircraft/*.ron sorted by filename)
# and its declared default_variant — northlander/floats today.
cargo run -p flowplan-app --release

# Aircraft selection (catalog-driven). Pick aircraft + variant explicitly:
cargo run -p flowplan-app --release -- --aircraft northlander --variant wheels
cargo run -p flowplan-app --release -- --aircraft catalina --variant floats
# Or via env var:
FLOWPLAN_AIRCRAFT=catalina:wheels cargo run -p flowplan-app --release
# Enumerate the catalog and exit:
cargo run -p flowplan-app --release -- --list-aircraft
```

In-window controls:

| Key | Action |
|---|---|
| W / S | throttle up / down |
| ↑ / ↓ | stick forward (dive) / back (climb) |
| ← / → | roll left / right |
| Q / E | yaw left / right |
| F / C | flaps extend / retract (4 detents: UP → CLIMB → TAKEOFF → LANDING) |
| 1 / 2 / 3 | view: first-person / chase / exterior orbit |
| V | cycle view |
| L | reset POV look |
| R | reset vehicle |
| Esc | quit |

Screen layout: top-left `debug` panel (tick, view, vehicle pos/vel/attitude, controls, flaps), top-right `dynamics` panel (airspeeds, α/β/Cl/Cd, stall alert, forces, moments-with-coupling, engine, aircraft spec), bottom-left `glide` panel (FPA / α / β / VS / L/D) above the attitude-indicator widget, bottom-right cel-shaded orientation gizmo (body axes + velocity + force vectors).

### Flight test bench (Phase 2.4.8)

The bench replays deterministic `FlightScenario`s through the production `SimState::tick` code path, capturing per-tick telemetry CSVs that feed the plotting pipeline. See [TEST_PLAN.md § Flight Test Bench](TEST_PLAN.md#flight-test-bench) for the design.

```bash
# Headless: iterates the (scenario × variant) matrix — each FlightScenario in
# BEAVER_SCENARIOS runs once per declared variant. Shared-flight scenarios
# (cruise, climb, stall, turns) run for every applicable variant; water
# scenarios only for floats; ground scenarios only for wheels. CSVs at
# target/flight-bench/<aircraft>/<variant>/<scenario>.csv. Fails on any
# assertion violation across the matrix.
cargo test -p flowplan-sim --test flight_bench

# Visual playback of a single scenario. The scenario's own aircraft_id +
# variants drive the catalog selection; --aircraft / --variant can override
# when compatible, error cleanly when they mismatch the scenario.
cargo run -p flowplan-app --release -- --scenario beaver_cruise_sl
cargo run -p flowplan-app --release -- --scenario beaver_land_taxi   # wheels scenario: flat-land terrain, water suppressed
cargo run -p flowplan-app --release -- --scenario beaver_cruise_sl --variant wheels  # run shared-flight scenario in wheels variant
cargo run -p flowplan-app --release -- --scenario beaver_vy_climb \
    --log-dir target/flight-bench --exit-on-scenario-end

# Full orchestrator: headless suite → CSVs → PNG plots (requires matplotlib +
# pandas). Add --visual to additionally watch each scenario in sequence.
# Add --pause to wait for Enter between visual runs.
tools/run_flight_bench.sh
tools/run_flight_bench.sh --visual
tools/run_flight_bench.sh --visual --pause

# Ad-hoc plot generation from captured CSVs (path now includes aircraft + variant).
python3 tools/plot_telemetry.py target/flight-bench/northlander/floats/*.csv
python3 tools/plot_telemetry.py target/flight-bench/northlander/wheels/*.csv

# List all known aircraft variants and their catalog entries.
cargo run -p flowplan-app --release -- --list-aircraft

# CLI reference.
cargo run -p flowplan-app --release -- --help
```

Scenario catalog lives in `crates/sim/src/flight_bench.rs::BEAVER_SCENARIOS`. Each `FlightScenario` declares its `aircraft_id` (catalog id) and `variants` (list of variant ids that exercise the scenario — e.g. `&["floats", "wheels"]` for airborne scenarios, `&["floats"]` for water, `&["wheels"]` for ground). Expand by appending to the array with the right variant list; no other wiring needed.

### Quality gate

Every phase ends with the same gate. Not negotiable — nothing moves forward with red bars.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo bench --bench frame_budget -- --test
```

### Smoke run

```bash
# 3-second boot + exit, for CI-style sanity checks.
timeout 3 env RUST_LOG=info cargo run -p flowplan-app --release
```

### Optional environment

```bash
# Silence noisy info logs (most commonly wgpu_hal). The default filter already
# clamps wgpu_core / wgpu_hal / naga / calloop to warn/error; override with
# RUST_LOG if you want something different.
RUST_LOG=info,wgpu=warn cargo run -p flowplan-app --release

# Shader hot-reload (dev-only convenience; feature-gated so release builds don't
# pull the file-watcher in).
cargo run -p flowplan-app --release --features shader-hot-reload
```

---

## What it is

A single-player, cel-shaded, lightweight 3D flight game. The plane is a floatplane or a bushplane (and eventually an amphibian). You fly deliveries, routes, supply runs, and survey missions between hubs of varying capability in regions shaped after real places -- coastal Alaska, boreal lake country, Mediterranean archipelagos, high-desert plateaus -- without ever being authored as a specific real place.

- **Feel over fidelity.** Frame time and response latency are the budget. The game targets 240Hz+ on mid-range hardware with GPU headroom still remaining. Cel-shading is not just the look; it is part of the budget strategy.
- **Emergent, not scripted.** Weather, wind, cargo, damage, fuel, daylight, community demand, and wildlife interact to produce situations. Missions are template-driven and parameterized by world state; no branching quest trees.
- **Sim-realism where it matters.** Aerodynamics are tabulated per-aircraft and faithful (airfoil lift curves, stall character, density altitude, ground effect). Systems are simplified to meaningful toggles (flaps, trim, mixture, feather, carb heat). Difficulty dials tune parameters in-game without swapping physics models.
- **Relaxing, rewarding both ways.** Pick-up-and-play accessible; rewarding for pilots who know what they're looking at.

Full creative pitch in [PLAN.md](PLAN.md).

---

## Features (planned + shipped)

Status tags: **✓** shipped · **→** active implementation · **⧖** designed, later phase · **⇢** sketched in design backlog. Depth for any item lives in the linked spec doc; the tables below are the contents index.

### World topology (Option A — locked 2026-04-21)

- ✓ Single continuous **procedural** world per seed — no regional abstraction, no loading screens between areas
- ✓ **Toroidal wrap** on all edges — fly off one edge, return at the opposite edge seamlessly
- ✓ Strawman **5000 km × 5000 km** per side (configurable `WORLD_EXTENT_M`)
- ⧖ **Compass locked to world-north** (flat torus has no pole; wrapping preserves orientation)
- ⧖ **Accelerated east-to-west day/night** cycle with longitude-varying local time (24:1 placeholder — 1 sim-hour ≈ 2.5 real-minutes)
- ⧖ **Inland seas** (great-lake-scale) + fjord/lake networks + rivers; circumnavigable-by-land paths exist

[MAPGEN.md § World topology](MAPGEN.md#world-topology)

### Terrain

- ✓ Procedural heightfield (sum-of-sines Phase 1)
- ✓ Beach-flattening near sea level
- ⧖ Ridged multifractal + domain-warp rebuild (Phase 3.6)
- ⧖ **Three-layer composition** — topography × Voronoi × feature-deltas, each ships playable on its own
- ⧖ Voronoi partition into sub-biome feature cells; decoupled streaming shape ≠ chunk shape
- ⧖ Feature-delta heightfield modifications (lakes carve, runways flatten, mesas raise, arches add rock)
- ⧖ Shore classification sampler (`sand_beach / gravel / rock / cliff / mudflat / delta / ice_edge`)
- ⧖ **Hydrology sub-system** — world-bake rivers, lakes, drainage basins, coastlines; wrap-respecting

[MAPGEN.md § Chunk model](MAPGEN.md#chunk-model) · [MAPGEN.md § Hydrology sub-system](MAPGEN.md#hydrology-sub-system)

### Biomes (planned catalog)

- ⧖ **Alaska / Canada baseline** — lakes, bush strips, coastline, boreal forest
- ⧖ **High desert** (Grand Mesa / Flatirons / Arches reference) — dust devils, mesa-top strips, dry-lake landings, thunderstorm anvils
- ⧖ **Mediterranean archipelago** — seaplane-forward, many small islands, open ocean vs protected sea
- ⧖ **Pacific Northwest** — dense fog, low cloud, fjords, logging camps, salmon canneries
- ⧖ **Rocky alpine** — density-altitude, narrow valleys, fire-hopping, glacier-dammed lakes
- ⧖ **Subarctic / North Atlantic** — basalt coasts, heavy fog, midnight sun, aurora in winter
- ⧖ **Tropical island chain** — reefs, squalls, trade winds, coconut atolls, protected lagoons

Biome adjacency is structurally constrained (no snow→desert, no snow→tropical) — enforced by the generator, not smoothed visually. [FEATURES.md § Regional Biomes](FEATURES.md#regional-biomes)

### Weather

Three-layer composition paralleling terrain — W1 atmospheric base × W2 Voronoi cells × W3 deltas. W1 + W2 + life-cycle evolution shipped; W3 + flight coupling in 3.2.3.

- ✓ **W1 atmospheric base:** pressure, temperature, humidity fields (real-world wavelengths 200–500 km)
- ✓ **Altitude-stratified wind** from pressure gradient (geostrophic + boundary-layer friction + 25° surface backing)
- ✓ **Terrain-induced turbulence** (lee rotors, valley channeling, altitude decay)
- ✓ **Frontal-boundary graph** (line-structure analogue of rivers)
- ✓ **W2 Voronoi weather cells** at 20–50 km spacing, with altitude-dependent wind-drift advection
- ✓ **Six cell types** — clear air / cumulus / nimbus / stratus / squall / frontal — with W1 placement-constraint gating (real-world-possible)
- ✓ **Per-type life-cycle evolution** — cumulus 1.5 h, nimbus 2.5 h, stratus 6 h, squall 1.5 h, frontal 8 h (player-tunable)
- ✓ **Player-tunable pacing** via `WeatherTuning` — per-type cycle sliders at seed creation + "fast / normal / slow" presets
- ✓ **37 deterministic visual-bench PNGs** per seed: pressure, temp, humidity, wind, turbulence, fronts, Voronoi, cell type (initial + evolved + phase), dewpoint spread, LCL altitude, synoptic composite, moisture composite, 2000 km continental view
- → **Orographic lifting + rain shadow** (windward humidity boost, lee-side desiccation — next near-term weather upgrade)
- ⧖ **Volumetric clouds** (Phase 3.2x) — fly-through support, CPU-samplable for turbulence/visibility/icing
- ⧖ Precipitation (drizzle / rain / squall / snow / blizzard / hail)
- ⧖ Fog / mist ladder (ground → wingtip → layer → dense)
- ⧖ Icing accumulation in cold + moist cloud interiors
- ⧖ Weather forecast at hubs (accuracy decays by range + time)
- ⧖ Seasonal overlay (spring / summer / autumn / winter)
- ⧖ Atmospheric optics (halos, sun dogs, rainbows, crepuscular rays)
- ⧖ Per-biome weather bias (coastal humid, desert arid, alpine turbulent) — lands with biomes in 3.6

[MAPGEN.md § Weather Voronoi layer](MAPGEN.md#w2--weather-voronoi-partition) · [FEATURES.md § Weather](FEATURES.md#weather)

### Flight model

- ✓ Airfoil lift curves tabulated per aircraft (pre-stall / stall / post-stall regions modeled)
- ✓ Drag polar + induced drag
- ✓ Moment-based rotational dynamics — roll / pitch / yaw with coupling
- ✓ **Secondary effects** — ground effect, P-factor, adverse yaw, torque roll, prop-slipstream
- ✓ Density altitude (temperature + humidity + altitude)
- ✓ Engine + prop + fuel system (mixture, prop pitch, throttle response)
- ✓ **4-detent flaps** — UP / CLIMB / TAKEOFF / LANDING
- ✓ **Buoyancy + water-contact physics** with chop modulation
- ✓ **Ground friction + gear contact** (tire compression, asymmetric damping, strike detection)
- → **Weather-affects-flight wiring** (3.2.3): wind on ground track, turbulence buffet, visibility cut, icing
- ⧖ Per-surface friction (drag differs on sand / gravel / rock / snow / mud)

[FEATURES.md § Flight Model](FEATURES.md#flight-model)

### Aircraft categories (broader catalog — post-MVP expansion)

- ✓ **MVP reference:** DHC-2 Beaver analogue — radial engine, high-wing strut-braced, floats + wheels variants
- ⧖ Classic puddle-jumpers — Cessna 172 / 180, Piper Cub, Beaver, Otter
- ⧖ Flying boats — Catalina, amphibious large-hull
- ⧖ Ultralights + microlights — trikes, powered parachutes, LSA
- ⧖ Cargo haulers — Twin Otter, Caribou, bush C-47 analogues
- ⧖ Water bombers + forestry — CL-215 / -415, single-engine tankers
- ⧖ Gliders + motorgliders — altitude-gated, thermal riding (ties to weather)
- ⧖ Agricultural applicators — Air Tractor analogues, crop-duster patterns
- ⧖ Ski-planes — snow + glacier operations
- ⧖ Amphibians — gear-down water landings locked out
- ⧖ Turbine utility — PC-6 Porter, Kodiak, Caravan analogues

Each aircraft has **floats / wheels / skis / amphib** variants where physically applicable. Placeholder third-party meshes get replaced with project-owner-authored originals before v1. [AIRCRAFT.md](AIRCRAFT.md)

### Upgrades — 5 categories × 3 tiers (branching, not linear)

Tier 3 is a **different optimization** from Tier 1 (not strictly better) — players pick the tree that fits their mission mix.

- ⧖ **Gear** — float / wheel / ski / amphib variants, tire pressure, brake upgrades
- ⧖ **Cargo** — rack / pannier / long-range tank / internal space trade-offs
- ⧖ **Aero** — STOL kit, wing extensions, vortex generators, tail mods
- ⧖ **Powerplant** — engine variants, prop pitch, fuel tuning, turbo / supercharger
- ⧖ **Electronics** — avionics, autopilot, GPS, fuel totalizer, night-ops lighting

[FEATURES.md § Aircraft Configuration & Upgrades](FEATURES.md#aircraft-configuration--upgrades)

### Hubs & navigation

- ⧖ **Five-tier hub system** — major / regional / local / remote / isolated (7–15 / 5–10 / 3–7 / 1 / barn-find mission counts respectively)
- ⧖ **Hub catalog** pre-baked per seed (~50–80K hubs for 5000 km world); late-binding fuzz+resolve pattern snaps hubs to best-fit Voronoi cells
- ⧖ **Real magnetic-heading runway numbering** (degree/360; L/R/C suffix for parallels); magnetic variation field is biome-conditioned
- ⧖ **2D hub map** (hotkey-toggled `M`) — vector/shader-drawn; pan, zoom, biome shading, mission waypoints, fuel-range ring, tier iconography
- ⧖ **Cartographer progression** — progressive reveal tiers (discovered → flown-over → detailed-survey)
- ⧖ **Mission capability overlay** on the hub map (green / yellow / grayed by aircraft capability check)

[FEATURES.md § Hubs & Landing Areas](FEATURES.md#hubs--landing-areas)

### Missions & economy

- ⧖ Template-driven, parameterized by world state (weather, cargo, season, reputation)
- ⧖ **Grayed-mission UX** — destinations show green / yellow / grayed per aircraft capability, with specific failure reason (runway length, fuel range, surface class, cargo capacity) + fleet-swap hint + upgrade hint
- ⧖ Mission types — cargo / passenger / supply run / survey / emergency / barn-find / fire-hopping water-bomber
- ⧖ **No branching quest trees** — emergence over scripting; complexity grows by template progression
- ⧖ **Alaskan-wilderness-community economy frame** — reputation, faction standings, regional supply patterns
- ⧖ **Damage & wear** — zone-local (wings / tail / gear / engine / cargo degrade independently); recovery gated by hub proximity + tier

[FEATURES.md § Missions & Tasks](FEATURES.md#missions--tasks) · [FEATURES.md § Damage & Wear](FEATURES.md#damage--wear)

### Pilot customization

- ⧖ **Metal Slug-inspired art direction**, integrated with cel-shaded world — cartoon simplicity + expressive silhouettes
- ⧖ **Body sliders as independent axes** — height, muscle/boxy, slender/curvaceous (not a gender axis; muscle + curvaceous coexist)
- ⧖ **Eight outfit slots** — hat / goggles / lower-face / shoulders-neck / torso / gloves-arms / pants-shorts / shoes-boots
- ⧖ Primary + optional secondary color per piece
- ⧖ Rig scales outfits during animations (no interpenetration)
- ⧖ **Cosmetic rewards as opt-in alternative** to financial mission reward (rarer at remote hubs — exploration incentive)

[FEATURES.md § On-Foot / Character](FEATURES.md#on-foot--character)

### Profile & world-seed management

- ⧖ **Pilots × seeds matrix** — pilots and seeds are **independent unbounded entities**; per-(pilot, seed) intersection holds local progress
- ⧖ **Unlimited pilot profiles** per install (identity, skills, fleet, currency, career stats, seed-independent)
- ⧖ **Unlimited world seeds** per install — shareable by value (`u64 master_seed + WeatherTuning`); tell a friend a seed, they fly the same world
- ⧖ **Per-(pilot, seed) progress** — fog-of-war map, discovered hubs, per-seed mission completions, resume-point
- ⧖ **NG+ = any pilot with prior progression entering an unvisited seed** (not gated on completion %)
- ⧖ **Fresh pilot** is an explicit opt-in at pilot-creation time for a clean-slate run
- ⧖ World-level state (weather storms, hydrology changes) shared across all pilots on the same seed
- ⧖ Save layout: `saves/pilots/<uuid>.save` + `saves/pilots/<uuid>/<seed_hex>.progress` + `saves/worlds/<seed_hex>.world`; ~100 MB worst-case footprint for 10 pilots × 100 seeds
- ⧖ **Async ghost-planes** (post-v1 MP feature) — any ghost is `(pilot, seed, flight_log)`, replays deterministically

[MAPGEN.md § Determinism & save-format implications](MAPGEN.md#determinism--save-format-implications) · [FEATURES.md § Flight Log & Save Format](FEATURES.md#flight-log--save-format)

### Side activities

- ⧖ **Fishing** — freshwater / saltwater / fly / ice; species-per-biome; seasonal availability
- ⧖ **Wildlife observation** — photography, logging, seasonal migrations (camera mechanic TBD)
- ⧖ **Camping** — status restoration, time passing, campsite establishment at remote sites
- ⧖ **Astrophotography** (post-v1) — biome-dependent dark-sky sites tied to the night-sky shader

[FEATURES.md § Fishing & Side Activities](FEATURES.md#fishing--side-activities)

### Audio

- ✓ Basic audio scaffolding (Phase 0)
- → **Engine sound** — Glicol patch modulated by RPM / load / airspeed (DHC-2 R-985 radial flagship)
- → Wind noise — filtered noise patch keyed to airspeed
- → Water lapping — shoreline procedural
- ⧖ **Seeded deterministic soundtrack** — no pre-recorded music stems; generated per session from world state
- ⧖ Per-biome motifs, storm-cell proximity layers, hub-approach radio, altitude-band pads, wildlife stereo panning
- ⧖ Landing / takeoff musical flourishes at career milestones
- ⧖ **Lock-free audio callback** — no allocations, no logs, no locks in the hot path (discipline invariant)

[FEATURES.md § Audio / Soundtrack](FEATURES.md#audio--soundtrack)

### Visuals & rendering

- ✓ **Cel-shaded pipeline** — flat fills + chosen shader accents, deliberately not PBR
- ✓ **Gerstner water surface** (Phase 2)
- ✓ Procedural sky
- ✓ **Attitude indicator + orientation widget + dynamics panels** (vector-drawn)
- ⧖ **FFT ocean** (Phase 3.5b, Tessendorf method) — reusable compute-pipeline infrastructure shared with 3.2x clouds + 3.6 hydrology bake
- ⧖ **Volumetric clouds** (Phase 3.2x) — fly-through + CPU-samplable shared noise-field
- ⧖ **Particle system** — weather, debris, splash, dust
- ⧖ Aurora (post-v1)
- ⧖ Astrophotography-grade night sky (post-v1)
- ⧖ Target frame budget: **240 Hz+ on mid-range hardware** with GPU headroom remaining

[DESIGN.md § Rendering Budget](DESIGN.md#rendering-budget) · [FEATURES.md § UI Direction](FEATURES.md#ui-direction)

### UI, controls, accessibility

- ✓ **Debug HUD** with toggle (kept indefinitely as in-game overlay; not a dev-only tool)
- ✓ Cockpit dynamics + glide panels + attitude indicator + orientation gizmo
- ⧖ **UI framework** — vector / math / shader-drawn wherever possible, minimal bitmap assets
- ⧖ **Full key/button remapping** (keyboard + mouse + gamepad + HOTAS stub for wider support)
- ⧖ **Difficulty dials** — wind magnitude, turbulence, stall severity, damage sensitivity — tunable in-game any time
- ⧖ **Graphics quality slider** — Low / Medium / High / Ultra / Custom; per-subsystem overrides; streaming disk radius is a knob
- ⧖ **Accessibility** — colorblind-safe UI palettes, remap affordances, magnification

[FEATURES.md § Control Bindings & Mapping](FEATURES.md#control-bindings--mapping) · [FEATURES.md § Display & Graphics Settings](FEATURES.md#display--graphics-settings) · [FEATURES.md § Difficulty & Accessibility](FEATURES.md#difficulty--accessibility)

### Technical foundations

- ✓ Rust (stable) — no GC, predictable frame time
- ✓ `wgpu` direct graphics; no engine framework
- ✓ **Fixed-timestep 120 Hz simulation**, interpolated rendering, lock-free triple-buffered snapshots
- ✓ **Determinism invariant** — `master_seed` → byte-identical simulation across runs (within-platform verified)
- ✓ **Flight-test bench** — deterministic scenario replay, CSV telemetry, matplotlib plot pipeline
- ✓ **CI quality gate** — fmt / clippy / workspace test / frame-budget bench / cross-platform determinism matrix
- ✓ `libm` discipline for platform-independent transcendentals
- ⧖ **Cross-platform determinism** (Linux / Windows / macOS / Steam Deck bit-identical) — Phase 6.1
- ⧖ Sim worker thread + `rtrb` SPSC ring to audio (Phase 3.1 / 3.3 co-land)
- ⧖ **Deterministic event-log save format** — the save *is* the flight log

[DESIGN.md](DESIGN.md) · [CODING_RULES.md § Determinism Discipline](CODING_RULES.md#determinism-discipline) · [TEST_PLAN.md](TEST_PLAN.md)

### Multiplayer (post-v1 stretch goal)

- ⧖ **2-4 player friends-list co-op** — tractable because determinism + fixed-timestep + scenario-replay are already foundational
- ⧖ **Async ghost-planes** — smallest MP feature; ships independently; any pilot's flight log replays deterministically on any install with the same seed
- ⧖ Shared seeds — joining a friend's game = load their world event log + your pilot + fresh per-(you, their-seed) progress
- ⧖ Lockstep netcode on deterministic sim

[ROADMAP.md § Phase 6](ROADMAP.md#phase-6-stretch-goal-post-v1-multiplayer) · [FEATURES.md § Multiplayer](FEATURES.md#multiplayer-stretch-goal-post-v1)

---

## Design pillars

- Fixed-timestep simulation, interpolated rendering, lock-free audio. Determinism is a first-class invariant.
- Narrow, frozen boundaries between subsystems. `core -> sim / progression -> world -> render / audio -> app`, strict DAG.
- Cel-shaded base + sparing shader accents (Gerstner water, procedural sky, aurora post-v1, astrophotography-grade night sky post-v1).
- One continuous procedural world per seed with toroidal wrap (locked 2026-04-21, strawman 5000 km per side); no open-world content treadmill.
- Non-financial progression protected: wildlife, exploration, and astrophotography unlock cosmetic and navigational rewards that cannot be bought. This guards the "relaxing" pillar against economy-first pressure.

Full treatment in [DESIGN.md](DESIGN.md).

---

## Tech stack (planned)

| Layer       | Choice             | Rationale                                                                |
| ----------- | ------------------ | ------------------------------------------------------------------------ |
| Language    | Rust (stable)      | No GC, predictable frame time, matches Glicol.                           |
| Graphics    | `wgpu`             | Modern GPU access, Vulkan / Metal / DX12 cross-platform.                 |
| Windowing   | `winit`            | Standard pairing with wgpu.                                              |
| Math        | `glam`             | SIMD-friendly gamedev math.                                              |
| Audio DSP   | `glicol_synth`     | Sound-design-oriented, native Rust, graph-based.                         |
| Audio I/O   | `cpal`             | Cross-platform device abstraction; what Glicol targets.                  |
| Asset import| `gltf`             | Standard 3D format; Blender pipeline.                                    |

Deliberately **not** using Bevy / Fyrox / Unity / Unreal. Engines hide the frame; the appeal here is feeling the machine. Full list and version policy in [DESIGN.md § Tech Stack](DESIGN.md#tech-stack).

---

## Repository layout

```
flowPlan/
  CLAUDE.md                              # Entry point for AI agents (read order)
  README.md                              # This file
  PLAN.md                                # Creative vision -- the "why"
  DESIGN.md                              # Technical architecture -- the "what"
  CODING_RULES.md                        # Rust + real-time discipline -- the "how"
  ROADMAP.md                             # Phased execution plan -- the "order"
  TEST_PLAN.md                           # Verification strategy -- the "verify"
  FEATURES.md                            # Feature backlog -- every captured detail
  AIRCRAFT.md                            # Living aircraft catalog with design-model specs
  skills/flowplan-methodology/SKILL.md   # Design-first discipline, applied to game dev
  crates/          # One per subsystem (DAG per DESIGN.md); populated in Phase 0
    README.md      # Layer map
    {core, assets, sim, progression, world, render, audio, app}/
  data/            # Content-only .ron files (aircraft, hubs, cargo, missions, wildlife)
    README.md      # Authoring conventions
  assets/          # Source assets (shaders, models, textures, audio) baked at build time
    README.md
  benches/         # Criterion frame-budget benches; per-phase perf gate
    README.md
  tests/           # Cross-crate integration tests
    README.md
  tools/           # Build-time tools (shader baker, data validators, etc.)
    README.md
```

Subdirectories under `crates/`, `data/`, and `assets/` currently hold only `.gitkeep` files; they populate as ROADMAP phases advance. See [DESIGN.md § Directory Structure](DESIGN.md#directory-structure) for the layout rationale.

---

## How to read this repo

### Humans

Start with **[PLAN.md](PLAN.md)** for the feel of the thing. Then **[DESIGN.md](DESIGN.md)** if the architecture interests you. Then **[ROADMAP.md](ROADMAP.md)** for where the project currently stands and how it will advance. **[FEATURES.md](FEATURES.md)** is the backlog of captured ideas and open questions.

### AI agents

Read the specs *in the order declared in [CLAUDE.md](CLAUDE.md)* before any work in the repo. Skim is not read. Every line of code must trace to a spec section. Ambiguity is resolved by proposing an amendment, not by guessing. See [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md).

---

## Core principle

> **The simulation holds the invariants. Rendering interprets state at display rate. Audio traces state lock-free. The boundaries are narrow and frozen.**

Every design and implementation decision honors this one-liner.

---

## What flowPlan is not

- **Not an engine.** flowPlan uses `wgpu` directly. No reusable engine will emerge from v1.
- **Not an MMO.** Single-player in v1. 2-4 player friends-list co-op is a stretch goal for post-v1 — tractable because flowPlan's determinism invariant, fixed-timestep sim, and scenario-replay machinery already provide the foundation lockstep multiplayer needs. See [ROADMAP.md § Phase 6](ROADMAP.md#phase-6-stretch-goal-post-v1-multiplayer).
- **Not photorealistic.** Cel shading is the art direction.
- **Not a full flight sim.** No CFD, no engine-management minigames. Aerodynamics are tabulated and faithful; systems are simplified to meaningful toggles.
- **Not a scripted narrative game.** No authored dialog trees, no branching quests. Missions are template-driven and parameterized by world state.
- **Not content-heavy in the traditional sense.** World scope is one continuous procedural world per seed, wrapping on all edges (locked 2026-04-21, strawman 5000 km × 5000 km). No open authored sprawl; no regional-map abstraction; no randomization that breaks determinism.
- **Not a physics toy.** No rigid-body destruction stacks, no cloth or softbody, no ragdolls.

Full list and rationale in [DESIGN.md § Non-Goals](DESIGN.md#non-goals).

---

## Status badges (planned)

CI / lint / test / frame-budget badges will land when Phase 3 adds the CI workflow. Until then, the local gate is the only gate -- see [ROADMAP.md § Per-Phase Quality Gate](ROADMAP.md#per-phase-quality-gate).

---

## Contributing

This is a personal project. If collaboration opens up, contributions will follow the rules in [CODING_RULES.md](CODING_RULES.md) and the design-first discipline in [skills/flowplan-methodology/SKILL.md](skills/flowplan-methodology/SKILL.md):

- Read the specs first. Every line traces to a spec section.
- Propose design amendments *before* writing code that doesn't already trace.
- Phase gates are non-negotiable: `cargo fmt`, `cargo clippy -D warnings`, `cargo test`, and the phase's frame-budget bench all green before any merge.
- One logical change per commit. Imperative subject, capital letter. Explain *why*, not *what*.
- No speculative features, no engine-ification, no allocations or locks in hot paths.

---

## License

TBD. Placeholder pending decision. Likely permissive (MIT / Apache-2.0) for code; assets may carry a separate license once content exists. Decision captured during Phase 0 skeleton work.

---

## Acknowledgements

Methodology borrowed from two prior projects by the author:

- **hadlink** -- formally verified URL shortener. Skeleton-first commit, contract-first design, invariants as structural guarantees, non-goals as scope protection, phased roadmap with testable gates.
- **admt** -- CLI tool built through multi-agent design convergence. Four foundational spec docs as the single source of truth; code that does not trace is rejected.

The front-loaded design discipline these projects demonstrated is the reason flowPlan's specs exist before its code.
