# flowPlan Map Generation (design draft)

> **Role in the docs stack:** MAPGEN.md is the design doc for flowPlan's procedural world generation and terrain representation. It sits between [PLAN.md](PLAN.md) (vision), [DESIGN.md](DESIGN.md) (architecture + invariants), [FEATURES.md](FEATURES.md) (feature backlog), and [ROADMAP.md](ROADMAP.md) (phased execution). This document is **design, not implementation**: it records decisions, proposed strategies, and explicitly-open questions so Phase 3+ terrain / hydrology / world-scope work can start from a concrete specification rather than rediscovering tradeoffs.
>
> **Implementation companion**: [`docs/WORLD_GEN.md`](docs/WORLD_GEN.md) covers the *how* — actual algorithms, function signatures, octave-offset convention, performance characteristics, and Marching Squares contour rendering. Read MAPGEN for design rationale, WORLD_GEN when working in the code.
>
> **Status:** Phase 3.6.0 + 3.6.1a + 3.6.1b implementation shipped (terrain Perlin-fBm, biomes with soft blending, per-biome ridged/domain-warped primitives, weather Perlin-fBm migration, variable Voronoi cell size). 3.6.1c–3.6.4 still ahead. Updates land by amendment PR.

## Contents

- [Core principles](#core-principles)
- [World scope decision](#world-scope-decision)
- [World topology](#world-topology)
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

**LOCKED (2026-04-21):** Option A — **one continuous procedural world per seed, toroidal wrap on all edges.** Closes the prior Candidates A–D open question. Resolved jointly with the weather-W2 close-out once the pure-function architecture proved it naturally supports Option A; the remaining candidates (B, C, D) would have required retrofitting.

**What Option A means in practice:**

- **One seamless procedural world.** No regional map-transitions, no loading screens between regions, no "4 separate worlds per seed" abstraction.
- **Wraps in both axes.** Flying off the east edge returns you at the west edge with the same orientation; same for north/south. The world is topologically a torus.
- **Size is configurable.** Strawman: 5000 km per side (`WORLD_EXTENT_M = 5_000_000.0`). Traversal time at ferry speed (~50 m/s) is ~28 hours; at bush cruise, ~46 hours. Large enough that organic play never hits the edge unless the player deliberately traverses.
- **Procedural per master_seed, persistence per event-log.** Terrain, weather, hydrology, biomes, hub-catalog — all regenerate from `master_seed` deterministically. Player-caused deltas (built waypoints, discovered hubs, storm events witnessed) persist in the world event log.
- **Hub catalog is pre-baked at world-bake time** (not streaming-dependent): ~50–80K hubs in a 5000 km world. Hubs have stable IDs + approximate coords at bake time; the hub late-binding (fuzz + resolve) pattern snaps hubs to nearest-compatible Voronoi cell when streaming catches up.
- **Puddle-jumper emphasis.** Inland seas (great-lake-scale, few hundred km across) + fjord/lake networks + rivers provide water-flying gameplay. Large continuous oceans aren't the focus; circumnavigable-by-land paths exist if you pick the right heading.

**What Option A *doesn't* mean:**

- Not "infinite streaming into random noise" — the world has a finite size and wraps. Players can complete discovery.
- Not "single authored map" — content is procedural from seed, not hand-placed per asset.
- Not "spherical geometry" — the world is a flat torus. Compass rules and day/night cycle are defined under [World topology](#world-topology) below, not derived from latitude/longitude on a sphere.

The pre-Phase-3 world-scope prototype remains valuable, now re-scoped to **validate toroidal wrap mechanics** (terrain + weather + Voronoi + hydrology all wrap cleanly at the seam without artifacts) rather than pick between candidates.

---

## World topology

The world is a **flat square torus** of side `WORLD_EXTENT_M` (strawman 5000 km). All world-gen samplers operate on world coordinates `(x, z)` that conceptually wrap: `sample(x, z) ≡ sample(x + WORLD_EXTENT_M, z) ≡ sample(x, z + WORLD_EXTENT_M)`.

### Sampler-wrap requirements

For samplers to produce continuous output at the seam, every noise/harmonic wavelength must **divide `WORLD_EXTENT_M` evenly.** At 5000 km:

| Field | Current wavelength | Cycles per world | Action |
|---|---|---|---|
| Temperature climate | 500 km | 10 ✓ | Keep |
| Temperature synoptic | 350 km | 14.29 ✗ | Adjust to 357.14 km (14 cycles) or 333.33 km (15) |
| Humidity regional | 200 km | 25 ✓ | Keep |
| Humidity synoptic | 350 km | 14.29 ✗ | Same adjustment as temperature synoptic |
| Pressure synoptic | 350 km | 14.29 ✗ | Same |
| Terrain octaves | various (~21 m to ~1047 m) | non-integer | Quantize frequencies to `1/WORLD_EXTENT_M × k` for some integer `k`; quantization error < 0.1% at finest octaves |
| Water harmonics | (Phase 1 placeholder) | — | Quantize when 3.5b FFT ocean replaces them |

These adjustments are drop-in and do not change the character of the fields — wavelengths shift by < 2% to land on integer cycle counts.

### Voronoi-wrap mechanics

Voronoi cell lookup wraps via mod-N indexing: `cell_id(ix, iz) ≡ cell_id((ix + N) % N, (iz + N) % N)` where `N = WORLD_EXTENT_M / GRID_SPACING_M` is integer.

At 5000 km + 25 km grid spacing → N = 200 exactly. Weather-Voronoi (3.2.2) runs 30 km spacing → choose between 20 km / 25 km (fits N=250 / 200) or adjust `WORLD_EXTENT_M` to match 30 km. Terrain-Voronoi (3.6.2) at ~2 km spacing → N = 2500.

Neighbor search at a query point near the seam: when resolving the 3×3 grid neighborhood, wrap grid indices through mod-N. Seed positions for wrapped cells are computed relative to the query point (subtract `WORLD_EXTENT_M` as appropriate so the distance calculation doesn't span the whole world). This is a 5-line modification to the current `cell_at` implementation.

### Hydrology-wrap mechanics

River continuity must respect wrap: a river flowing east eventually returns from the west. Hydrology graph nodes are keyed by mod-N cell identity; downslope traversal wraps at the seam. When the hydrology bake produces a catchment basin, the basin may span the seam (visually appears as two disjoint regions on a flat-map projection; is actually one on the torus).

### Compass orientation

**Compass is locked to world-north**, independent of wrap:

- Fly due north indefinitely: compass continues to read "N"; the world just wraps under you.
- Wrapping an edge does **not** flip compass orientation. Exiting the top edge and re-entering at the bottom preserves the heading.
- No true pole exists (torus has none); no "compass goes crazy at the pole" edge case.

Tradeoff accepted: no spherical great-circle effects (shortest path between two points is always the straight line, possibly wrapping). For puddle-jumper local-navigation gameplay — relative bearings, heading holds, intercept courses — this is a clean win. The realism payoff of great-circle navigation doesn't outweigh the edge-case complexity at flowPlan's scale.

### Day/night cycle

**Accelerated east-to-west.** Sun progresses across the sky faster than real-time wall-clock:

- Placeholder acceleration: **24:1** (1 sim-hour = ~2.5 real-minutes → full day = ~1 real-hour). Tunable at playtest; the right number falls out of "how many in-game days per typical play session feels right."
- **Time-of-day varies with longitude.** Local time at world-x coordinate `x` is `sim_time_of_day - (x / WORLD_EXTENT_M) × DAY_LENGTH_S`. Different longitudes see different local time simultaneously — you can fly east to "chase the sunset" or west to "race the sunrise."
- **Wrap:** crossing the east edge keeps the time-of-day progression continuous — it's just another 5000 km of longitude.
- **No polar extremes.** A flat torus has no analog for midnight sun or polar night. High-latitude-style aesthetics (aurora, extended twilight) are per-biome visual overlays, not derived from spherical math.

### Inland seas

Water-flying gameplay is served by **inland seas + lake networks + rivers**, not oceanic geography:

- **Great-lake-scale inland seas** (few hundred km across) in major biome regions: delivers long water landings, sea-state variability, coastal-to-offshore flight variety.
- **Fjord / lake networks** in coastal sub-biomes: narrow-water landings, shore-following navigation.
- **River systems** everywhere hydrology permits: hub placement, float-plane downwind landings, white-water spray.

Circumnavigable-by-land paths exist if a pilot picks the right heading. This matches the bush-flying ethos — the world isn't structured around trans-oceanic crossings.

### World-bake parameters

The world is parameterized by:

- `master_seed: u64` — entry seed for all derived sub-seeds.
- `WeatherTuning` — per-type weather cell cycle durations (see [FEATURES.md § Weather State](FEATURES.md#weather-state)).
- `WORLD_EXTENT_M` — strawman 5000 km; locked at world creation, stored with the seed.
- Future additions: biome palette choices, hydrology density factor, hub density factor — all player-facing at seed creation.

Together these form the "seed bundle" that uniquely identifies a world. Two worlds are byte-identical if and only if all bundle parameters match.

---

## Chunk model

### The pinned direction: streaming shape ≠ chunk shape

The user has pinned a **decoupled** chunk architecture:

- **Streaming shape** = **hex or circular disk** around the aircraft. This is the technical bound: "what cells do I need in memory right now?" Uniform disk → good cache locality, simple priority queue.
- **Chunk shape** = **Voronoi cells**. Each cell is a semantic unit of world content, not a lattice tile. Cells are variable-size, variable-shape, organic — they follow natural feature boundaries rather than a grid.
- **Voronoi seed points** = **sub-biome feature placements**. At world-bake time, each Voronoi seed is RNG-assigned a sub-biome feature type from its parent biome's library; the Voronoi partition around the seeds defines the world's chunks; each chunk takes on its seed's feature character.

This reframes the problem. Chunks stop being arbitrary lattice tiles and become **mosaic pieces** of a natural-looking world where one lake cluster is one chunk, one alpine valley is one chunk, one red-rock canyon is one chunk. Voronoi boundaries align with where a cartographer would draw them.

### Streaming disk (the technical layer)

Aircraft ground speed: cruise ≈ 55 m/s ≈ 200 km/h. VNE 71.5 m/s is a short-duration dive speed, not sustained. At common delivery distances (50–200 km), a flight is 15–60 minutes. Streaming needs to keep the aircraft in loaded cells continuously without frame drops.

**Visibility baseline (Phase 2 playtest observation):** cel-shaded + height-fog rendering produces plausible visibility out to ~6–8 km before fog fully attenuates distant geometry. This is the effective "you can see it" horizon. Streaming disk should outrun it.

**Target streaming radii:**

| Band | Radius | Detail | Rationale |
|------|--------|--------|-----------|
| Inner (full detail) | 0–5 km | Full-LOD heightfield, flora instances, feature detail | Where the aircraft is actively flying through + visible for sure |
| Mid (coarse detail) | 5–8 km | Mid-LOD heightfield, billboard flora, simplified hydrology | Visibility zone — fog attenuation starts blending distant detail |
| Outer (pre-load zone) | 8–10 km | Generated but coarse-LOD only; upgrades as aircraft approaches | Pre-loaded to be ready when the aircraft arrives; ~3 min ahead at cruise |
| Unload hysteresis | 10–13 km | Retained briefly to avoid thrashing on heading reversals | Aircraft can 180° without reloading |
| Beyond 13 km | — | Evicted | Outside the hysteresis band |

At 55 m/s cruise the 8 km outer boundary is ~2.4 min ahead; 10 km is ~3 min. Comfortable for background generation. VNE 71 m/s dive shortens to ~2.3 min at the 10 km boundary — still sufficient for short-duration dive behavior.

**Tier-adjusted defaults + user-facing slider:** the radii above are the **Medium-preset** target matching flowPlan's Phase 2 observed visibility. Higher hardware tiers get more, exposed as a graphics-quality setting so users with headroom can open up the world:

| Preset | Inner full-LOD | Mid coarse | Outer pre-load | Hysteresis | Notes |
|--------|----------------|------------|----------------|------------|-------|
| Low / Steam Deck | 3 km | 5 km | 7 km | 9 km | Conservative; denser fog hides the edge |
| Medium | 5 km | 8 km | 10 km | 13 km | Matches Phase 2 baseline observation |
| High | 7 km | 12 km | 15 km | 19 km | Reference-tier default |
| Ultra | 10 km | 18 km | 22 km | 28 km | High-tier hardware |
| Custom | user slider (3–30 km on outer) | linked bands scale proportionally | | | Exposed in graphics panel |

**User-facing slider:** "Streaming distance" (3 km → 30 km) lives under [FEATURES.md § Display & Graphics Settings](FEATURES.md#display--graphics-settings) Per-subsystem quality. Couples to fog density (longer streaming wants lighter fog to make the extra range visible) — the two sliders can link with a toggle or move independently for power users. Determinism-neutral: streaming distance only affects what's loaded / visible, not what's generated (generation is deterministic per master seed).

**Memory scaling with radius** (at 2 km seed spacing, full-LOD cells):

| Outer radius | Active cells | Memory (full-LOD equivalent) |
|--------------|--------------|------------------------------|
| 7 km (Low outer) | ~51 | ~13 MB |
| 10 km (Medium outer) | ~105 | ~26 MB |
| 15 km (High outer) | ~236 | ~59 MB |
| 22 km (Ultra outer) | ~507 | ~127 MB |
| 30 km (Custom max) | ~942 | ~236 MB |

All comfortably under DESIGN.md's 1 GB RAM ceiling even at 30 km custom max. GPU cost scales less sharply — distant cells render at coarse-LOD, flora drops to billboards beyond mid-range, so a 22 km Ultra view costs ~2–3× a Medium 10 km view in raster time, not ~5× the streaming-count.

- **Streaming shape:** circular disk is the natural representation. Hex-tiled streaming grid is an equivalent alternative where streaming query walks outward from player's hex. Pick during prototype.
- **Spatial indexing:** coarse grid hash (e.g. 2–4 km cells) maps world coordinates → set of Voronoi cells intersecting that coarse cell. Streaming query = "enumerate Voronoi cells in the coarse cells intersecting a disk of radius R." O(log N) or better with a well-chosen grid size.
- **Preallocated pool** sized to max streaming disk footprint. No heap allocation during flight.
- **Priority queue** of chunks to generate, sorted by distance from aircraft.
- **Background generation thread** consumes the queue; publishes completed chunks to the main-thread handle table.
- **LRU + distance eviction** returns out-of-disk chunks to the pool.
- **Visible-LOD fallback:** chunk-generation budget capped per tick; if a chunk isn't ready, the renderer draws a lower-LOD tier until it is. Honest visible LOD over frame drops.

### Memory + count budget

At 2 km average Voronoi seed spacing (blue-noise packing ~3 km² per cell):

| Disk radius | Area | Active cells | Full-LOD memory @ 4 m heightfield (~250 KB/cell) |
|-------------|------|--------------|--------------------------------------------------|
| 5 km (inner) | ~79 km² | ~26 cells | ~6.5 MB |
| 8 km (mid) | ~201 km² | ~67 cells | ~17 MB (mostly mid-LOD, smaller per cell) |
| 10 km (outer) | ~314 km² | ~105 cells | ~26 MB (mostly outer-LOD, smaller still) |
| 13 km (hysteresis) | ~531 km² | ~177 cells | ~45 MB (outer-most retain coarse only) |

Comfortably within DESIGN.md's 1 GB RAM ceiling. Flora instances + hydrology references + feature placements roughly double the per-cell footprint at full-LOD — still well inside budget.

### Voronoi partition (the semantic layer)

- **Seed distribution:** blue-noise (Poisson-disk) sampling over world-space at world-bake time. Seed density varies by region based on target feature size. Seeded by `voronoi_seed = hash(master_seed, "voronoi")`.
- **Target seed spacing:** **~2 km average center-to-center** as the default anchor. Blue-noise packing → **~3 km² average cell area**. Sub-biome feature types can scale around this: a tight alpine tarn might request 0.5 km² (1 km spacing); a broad floodplain or open plain might request 6 km² (3 km spacing); a major-hub chunk might be authored at 2–4 km². Seed density is feature-type-declared, not globally uniform.
- **Voronoi triangulation:** Fortune's sweep or equivalent at world-bake time. O(n log n) in seed count. Computed once, cached by master-seed hash. For a 1000 × 1000 km world at ~3 km² average cell area, ~333,000 seeds → ~30 MB graph (edges + neighbor adjacency). Bake time on the order of seconds at load.
- **Cell indexing:** each cell gets a stable `CellId` (e.g. cell-index within the Voronoi tessellation). The spatial grid hash provides the coarse world-coord → CellId lookup.
- **Cell data:** each cell stores `{seed_point, polygon_edges, neighbor_cell_ids, parent_biome, sub_biome_feature_type, heightfield, hydrology_references, feature_placements}`.
- **Heightfield resolution per cell:** hybrid — the cell's authored feature-type declares a base resolution + maximum resolution. A ridgeline cell might want 2 m per sample; a broad flat cell 8 m. Near-aircraft refinement still applies per-cell.
- **Boundary stitching:** Voronoi neighbor relationships are known at bake time. Heightfield continuity at edges is enforced by sharing heightfield samples at Voronoi edge vertices between neighboring cells (each edge vertex has one canonical height shared by all adjacent cells). Same discipline for biome blending, flora density, palette.

### Variable-LOD per cell

Voronoi cells don't fit uniform LOD tiers neatly — cells are variable-size. Instead, LOD is driven by **distance from the aircraft** per-cell:

- **Near** (within 2 km of aircraft): full heightfield resolution, full flora, full detail.
- **Mid** (2–8 km): half-resolution heightfield, billboards replace instanced flora, simplified hydrology features.
- **Far** (8–15 km): coarse heightfield silhouette, color/palette only, skip most feature detail.
- **Beyond streaming disk:** not rendered.

LOD transitions use cell-centroid distance as a first approximation; a cell that straddles the aircraft's edge can render at two LODs on opposite ends without visible seams if the heightfield samples along its interior are interpolated.

### Why Voronoi (the architectural payoff)

- **Natural boundaries.** Cell edges align with feature transitions, not arbitrary lattice lines. No staircase artifacts on diagonal rivers / roads / biome transitions.
- **Sub-biome features are first-class.** A cell *is* a sub-biome feature. No separate "feature placement layer" trying to sprinkle features onto a monolithic chunk substrate.
- **Palette adaptation is natural.** Cell-to-cell palette shifts are per-cell, not per-lattice-edge — transitions follow feature boundaries, which is what a player's eye expects.
- **Hydrology stays global.** The pre-baked hydrology graph doesn't care about chunk shape; Voronoi cells query it exactly like square chunks would.
- **Hub placement fits.** Tier-1 major hub = an authored Voronoi cell. Tier-2 regional = authored-feature-boundary Voronoi cell. Tier-4/5 remote/isolated = procedural cells where the feature-type happens to be a "hub-hosting clearing". No separate hub-chunk concept.

### Acknowledged complexity (name it, don't avoid it)

- **Voronoi triangulation at bake time** — Fortune's sweep is O(n log n); bake time scales with world seed count. Likely acceptable at session load (seconds); if it balloons, cache bake output to disk keyed by master seed.
- **Spatial grid hash for streaming** — coarse grid cell size is a tuning parameter: too coarse wastes cycles enumerating candidate Voronoi cells that don't actually intersect the streaming disk; too fine wastes memory.
- **Variable-size cell LOD** — each cell needs distance-driven LOD logic; no uniform ring structure.
- **Cell-boundary heightfield stitching** — shared Voronoi-edge vertices need canonical heights; requires careful bake-time data structure.
- **Feature-library authoring overhead** — each sub-biome feature type needs its authoring work (geometry rules, palette, flora, fauna, hub-compatibility, inter-biome adaptation rules). The user has explicitly welcomed this authoring work as the **design effort** rather than a cost — library-authoring is the design surface, not a burden.

### Decision status

**PINNED as the primary architectural hypothesis to validate in the Pre-Phase-3 world-scope prototype.** Prototype should test: Voronoi generation + streaming-disk query + sub-biome feature selection + palette adaptation. If the prototype surfaces a performance wall or an aesthetic problem, this bends — but this is the direction to try first.

**Fallback:** if Voronoi turns out to be infeasible at performance targets, fall back to hex chunks (2nd choice) or square chunks (3rd choice). The sub-biome-feature concept still applies at lattice scale, just with less natural boundaries.

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

## Layered generation — three-layer composition (pinned)

World generation decomposes into **three independent, composable layers.** This is not "5 layers in a pipeline" — it's **three concerns that compose orthogonally**, each of which can ship on its own and be tested independently. The composition is **additive at the heightfield level**: `final_height(x, y) = base_topography(x, y) + Σ feature_deltas(x, y)`.

```
Layer 1 — TOPOGRAPHIC BASE (global, one-time bake per master seed)
  ├─ coarse global heightfield (biome-parameterized ridged multifractal + domain warp)
  ├─ biome climate fields (temp / humidity / elevation) → classification w/ adjacency rules
  ├─ hydrology graph (D8 drainage, rivers, lake footprints, coastlines)
  ├─ magnetic variation field
  └─ hub catalog (fuzz+resolve — see § Hub late-binding)

                                                          │
        Layer 1 shippable on its own → playable world     │
        with real biome-coherent terrain + hydrology      │
                                                          │
                                                          ▼
Layer 2 — VORONOI PARTITION (global, one-time bake; consults Layer 1)
  ├─ blue-noise seed distribution (~2 km average spacing)
  ├─ sub-biome feature type assigned per seed (biome-library RNG selection)
  ├─ placement constraints evaluated against L1 coarse topography:
  │    slope tolerance (tighter for runways, looser for remote strips)
  │    roughness tolerance (tight for lakes, loose for dune fields)
  │    water-adjacency required? (yes for docks + bush strips)
  │    altitude range (glacier vs floodplain)
  ├─ palette-adaptation curves (sub-biome feature in host biome → adapted palette)
  └─ neighbor adjacency graph

                                                          │
        Layer 2 ships semantics only — no heightfield     │
        impact yet. Validates partition + feature         │
        placement logic without risking terrain.          │
                                                          │
                                                          ▼
Layer 3 — FEATURE-DELTA HEIGHTFIELD MODIFICATIONS (streaming-time, per cell)
  ├─ per-feature-type delta function: lake carves basin, runway flattens
  │   strip with short skirt, mesa raises plateau with cliff edges,
  │   arch adds rock geometry, dune adds noise undulation, etc.
  ├─ delta falls off smoothly to zero at cell boundaries (bump-function)
  ├─ additive composition: final_height = base + Σ deltas
  ├─ flora placement (billboards / instances), decoration (rocks, snags)
  └─ shore classification sampler (slope × biome × water-adjacency)
```

### Layer 1 — Topographic base

Purely answers: *"What does the land look like before any features?"* Shippable as a minimum-viable procedural world.

- **Heightfield generator:** biome-parameterized ridged multifractal + domain warping. Per-biome amplitude, frequency, and gradient profiles — enables independent amplitude/gradient tuning (closes the Phase 2 sum-of-sines limitation). Coarse resolution (e.g. 100 m per sample) for the global bake; fine resolution (~4–8 m) generated per-cell at streaming time.
- **Biome classification:** climate-field driven (temperature + humidity + elevation + latitude proxy) → biome tag per world coordinate. Adjacency rule enforced at bake time (no snow→desert, no snow→tropical).
- **Hydrology graph:** D8 drainage-network computation on the coarse heightfield → sparse graph of river edges + lake footprints + coastlines. See § Hydrology sub-system. Bit-identical across platforms.
- **Magnetic variation field:** biome-dependent at world-bake time, held deterministic. Powers runway-number authoring correctness (see [FEATURES.md § Runway authoring rules](FEATURES.md#runway-authoring-rules-reminder)).
- **Hub catalog:** the ~50–80K-entry hub catalog per [§ Hub late-binding (fuzz + resolve)](#hub-late-binding-fuzz--resolve) is baked alongside. It references Layer 1 topography for placement viability (a major hub won't be committed to a vertical cliffside).
- **Altitude envelope enforcement:** see [§ Altitude envelope + real-world-possible gate](#altitude-envelope--real-world-possible-gate) for bounds + soft-clamp mechanism.

### Altitude envelope + real-world-possible gate

**Design gate (pinned):** *"is this possible in the real world? if not, we may have allowed too much."* Applied to terrain altitude first; generalizes to all generative systems (weather, wildlife, hubs, missions). See user memory: `real_world_possible_gate.md`.

Unconstrained ridged multifractal drifts. Some regions trend arbitrarily high, others arbitrarily low. At 1000 × 1000 km world scale, a random region could produce Everest-level peaks or unrealistic trench-scale depressions. The fix is a three-part altitude enforcement at Layer 1:

**1. World altitude bounds (hard limits, never exceeded):**

| Bound | Value | Real-world analogue |
|-------|-------|---------------------|
| World max | ~4,500 m | Mt Rainier 4,392 m; Mauna Kea 4,205 m; Mt Elbert 4,401 m |
| World min | ~-100 m | Death Valley -86 m (rare; highly localized) |

No Himalayan biomes in flowPlan's catalog — 8,000 m isn't needed and would only break the physics envelope (Beaver service ceiling ~5,500 m; turbine conversions reach ~7 km). 4,500 m is within the flyable envelope for MVP + planned aircraft with margin for full-power climbs.

**2. Per-biome altitude envelopes (soft clamps via smoothstep):**

| Biome | Real reference | Typical range | Peak bound | Sub-SL extent |
|-------|----------------|---------------|------------|---------------|
| Coastal Alaska / Canadian boreal | Chugach / Wrangell / Alaska Range | 0–1,500 m valley; 3,000–5,000 m peaks | 4,500 m (Denali-scale capped to world max) | rare tide flats (0 to -5 m) |
| Canadian boreal interior | Canadian Shield | 100–1,000 m | ~2,000 m | none |
| Mediterranean archipelago | Aegean / Corsica / Sardinia | 0–1,500 m | 2,706 m (Etna) | none |
| Pacific Northwest | Cascades / Coast Range | 0–2,500 m | 4,392 m (Rainier) | none |
| Rocky / Alpine | Colorado Rockies | 1,500–4,000 m | 4,401 m (Elbert) | none |
| Subarctic / North Atlantic | Iceland / Norwegian fjords | 0–1,500 m | 2,119 m (Hvannadalshnjúkur) | none |
| Tropical island chain | Hawaii / Polynesia | 0–2,000 m | 4,205 m (Mauna Kea) | none |
| High Desert / Plateau | Colorado Plateau | 1,500–3,000 m | 3,851 m (Uncompahgre) | -86 m (Death Valley; rare + localized) |

Each biome declares `(min_altitude, typical_min, typical_max, max_altitude)`. Heightfield output is **soft-clamped** into this range with smoothstep transitions — most terrain in the biome sits inside `typical_min..typical_max`; the `typical_max..max_altitude` band allows rare peaks; `min_altitude` is a hard floor (smoothstepped to avoid visible banding).

**3. Region-scale altitude trend (low-freq layer):**

The world-bake heightfield is the sum of two components:
- **Regional trend:** very-low-frequency noise at ~100 km scale. Sets the *mean altitude target* for a region. Alaska-coastal regions trend 0–500 m mean; Rocky-Alpine trend 1,500–3,000 m mean; high-desert plateaus trend 1,500–2,500 m mean.
- **Fine variation:** ridged multifractal + domain warping on top. Adds ±500–1,500 m peak-to-valley variation within the regional mean.

The two compose additively, then the biome-envelope soft-clamp caps the final output. Result: each region has plausible mean altitude for its biome, with plausible variation around it.

**4. Feature-level cross-validation:**

Layer 2 feature placement (`altitude_range` constraint) catches inconsistencies at bake time:
- A `beach` feature in a cell whose Layer 1 altitude is 3,000 m fails the feature's `altitude_range` (-5 to 50 m) → feature rejected, seed re-rolls.
- A `mesa_top` feature at Layer 1 altitude 200 m fails its `altitude_range` (1,500 to 3,500 m) → reject.
- A `glacier_tongue` at altitude 500 m in a tropical biome fails both its altitude_range AND its biome compatibility → reject.

This triple gate — world bounds + biome envelope + feature altitude_range — enforces the "real-world possible" principle at bake time. Errors surface early, not at runtime.

**Interaction with flight physics:**

- Beaver service ceiling ~5,500 m MSL (density altitude + engine power from 2.4.3).
- Terrain up to ~4,000 m MSL: flyable at cruise power with margin.
- Terrain 4,000–4,500 m MSL: full-power climb required; aircraft can go over but it's demanding.
- Terrain > 4,500 m MSL: not in flowPlan's world envelope. Doesn't arise.

**Realistic unflyable corridors are OK.** A 4,500 m ridge between two valleys is flyable-around but not flyable-over in a loaded Beaver. Real bush pilots route around the Alaska Range, not over it. flowPlan's world should produce these choices naturally.

**Flag for Phase 4+ (aircraft-aware mission routing):**

If terrain can produce 4,500 m ridges, mission routing must check: does a flyable great-circle-or-optimal-altitude path exist between source and destination hubs for the player's aircraft? If not, the mission doesn't generate. Fleet service ceilings come from AircraftSpec; routing checks Layer 1 heightfield maxima along candidate paths. Easy to layer in once the mission-generation machinery lands at [Milestone 4.5](ROADMAP.md#milestone-45-missions-and-template-progression). Not a 3.6 concern.

### Layer 1 noise primitives (reference)

Layer 1 heightfield composes **noise primitives** per biome. Phase 3.6.0 ships one primitive (Perlin-fBm) applied uniformly; Phase 3.6.1 adds the others and routes them by biome. Each primitive below is specified with the formula, the feel it produces, and the biomes it suits — so biome authoring can pick without re-derivation.

#### Primitive 1: Perlin-fBm (the baseline, Phase 3.6.0)

Sum of 2D Perlin noise octaves at halving spacing + halving amplitude. Natural smooth undulation with no periodic structure. Always the starting layer; other primitives compose on top.

```
fbm(x, z) = Σ_{i=0..N-1} amp × persist^i × perlin(x, z, spacing / 2^i)
```

- **Parameters:** `base_spacing_m`, `octaves`, `amplitude_m`, `persistence` (default 0.5)
- **Feel:** rolling terrain, gentle gradients, no sharp features. Looks like mid-latitude hill country.
- **Per-biome use:** baseline for every biome. Most biomes add other primitives on top.
- **Wrap:** periodic by gradient-grid construction; `spacing_m` must divide `WORLD_EXTENT_M` evenly.
- **Reference implementation:** `crates/world/src/terrain.rs` as of Phase 3.6.0.

#### Primitive 2: Ridged multifractal (for alpine / mesa biomes)

fBm variant where each octave inverts `|perlin|` to produce sharp ridges. Multifractal adjustment scales each octave by the previous octave's output, amplifying amplitude at already-high locations (more dramatic ridges on mountains, gentler on plains).

```
ridged(x, z) = Σ_{i=0..N-1} amp × persist^i × (offset − |perlin(x, z, spacing/2^i)|)^power
```

- **Parameters:** `offset` (default 1.0; controls ridge sharpness), `power` (default 2.0; increases = sharper ridges), plus fBm params.
- **Feel:** crested ridges with valleys. Classic "mountain range" or "canyon mesa" look. At high `power` becomes cliff-like.
- **Per-biome use:**
  - **Rocky alpine** — primary primitive. High `power` (~2.5), strong amplitude (250+ m fine layer). Sharp crests.
  - **High desert / plateau** — secondary primitive. Mid `power` (~1.8) for mesa edges; pair with domain-warping to avoid grid-aligned structure.
- **Variant:** "hybrid multifractal" scales each octave by the previous iteration's output raw (not absolute-valued), producing flatter lowlands + dramatic highlands. Useful for coastal alpine where plains meet abrupt mountain fronts.

#### Primitive 3: Domain warping (for organic coastlines / varied terrain)

Feed the fBm output back as a perturbation to the input coordinates before sampling a second fBm. Breaks up straight lines and grid alignment, produces organic wrinkled distortion.

```
warp_x = warp_amplitude × fbm_warp(x, z)
warp_z = warp_amplitude × fbm_warp(x + 1000, z + 1000)   // decorrelation offset
terrain(x, z) = fbm_base(x + warp_x, z + warp_z)
```

- **Parameters:** `warp_amplitude_m` (how far coordinates get pushed; 500–5000 m typical), `fbm_warp` config (usually lower-frequency than `fbm_base`).
- **Feel:** curved coastlines, swirled mountain patterns, natural-looking distortion. Removes the slight "Perlin-gridness" at octave boundaries.
- **Per-biome use:**
  - **Coastal boreal / PNW** — primary primitive. Strong warp (~3000 m) over fBm base for fjord-like coastlines.
  - **Mediterranean archipelago** — strong warp + shallow fBm base gives varied island shapes without grid pattern.
  - **Tropical island chain** — warp as second pass over island-specific placement noise.

#### Composition rules

Primitives compose **additively at the heightfield level** (same as Layer 3 feature-deltas). A biome's heightfield is typically:

```
height(x, z) = regional_fbm(x, z)                       // L1 Phase 3.6.0 baseline
            + biome_primitive(x, z)                     // ridged or domain-warped fBm
            + Σ sub-biome-feature-deltas(x, z)          // L3 (lake basins, etc.)
```

Each biome declares its `(primary_primitive, secondary_primitive, warp_amplitude, ridged_power, amplitude_scales)` tuple at bake time. A catalog of biome noise profiles lives at [`FEATURES.md § Sub-biome feature libraries`](FEATURES.md#sub-biome-feature-libraries-pinned-architecture) alongside the semantic feature data.

#### Implementation notes

- All three primitives are **toroidal-wrap-continuous** when built on Perlin (gradient-grid mod-N). Domain warping preserves wrap as long as the warp itself wraps (which follows automatically since it's fBm).
- **Determinism** requires fixed octave-offset indices per-primitive per-biome to avoid gradient-hash collisions between layers. Convention: reserve `octave_offset` ranges `[0..100)` baseline, `[100..200)` secondary, `[200..300)` warp displacement x, `[300..400)` warp displacement z, etc.
- **Performance:** domain warp doubles the sample cost (two fBm passes); ridged is same cost as fBm. Both within budget at Layer 1 coarse (100 m) resolution.
- **Altitude envelope soft-clamp** applies AFTER all primitive composition, so primitive output can overshoot the biome band and the clamp tames it.

### Layer 2 — Voronoi partition + sub-biome feature assignment

Purely answers: *"What is where, semantically?"* No heightfield modification.

- **Seed distribution:** blue-noise Poisson-disk sampling at ~2 km average spacing. See § Chunk model for the seed density and packing details.
- **Feature-type assignment:** each seed is RNG-assigned a sub-biome feature type from its parent biome's library (see [FEATURES.md § Sub-biome feature libraries](FEATURES.md#sub-biome-feature-libraries-pinned-architecture)). RNG selection respects **placement constraints** against the Layer 1 topography:

| Constraint | Semantics | Example |
|------------|-----------|---------|
| `slope_tolerance_max` | Max slope (deg) from L1 coarse heightfield the feature accepts | Major runway: 0.5°; remote bush strip: 4° |
| `roughness_tolerance` | Max undulation across cell footprint (m) | Lake: 0.5 m; dune field: unbounded |
| `water_adjacency_required` | Must touch a hydrology-graph water body | Dock: yes; mesa_top: no |
| `altitude_range` | Min / max altitude (m MSL) | Alpine tarn: 1500–3000 m; beach: -5 to 50 m |

If a seed's L1 sample doesn't satisfy the assigned feature's constraints, RNG re-rolls to a compatible feature from the biome's library. If no feature is compatible after N rolls, the seed falls back to a generic "plain" feature (still valid).

- **Palette adaptation curves:** each sub-biome feature declares its home palette + adaptation rules per host biome. When placed in a non-home biome, the palette interpolates toward the host. See [FEATURES.md § Sub-biome feature overlap with palette adaptation](FEATURES.md#sub-biome-feature-overlap-with-palette-adaptation).
- **Neighbor adjacency graph:** Voronoi cell neighbor relationships cached at bake time for hub resolution, boundary stitching, feature-group coordination.

### Layer 3 — Feature-delta heightfield modifications

Purely answers: *"How does this feature express in terrain?"* Additive composition on top of Layer 1.

**Delta functions per feature type:**

| Feature type | Delta signature | Falloff | Notes |
|--------------|-----------------|---------|-------|
| `lake_cluster` | Negative smooth bowl | Bump-function to zero at cell edge | Basin depth from feature parameters |
| `runway_strip` | Flatten rectangle + short skirt | Linear blend over 50 m skirt | Heading from Voronoi-cell long-axis |
| `mesa_top` | Positive plateau + cliff edge | Sharp falloff at cliff; bump elsewhere | Plateau altitude from feature params |
| `arch_formation` | Positive rock geometry + void | Compact local feature | Adds mesh structure, not just heightfield |
| `dune_field` | High-frequency positive noise | Gradual bump-function | Amplitude scales with cell area |
| `river_floodplain` | Slight negative trough along river | Matches hydrology-graph river waypoints | Coherent with Layer 1 hydrology |
| `alpine_tarn` | Small negative circular basin | Steep cliff-like falloff | Positioned inside elevated topo |
| `beach` | Slight negative along shore | Smooth ramp | Classification only; no major geometry |

**Invariant: hydrology continuity.** Deltas in Layer 3 **cannot** violate the Layer 1 hydrology graph. Specifically:

- Cannot elevate terrain along a river channel (feature rejected at L2 placement).
- Cannot fill an existing lake basin.
- Cannot dam a drainage network.
- A `mesa_top` feature rejects placement where the hydrology graph has a river passing through the cell.

The hydrology graph is **immutable across layers**. Delta functions respect it or get rejected.

**Boundary falloff invariant.** Each feature's delta falls smoothly to zero at its Voronoi cell edge (bump function / cosine falloff). Neighboring cells don't fight over boundary heights — each contributes zero there, final height at the boundary is the Layer 1 base value only. Continuous terrain across Voronoi edges by construction.

**Flora + decoration** layers on top of the final heightfield in Layer 3. LOD-aware: near-aircraft cells render full flora instances; mid-range cells downgrade to billboards; far cells skip flora entirely.

**Shore classification sampler** (`world::surface_at(x, z) -> SurfaceClass`) derives from the final heightfield + hydrology graph + biome. Unblocks [Milestone 2.5b.7 per-surface friction](ROADMAP.md#milestone-25-ground--water-contact-physics).

### Composition properties (why this separation matters)

- **Independent development.** Each layer lands on a separate milestone. Layer 1 = Phase 3.6.1; Layer 2 = Phase 3.6.2; Layer 3 = Phase 3.6.3 + 3.6.4.
- **Independent testing.** Per-layer bench assertions (generalizing the flight-bench pattern): L1 produces bit-identical heightfield + hydrology across platforms; L2 produces bit-identical partition given same L1 inputs; L3 produces bit-identical final heights given same L2 partition + L1 base.
- **Playable-at-each-step.** Phase 3.6.1 ships L1 alone — flowPlan already has real biome-coherent terrain + hydrology as a playable world. Phase 3.6.2 adds L2 semantics without breaking L1. Phase 3.6.3+ adds L3 geometry.
- **Concern separation for later refinement.** Changing the heightfield generator (swap ridged multifractal for something else) affects L1 only. Adding a new sub-biome feature affects L2 + L3 authoring, not L1. Retuning the chunk streaming algorithm affects neither.
- **Cross-cell feature groups.** When a feature should span multiple cells (a ridgeline, a salmon-run river through several cells), feature-group declarations at Layer 2 coordinate delta functions at Layer 3. Groups don't break the layer boundary — they're still L2 placement + L3 geometry.

### Variable Voronoi cell size (PINNED — all Voronoi applications)

**Principle:** uniform cell size is visible as regular-grid structure even with jittered seed positions. All Voronoi applications in flowPlan — weather (W2, ~25 km scale), terrain sub-biome features (L2, ~2 km scale), any future Voronoi layers — use **weighted distance metric** with per-cell weights sampled from a coarse Perlin field, producing organic cell-size variation at ~10-20× the base cell scale.

**Mechanism** (additive-weighted Voronoi / power diagram):

```
weight(seed_i) = Perlin(master_seed, seed_pos.x, seed_pos.z, weight_spacing)
distance(query, seed_i) = ||query - seed_pos_i||² − weight(seed_i)
owning_cell = argmin_i(distance(query, seed_i))
```

One-line change to the standard Voronoi distance calc. Cells with higher weights "win" more territory (grow larger); lower-weight cells shrink or are absorbed by stronger neighbors. The weight field's spacing is 10-20× the base cell size, so variation happens at regional scales — "this area has mostly-large cells; that area has mostly-small cells."

**Per-application tuning:**

| Application | Base spacing | Weight-field spacing | Amplitude |
|---|---|---|---|
| Weather Voronoi | ~25 km | ~500 km | moderate (2-3× size variation) |
| Terrain sub-biome | ~2 km | ~40 km | high (5-10× — some regions fine-grained detail, others broad sweeps) |

**Feel:** real-world weather has both compact convective cells and sprawling frontal systems; real geography has both tightly-packed features (archipelago islands) and broad expanses (desert plateaus). Uniform cell size can't express this. Variable cell size does.

**Determinism:** Perlin weights are deterministic per `(master_seed, seed_pos)`. Same seed → same weights → same cell boundaries.

### Biome edge blending + tri-biome rare experiences (PINNED)

**Principle:** biome boundaries are where content diversity concentrates. Players crossing from coastal-boreal into rocky-alpine should see *mixed-character terrain* for ~50-100 km of transition — neither purely coastal nor purely alpine, but recognizably both, with features from each appearing in the other's territory before the full "other biome" character dominates. Sharp biome transitions feel artificial; blended edges feel geographic.

**Mechanism** (extends the Layer 2 palette-adaptation rules):

1. **Bleed-over radius:** each sub-biome feature declares a `bleed_distance_m` (typical 10-50 km). Within this distance of an adjacent biome, the feature can be *selected from either biome's library* with probability proportional to distance-to-biome-edge. So a coastal-boreal sub-biome cell 10 km from the alpine border has ~50/50 chance of spawning a coastal feature (with palette shifted toward alpine) vs an alpine feature (with palette shifted toward coastal).

2. **Palette shifting:** already specified per [FEATURES.md § Sub-biome feature overlap with palette adaptation](FEATURES.md#sub-biome-feature-overlap-with-palette-adaptation). At transition edges, palette interpolation runs between the home biome and the nearest neighbor biome.

3. **Adjacency rule enforcement still applies:** forbidden transitions (e.g. snow → tropical — see § Biome system) are rejected regardless of bleed distance. Physical + climate adjacency rules override cosmetic bleeding.

4. **Tri-biome intersections:** where three biome regions meet within a cell's bleed radius, the cell has access to all three biomes' feature libraries. These triple-points are **rare by construction** (3-way meetings are topologically sparse in a 2D Voronoi) but offer the richest mix. Authoring gets a small explicit set of triple-point-only features (crystal-lake amid forest amid desert mesa — the "places that shouldn't exist") gated to spawn only at tri-biome intersections.

5. **Fine-octave climate helps** — adding fine octaves to `temperature_at` / `humidity_at` fBm produces microclimate pockets at ~15-30 km scale. Within-region biome variety increases (a warm valley inside a cold region) without shrinking the world. Microclimates enable tri-biome intersections to actually form at the 2 km feature-cell scale; without them, the three biomes might only meet at very coarse regional boundaries.

**Gameplay payoff:**

- **Discovery:** players seeking rare combos fly transition zones deliberately.
- **Photo mode (Phase 5.5):** transition-edge composites are naturally striking backgrounds.
- **Mission design:** Phase 4 mission templates can prefer tri-biome landmarks for "find the odd place" missions.
- **Progression:** rare triple-point features can gate upgrade cosmetics or lore drops without feeling grindy (since they're just "fly where the climate maps cross").

**Implementation phase:** Layer 2 feature assignment (Phase 3.6.2) gets the bleed-radius + neighbor-biome-probability logic; palette adaptation already pinned; tri-biome authoring hooks added then; fine-octave climate tuning happens alongside 3.6.2 biome-terrain work.

---

## Three-layer weather composition (second application of the pattern)

The three-layer composable architecture used for terrain (Topographic base × Voronoi partition × Feature deltas) applies directly to weather. Same pattern, different scale, different state, temporal drift.

| Terrain | Weather |
|---------|---------|
| **L1 Topographic base** — continuous heightfield + biome + hydrology + magnetic variation | **W1 Atmospheric base** — continuous pressure + temperature + humidity + altitude-stratified wind + frontal-boundary graph |
| **L2 Voronoi partition** (~2 km cells; placement constraints vs L1: slope, roughness, altitude, water-adjacency) | **W2 Weather Voronoi** (~20–50 km cells; placement constraints vs W1: pressure range, humidity threshold, temperature range, wind-shear threshold) |
| **L3 Feature deltas** (per-cell heightfield modifications: lakes carve, runways flatten, mesas raise) | **W3 Cell atmospheric deltas** (per-cell updrafts/downdrafts, gusts, cloud density, precipitation; storm cells add local turbulence; clear-air contributes nothing) |
| **Invariant: hydrology graph immutable across layers** — L3 deltas cannot elevate rivers, fill lakes, dam drainage | **Invariant: frontal-boundary graph immutable across layers** — W3 deltas cannot place weather inconsistent with W1 pressure field; placement rejected at W2 |

Incremental-development dividend applies the same way: **W1 ships useful alone** (real atmospheric state — wind affects flight, density altitude changes with weather), **W2 adds semantics** (where weather happens), **W3 adds visible geometry + active physics** (rendered clouds, turbulence, precipitation).

### W1 — Atmospheric base field

The continuous Layer 1 for weather. All deterministic per `(master_seed, world_coord, sim_time)`; static in world-space; evolves only through slow prevailing-wind shifts.

- **Pressure field:** low-frequency global field shaped by latitude (equatorial low, mid-latitude high), seasonal shifts, and large-scale regional patterns (Aleutian Low, Pacific High analogs). Drives prevailing wind.
- **Temperature field:** latitude + elevation + season + diurnal cycle. Feeds density altitude, icing probability, cloud base altitude (condensation level).
- **Humidity field:** regional (coastal moist, interior continental dry, PNW perpetual-moist, high-desert dry). Feeds cloud formation probability and icing risk.
- **Prevailing wind field, altitude-stratified:** derived from pressure gradient via approximate geostrophic balance. Low-altitude wind (terrain-influenced) differs from wind aloft. Feeds the W2 temporal drift offset (clouds at different altitudes drift at different rates).
- **Frontal-boundary graph:** pre-baked lines of maximal temperature/pressure gradient — the weather analog of L1's hydrology river graph. Cells along a frontal line bias toward frontal-passage cell types in W2. Global structure, immutable across W2/W3.
- **Terrain-induced turbulence map:** derived from L1 topographic gradients — lee-side rotors, valley wind channeling, ridge thermals. Static per master seed once terrain is baked.

Physics consumers (wind at aircraft position, density altitude, turbulence baseline) sample W1 directly. W2 cells + W3 deltas augment — they don't replace.

### W2 — Weather Voronoi partition

## Weather Voronoi layer (second partition)

Weather is organized as a **second Voronoi partition** independent of the terrain Voronoi — same architectural pattern, different scale, different cell semantics, temporal drift. The parallel is intentional: Voronoi cells align with natural weather-system boundaries (frontal passages, convective cells, clear-air gaps) the same way terrain Voronoi cells align with natural landscape features.

### Why a separate partition

- **Scale mismatch.** Terrain Voronoi cells are ~2 km (feature scale). Weather systems are 20–50 km (frontal / convective-cell scale). Using the same partition for both would either make weather cells far too small (per-2-km-cell weather looks chaotic) or terrain cells far too large (a 30 km "lake cluster" is absurd).
- **Temporal mismatch.** Terrain is static in world-space. Weather moves. They need independent data flow.
- **Semantic mismatch.** Terrain cells carry sub-biome-feature types (geometry-determining); weather cells carry storm-state + cloud-deck parameters (atmospheric-determining). Different authoring surfaces.

### Seed + bake

- **Seed derivation:** `weather_voronoi_seed = hash(master_seed, "weather_voronoi")`. Independent stream from terrain Voronoi.
- **Blue-noise distribution** at **20–50 km average seed spacing**. For a 1000 × 1000 km world at 30 km average spacing, ~1,100 weather cells per region and ~11,000 world-wide. Much sparser than terrain (333K) because of the larger cell size.
- **Partition triangulation:** Fortune's sweep at world-bake time; cached alongside the terrain partition + hydrology graph.
- **Neighbor adjacency graph** cached at bake time for adjacency-bias evolution and edge-interpolation lookup.

### Per-cell state

```rust
struct WeatherCell {
    id: WeatherCellId,
    seed_point: Vec2,
    polygon_edges: Vec<Edge>,
    neighbor_ids: Vec<WeatherCellId>,
    cell_type: CellType,          // clear_air | cumulus | nimbus | stratus | squall | frontal
    base_altitude_m: f32,         // cloud deck bottom
    top_altitude_m: f32,          // cloud deck top
    density: f32,                 // 0..1 scalar
    column_height_m: f32,         // redundant-ish with top-base; kept for convective towers
    precip_type: PrecipType,      // none | drizzle | rain | snow | hail
    storm_state: StormState,      // state-machine instance from FEATURES.md § Weather
    evolution_phase: f32,         // 0..1 within the current state (for build/peak/dissipate curves)
}
```

### Temporal drift

Cells are static in world-space (determinism preserved) but consulted via a drift-offset coordinate driven by prevailing wind aloft:

```rust
fn weather_cell_at(world_pos: Vec2, sim_time: f32, altitude: f32) -> WeatherCellId {
    let wind = prevailing_wind(altitude);  // from Phase 3.2 wind field
    let drift = wind * sim_time;            // cumulative offset
    weather_voronoi.cell_at(world_pos - drift)
}
```

Weather moves past a stationary observer at the prevailing wind-aloft speed. Deterministic per `(master_seed, sim_time, world_pos, altitude)`. No partition re-bake. **Altitude-dependent drift** is a free win: a low-level stratus deck drifts at one speed, high-altitude cirrus at another — matches real atmospheric layering.

### Adjacency-bias evolution

Cells evolve through their storm state machine on a **slow cadence** (e.g. once per sim-second, not per-tick — folds into `state_hash` for determinism). When a cell transitions, neighbor-content biases the choice:

- A cell adjacent to a `building_cumulus` neighbor has elevated probability of entering `building_cumulus` itself. Produces large sprawling systems.
- A cell surrounded by `clear_air` neighbors tends to stay clear or transition to mild `cumulus`. Clear-air regions stay clear; weather concentrates coherently.
- Storm dissipation also biases — an `organized_storm` cell with clear neighbors is more likely to dissipate than one with storm neighbors (systems die out at their edges).

This is the mechanism that produces **weather-that-looks-like-weather**: frontal bands, organized convection, clear-air gaps, storm-free regions. Per-cell random weather looks chaotic; adjacency-biased cells look like real weather systems.

### "No-weather is a type of weather"

Empty cells are first-class `cell_type = clear_air` cells with `density = 0` and normal atmospheric-state fields (temperature, humidity, pressure from Phase 3.2 state). Transitions between clear-air and stormy cells at Voronoi edges happen via standard density interpolation — no special-case code for "where does weather start / end". The model is uniform.

### Cloud rendering consumption

Raymarched volumetric clouds (Phase 3.2x) consult the weather Voronoi at each sample point:

1. Apply altitude-dependent drift offset to the sample's world coord.
2. Query weather cell at the drifted coord.
3. If cell is `clear_air` or sample's altitude is outside cell's `[base_altitude, top_altitude]` → density = 0.
4. Otherwise: compute density from cell parameters + sub-cell Perlin-Worley noise detail. Interpolate with adjacent cells' densities near Voronoi edges.

CPU-side physics (Phase 3.2 weather-affects-flight) samples `world::cloud_density(x, y, z, t)` using the same function. Turbulence, visibility, icing all consume it. Same determinism guarantee as the rendering side.

### Coupling with water-body chop (cross-system integration)

Water-body chop state is **derived from the overlying weather cell**:

```
water_body.chop = chop_fn(
    wind_at_body.magnitude,
    weather_cell_over_body.storm_state,
    body.fetch_distance,
    body.surface_area
)
```

A storm cell over a lake drives heavy chop; a clear-air cell leaves calm water. **Unidirectional coupling** — weather drives chop, not the reverse. Closes the 2.5a.7 chop hook with a real weather input in Phase 3.

### Storage + persistence

- **W1 atmospheric base** — pressure + temperature + humidity + wind + frontal graph — baked once at session load, ~10-20 MB. Deterministic per master seed; cached on disk keyed by seed if needed.
- **W2 Weather Voronoi partition** (~11K cells × ~200 bytes per cell) ≈ 2–3 MB. Trivial compared to the hub catalog + hydrology graph + terrain partition.
- **W3 Per-cell state evolution deltas** are persisted in the save-format event log (storm initiations, dissipations, state-machine transitions). Replay reconstructs weather state exactly.
- **Drift offset** is a pure function of `(master_seed, sim_time)` — no persistence needed.

### W3 — Cell atmospheric deltas (how weather manifests in physics + rendering)

Cells modify W1 locally. Within a cell's footprint, the aircraft-position wind / turbulence / visibility / precipitation is:

```
wind(x, y, z, t)       = W1.wind(x, y, z, t) + active_cell.wind_delta(x, y, z, t)
turbulence(x, y, z, t) = W1.turbulence(x, y, z) + cell-edge-gradient + active_cell.convective_turb
visibility(x, y, z, t) = W1.visibility × (1 - active_cell.cloud_density(x, y, z, t))
precip(x, y, z, t)     = active_cell.precip_rate × precip_falloff(altitude)
```

Delta functions per cell type:
- **`clear_air`**: all deltas are zero. Pure W1 state.
- **`cumulus`**: mild updraft column beneath; mild turbulence at cloud base; cloud density bounded [0, 0.6] at `[base_altitude, top_altitude]`.
- **`nimbus`**: strong updraft core + downdraft fringe; moderate turbulence throughout cell; cloud density up to 0.9; rain precipitation below cloud base.
- **`stratus`**: no significant updraft/downdraft; very low turbulence; horizontal cloud layer at moderate density.
- **`squall`**: strong horizontal wind-shift as the cell moves (gust front at leading edge); heavy rain; sharp turbulence peak at gust front; passes relatively quickly.
- **`frontal`**: cells along a W1 frontal-boundary line segment; strong wind shift across the line; mixed precipitation; cells biased to evolve together (the whole frontal band moves as a system).

Adjacent-cell blending at Voronoi edges uses standard Voronoi-edge distance weighting — density / wind / turbulence interpolate across the edge for organic transitions rather than step discontinuities.

**Invariant: W3 deltas respect W1 frontal graph.** Just as L3 feature deltas cannot violate the hydrology graph, W3 cell deltas cannot place weather inconsistent with the W1 pressure + frontal structure. A storm cell cannot form in a stable high-pressure core (constraint enforced at W2 placement); frontal cells must align with a frontal-boundary line segment.

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

### Hub late-binding (fuzz + resolve)

**Problem:** missions need to reference hubs that may be 50–200 km away — outside the streaming disk's 8–22 km radius. Voronoi cells only exist for the streaming disk. How do we reference a hub before its cell is generated?

**Solution:** **two-phase hub binding.**

**Phase 1 — World-bake (once per session, deterministic per master seed):**

A **hub catalog** is committed as part of the world-bake output, alongside the hydrology graph + biome mask. Each entry:

```
struct HubEntry {
    id: HubId,                         // stable cross-session identifier
    approx_world_pos: (f32, f32),      // fuzzed coordinates — approximate placement
    tier: HubTier,                      // major | regional | local | remote | isolated
    parent_biome: BiomeId,              // for palette / sub-biome compatibility
    required_surfaces: &[SurfaceClass], // what the hub needs to land
    display_name: Option<String>,       // visible once discovered; pre-generated
    region_id: RegionId,                // for missions + community-event scoping
}
```

Placement heuristics per tier:
- **Major** (1–2 per region): biome-core + water-adjacent + flat-ground-suitable.
- **Regional** (~3–5 per region): biome-mid + hydrology-adjacent + populated-area.
- **Local** (~10–20 per region): scattered across accessible terrain.
- **Remote** (~30+ per region): scenic anchors — fishing-camp-plausible lakeshores, clearings, ranger-station spots.
- **Isolated** (~500+ per region, discovery-gated): highly constrained placement — hidden tarns, abandoned strips, pristine coves. Not shown on any map until discovered.

Catalog size: ~500–800 hubs per region × ~100 regions in a 1000 × 1000 km world = ~50–80K hubs. At ~200 bytes per entry, total ~15 MB. Persisted with the hydrology graph.

**Phase 2 — Streaming-time resolution (when a hub's approximate coords fall into the streaming disk):**

When the Voronoi generator produces a cell whose footprint contains (or is nearest to) a hub's `approx_world_pos`:

1. Check the cell's `sub_biome_feature_type` against the hub's `required_surfaces` — is this cell the right shape/surface for the hub?
2. If yes: **snap** the hub to the best-fit point within the cell (e.g. a lake-edge dock point for a floatplane base, a flat patch for a runway). Exact coords cached to the hub entry.
3. If no: walk the Voronoi neighbor adjacency graph looking for a compatible neighbor cell. Pick the nearest compatible. Snap there.
4. If **no neighbor within N hops** is compatible: hub is flagged `resolution_failed` — surfaced as a bake-time warning for the user to manually place the hub. Rare edge case; most hubs resolve on first or second-nearest attempt.

**Why this works:**

- **Mission routing is hub-ID-based.** A mission references `(source_hub_id, destination_hub_id)` and uses approximate coords for load / fuel / distance planning. Routing doesn't wait for cells to exist.
- **2D hub map shows hubs as icons.** Pulls from the catalog directly; no need for cells to be generated.
- **Hubs survive Voronoi re-partition.** If the Voronoi seed distribution changes between builds (determinism-preserving re-bake), hub IDs stay stable — they just re-resolve to the new partition.
- **Deterministic.** Hub catalog bake is pure function of master seed; resolution is pure function of (master seed, streaming generation order). Save-format event log captures discovery deltas.

**The pattern generalizes.** Beyond hubs, the same fuzz+resolve pattern applies to any "global reference to eventually-generated local feature":
- **Wildlife zones** (rookeries, salmon-run rivers, migration paths) — committed at bake, resolved to cells as aircraft approaches.
- **Mission anchors** (SAR beacons, cartography waypoints, fire-fighter sites) — same pattern.
- **Barn-find aircraft in isolated hubs** — discovery pairs a hub with a barn-find entry; both bake-committed, both resolve on arrival.
- **Landmark placements** referenced by NPC dialog / radio / message-in-a-bottle — stable IDs + approximate coords + resolve-on-arrival.

---

## Consumer API surface

World-gen subsystems are consumed by multiple downstream callers (flight model, rendering, bench, UI). The API surface is **emergent** — exact signatures finalize at implementing-milestone time — but the **shape is load-bearing**: changes to these signatures touch many callers simultaneously, so design with that coupling in mind.

**Core world-sampling functions** (exposed by world-crate subsystems, consumed CPU-side by sim + render + bench; some also have matching WGSL shader implementations for GPU-side sampling):

```rust
// Layer 1 topographic base (ships Phase 3.6.1)
world::height_at(x: f32, z: f32) -> f32;                        // final heightfield, post L3 deltas
world::biome_at(x: f32, z: f32) -> BiomeId;
world::surface_at(x: f32, z: f32) -> SurfaceClass;              // unblocks 2.5b.7 per-surface friction
world::magnetic_variation(x: f32, z: f32) -> f32;

// W1 atmospheric base (ships Phase 3.2.1)
world::pressure_at(x: f32, y: f32, z: f32) -> f32;
world::temperature_at(x: f32, y: f32, z: f32, t: f32) -> f32;
world::humidity_at(x: f32, y: f32, z: f32, t: f32) -> f32;
world::wind_at(x: f32, y: f32, z: f32, t: f32) -> Vec3;         // W1 baseline in 3.2.1;
                                                                 // composes with W3 deltas in 3.2.3
world::turbulence_at(x: f32, y: f32, z: f32, t: f32) -> f32;    // W1 baseline 3.2.1;
                                                                 // W3-augmented 3.2.3

// W3 weather deltas (ships Phase 3.2.3)
world::cloud_density(x: f32, y: f32, z: f32, t: f32) -> f32;    // CPU-GPU shared formula
world::precipitation_at(x: f32, y: f32, z: f32, t: f32) -> PrecipRate;

// Existing (shipped Phase 2.2)
world::water_height(x: f32, z: f32, t: f32) -> f32;             // Gerstner; replaced by
                                                                 // FFT ocean in 3.5b
```

**Hub catalog + Voronoi partition** (exposed by world-gen subsystems, consumed at scale by UI + mission systems):

```rust
world::hub_catalog() -> &HubCatalog;                             // Phase 3.6.1; read-only
world::voronoi_cell_at(x: f32, z: f32) -> CellId;                // Phase 3.6.2
world::voronoi_neighbors(cell: CellId) -> &[CellId];
world::weather_cell_at(x: f32, z: f32, y: f32, t: f32) -> WeatherCellId;  // Phase 3.2.2
```

### Awareness directives

- **These signatures will emerge through the implementing milestones** — not rigidly pre-specified. The shape sketch above is a **forward-looking contract** to keep in mind so early milestone decisions don't accidentally foreclose on downstream needs.
- **Downstream consumers depend on this surface existing** by the time their milestone starts. E.g. `world::cloud_density` is produced by 3.2.3 and consumed by 3.2x (cloud rendering) + 2.5 turbulence hooks + 4.5 mission capability-gating. Changes to its shape mid-Phase-3 would ripple.
- **Same formula on CPU + GPU** where both consume — `cloud_density`, `water_height`, noise-based fields. Uses the `libm` discipline (same transcendentals on CPU; matching WGSL on GPU) for bit-identical output. See [CODING_RULES.md § Determinism Discipline](CODING_RULES.md#determinism-discipline).
- **Determinism contract:** every sampler above is a pure function of `(master_seed, inputs, [time])`. No hidden state, no wall-clock reads, no thread-local RNG. Bit-identical cross-platform with libm + `-fma` pinned.
- **Coupling with physics invariants:** sampled values feed `SimState` ticks (turbulence, visibility, icing, buoyancy). Any change to sampling semantics can affect scenario-bench reproducibility — flag in PR + re-baseline bench CSVs if the physics-facing output shifts.

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
- **Save-format directory structure** (locked 2026-04-21 with world-scope Option A):

  ```
  saves/
    pilots/
      <pilot_uuid>.save          # global pilot data (identity, fleet, currency, career stats)
      <pilot_uuid>/
        <seed_hex>.progress      # per-(pilot, seed): fog-of-war, hubs, missions, resume-point
    worlds/
      <seed_hex>.world           # per-seed world event log (storms, hydrology changes, shared across pilots)
  ```

  `<seed_hex>` = hex of `master_seed XOR hash(WeatherTuning)` so seeds with different tuning are distinct worlds. Deleting a pilot cleanly removes all their per-seed progress.

- **Persistence entities are orthogonal.** Pilot profile (global), world seed (global, shared across pilots), per-(pilot, seed) progress (intersection). Each versions independently via `(game_version, pilot_schema, world_schema, progress_schema)`.
- **Save footprint estimate.** Pilot globals <1 MB; world event log 10–100 KB per seed; per-(pilot, seed) progress ~50–100 KB (fog-of-war bitmap dominates, compresses to 5–20 KB RLE). Worst-case install (10 pilots × 100 seeds): ~100 MB total.
- **NG+ definition.** Any pilot with prior progression entering an unvisited seed. Not gated on completion threshold. "Fresh pilot" is an opt-in at pilot-creation time.
- **Sharing model.** Seeds are shareable by value — `(u64 master_seed, WeatherTuning)`. Joining a friend's game loads their world event log + your pilot + a fresh per-(you, their-seed) progress file. Your pilot brings career, starts fresh on their specific seed.
- **Cross-platform determinism.** Hashing function choice + IEEE float discipline per [ROADMAP Milestone 6.1](ROADMAP.md#milestone-61-determinism-hardening-cross-platform) extends here. The world-bake hydrology graph must be bit-identical across Linux / Windows / macOS / Steam Deck. **With Option A torus**, every sampler must wrap correctly — add per-sampler wrap-continuity tests to the determinism battery.
- **Scenario bench compatibility.** Flight-bench scenarios pin a master seed explicitly; they do not depend on hub placement or biome layout unless the scenario explicitly positions the aircraft at a specific hub. Most scenarios use flat terrain (`Terrain::flat()` / `Terrain::flat_land()`) and are unaffected.

---

## Open questions

Grouped by urgency to Phase 4 lock-before milestone. Resolved items migrated to [World scope decision](#world-scope-decision) and [World topology](#world-topology).

### ✓ Resolved in 2026-04-21 world-scope lock

- World scope candidates: **locked Option A (continuous toroidal world per seed).** Pre-Phase-3 prototype re-scoped to validate wrap mechanics.
- Circular chunks with player-center indexing: **no for persistence** (Voronoi is the chunk shape); **yes for rendering** (streaming disk is circular by design).
- Save-format directory structure: **locked to pilots × seeds matrix** (see above).

### Before terrain rebuild lands (during Phase 3.6)

1. **Chunk size**: 1 km² vs 4 km² vs adaptive. Prototype with target frame budget.
2. **Heightfield representation**: ridged multifractal + domain warp sufficient, or does the terrain rebuild want something more exotic (erosion simulation, gradient-conditioned noise, machine-learned biome exemplars)?
3. **Hydrology graph size**: can the world-bake produce a < 100 MB graph for a 5000 × 5000 km toroidal world, or do we need a sparser representation / on-disk cache?
4. **Biome spatial generator**: Voronoi vs climate-field vs hand-authored mask. Proposed climate-field with adjacency rejection; verify computationally.
5. **Hub placement strategy**: authored chunks vs procedural. Proposed: Option 2 for major, Option 3 for regional, Option 1 for remote. Validate at first hub-generation pass.
6. **Seam blending algorithm**: which edge-matching technique handles authored↔procedural cleanly at the boundaries the playtest will actually traverse? Note: with toroidal wrap, the seam-blend problem also applies at the world-edge wrap itself.
7. **Hydrology bake time budget**: seconds is fine at load; but can it become real-time at world-scope lock if a player changes dial-state mid-game? (Probably not; dial changes don't invalidate the hydrology graph.)
8. **Sampler wrap quantization**: terrain octave frequencies need quantization to `1/WORLD_EXTENT_M × integer` so wrap is clean. Test quantization error at finest octaves — expected negligible but verify.

### Before save format locks (pre-Phase-4.10)

9. **Delta schema**: per-category event formats finalized (per-pilot-per-seed progress deltas + per-world event log entries).
10. **Delta compaction**: append-only during play, compact at save time; what's the compaction algorithm?
11. **Cross-version save migration**: how much forward/backward compatibility is promised per-entity (pilot / world / progress)?
12. **Shared-seed hash compatibility**: if two players share a master seed for async ghosts, the hydrology-graph bake must produce byte-identical output on their machines.
13. **Day/night acceleration ratio**: placeholder 24:1. Playtest to lock the right number. Too fast = arcade-y; too slow = single-session only ever sees one daytime.

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
