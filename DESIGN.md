# flowPlan -- Technical Architecture

This document is the authoritative reference for how flowPlan is structured, which modules exist, how they communicate, and what rules govern dependencies. Code examples are illustrative starting points; the architectural *principles*, *invariants*, and *dependency rules* are non-negotiable.

Together with [PLAN.md](PLAN.md), [CODING_RULES.md](CODING_RULES.md), [ROADMAP.md](ROADMAP.md), and [TEST_PLAN.md](TEST_PLAN.md), it defines what flowPlan is and how it is built. Implementation must trace to these documents. Changes to behavior, interfaces, or module boundaries require a design amendment *here* first -- not just a code change.

## Contents

- [Core Principle](#core-principle)
- [Invariants](#invariants)
- [Non-Goals](#non-goals)
- [Design Drivers](#design-drivers)
- [Subsystems](#subsystems)
- [Dependency Rules](#dependency-rules)
- [Threading Model](#threading-model)
- [Data Flow](#data-flow)
- [State Discipline](#state-discipline)
- [Rendering Architecture](#rendering-architecture)
- [Audio Architecture](#audio-architecture)
- [Simulation Architecture](#simulation-architecture)
- [Asset Pipeline](#asset-pipeline)
- [Tech Stack](#tech-stack)
- [Directory Structure](#directory-structure)
- [Trust Boundaries](#trust-boundaries)
- [Performance Budget](#performance-budget)
- [How This Scales](#how-this-scales)

---

## Core Principle

> **The simulation holds the invariants. Rendering interprets state at display rate. Audio traces state lock-free. The boundaries are narrow and frozen.**

This is the design's one-line governance statement. Every subsystem rule, every dependency rule, every threading decision below is a consequence of this principle. If something in this doc appears to contradict it, the doc is wrong; fix the doc first.

---

## Invariants

Invariants are not guidelines. **Violations are bugs.** Every subsystem is designed to make these true structurally, not aspirationally.

1. **Fixed-timestep simulation.** The simulation ticks at a fixed rate (target: 120 Hz). It is driven by an accumulator, not by render frame time.
2. **Interpolated rendering.** The renderer always interpolates between the two most recent simulation states; it never samples the live simulation mid-tick.
3. **Simulation-to-render is read-only.** The render thread reads an immutable snapshot. It cannot mutate simulation state.
4. **Simulation-to-audio is read-only and lock-free.** The audio thread reads via a lock-free single-producer-single-consumer (SPSC) ring buffer. It cannot lock, block, or syscall.
5. **Zero allocations in render and audio hot paths.** Per-frame and per-audio-callback state comes from arenas or preallocated pools. `alloc`/`dealloc` in the hot path is a bug.
6. **Audio callback is lock-free and deterministic-latency.** No mutexes, no condition variables, no `println!`, no file I/O, no syscalls beyond what `cpal` itself performs.
7. **Input feeds simulation, never renders.** Raw `winit` events convert to structured input events and go into the sim input queue. The render path has no input dependency.
8. **Deterministic simulation for the same input sequence and seed.** Given identical seeds, identical input events, and identical fixed-timestep advancement, the simulation produces identical state. This enables replay, testing, and (potentially) async ghost cartography.
9. **Window/device events never stall the simulation.** Resize, focus, and device-lost events are handled in the main thread and do not pause the fixed-timestep accumulator.
10. **Frame budget has headroom.** On the reference hardware tier, GPU utilization at target framerate is <=60% and CPU frame time is <=50% of the display period. Headroom is the feature; using it all for more stuff is a loss.

Any feature, asset, or dependency that cannot be added while preserving all ten invariants is out of scope or requires a documented invariant change.

---

## Non-Goals

Non-goals are **load-bearing**. They prevent scope creep and communicate intent. They are enforced, not aspirational. Requests that conflict with non-goals are rejected or require an explicit amendment to this doc.

### Product non-goals

- **Not an engine.** flowPlan uses `wgpu` directly and does not attempt to produce a reusable engine. Generalization happens after three concrete instances, not before.
- **Not an MMO, not multiplayer-first.** Single-player only in v1. 2-4 player friends-list co-op is a post-v1 stretch goal (see [ROADMAP.md § Phase 6](ROADMAP.md#phase-6-stretch-goal-post-v1-multiplayer)) — tractable because flowPlan's determinism invariant, fixed-timestep sim, snapshot architecture, and deterministic scenario replay are exactly what lockstep multiplayer needs. The methodology dividend lets us defer multiplayer without foreclosing on it. Explicitly still excluded: MMO, dedicated servers, public matchmaking, leaderboards / seasonal events, PvP, more than 4 players, cloud features.
- **Not photorealistic.** Cel shading + chosen shader accents is the art direction. No PBR pipeline, no baked global illumination, no raytracing, no denoiser pipeline.
- **Not a scripted narrative game.** No branching quest trees, no authored dialogue trees, no cutscenes beyond incidental camera work. Missions are template-driven and parameterized by world state (weather, cargo availability, time-of-day, season, reputation); sequential progression increases template complexity. Emergence remains preferred for moment-to-moment situations; missions are a legible scaffold over the simulation.
- **Not content-heavy in the traditional sense.** World scope **locked 2026-04-21 to Option A**: one continuous procedural world per seed with toroidal wrap on all edges (strawman 5000 km per side). Compass locked to world-north (no spherical-pole geometry); day/night cycle runs accelerated east-to-west with longitude-varying local time. Inland seas (great-lake-scale) + fjord/lake networks + rivers cover water-flying gameplay. Progression uses a pilots × seeds matrix: unlimited pilots (global state) × unlimited seeds (shareable by value) × per-(pilot, seed) progress. See [FEATURES.md § World Scope & Map Layout](FEATURES.md#world-scope--map-layout) and [MAPGEN.md § World topology](MAPGEN.md#world-topology). No scripted city-building, no unbounded infinite world (the torus wraps), no per-playthrough randomization that breaks determinism.
- **Not a full flight sim.** No CFD, no engine-management minigames, no weather-radar interpretation minutiae. Aerodynamics are tabulated per-aircraft and faithful (airfoil lift curve, drag polar, stall region, ground effect, P-factor, adverse yaw, density altitude). Systems are simplified to meaningful toggles (carb heat, mags, fuel selector, feather, tire pressure). Sim-realism is selectively deep where it affects feel; accessibility dials tune parameters (gust magnitude, stall severity, damage sensitivity, etc.) without swapping physics models. Player-adjustable in-game any time.
- **Not a physics toy.** Destructible scenery, ragdolls, complex rigid body stacks, cloth, and softbody are all out.

### Technical non-goals

- **No game engine framework.** Not using Bevy, Fyrox, or any ECS engine. flowPlan's ECS (if one emerges) is minimal and grows from need, not from adoption.
- **No GC runtime.** Rust only. No embedded Lua/JS/C# scripting runtime in v1.
- **No dynamic dispatch in hot paths.** Trait objects are fine for cold/setup code; hot paths use static dispatch.
- **No plugin / DLC loader.** No runtime module loading.
- **No cloud features, telemetry, DRM, or account system.**
- **No asset streaming service.** All assets ship with the binary or are downloaded once on first run via a plain archive.
- **No UI toolkit dependency.** Immediate-mode UI at most (`egui` acceptable only for debug overlays; the in-game HUD is hand-rolled).
- **No TAA, no DLSS, no FSR.** These conflict with the cel-shading art direction and add latency.
- **No `unsafe` outside documented FFI sites.** See [CODING_RULES.md](CODING_RULES.md#unsafe-and-ffi).

### Explicit deferrals (may revisit after v1)

- HOTAS / joystick support
- VR support
- Modding interface
- Steam Workshop / user-generated content
- Asynchronous multiplayer (ghost planes, shared cartography)
- Additional weather regimes (snow, sandstorm, thunderhead convective systems)

Deferral is not the same as a non-goal. Deferrals are on the roadmap; non-goals are not.

---

## Design Drivers

Four forces shape every architectural choice. When they conflict, the earlier one wins.

1. **Frame time discipline.** Every subsystem must meet its budget on the reference hardware. See [Performance Budget](#performance-budget).
2. **Emergent, not scripted.** Systems interact to produce situations. Before adding an event trigger, ask whether the simulation can produce the situation naturally.
3. **Narrow, frozen boundaries.** Subsystems communicate through small, explicit interfaces. Changes to an interface require updating every caller in the same commit.
4. **Feel over fidelity.** If a fidelity decision conflicts with feel (response latency, tactile feedback, audio-visual coherence), feel wins.

---

## Subsystems

flowPlan is a layered system. Outer layers depend on inner layers; never the reverse.

```
+---------------------------------------------------------------------------+
|                                   app                                     |
|          Main loop, window, input routing, state machine, lifecycle       |
+---------------------------------------------------------------------------+
|                      render          |           audio                    |
|         GPU, shaders, draw           |      Glicol graph, cpal, SPSC      |
+---------------------------------------------------------------------------+
|                                  world                                    |
|            terrain, water, sky, weather, time-of-day, wind                |
+---------------------------------------------------------------------------+
|             sim (aircraft)        |      progression                      |
|    flight dynamics, buoyancy,     |   damage state, economy ledger,       |
|    cargo physics, fuel,           |   inventory, reputation, loans,       |
|    time-of-day clock              |   missions, character state           |
+---------------------------------------------------------------------------+
|                                   core                                    |
|       math types, arenas, ring buffers, handles, fixed-point, RNG         |
+---------------------------------------------------------------------------+
|                                  assets                                   |
|         loaders, handle table, glTF import, texture/shader cache          |
+---------------------------------------------------------------------------+
```

**Note on `progression`:** The `progression/` layer holds gameplay-state that persists across flights: damage, economy, inventory, reputation, missions, character. Conceptually a sibling of `sim/` (aircraft). Both sit between `core` and `world`. Implementation may be one crate or split into `damage/`, `economy/`, `inventory/`, `missions/`, `character/` -- decide during Phase 4 design.

### Layer responsibilities

| Layer        | Owns                                                                                | Must not                                                        |
| ------------ | ----------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| core         | Math, containers, handles, arenas, SPSC ring, deterministic RNG, time types.        | Touch GPU, audio, OS events, or file I/O beyond its tests.      |
| assets       | glTF loading, texture/shader compile cache, async load jobs, handle table.          | Know about simulation state or render pipelines.                |
| sim          | Pure fixed-timestep aircraft state: flight, buoyancy, cargo physics, fuel, clock.   | Reference GPU or audio types. Allocate inside `tick()`.         |
| progression  | Persistent gameplay state: damage, economy, inventory, reputation, missions, character. Integrated into the sim tick for time-advancing components. | Bypass the tick. Mutate from render/audio. Allocate in hot path. |
| world        | Terrain heightfield, water surface params, weather/wind state, sun direction.       | Do rendering. Own draw calls.                                   |
| render       | Draw calls, shader graphs, post chain, HUD, camera. Reads sim + progression snapshots. | Mutate sim/progression state. Allocate inside the hot path.    |
| audio        | Glicol graph, cpal stream, SPSC reader. Reads sim state via SPSC.                   | Acquire locks. Allocate. Log. Block.                            |
| app          | Window, main loop, accumulator, state machine, event dispatch.                      | Contain gameplay logic.                                         |

### Communication between layers

- **sim -> render:** double-buffered `SimSnapshot` structs. Sim writes to back buffer each tick; render reads the front buffer. Swap under a short mutex at tick boundary, *never* during rendering.
- **sim -> audio:** lock-free SPSC ring of `AudioParamUpdate` events. Sim pushes; audio callback pops up to N per callback.
- **input -> sim:** a bounded MPSC queue of `InputEvent`. Main thread pushes winit-derived events; sim drains at tick start.
- **assets -> render:** opaque `Handle<T>` values. Render never holds raw GPU resources; it goes through the handle table.

Detailed signatures live in their respective module docs (to be added per-phase in [ROADMAP.md](ROADMAP.md)).

---

## Dependency Rules

These rules prevent the architecture from degrading as the codebase grows. They are enforced by automated tests; see [TEST_PLAN.md](TEST_PLAN.md#architectural-enforcement-tests).

1. **`core`** depends on `std`, `glam`, `bytemuck`, and minimal numeric/util crates only. It never depends on wgpu, winit, cpal, glicol, gltf, or any other subsystem in this repo.
2. **`assets`** depends on `core` and asset-format crates (`gltf`, `image`, WGSL parsing). It never depends on `sim`, `progression`, `world`, `render`, `audio`, or `app`.
3. **`sim`** depends on `core`, `world` (pure-primitive samplers: `water_height`, `terrain::height`, `sun_direction`), and — where strictly necessary — asset *types*/handles from `assets`. It never depends on `progression`, `render`, `audio`, `app`, wgpu, winit, or cpal. Aircraft state must not see progression state directly; the tick harness in `app` passes both in. The sim → world direction supports Phase 2.5 buoyancy (per-pontoon water-surface sampling) and is consistent with the layer diagram (world is below sim in the foundation stack). Weather rules that advance sim-level state are hosted inside `sim::tick` when they land, not by inverting the crate dependency.
4. **`progression`** depends on `core` (and asset *types*/handles from `assets` for cargo/paint/aircraft references). It never depends on `sim`, `world`, `render`, `audio`, `app`, wgpu, winit, or cpal. Persistent gameplay state is pure data and deterministic transitions.
5. **`world`** depends on `core` only. It never depends on `sim`, `progression`, `render`, `audio`, or `app`. Terrain / water / sky samplers are pure functions with no state mutation; sim queries them during buoyancy and ground-effect physics.
6. **`render`** depends on `core`, `assets`, `sim` (read-only snapshots), `progression` (read-only snapshots), `world` (read-only state), and `wgpu`/`winit`. It never depends on `audio` or `app`.
7. **`audio`** depends on `core`, `sim` (read-only via SPSC), `progression` (read-only via SPSC where needed for mission/music state), `glicol_synth`, and `cpal`. It never depends on `render`, `world`, `assets`, or `app`.
8. **`app`** depends on all lower layers. Nothing depends on `app`.
9. **No circular dependencies.** The dependency graph is a strict DAG.

Enforcement: a dedicated import-linting test fails CI if these rules are violated. When a developer or agent finds themselves wanting to violate a rule, the answer is "something belongs in a different layer," not "relax the rule."

### Approved unsafe boundaries

`unsafe` is permitted only at the following documented sites:

- wgpu resource lifetime management where the safe API is insufficient (rare; comment with justification).
- cpal audio callback trampolining (framework-required).
- Glicol FFI (if we wrap C++ Glicol rather than `glicol_synth` Rust; justify if adopted).
- `bytemuck::cast_slice` on repr(C)/repr(transparent) POD types.

Every `unsafe` block gets a `// SAFETY: ...` comment explaining why the invariants hold. New `unsafe` sites require the same review attention as a new dependency.

---

## Threading Model

Three threads with strictly bounded responsibilities:

| Thread      | Owned by     | Work                                                    | Cadence            |
| ----------- | ------------ | ------------------------------------------------------- | ------------------ |
| main        | `winit`      | Window/device events, input translation, accumulator, render submit. | display rate |
| sim worker  | `std::thread`| Fixed-timestep tick, snapshot publication, audio event push.          | 120 Hz |
| audio       | `cpal`       | Glicol graph evaluation, SPSC drain, buffer fill.        | device callback (typically 1-10 ms periods) |

### Synchronization primitives permitted

- **sim snapshot:** `parking_lot::Mutex<TripleBuffer<SimSnapshot>>`, held only at tick boundary on the sim thread and at render-prep on the main thread. Hold time <100 us.
- **input queue:** `crossbeam::queue::ArrayQueue<InputEvent>` (bounded MPSC-like; main -> sim).
- **audio param ring:** `rtrb::RingBuffer<AudioParamUpdate>` (strict SPSC; sim -> audio).
- **asset loader:** `std::sync::mpsc` for load requests; results delivered back via handle table updates guarded by an RwLock that render acquires read-only during frame prep (never during draw).

### Synchronization primitives forbidden

- Any lock acquired by the audio thread.
- `std::sync::Mutex` in any hot path (prefer `parking_lot` or lock-free structures).
- Condition variables.
- Spinning waits in the render or audio path.

---

## Data Flow

```
winit events ----> InputEvent queue ----> sim.tick() ----> SimSnapshot (back)
                                                |
                                                |--> AudioParamUpdate --> ring buffer
                                                |
                                    swap back/front at tick end
                                                |
                                                v
                                         SimSnapshot (front)
                                                |
                              main thread reads on render prep
                                                |
                                                v
                       render interpolates (front, prev_front) by alpha
                                                |
                                                v
                                       wgpu::CommandBuffer submit
```

```
cpal audio callback:
  drain ring (bounded N)
  update Glicol node params
  call Glicol fill
  write output buffer
```

### Telemetry derivation (render side)

Instrument displays, HUD elements, and debug visualizers all need the *same* derived quantities -- airspeed (total / horizontal / vertical components), altitude, attitude in degrees, heading, bank angle, vertical speed, angle of attack, sideslip, G-load, slip/skid, stall margin, force breakdown vectors, etc. These are *derivable from* `SimSnapshot + FlightOutputs`, not additional state. They are computed **once per render frame**, on the render side of the boundary, and consumed in parallel by every display surface:

```
SimSnapshot (front) ----\
                        +--> Telemetry (derived, per-frame) --> consumers:
FlightOutputs  --------/                                        - debug HUD (egui)    [Phase 2]
                                                                 - debug gizmo (3D)    [Phase 2.4.2c]
                                                                 - hand-rolled HUD     [Phase 4]
                                                                 - 3D cockpit gauges   [Phase 4+]
                                                                 - damage / engine     [Phase 4+]
                                                                 - autopilot readout   [Phase 4+]
                                                                 - flight log entries  [Phase 5+]
```

**Invariants for the telemetry layer:**

- **Derivation, not state.** `Telemetry` is a pure function of `(SimSnapshot, FlightOutputs, dt)`. No hidden memory, no per-tick accumulators beyond what `SimSnapshot` already carries. If a reading needs history (e.g., G-load peak-hold), the history lives in the *consumer*, not in the telemetry struct.
- **Render-rate, not sim-rate.** Telemetry is computed at display rate, like interpolated render state. This is why it lives in `render` or `app`, not `sim` -- producing it in the sim would bloat the snapshot with values that are just derived quantities.
- **One struct, many consumers.** A single `Telemetry` struct drives every display surface so that the debug HUD's airspeed and the Phase 4 cockpit ASI read from the exact same source. No drift, no duplicated compute, no "why does the gauge disagree with the HUD?" bugs.
- **SI internally, presentation conversions at the consumer edge.** `Telemetry` fields are SI (m/s, rad, m). Knots / feet / degrees / mph conversions happen when the consumer formats a number or positions a needle. This is a one-line detail but locks the invariant that the sim is SI throughout.
- **Determinism unaffected.** `Telemetry` is presentation-only, so wall-clock effects (rolling averages for noisy readings, needle-lag smoothing) are allowed here but never in sim.

This boundary means debug visualizations built in Phase 2 and Phase 3 are the *same plumbing* that Phase 4 hand-rolled HUD elements and Phase 4+ 3D cockpit instruments will plug into. Adding a new instrument later is "consume a field from `Telemetry`; draw it", not "reach into `SimState` and recompute". See [FEATURES.md § Telemetry & Instrumentation Data Flow](FEATURES.md#telemetry--instrumentation-data-flow) for the consumer-side progression.

---

## State Discipline

- **Simulation state is one owner.** The sim module owns `SimState`. No one else has a mutable reference. Sim publishes read-only snapshots.
- **Render has no persistent scene graph.** Each frame's draw list is built from the sim snapshot plus cached GPU resources. No "dirty flag" retained scene; dirty flags are an anti-pattern here.
- **World is a tier of sim state.** `World` structs live inside `SimState`. The `world/` module provides the *rules* for advancing them; it does not hold state itself.
- **Audio's mutable state lives in the Glicol graph** (node parameters and the Glicol node tree). The audio module never observes `SimState` directly -- only the SPSC ring.
- **No globals, no singletons, no lazy_statics.** State is passed explicitly.

---

## Rendering Architecture

See [PLAN.md#rendering-approach](PLAN.md) for the creative direction. The technical architecture:

- **Forward renderer, no deferred.** Cel shading does not benefit from deferred G-buffer complexity; forward keeps MSAA viable and memory bandwidth low.
- **Pipeline layout is static.** All pipelines are created at startup or in asset load jobs. No pipeline creation in the draw path.
- **Shader hot-reload in dev builds only.** Behind a `#[cfg(debug_assertions)]` + a feature flag. Shipping builds use compiled WGSL bundled in.
- **Draw list is an arena-allocated Vec<DrawItem>.** Reset per frame; backing storage persists.
- **Camera interpolation.** Camera follows the same interpolation rule as scene state: smooth between sim ticks.
- **No TAA.** Crisp cel outlines require no temporal smearing. MSAA 4x is acceptable; beyond that is budget-violating.

### Render passes (v1)

1. Depth prepass (opaque geometry, cheap).
2. Opaque forward pass (banded lighting, outline via inverted-hull or screen-space; decide in Phase 2).
3. Water pass (displacement in VS, foam from depth compare).
4. Sky (procedural, full-screen).
5. Particles/decals (splash, wake).
6. Post chain: tonemap, optional bloom (dithered), optional LUT.
7. HUD overlay.

Each pass is a function that takes the command encoder, the frame's resources, and the sim-derived state. No pass pulls directly from `SimState`; all reads go through the frame's interpolated render state.

### Camera-relative rendering (Phase 3.6.3c.5d invariant)

The 5000 km toroidal world puts camera positions routinely past 10⁶ m from origin. At that magnitude, f32 has ~0.5 m ULP, which destroys view-matrix matmul precision. Invariants to preserve:

- **`perspective_view_proj` returns `proj * rotation_only_view`** — no translation column. `Mat4::look_to_rh` is called with `Vec3::ZERO` as the eye, not the world camera position.
- **`CameraUniform.view_pos` carries the world-space camera position**; vertex shaders compute `rel_to_cam = world_pos - camera.view_pos.xyz` before applying `view_proj`. This moves the "large minus large = small" subtraction into a f32 op whose precision depends on the small result magnitude, not the huge operands.
- **All scene-shader descendants** (`shader.wgsl`, `water.wgsl`, `instances.wgsl`) must follow this pattern. Sky and outline pipelines are exempt (full-screen / screen-space).
- **Regression guard**: `view_proj_has_no_translation_column` unit test in `renderer.rs`. Hardware-gated `instance_preview_far` test renders at 2.5 Mm world-coord offset and must be visually identical to the origin render.

**Deferred CPU-side precision work**: sim-side `sim.camera.position` and `sim.vehicle.position` remain `Vec3` (f32). Past ~1 Mm world coords, chase-camera smoothing + aircraft integration both quantize to their f32 ULP (~0.25 m at 2.5 Mm), producing frame-to-frame jitter on dynamic objects. Static geometry (terrain, water, baked instances) is unaffected — their vertex positions are fixed f32 at bake time. Jitter only manifests on debug-cam teleport scenarios (aircraft reaches 1 Mm+ from origin only after hours of normal-play flight). See memory `render_precision_at_world_scale` for the three fix options (A: document + defer — current; B: debug-mode render-origin rebasing; C: full end-to-end camera-relative CPU refactor including potential `DVec3` upgrade for sim-side positions).

---

## Audio Architecture

See [PLAN.md#audio-architecture](PLAN.md) for creative direction. Technical architecture:

- **Glicol graph is constructed at startup.** Node topology does not change per-frame; only node parameters.
- **Parameter update events are bounded POD structs.** `#[repr(C)]`, `Copy`, no heap pointers. They cross the SPSC ring by value.
- **Audio callback drains up to N updates per invocation** (N = 64 for a starting point; revisit with profiling). If the ring is empty, the callback continues with the last state -- no blocking.
- **Generative music state lives in Glicol nodes.** Musical rules execute in the Glicol graph, driven by parameters from sim (time-of-day, altitude, weather).
- **Fallback plan:** if `glicol_synth` blocks progress (per PLAN.md risks), swap to `kira` or a hand-rolled DSP graph via `fundsp`. The SPSC protocol is designed so the audio consumer is replaceable.

### Audio callback contract (hard rules)

Inside the cpal callback:

- No allocations. No `Box`, `Vec::push` (grown), `format!`, or `to_string()`.
- No locks. No `Mutex`, `RwLock`, `parking_lot::*` acquires.
- No syscalls besides what `cpal` does itself.
- No panics on non-fatal input (malformed parameters are clamped or ignored).
- No `println!` / `eprintln!` / `log::*`. Instrumentation goes through a lock-free counter that the main thread reads.

These rules are tested by a custom lint in [TEST_PLAN.md](TEST_PLAN.md) and by explicit `assert_no_alloc` wrappers in debug builds.

---

## Simulation Architecture

- **Fixed timestep via accumulator.** The main thread computes `accumulator += dt`; while `accumulator >= dt_fixed`, the sim thread is nudged to tick. In the starting implementation, main thread runs ticks directly; moving to a sim worker thread is a Phase 3 milestone.
- **Tick is pure.** `fn tick(&mut self, input: &InputEvents, difficulty: &DifficultyDials, dt_fixed: f32)` mutates `SimState` and `ProgressionState` deterministically. No I/O, no time queries, no allocations.
- **Deterministic RNG.** A seeded PCG or xorshift in `core`. No thread-local randomness. Stream separation per subsystem (weather, wildlife, mission-parameterization, wear-accretion) so features can be toggled without reshuffling unrelated streams.
- **Flight model:** per-aircraft tabulated aerodynamic coefficients -- `Cl(alpha)`, drag polar, stall region, flap contributions, ground effect, P-factor, adverse yaw, density-altitude response. Keyed to airspeed, AoA, and atmospheric state. See [FEATURES.md § Flight Model](FEATURES.md#flight-model). No CFD.
- **Buoyancy / hydrodynamics:** per-pontoon / per-hull sampled water height + vertical velocity damping + horizontal drag + step transition (on/off plane) + porpoise risk. This is the load-bearing "feel" moment of the whole game.
- **Damage state:** zone-localized damage (see [FEATURES.md § Damage & Wear](FEATURES.md#damage--wear)) feeds back into the flight model as modifiers to lift, drag, control authority, and thrust. Updated from impact events detected during the tick.

### Weather state variables

Updated each tick on a slow cadence (many ticks per change). All live in `WorldState`:

- `precipitation: PrecipRegime` -- `none | drizzle | light_rain | moderate_rain | heavy_rain | squall | light_snow | heavy_snow | blizzard | hail`.
- `storm: StormState` -- `clear | building_cumulus | scattered_thunderheads | squall_line | organized_storm | frontal_passage`.
- `fog: FogState` -- `none | ground_mist | wingtip_mist | layer_fog | dense_fog`, with a visibility scalar (meters).
- `temperature: Celsius` -- diurnal + seasonal variation, drives density altitude and icing probability.
- `humidity: RelativeHumidity` (0.0..1.0) -- feeds density altitude and icing.
- `wind: WindField` -- base direction + magnitude + gust cycle + turbulence per region, with terrain-induced lee rotor modeled.
- `wind_shear: ShearEvents` -- transient, at terrain edges and storm gust fronts.
- `water_chop: ChopState` per water body -- `calm | ripple | moderate | heavy | breakers`, driven by wind fetch + duration.
- `debris: DebrisRegime` -- dust, leaves, ash/smoke, blowing snow, keyed to biome and weather.
- `season: Season` -- `spring | summer | autumn | winter`, biome-dependent overlay on all of the above.

Weather advances on a slow RNG stream. Forecast accuracy at hubs decays with distance and time.

### Difficulty dials and determinism

Difficulty dials (see [FEATURES.md § Difficulty & Accessibility](FEATURES.md#difficulty--accessibility)) are *parameters* to the sim. They scale magnitudes -- wind, gust variance, shear, storm intensity, stall severity, damage sensitivity, economy harshness, autopilot authority -- but never swap physics models. The determinism invariant remains: for a given `(seed, input trace, difficulty state, dt, min_altitude trace, tick count)` the sim produces bit-identical output. `min_altitude` is the per-tick surface-floor the app supplies (added in Milestone 2.2 to decouple terrain sampling from the sim; it's still caller-supplied until 2.5's proper contact physics lands). Dial changes take effect at the next tick and are themselves part of the state that feeds the deterministic hash.

### Time-of-day and season

- Day/night length is configurable (default ~15-20 min day / 5 min night; slower settings available for relaxed immersion).
- v1 gameplay caps to daylight. Night flight and dawn/dusk flight are post-v1.
- "Camp until morning" action advances time safely to pre-dawn anywhere the aircraft is parked.
- Seasonal cycle (enable vs. static-per-region in v1) is an open question; tracked in [FEATURES.md § Remaining Open Questions](FEATURES.md#remaining-open-questions).

---

## Asset Pipeline

- **Source formats:** glTF 2.0 for meshes, PNG/KTX2 for textures, WGSL for shaders, `.ron`/`.toml` for data.
- **No runtime asset import.** Assets are pre-processed at build time (Phase 1+) into engine-native forms loaded via `bytemuck::cast_slice`.
- **Handle table.** `Handle<T>` is a `u32` index + generation. Asset slots live in a vector; freed slots increment generation to invalidate stale handles.
- **Loaders are async (via `pollster` or a small thread pool).** The main thread never blocks on asset load in the hot path; loading assets during a pause/loading screen is explicit.
- **Hot reload in dev.** `notify` watches source dirs; modified assets re-run the loader, swap the slot, and bump the generation. Shipping builds disable hot reload.

---

## Tech Stack

Deliberately conservative. New dependencies require an amendment to this doc and justification in the PR.

| Layer      | Choice                | Version strategy                                 | Rationale                                                                 |
| ---------- | --------------------- | ------------------------------------------------ | ------------------------------------------------------------------------- |
| Language   | Rust (stable)         | Rolling stable; MSRV documented                  | No GC, strong types, Cargo, matches Glicol.                               |
| Graphics   | `wgpu`                | Pin exact; migrate deliberately every ~6 months  | Modern GPU access, Vulkan/Metal/DX12.                                     |
| Windowing  | `winit`               | Pin with wgpu                                    | Standard pairing.                                                         |
| Math       | `glam`                | `^0.X` with periodic review; `libm` feature enabled | SIMD-friendly gamedev math. `libm` feature engages platform-independent transcendentals for cross-platform determinism. |
| Determinism math | `libm`          | Pin                                              | Platform-independent `sin`/`cos`/`atan2`/`exp`/`ln`/`powf` for deterministic sim + world-gen paths. Replaces platform-varying stdlib transcendentals. See [CODING_RULES.md § Determinism Discipline](CODING_RULES.md#determinism-discipline). |
| Audio DSP  | `glicol_synth`        | Pin; fallback plan documented                    | Native Rust sound design language.                                        |
| Audio I/O  | `cpal`                | Pin                                              | Cross-platform audio device abstraction.                                  |
| Asset import | `gltf`              | Pin                                              | Standard 3D format.                                                       |
| POD casts  | `bytemuck`            | Pin                                              | Safe byte casts for GPU uploads.                                          |
| Images     | `image`               | Pin                                              | PNG/KTX2 decoding.                                                        |
| SPSC ring  | `rtrb`                | Pin                                              | Lock-free SPSC for audio param updates.                                   |
| Bounded Q  | `crossbeam-queue`     | Pin                                              | MPSC-like bounded input queue.                                            |
| Lock prim. | `parking_lot`         | Pin                                              | Faster, fairer than `std::sync::Mutex`.                                   |
| Logging    | `tracing` (+ `tracing-tracy` dev only) | Pin                                 | Structured logging; Tracy for profiling in dev.                           |
| Time       | `std::time` + `quanta` if needed | Pin                                    | High-resolution timestamps.                                               |
| Hot reload | `notify` (dev only)   | Pin, cfg-gated                                   | Shader/asset source watching.                                             |
| Data parallelism | `rayon`         | Pin (workspace dep, world-bake only)             | World-bake hot paths only. Phase 3.6.3c.1c uses `par_iter` to split per-cell sub-biome rule evaluation across cores (~5× speedup, ~275 s → ~55 s on canonical seed). **NEVER on render or audio hot paths** — those follow the no-spawn / no-collect rules in [CODING_RULES.md § real-time discipline](CODING_RULES.md#real-time-discipline). World-bake is a one-time world-load cost, not a per-frame concern. |

### Explicitly not in the stack

Bevy, Fyrox, rapier, specs/legion/hecs, serde-anything-at-runtime, tokio (unnecessary for this workload), async-std, wasm-bindgen, egui outside debug overlays, imgui, cbindgen, web-sys.

If a task seems to require one of these, stop and propose an amendment.

---

## Directory Structure

```
flowPlan/
  Cargo.toml                      # workspace manifest
  rust-toolchain.toml             # pinned stable channel
  .rustfmt.toml                   # style config
  .clippy.toml                    # lint thresholds
  CLAUDE.md                       # agent entry point
  PLAN.md                         # creative vision (read first)
  DESIGN.md                       # this document
  CODING_RULES.md                 # the "how"
  ROADMAP.md                      # phased execution
  TEST_PLAN.md                    # verification strategy
  skills/
    flowplan-methodology/
      SKILL.md                    # design-first discipline
  crates/
    core/                         # math, arenas, handles, RNG, time
      src/
      Cargo.toml
    assets/                       # glTF, textures, shaders loading
    sim/                          # fixed-step simulation state + tick
    world/                        # terrain/water/sky/weather rules
    render/                       # wgpu pipelines, passes, HUD
    audio/                        # cpal + glicol_synth
    app/                          # main loop, winit, state machine
  assets/                         # source assets (committed if <few MB); .gitignored if larger
    shaders/
    models/
    textures/
  tools/                          # build-time preprocessors (asset baking)
  tests/                          # cross-crate integration tests
  benches/                        # criterion benchmarks
  xtask/                          # cargo xtask runners (optional, later phases)
```

One crate per subsystem. The workspace enforces the dependency DAG via `Cargo.toml` `[dependencies]` entries plus a dedicated test (see [TEST_PLAN.md](TEST_PLAN.md)). The crate boundaries are the architectural enforcement mechanism -- don't route around them with a catch-all shared crate.

---

## Trust Boundaries

Even though flowPlan is not a security-sensitive service, the high-assurance habit of naming trust boundaries pays off: it clarifies where validation happens and where state is read-only.

| Boundary                 | Direction      | Rule                                                                 |
| ------------------------ | -------------- | -------------------------------------------------------------------- |
| winit events -> input    | untrusted -> trusted | Validate: clamp cursor deltas, reject unmapped keys. Convert in main thread. |
| input -> sim             | trusted        | Bounded queue, drained at tick start. No unbounded growth.          |
| sim -> render snapshot   | read-only      | Render has a `&SimSnapshot`; mutation is a compile error.           |
| sim -> audio ring        | read-only via queue | Audio consumes POD structs; no references cross the boundary.  |
| asset files -> loader    | untrusted -> trusted | Validate glTF/image/WGSL before producing engine-native data.  |
| config file -> settings  | untrusted -> trusted | Validate at startup; defaults are safe.                        |
| user-provided mods       | N/A            | Not supported in v1 (non-goal).                                     |

Data that crosses a boundary is either validated (at ingress) or strongly typed (so that misuse is a compile error, not a runtime check).

---

## Performance Budget

Targets on the reference hardware tier (NVIDIA RTX 3060 / AMD RX 6600 class GPU, 8-core modern CPU):

| Metric                                   | Budget                                    |
| ---------------------------------------- | ----------------------------------------- |
| Display refresh targets supported        | 30 / 60 / 90 / 120 / 144 / 165 / 240 Hz   |
| Primary tuning target                    | 240 Hz on reference tier                  |
| GPU utilization at 240 Hz                | <= 60%                                    |
| CPU frame time at 240 Hz (~4.2 ms period)| <= 2.1 ms total (<= 50% of period)        |
| Sim tick (120 Hz fixed, display-independent) | <= 2.0 ms per tick, <= 0.5 ms avg     |
| Audio callback (per invocation, ~5 ms)   | <= 0.5 ms                                 |
| Input-to-photon latency                  | <= 2 frames at display rate               |
| Memory working set                       | <= 1 GB RAM, <= 2 GB VRAM                 |
| Binary size                              | <= 50 MB stripped (excluding assets)      |
| Cold start to playable                   | <= 3 s                                    |
| Hot shader reload (dev)                  | <= 250 ms                                 |

These are not aspirations. A phase's quality gate includes an automated check that the current scene meets the budget on the reference hardware, or has an explicit, time-boxed exception.

See [TEST_PLAN.md](TEST_PLAN.md#frame-budget-tests) for how these are measured.

### Display-refresh independence

Sim tick rate is fixed at 120 Hz (see [Invariants](#invariants) #1). Render rate follows the display. This means:

- **Physics is identical at every display rate.** A coordinated 60° turn traces the same path at 30 Hz as at 240 Hz. Flight model, buoyancy, weather advancement, damage events, fuel burn, determinism invariants — all unaffected by display choice.
- **Interpolated rendering fills intermediate frames.** Between sim ticks the renderer alpha-lerps position, slerps orientation, and reads telemetry off the most recent snapshot. Smoothness at 30 Hz and 240 Hz uses the same machinery.
- **The accumulator handles display rates slower than sim.** At 30 Hz display, each render frame consumes up to 4 sim ticks (120 / 30); the accumulator is capped at 8 ticks / 250 ms to prevent spiral-of-death under hitches.
- **Display rates faster than sim fill intermediate frames via interpolation.** The renderer keeps two snapshots (`prev_front` at tick N-1, `front` at tick N) and lerps position / slerps orientation by `alpha = accumulator / dt_sim ∈ [0, 1)`. At 240 Hz display with 120 Hz sim, two consecutive display frames render at different alphas (roughly 0.5 and 0.0) between the same two snapshots, producing smoothly continuous motion. At 500 Hz, four display frames span a sim tick. Physics resolution is fixed at 120 Hz regardless — the extra display frames are interpolated visual frames, not new physics states. Visual smoothness scales with display rate; physics granularity does not.
- **Why sim rate isn't raised with display rate.** Bumping sim to 240 Hz would double CPU cost, break every saved scenario's determinism trace (input traces baked at 120 Hz tick IDs), and re-tune every coefficient. The feel payoff is small for slow-rotation subject matter: Beaver steady-state roll is ~60°/s, worst-case aerobatic roll ~180°/s = 1.5°/tick at 120 Hz. Interpolation between those ticks is visually indistinguishable from a raw higher-rate sim. Input-to-photon latency is sim-rate-bounded (≤ 8.3 ms at 120 Hz) plus display-rate-bounded (≤ 4 ms at 240 Hz, ≤ 17 ms at 60 Hz). 120 Hz sim is the sweet spot for flowPlan's motion profile.
- **Input-to-photon latency scales with display rate.** At 240 Hz, one display frame is 4.2 ms; at 60 Hz, 16.7 ms. Sim-input latency is constant at 8.3 ms (120 Hz); the display-rate piece is added on top. This is a feel tradeoff, not a physics one — low-FPS displays feel "laggier" but the plane is not.

### Hardware tiers

Rates here are *targets* (the primary refresh the hardware is expected to hit at the stated quality preset). All rates in the supported list (30 / 60 / 90 / 120 / 144 / 165 / 240) work on every tier; higher tiers simply have more visual-quality headroom at their target rate.

- **Low-end / Steam Deck:** GTX 1050 / RX 560 / Steam Deck APU, 4-core CPU, 8 GB RAM. Target 60 Hz at "Medium" preset, 90 Hz on Steam Deck handheld mode, reduced water-tessellation tier.
- **Minimum:** GTX 1060 / RX 580, 4-core CPU, 8 GB RAM. Target 120 Hz at "High" preset with all visual accents enabled.
- **Reference:** RTX 3060 / RX 6600, 8-core CPU, 16 GB RAM. Target 240 Hz at "High" with headroom.
- **High:** RTX 4070+ / RX 7800+, modern CPU. 500 Hz capable at "Ultra."

### Presentation mode policy

Default behavior: `wgpu::PresentMode::Mailbox` where supported (tearing-free without the forced frame pacing of FIFO), falling back to `AutoVsync` otherwise. User-configurable modes ship as part of the graphics-settings UI (see [ROADMAP.md § Milestone 4.8.5](ROADMAP.md#milestone-485-graphics--display-settings)):

- **VRR / Mailbox** (default on supported hardware) — tearing-free, lowest latency at the display cap, plays well with G-Sync / FreeSync.
- **VSync / FIFO** — tearing-free, locked to display; one extra frame of latency vs. mailbox.
- **No VSync / Immediate** — tearing, lowest latency, uncapped GPU (highest power/heat).
- **Frame cap** — user-chosen limit (30 / 60 / 90 / 120 / 144 / 165 / 240 / unlimited) to manage GPU utilization and thermals independently of VSync mode.

The sim tick is never coupled to the present mode.

---

## How This Scales

Growth happens **within** crates, not across the boundaries.

| Change                                        | What gets added/modified                                                       |
| --------------------------------------------- | ------------------------------------------------------------------------------ |
| New sim subsystem (e.g., radio comms)         | One module in `crates/sim/`. Snapshot gains fields. Render/audio optionally consume. |
| New shader (e.g., ice surface)                | One file in `assets/shaders/`. One pass wiring change in `crates/render/`.    |
| New asset kind                                | One loader in `crates/assets/`. One handle type in the handle table.          |
| New input device                              | One translation in the main thread. Sim input queue is unchanged.             |
| New particle effect                           | One entry in the particle table. Render knows nothing about gameplay meaning. |

No subsystem needs to know about another subsystem's internals. The system grows additively, not by modifying load-bearing interfaces.

---

## Open design questions

Tracked here so they are visible and owned, not buried. Resolution happens via amendments to this doc during the relevant phase.

- **Sim thread separation timing.** Main-thread ticks in Phase 1; dedicated sim thread in Phase 3. Resolve whether to promote earlier if CPU overhead demands.
- **Draw list structure.** Flat `Vec<DrawItem>` sorted per frame vs. retained bucketed arenas per material. Decide in Phase 2 after first real scene.
- **Water LOD.** Vertex-displaced near, normal-mapped far. Decide transition distance and handoff rule in Phase 2.
- **Flight feel.** Arcade vs. sim-lite. Decide by Phase 2 playtest and lock.
- **Camera model.** Smoothed chase vs. cinematic dolly on landing. Prototype both in Phase 2.

Open questions that drag beyond their phase without resolution trigger a scope review.
