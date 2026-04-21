# flowPlan Map Generation (design draft)

> **Role in the docs stack:** MAPGEN.md is the design doc for flowPlan's procedural world generation and terrain representation. It sits between [PLAN.md](PLAN.md) (vision), [DESIGN.md](DESIGN.md) (architecture + invariants), [FEATURES.md](FEATURES.md) (feature backlog), and [ROADMAP.md](ROADMAP.md) (phased execution). This document is **design, not implementation**: it records decisions, proposed strategies, and explicitly-open questions so Phase 3+ terrain / hydrology / world-scope work can start from a concrete specification rather than rediscovering tradeoffs.
>
> **Status:** draft. No code yet. Updates land by amendment PR. Resolution of open questions closes with either a follow-up amendment or a ROADMAP milestone entry.

## Contents

- [Core principles](#core-principles)
- [World scope decision](#world-scope-decision)
- [Chunk model](#chunk-model)
- [Seed topology](#seed-topology)
- [Layered generation](#layered-generation)
- [Hydrology sub-system](#hydrology-sub-system)
- [Biome system](#biome-system)
- [Landscape features](#landscape-features)
- [Hub persistence](#hub-persistence)
- [Flight-log memory](#flight-log-memory)
- [Determinism & save-format implications](#determinism--save-format-implications)
- [Open questions](#open-questions)

---

## Core principles

1. **Deterministic from seed + world-scope choice.** Given the same master seed and the same world-scope mode, the generated world is bit-identical on every machine, every time. This extends the existing [DESIGN.md § Invariants](DESIGN.md#invariants) Invariant #8 (deterministic simulation) to the world-generation layer.
2. **Generate what's needed, persist only what changes.** Chunks far from the player are dormant; regenerated from seed on approach. Player-caused deltas (built sandbar markings, placed waypoints, mission landings, barn-find discoveries) are persisted, not the regenerated terrain itself. This is the [FEATURES.md § World Scope flight-log memory](FEATURES.md#world-scope--map-layout) bound on save size.
3. **Hydrology is global, not per-chunk local.** Rivers, lakes, drainage basins, and coastlines cannot be generated purely from local neighborhood information — they require global coherency. Hydrology is its own sub-system, pre-baked at world-seed time, and chunks query it during their own generation.
4. **Biome adjacency is constrained, not arbitrary.** Forbidden transitions (snow → desert, snow → archipelago, tundra → tropical) are structurally rejected by the generator, not aesthetically smoothed.
5. **The boundary between procedural and authored is explicit.** Hubs, narrative landmarks, and hand-crafted set-pieces live in authored chunks; the bulk of the world is procedural. The seam between the two is part of the design, not an afterthought.
6. **No wall-clock state in generation.** Generation is a pure function of `(master_seed, world_scope_mode, chunk_coords)`. No wall-time, no hidden RNG, no generation order dependency. This lets generation run in parallel, off the main thread, and lets the determinism invariant extend to replay.

---

## World scope decision

The world-scope decision (single large map / 4 regional maps / fresh-per-session procedural / hybrid persistent-hubs + procedural surroundings) is open per [FEATURES.md § World Scope & Map Layout](FEATURES.md#world-scope--map-layout) and must lock before Phase 4.

**MAPGEN.md's position:** this document is written to support **Candidate D (hybrid persistent hubs + procedural exterior)** as the most-flexible-but-tractable option. Under Candidate D:

- Hubs + their immediate zones (~5–10 km radius) are authored or seed-locked per save.
- Distant terrain regenerates from seed on approach via flight-log memory.
- Hydrology is pre-baked once at world-seed time; chunks query it.

Under Candidate C (fresh-per-session), MAPGEN.md applies directly — each session is a new master seed. Under Candidate B (4 regional maps), each region is its own deterministic generated world with its own seed. Under Candidate A (single large authored map), MAPGEN.md is mostly moot except for the biome-classification and hydrology-authoring parts.

**The rest of this doc reads most cleanly under Candidate D assumptions** but should apply with minor edits to C, and partial edits to B.

---

## Chunk model

### Chunk shape: circular or square? (OPEN)

**Square chunks** (the obvious choice, Minecraft pattern):
- Simple indexing: `(i, j)` integer pair, neighbors at `(i±1, j)` / `(i, j±1)` / diagonals.
- Good cache locality for heightmap storage.
- Streaming radius as a Chebyshev (rook) disk around the player.
- Boundary stitching is 4-way.
- Well-understood by every engine out there.

**Hexagonal chunks** (interesting middle ground):
- 6-way boundary stitching (more continuous rivers / roads / biome transitions — less "staircase" artifacting at diagonals).
- Chunk indexing via axial or cube coordinates (well-documented; red-blob-games-level known territory).
- Good streaming radius approximation (near-circular at large radii).
- Fits game-dev precedent for geographic simulations.
- Loses some cache locality vs square but a known-manageable amount.

**Circular / concentric-ring chunks** (the user's question):
- Player-centered rings, radial slices within rings.
- Naturally bounds streaming by radius.
- Boundary stitching is *irregular* — inner-ring / outer-ring boundaries have different arc lengths.
- Chunks shift as the player moves — a chunk at ring 3 relative to player A is at ring 4 relative to player B. This breaks determinism unless chunks are keyed to world coords, not player coords.
- **Likely impractical for persistence** — if chunks are player-centered, the "same chunk" has different coordinates depending on who observed it, and save-deltas become ambiguous.

**Proposal:** default to **hexagonal chunks** if we want to differentiate from Minecraft. Square is the safer choice; hex is the interesting one. Circular-player-centered is probably a no-go for persistence reasons, but circular-world-centered Voronoi cells could work if we commit to a world-seed-locked Voronoi partition.

**Decision deferred** until a computational prototype pass confirms which model can sustain streaming + boundary stitching + hydrology-query + delta-persistence at target performance. Pre-Phase-4 design work.

### Chunk size

Aircraft ground speed: cruise ≈ 55 m/s ≈ 200 km/h. In 1 minute the aircraft covers ~3 km. In 10 minutes, ~30 km. At common delivery distances (50–200 km), a flight is 15–60 minutes.

**Minecraft chunk: 16 m × 16 m.** Far too small for aircraft — thousands of chunks would need streaming.

**Proposed flowPlan chunk: 1 km × 1 km** (square) or ~1 km² equivalent area (hex side ≈ 620 m). Gives ~100 chunks within a 5 km streaming radius and ~900 chunks within a 15 km radius. Manageable as preallocated pool; chunk-generation budget per chunk is moderate.

**Alternative:** 4 km² chunks (2 km side) if heightfield resolution can stay coarse. Larger chunks = fewer boundary-stitching problems but more work per chunk generation.

**Decision deferred** to prototype.

### Heightfield resolution per chunk

For 1 km² chunks:
- **1 m per sample** (1024×1024): ~4 MB per chunk heightfield (f32). Too expensive to persist raw; fine for in-memory.
- **4 m per sample** (256×256): 256 KB per chunk. Reasonable.
- **16 m per sample** (64×64): 16 KB per chunk. Fast, loses ground detail.

Proposed hybrid: **16 m base** for distant chunks, **4 m refinement** for chunks within 2–3 km of the aircraft, **1 m refinement** only for the chunk the aircraft is in + immediate neighbors. LOD streaming, not uniform detail.

### Streaming model

- **Preallocated chunk pool** sized to max streaming radius. No heap allocation during flight.
- **Priority queue** of chunks to generate, sorted by distance-to-aircraft.
- **Background generation thread** that consumes the queue and publishes completed chunks to the main-thread handle table.
- **Eviction** by LRU + distance: chunks outside the max streaming radius are returned to the pool.
- **No spiral-of-death behavior**: chunk-generation budget is capped per tick; if chunks aren't ready, the renderer draws a lower-LOD tier (or a placeholder visible-edge marker) until they are. Honest visible LOD is better than frame drops.

---

## Seed topology

### Seeds at each layer

A single **master world seed** (u64) is the entry point. Per-subsystem sub-seeds derive deterministically:

```
master_seed
├── terrain_seed       = hash(master_seed, "terrain")
├── hydrology_seed     = hash(master_seed, "hydrology")
├── biome_seed         = hash(master_seed, "biome")
├── features_seed      = hash(master_seed, "features")
├── hubs_seed          = hash(master_seed, "hubs")
├── weather_seed       = hash(master_seed, "weather")  # already used
├── wildlife_seed      = hash(master_seed, "wildlife") # already used
└── missions_seed      = hash(master_seed, "missions") # already used
```

Per-chunk seeds are further derived from sub-seed + chunk coords:

```
chunk_terrain_seed   = hash(terrain_seed, i, j)
chunk_biome_seed     = hash(biome_seed, i, j)
chunk_features_seed  = hash(features_seed, i, j)
```

**Hashing function:** a deterministic platform-independent 64-bit hash. SipHash or wyhash; must match across OSes / architectures for cross-platform determinism (cf. [ROADMAP.md § Milestone 6.1 determinism hardening](ROADMAP.md#milestone-61-determinism-hardening-cross-platform)).

**RNG stream:** each per-chunk-seed seeds a fresh PCG or xorshift RNG for that chunk's generation. No cross-chunk RNG dependency.

### Why deterministic sub-seed derivation matters

- **Replay:** save-games (which only persist deltas) regenerate the world exactly from the master seed.
- **Cross-platform save sharing:** a save from Linux plays identically on Windows / Steam Deck.
- **Async ghost-plane multiplayer:** a ghost flight's map context is the shared master seed; no need to ship terrain with the flight log.
- **Scenario bench compatibility:** scenarios pin master seed, produce bit-identical CSV output regardless of generation order.

---

## Layered generation

Each chunk is generated by a fixed pipeline of layers. Each layer is a pure function of `(chunk_coords, sub_seed, output_of_prior_layers, hydrology_graph_reference)`.

```
Layer 0: Biome classification     (lookup from pre-baked biome map)
Layer 1: Heightfield base         (noise + domain warp, biome-parameterized)
Layer 2: Hydrology overlay        (carve rivers, shape lake basins from graph)
Layer 3: Shore classification     (slope × biome × water adjacency)
Layer 4: Feature placement        (landmarks, hubs, mission anchors)
Layer 5: Decoration               (flora patches, rocks, small detail)
```

**Layer 0 (Biome):** queries the world-bake biome map at chunk center → returns biome tag + blend ratios at chunk edges. Biome adjacency rule: forbidden pairs (snow ↔ desert, etc.) are structurally rejected in the bake, so a chunk can never inherit a forbidden-adjacency biome from its neighbors.

**Layer 1 (Heightfield):** biome-parameterized noise generator. Replaces Phase 2's sum-of-sines. Proposed: ridged multifractal + domain warping, with per-biome amplitude and frequency profiles. Supports independent amplitude and gradient tuning (the Phase 2 limitation). Edge-matching via seeded boundary conditions from neighbor chunks.

**Layer 2 (Hydrology):** carves rivers (width, depth) and lake basins into the heightfield per the world-bake hydrology graph. The graph tells the chunk "a river of X width and Y flow direction crosses this chunk from edge A to edge B, passing through these waypoints"; the chunk carves accordingly.

**Layer 3 (Shore classification):** detects water-adjacent cells (lake edge, river bank, coast) and classifies `{sand_beach, gravel_beach, rock, cliff, mudflat, delta, ice_edge}` from `(slope, biome, climate_overlay)`. Drives hub placement feasibility, splash behavior, flora tables.

**Layer 4 (Feature placement):** deterministic poisson-disk or grid-jitter placement of landmarks (towers, lighthouses, wrecks, boulders, waterfalls, bridges). Biome-parameterized: a high-desert chunk places mesas and arches; a boreal chunk places lakes and cabins.

**Layer 5 (Decoration):** flora billboards / instances. Density per biome. LOD-aware — distant chunks skip Layer 5 entirely; they're too far to render flora detail.

---

## Hydrology sub-system

Hydrology deserves its own section because the design problem is harder than any other layer.

### The problem

Rivers flow **downhill** coherently across the whole world. Lakes collect water at basin minima. Drainage basins are topographically defined. These are inherently **global** properties — you can't generate a coherent river from local chunk information alone.

Meanwhile, our generation is **per-chunk** for streaming + parallelism reasons.

### Proposed approach: pre-bake a sparse hydrology graph at world-seed time

At world-seed time (once per save, before any chunks stream), run a one-shot **global hydrology bake**:

1. **Coarse world heightfield** at low resolution (e.g. 256×256 samples for a 1000 km × 1000 km world = 4 km per sample). Cheap enough to hold entirely in memory during the bake.
2. **Drainage-network computation** using standard topographic algorithms (D8 flow direction, accumulation threshold). Output: a sparse graph of river edges with `(start_point, end_point, waypoints, width, flow_rate)`.
3. **Lake detection**: local minima + overflow threshold → basin footprint + outflow edge. Output: list of lakes with `(center, footprint_polygon, surface_elevation, inflow_edges, outflow_edges)`.
4. **Coast detection**: sea-level contour in the coarse heightfield. Output: coastline polyline segments.
5. **Graph serialization**: the whole hydrology graph is ~small-MB (graphs are cheap). Stored in-memory for the session; re-derivable from master seed.

Per-chunk hydrology generation (Layer 2 above) becomes a **query** against this pre-baked graph: "what river edges / lake footprints / coastline segments pass through this chunk's bounding box?" Deterministic, local, fast.

### River boundary continuity

The user's explicit rule: a river entering a chunk at one edge must emerge coherently — either continuing to another edge, terminating at a lake / sea, or originating from a spring. It cannot bend back into the originating chunk unless there's a separate valid inlet.

Under the pre-baked-graph approach, this is **automatic**: the graph is computed globally, so river edges cross chunk boundaries consistently by construction. Chunks don't decide where rivers go — they just carve the heightfield along pre-decided waypoints.

### Rivers grow into lakes (and vice versa)

Lakes are nodes in the hydrology graph with `type=Lake`. River edges flow into lakes (inlet edges) and out of lakes (outlet edges). A lake with inlets but no outlets is a **closed basin** (e.g. Great Salt Lake analog). A lake with outlets but no inlets is a **spring-fed** lake. A lake with both is a **flow-through** lake.

Rivers can widen as they pick up tributaries (accumulate flow); the pre-bake's `flow_rate` parameter drives river width at each edge.

### Small aesthetic water features

Waterfalls, sandbars, creeks, inland ocean crossings, cliff coasts with big beaches — each is a **feature type** that emerges from the hydrology graph + shore classification:

- **Waterfall**: river edge where elevation drops > threshold over short distance (adjacent graph nodes with big ΔZ).
- **Sandbar**: delta zone at river-mouth chunks, shore-classified as `sand_beach` with extended shallow area.
- **Creek / stream**: small-flow river edge (below a width threshold).
- **Inland sea**: large lake (above a size threshold).
- **Big beach below cliff**: shore chunks with coast + steep inland heightfield gradient.

Each of these is a potentially-large authoring task when they land, but the hydrology graph + shore classification + biome tag give each feature enough context to render deterministically.

### Hydrology graph size budget

Target: 1000 km × 1000 km world, ~several thousand river edges, ~several hundred lakes, coastline polyline ~tens of thousands of segments. Graph total < 100 MB in memory. Bake time: one-time at world load, on the order of seconds. If bake becomes expensive, cache it on disk keyed by master seed.

---

## Biome system

### Biomes in the world

Enumerated from [FEATURES.md § Regional Biomes](FEATURES.md#regional-biomes) with the user's explicit real-world references:

- **High Desert / Plateau** — Grand Mesa / Flatirons / Arches flavor. Flat-top mesas, tilted sedimentary slabs, red-rock arches, scrubby plains, rattlesnake canyons, rivers cutting through rock, rolling dunes. Railroads as industrial signature. Dry lake landings, dust devils, heat shimmer. Dark skies for astrophotography.
- **Coastal Alaska / Canadian boreal** — mountainous + plains + snowy/tundra. Stunning alpine lakes, cliff regions, remote recreational landing options. The **baseline** — most of flowPlan evokes this.
- **Mediterranean archipelago / coastal** — turquoise water, white/terracotta ports, dense island clusters, inland seas. Blends naturally with coastal Alaska.
- **Pacific Northwest** — dense fog, low cloud, heavy rain, fjords, logging camps, lumber mill docks, salmon-smoking canneries.
- **Rocky / Alpine** — density-altitude, short lake windows, narrow valleys, fire-hopping, water-bomber drops, glacier-dammed lakes.
- **Subarctic / North Atlantic** — basalt coasts, stormy sea, heavy fog, reindeer/puffin, geothermal steam plumes, midnight sun in summer, aurora in winter.
- **Tropical island chain** — reefs, squalls, trade winds, coconut-palm atolls, protected lagoons.

### Biome adjacency rule

Biome transitions must be **climate-gradient coherent**. Two biomes can be adjacent iff there's a plausible climate gradient between them.

**Forbidden transitions (structurally rejected by the biome bake):**
- snow / tundra ↔ dry desert
- snow / tundra ↔ tropical archipelago
- high desert ↔ temperate archipelago (without a buffer biome)
- any "Category A ↔ Category C" without a Category B buffer between them

**Acceptable transitions (encouraged):**
- alpine → tundra → boreal forest → temperate forest → plains
- coastal boreal → coastal temperate → Mediterranean coastal
- Rocky alpine → high-desert plateau (geographically real in many mountain-basin interfaces)

**Implementation sketch:** a biome-adjacency graph encodes legal transitions as a weighted adjacency matrix. Biome placement (world-bake layer) samples from the graph with adjacency constraints — Wave-Function-Collapse-style or blue-noise sampling with rejection for forbidden pairs.

### Biome spatial layout

Options:
- **Voronoi partition** of the world by biome centers placed at seed time (blue-noise distribution). Hard edges between biomes.
- **Climate variable fields** (temperature + humidity + elevation) → biome classification per chunk center with soft blend zones.
- **Hand-authored biome mask** for Candidate A (single large map); procedural for B / C / D.

**Proposal:** climate-variable fields with biome-adjacency rejection. This gives soft blend zones naturally (biome transitions happen where climate variables cross classifier boundaries) and respects the adjacency rule by construction (you can't have snow-tundra and desert in the same cell because the climate fields don't place them in the same place).

---

## Landscape features

Beyond biomes + hydrology, the world needs **landmarks and character**. Feature placement is Layer 4 of the per-chunk pipeline.

### Placeable feature categories

- **Landmarks** (navigational) — lighthouses, radio towers, mountain peaks with names, historic wrecks, bridges, rail trestles.
- **Terrain features** — arches, mesas, individual mountain peaks, unique rock formations, waterfalls, caves (limited scope, for low-pass exploration).
- **Hubs** — airfields, seaplane bases, mixed, remote resupply, isolated (see [Hub persistence](#hub-persistence)).
- **Wildlife zones** — rookeries, salmon-run rivers, whale corridors, migration paths (deterministic from seed; Phase 4 wildlife system drives actual spawning within these zones).
- **Mission anchors** — beacon sites, SAR targets, cartography waypoints, fire-fighter forward points (deterministic placement; Phase 4 mission system draws from this pool).
- **Industrial / settlement detail** — logging camps, cannery docks, lumber-mill structures, fishing camps, trapper cabins, remote ranger stations, research outposts.
- **Natural detail** — flora patches, rocks, boulders, driftwood, snags, dead trees.

Each category has a **density** and **placement rule** per biome. High-desert places mesas and arches densely, lakes rarely; boreal places lakes and cabins densely, arches never.

### Placement algorithm

Deterministic poisson-disk sampling per chunk, with:
- Per-biome density parameter (mesas per km² in high desert, lakes per km² in boreal, etc.).
- Per-feature-type minimum spacing (mesas can't overlap; rookeries need shoreline proximity).
- Per-biome feature whitelist (no palm trees in tundra, no spruce in desert).

**Seed flow:** chunk_features_seed seeds a PCG RNG → poisson-disk sampler consumes the RNG → outputs placed-feature list. Reproducible, deterministic, local.

---

## Hub persistence

Hubs are narrative anchors. Their locations and services need to be stable across sessions, not regenerated fresh each session (Candidate C would lose this; D / B preserve it).

**The hub tier taxonomy is authoritative in [FEATURES.md § Hubs & Landing Areas](FEATURES.md#hubs--landing-areas)** — this section covers placement + generation concerns only. Five tiers: `major | regional | local | remote | isolated`. Each tier has distinct layout conventions (runway count, hangar types, aircraft parking RNG), mission count (7–15 / 5–10 / 3–7 / 1 / rare-barn-find), and social affordances. Runway numbering follows real magnetic-heading convention (degree/360); parallel runways use L/R/C suffixes. See FEATURES.md for the full per-tier specification.

### Hub placement strategies

**Option 1: Seed-locked procedural.** Hubs are feature-placement Layer 4 results, but with higher weight and spacing guarantees. A given (master_seed, world_scope) always places Hub X at coordinates (i, j, x, y). Works for Candidate D if procedural + seed is enough narrative anchor.

**Option 2: Authored hub chunks.** Certain chunks (at authored coordinates) are *not* procedurally generated — they load from a hand-authored data file with bespoke heightfield, hub services, dock placement, taxi routes. Procedural neighbors stitch to the authored chunk's boundary heightfield. Candidate D's best fit.

**Option 3: Authored layout with procedural detail.** Hub locations and their boundary conditions are authored (coordinates, boundary heightfield, dock orientation); the chunk interior is procedurally generated with the authored boundaries as constraints. Middle ground.

**Proposal (per-tier):**
- **Tier 1 (major):** Option 2 — **authored hub chunk** (hand-crafted heightfield for the 2–3 runways + hangars + tower + dock layout). Highest player visibility; this is where the "home airfield" feel lives. Worth the authoring cost for the ~handful of tier-1 hubs a world has.
- **Tier 2 (regional):** Option 3 — **authored boundary + procedural interior**. The chunk declares "runway from (x1,z1) to (x2,z2), heading 270°; dock at (x3,z3); quonset hangar at (x4,z4)"; the chunk's heightfield procedurally fills in around these anchors with biome-coherent terrain.
- **Tier 3 (local):** Option 3 at a lighter touch — declare the dirt strip + road spine + a hub-structure blob; everything else procedural.
- **Tier 4 (remote):** Option 1 — **pure procedural** with a placed-feature anchor. The hydrology graph + biome + feature-placement layer produces the clearing / beach / lakeshore that constitutes the hub; structure (cabin / dock / bait shop) snaps to the anchor.
- **Tier 5 (isolated):** Option 1 — pure procedural. No placed anchor at authoring time; the hub is *discovered* by the hydrology + terrain generator producing a landable surface in an unlikely place, with a small random chance of a barn-find artifact attached.

This gives hand-crafted weight where it counts (tier 1 / 2) without requiring every minor airstrip be hand-authored, and lets the procedural generator surface tier-4 / tier-5 hubs organically as rewards for exploration.

### Seam blending

Authored ↔ procedural boundary is where edge-matching matters most. Techniques:

- **Skirted boundary heightfield** — authored chunk extends height samples 1–2 cells past its own boundary into the neighbor. Neighbor's first cells blend to match.
- **Feature masking** — authored hub chunks declare "no flora within 500 m of the dock" so neighbor procedural flora respects the boundary.
- **Biome continuity** — authored hub declares its biome; neighbors inherit the biome adjacency rule (cannot place a hostile-biome chunk immediately next to an authored coastal hub).

---

## Flight-log memory

The save file holds the **event log** that generated the current state, per [FEATURES.md § Flight Log & Save Format](FEATURES.md#flight-log--save-format). For MAPGEN, this means:

- **Raw terrain is never persisted.** It regenerates from master seed + chunk coords.
- **Player-caused deltas are persisted.** These are specific, small events:
  - Hub discovery (revealed a previously-unknown hub at coordinates).
  - Cartography runs (revealed region to map-view).
  - Landing events (logged touchdown at a specific location — feeds logbook).
  - Barn-find discoveries (recorded derelict aircraft at a location).
  - Mission-anchor interactions (accepted a mission, completed a mission, declined a mission).
  - Custom annotations (player pinned a waypoint).
  - Reversible player edits to hubs (livery painted in a paint shop, stored aircraft in a hangar).
- **Delta size budget:** an entry in the event log is ~100 bytes on average (timestamped + category + compact fields). A 10-hour playthrough with 1000 events = 100 KB. Comfortably inside the save file budget of < 5 MB per [ROADMAP Milestone 4.10](ROADMAP.md#milestone-410-saveload).

**Load-time:** read master seed, re-bake hydrology graph (seconds), replay event log in order (instant for deltas), then regular chunk streaming resumes.

---

## Determinism & save-format implications

- **State hash extension.** The per-tick `state_hash` currently covers `(tick, seed, vehicle, camera, time_of_day, wave_time, chop_intensity)` per [crates/sim/src/hash.rs]. When world generation gets state that changes during play (e.g. player-built sandbar markers, hub discovery flags), those must join the hash.
- **Save-format versioning.** Save format carries `(game_version, save_schema_version, master_seed, world_scope, dial_state_history, event_log)`. Migrations are documented per [FEATURES.md § Flight Log & Save Format](FEATURES.md#flight-log--save-format).
- **Cross-platform determinism.** Hashing function choice + IEEE float discipline per [ROADMAP Milestone 6.1](ROADMAP.md#milestone-61-determinism-hardening-cross-platform) extends here. The world-bake hydrology graph must be bit-identical across Linux / Windows / macOS / Steam Deck.
- **Scenario bench compatibility.** Flight-bench scenarios pin a master seed explicitly; they do not depend on hub placement or biome layout unless the scenario explicitly positions the aircraft at a specific hub. Most scenarios use flat terrain (`Terrain::flat()` / `Terrain::flat_land()`) and are unaffected.

---

## Open questions

Grouped by urgency to Phase 4 lock-before milestone.

### Before world-scope decision locks (pre-Phase-4)

1. **Chunk shape**: square / hexagonal / world-centered Voronoi / other? (See [Chunk model](#chunk-model).)
2. **Chunk size**: 1 km² vs 4 km² vs adaptive. Prototype with target frame budget.
3. **Heightfield representation**: ridged multifractal + domain warp sufficient, or does the terrain rebuild want something more exotic (erosion simulation, gradient-conditioned noise, machine-learned biome exemplars)?
4. **Hydrology graph size**: can the world-bake produce a < 100 MB graph for a 1000 × 1000 km world, or do we need a sparser representation / on-disk cache?
5. **Circular chunks with player-center indexing**: the user asked. Answer is probably "no for persistence reasons, yes for rendering LOD rings"; document clearly.
6. **World scope candidates**: prototype Candidate C and D in Phase 1 scratch branch per FEATURES.md; apply this doc's architecture to both.

### Before terrain rebuild lands (during Phase 3.6)

7. **Biome spatial generator**: Voronoi vs climate-field vs hand-authored mask. Proposed climate-field with adjacency rejection; verify computationally.
8. **Hub placement strategy**: authored chunks vs procedural. Proposed: Option 2 for major, Option 3 for regional, Option 1 for remote. Validate at first hub-generation pass.
9. **Seam blending algorithm**: which edge-matching technique handles authored↔procedural cleanly at the boundaries the playtest will actually traverse?
10. **Hydrology bake time budget**: seconds is fine at load; but can it become real-time at world-scope lock if a player changes dial-state mid-game? (Probably not; dial changes don't invalidate the hydrology graph.)

### Before save format locks (pre-Phase-4.10)

11. **Delta schema**: per-category event formats finalized.
12. **Delta compaction**: append-only during play, compact at save time; what's the compaction algorithm?
13. **Cross-version save migration**: how much forward/backward compatibility is promised?
14. **Shared-seed hash compatibility**: if two players share a master seed for async ghosts, the hydrology-graph bake must produce byte-identical output on their machines.

### Deferrals (post-Phase-4)

15. Erosion simulation (multi-pass water-carving at world-bake time) — cool if cheap, nice-to-have.
16. Seasonal biome overlays (winter tundra / summer tundra differ visually).
17. Player-driven terrain changes (building strips, carving sandbars in loose sand) — very long-tail, post-v1.
18. Multi-player world seed negotiation + shared evolution — [Phase 6 stretch](ROADMAP.md#phase-6-stretch-goal-post-v1-multiplayer).

---

## Relation to other specs

- **DESIGN.md:** the world-bake hydrology graph is a new sub-system; owner is `world::hydrology`. Confirm boundaries before Phase 3.6.
- **FEATURES.md:** this doc is the design detail for § World Scope & Map Layout, § Regional Biomes, § Terrain & Static Landscape, § Map Memory / Cartography. Those sections should cross-link here.
- **ROADMAP.md:** drives proposed Phase 3.6 (terrain rebuild + hydrology) and pre-Phase-4 world-scope lock milestone.
- **PROGRESS.md:** session-decisions log should reference this doc's creation as the anchor for later terrain-rebuild work.
- **TEST_PLAN.md:** new world-gen determinism test class: same master seed + same world-scope + same platform → bit-identical hydrology graph, bit-identical chunk outputs at any queried coordinates.
