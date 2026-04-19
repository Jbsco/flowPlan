<!-- L0: crate layer map for flowPlan; strict dependency DAG per DESIGN.md -->
# crates/

The Rust workspace crates. One crate per subsystem per [DESIGN.md § Subsystems](../DESIGN.md#subsystems). Each subdirectory will hold its crate's `Cargo.toml` and `src/` when Phase 0 scaffolding begins; currently only the directory skeleton exists.

## Layer map

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
|    flight dynamics, buoyancy,     |   damage, economy, inventory,         |
|    cargo physics, fuel,           |   reputation, missions, character     |
|    time-of-day clock              |                                       |
+---------------------------------------------------------------------------+
|                                   core                                    |
|       math types, arenas, ring buffers, handles, fixed-point, RNG         |
+---------------------------------------------------------------------------+
|                                  assets                                   |
|         loaders, handle table, glTF import, texture/shader cache          |
+---------------------------------------------------------------------------+
```

## Crates

| Crate           | Responsibility                                                      | Depends on                              |
| --------------- | ------------------------------------------------------------------- | --------------------------------------- |
| `core`          | Math, containers, handles, arenas, SPSC ring, RNG, time types       | stdlib, glam, bytemuck                  |
| `assets`        | glTF loading, texture/shader compile cache, handle table            | core                                    |
| `sim`           | Pure fixed-timestep aircraft state and tick                         | core, asset types                       |
| `progression`   | Persistent gameplay state: damage, economy, inventory, missions     | core, asset types                       |
| `world`         | Terrain, water, sky, weather, time-of-day rules                     | core, sim                               |
| `render`        | wgpu pipelines, passes, HUD, camera                                 | core, assets, sim (ro), progression (ro), world (ro), wgpu, winit |
| `audio`         | Glicol graph, cpal stream, SPSC reader                              | core, sim (ro via SPSC), progression (ro via SPSC), glicol_synth, cpal |
| `app`           | Main loop, accumulator, winit events, state machine, lifecycle      | all lower layers                        |

Strict DAG per [DESIGN.md § Dependency Rules](../DESIGN.md#dependency-rules). The layer boundary is the architectural enforcement mechanism -- do not route around it with a catch-all shared crate.

## Non-responsibilities

- Do not add a `util/` or `common/` crate. If a utility belongs in `core`, put it there; otherwise it lives in the one crate that needs it.
- Do not re-export types from one crate in another just to shorten imports.
- `app` depends on everything; nothing depends on `app`.

## Phase 0 is where this gets populated

Subdirectories currently hold `.gitkeep` only. Phase 0 Step 2 in [ROADMAP.md](../ROADMAP.md#phase-0-skeleton) creates each crate's `Cargo.toml` and initial `src/lib.rs` with the crate-level docs and a `#![forbid(unsafe_code)]` attribute where appropriate.
