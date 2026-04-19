# flowPlan — Floatplane Delivery & Exploration

> **Role in the docs stack:** PLAN.md is the *creative vision* — the mood, the loop, the thing being built. It is the "why". The technical architecture is in [DESIGN.md](DESIGN.md); the order of work is in [ROADMAP.md](ROADMAP.md); the rules are in [CODING_RULES.md](CODING_RULES.md); verification is in [TEST_PLAN.md](TEST_PLAN.md). Agents read them in the order declared by [CLAUDE.md](CLAUDE.md).

A lightweight, high-FPS 3D game built around the feel of low-altitude floatplane flight: quiet water, pontoon splashes, route-finding between islands. Cel-shaded to keep the GPU cool on high-refresh displays, with carefully placed shader effects so it still reads as pretty.

## Design Pillars

- **Feel over fidelity.** Frame time and response latency are the budget. Hitting 240Hz+ on mid-range hardware is a design goal, not an optimization afterthought.
- **Emergent, not scripted.** Systems interact (weather, cargo, water state, daylight, wildlife, community demand) to produce situations. Missions are template-driven and parameterized by world state -- a legible scaffold *over* the simulation, never a substitute for it. No authored dialog trees, no branching quest lines; sequential template progression layers complexity.
- **Small canvas, deep loop.** Map scope is under active design review (single large map with diverse regional biomes, ~4 separate regional maps, or procedural generation with flight-log memory; see [FEATURES.md § World Scope & Map Layout](FEATURES.md#world-scope--map-layout)). Regardless of choice, deliveries, exploration, weather, wildlife, and the plane itself must carry the game without asset-heavy content pipelines.
- **Sim-realism where it matters for feel.** Aerodynamics are faithful per aircraft; systems are meaningfully simplified. Difficulty dials tune parameters in-game any time; they never swap physics models, preserving determinism.
- **Relaxing, rewarding both ways.** Pick-up-and-play for anyone; rewarding for pilots familiar with mechanics and physics.

## Core Loop

1. Accept a delivery at a hub (cargo weight affects takeoff, fuel burn, CG).
2. Plan a route — fuel, weather, daylight window.
3. Fly low, land on water or at a strip, taxi, unload.
4. Discover something on the way (uncharted cove, weather shift, radio transmission, wildlife).
5. Return with logged discovery → unlocks new routes, charts, cargo types, cosmetic rewards.

Side vectors that can grow organically: mail runs, rescue flights, cartography, salvage, seasonal migration, weather research contracts, fire-hopping, wildlife observation runs. Implementation milestones for this loop are in [ROADMAP.md § Phase 4](ROADMAP.md#phase-4-delivery-loop).

## Tech Stack

| Layer      | Choice                         | Why                                                                                |
| ---------- | ------------------------------ | ---------------------------------------------------------------------------------- |
| Language   | Rust                           | Matches Glicol, no-GC for frame consistency, strong tooling.                       |
| Graphics   | `wgpu`                         | Modern GPU access without Vulkan boilerplate. Cross-platform (Vulkan/Metal/DX12).  |
| Windowing  | `winit`                        | Standard pairing with wgpu.                                                        |
| Math       | `glam`                         | SIMD-friendly, the de facto Rust gamedev math crate.                               |
| Audio DSP  | `glicol_synth`                 | Sound-design-oriented DSL, native Rust, graph-based. Supports seeded deterministic generative music. |
| Audio I/O  | `cpal`                         | Cross-platform audio device abstraction; what Glicol targets.                      |
| Assets     | `gltf` crate                   | Standard 3D format; Blender → glTF pipeline.                                       |

Deliberately **not** using: Bevy, Unreal, Unity. Engines hide the frame. The appeal here is feeling the machine.

## Rendering Approach

**Base:** forward renderer with cel shading — banded diffuse, rim light, outline pass via either inverted hulls or a screen-space sobel on depth+normals. Cel shading is genuinely cheaper (fewer per-pixel lighting ops, smaller texture budget) and forgiving on art.

**Shader accents** — used sparingly so they land:

- **Water** — Gerstner waves in the vertex stage for real geometry displacement on landing zones; simpler normal-map water at distance. Foam from depth difference against shoreline. This is where the budget goes.
- **Sky** — procedural (Preetham or Hosek-Wilkie). Free parallax, free time-of-day, no skybox textures.
- **Clouds** — 2D billboards near camera, optional volumetric only in cutscenes/special weather. Don't burn the frame on clouds.
- **Haze/fog** — exponential height fog with sun-direction tint. Huge atmosphere payoff for ~nothing.
- **Propeller** — disc shader with radial motion blur. Tiny effect, big tactile payoff.
- **Pontoon splash/wake** — particle + decal trail.

**Post:** dithered bloom (cheap), tonemapping, optional color grading LUT. Skip TAA — kills the crisp cel look and adds latency.

## Performance Philosophy

Targets: 240Hz on a 3060-class GPU, 500Hz capable on current-gen midrange. GPU utilization target <60% at target framerate — headroom is the feature.

- **Decouple update from render.** Fixed timestep simulation (120Hz physics/sim), interpolated rendering at display rate. Standard pattern but non-negotiable at these framerates.
- **No allocations in the hot path.** Arena/pool allocators for per-frame data. Real-time discipline applied to a game context.
- **Sparse redraw where possible.** UI layers, distant LODs updated at lower rates.
- **GPU-driven where it helps.** Indirect draw for foliage/props, compute-based culling.
- **Profile from day one.** `tracy` or `puffin` integrated before there's anything to profile.

## Audio Architecture

Glicol as the DSP runtime. Game state drives parameters; audio thread stays lock-free.

- **Engine sound** — Glicol patch modulated by RPM, load, airspeed. Continuous, not sample-based.
- **Environmental** — procedural wind (filtered noise), water lapping, rain (Glicol patches keyed to weather state).
- **Music** — generative, keyed to altitude / weather / time-of-day. Stems composed as Glicol graphs rather than recorded.
- **Comms/radio** — sampled where needed, but filtered through Glicol for the radio EQ, static, squelch.

**Threading:** game thread writes parameter updates to a lock-free ring buffer (`rtrb` or `ringbuf`); audio thread reads, updates Glicol node params, fills the buffer. No mutexes, no allocations, no `println!` in the audio callback.

## Emergence Vectors

Things the simulation should produce without scripting:

- Weather windows that make or break a route.
- Wind direction forcing you to land differently than you planned.
- Fuel math turning a simple delivery into a detour.
- Wildlife or water traffic near a landing zone.
- A discovered cove only reachable when tide/weather/daylight align.
- Radio transmissions from other aircraft creating unscripted awareness of traffic.
- Community demand shifting with seasons (salmon run, fire season, winter freeze-up, festival) creating natural delivery pressure.
- Damage and wear forcing rerouting to the nearest capable hub -- sometimes for hours of in-game time. Recovery quality tracks proximity: a complex hub sends a proper lift; a simple hub sends a pickup truck; remote incidents mean self-extraction or a wildlife-assisted rescue (the latter improving as wildlife-mission reputation grows).
- Forecast error punishing overconfidence: the remote-hub forecast was stale, and the storm arrived earlier.
- Morning mist curling off wingtips during a water takeoff; a post-rain sun-break with alpenglow on distant peaks -- not scripted, just what the weather system happens to produce that morning.
- Post-v1: a dark-sky remote hub on a new-moon night, clear weather, the Milky Way band tilting across the canopy, aurora curling through the north -- a destination is its own reward.

### Non-financial / "pure wildlife" vector

A protected, non-monetized layer of emergent play. The simulation produces moments whose reward is *not* credits. Examples: following an orca pod at dawn, circling a rookery to log a count, responding to a wolf chorus traced across a valley, tracking a moose migration. Rewards are cosmetics, liveries, map annotations, logbook "moments," unique paint-shop textures, and soundtrack stems. This vector is walled off from economy-first pressure by design -- wildlife rewards remain non-financial even when the player is deeply in debt. Preserves the "relaxing" pillar against the "progression treadmill" risk.

If it feels like the world is putting you in situations, the design is working. If you're writing event scripts, step back.

## Starting Path

The phased execution plan lives in [ROADMAP.md](ROADMAP.md), not here. PLAN.md stays above the implementation order. At a glance:

- **Phase 0** -- cargo workspace skeleton, toolchain, architectural enforcement tests, a window that clears.
- **Phase 1** -- fixed-timestep loop, interpolated render, input plumbing, camera, flat terrain, a cube in the air.
- **Phase 2 (Feel Core)** -- cel shading, water, plane, buoyancy, sky. The look and feel locked.
- **Phase 3 (Systems)** -- audio spine with seeded deterministic soundtrack, weather expansion, sim worker thread, CI.
- **Phase 4 (Delivery Loop)** -- hubs, cargo, damage, economy, missions, character foundation, difficulty dials, save/load.
- **Phase 5 (Polish & v1)** -- fleet/hangar, paint shop, celestial sky model (daytime foundation, night gameplay post-v1), wildlife polish + recovery-by-proximity, platform verification.

Each phase has a testable gate that must pass before the next phase begins. No phase is "done" until the gate is green. See [ROADMAP.md § Per-Phase Quality Gate](ROADMAP.md#per-phase-quality-gate).

## Open Design Questions

Live questions that shape the design but are not yet resolved. The authoritative list is in [FEATURES.md § Remaining Open Questions](FEATURES.md#remaining-open-questions); a brief snapshot:

- **World scope.** Single large map with regional biomes, ~4 separate regional maps, fresh-per-session procedural, or hybrid persistent-hubs-plus-procedural-surroundings. Prototype in Phase 1; locked before Phase 4.
- **Fourth region** (if multi-map path chosen). Candidates: Pacific Northwest, subarctic, tropical, high-desert. High-desert currently favored.
- **Procedural persistence cost.** Can flight-log memory bound the save size enough to make a procedural path feasible in v1? Prototype and measure.
- **Biome-seam continuity** (hybrid-candidate specific). How to blend persistent hubs into procedural surroundings without a visible seam.

Resolved creative questions (flight-model depth, mission style, multiplayer posture, platform, damage weight, economy shape) have moved into their authoritative docs. Follow the links in the table above.

## Risks

- **wgpu API churn.** Still pre-1.0. Pin versions. Budget a migration window roughly every six months.
- **Water shader rabbit hole.** Easy to spend months on water. Time-box it -- Phase 2's gate is the discipline.
- **Flight model feel without real hardware.** Tuning aerodynamics without a joystick or HOTAS is harder. v1 targets gamepad + keyboard; HOTAS support is deferred post-v1 ([ROADMAP.md § Future Considerations](ROADMAP.md#future-considerations-post-v1)).
- **Glicol production-readiness.** Great for live coding; less battle-tested as a game audio runtime. The audio architecture is designed so the consumer is replaceable; if Glicol blocks progress, `kira` or `fundsp` is the fallback ([DESIGN.md § Audio Architecture](DESIGN.md#audio-architecture)).
- **Celestial model scope creep.** The astrophotography night sky is aspirational; the renderer lays foundations in Phase 2 and populates in Phase 5, but daytime gameplay remains the v1 commitment.

## References to Chase

- LearnWGPU (sotrh.github.io/learn-wgpu) -- canonical wgpu tutorial
- Catlike Coding -- cel shading, water, general shader craft (Unity-based but concepts transfer)
- GPU Gems 2, Ch. 1 -- classic Gerstner water
- Acerola (YouTube) -- cel shading deep dives
- Glicol docs and `glicol_synth` crate examples
- *Game Engine Architecture* (Gregory) -- decoupled update/render, memory systems
- Stellarium source / Hipparcos catalog -- for the celestial model (Phase 2/5)
