# flowPlan -- Feature Backlog

Granular feature bullets for later expansion. Every idea captured here is intentionally preserved in detail; refinement and phase-assignment happen as items promote toward implementation. This document is the canonical place for design detail that is not yet specification. Nothing here has been lost to session compaction or agent handoff.

**Status legend:**

- **[sketch]** -- captured, not yet designed. May conflict with existing docs. Do not implement.
- **[ready]** -- designed, gated into a phase in [ROADMAP.md](ROADMAP.md).
- **[amend]** -- requires a [DESIGN.md](DESIGN.md), [PLAN.md](PLAN.md), or non-goal amendment before it can advance. See [Amendments Status](#amendments-status).
- **[shipped]** -- delivered in the phase noted.

**Relation to other docs:**

- [PLAN.md](PLAN.md) -- the creative "why".
- [DESIGN.md](DESIGN.md) -- the technical "what". Invariants and non-goals that bullets here must respect (or explicitly amend).
- [ROADMAP.md](ROADMAP.md) -- the "when/order". Features promote from this backlog into phase milestones there.

## Contents

- [Amendments Status](#amendments-status)
- [World Scope & Map Layout](#world-scope--map-layout)
- [Regional Biomes](#regional-biomes)
- [Map Memory / Cartography](#map-memory--cartography)
- [Terrain & Static Landscape](#terrain--static-landscape)
- [Hubs & Landing Areas](#hubs--landing-areas)
- [Aircraft Catalog](#aircraft-catalog)
- [Aircraft Configuration & Upgrades](#aircraft-configuration--upgrades)
- [Cockpit Controls & Autopilot](#cockpit-controls--autopilot)
- [Camera & Views](#camera--views)
- [Control Bindings & Mapping](#control-bindings--mapping)
- [Flight Model](#flight-model)
- [Telemetry & Instrumentation Data Flow](#telemetry--instrumentation-data-flow)
- [Ground & Water Surface Interaction](#ground--water-surface-interaction)
- [Weather](#weather)
- [Time of Day & Seasons](#time-of-day--seasons)
- [Night Sky & Astrophotography](#night-sky--astrophotography)
- [Damage & Wear](#damage--wear)
- [Economy & Community](#economy--community)
- [Fishing & Side Activities](#fishing--side-activities)
- [Hangar Storage & Fleet](#hangar-storage--fleet)
- [Missions & Tasks](#missions--tasks)
- [Wildlife](#wildlife)
- [On-Foot / Character](#on-foot--character)
- [Camping & Status Restoration](#camping--status-restoration)
- [Vendors & Progression](#vendors--progression)
- [Livery & Customization](#livery--customization)
- [Audio / Soundtrack](#audio--soundtrack)
- [UI Direction](#ui-direction)
- [Display & Graphics Settings](#display--graphics-settings)
- [Difficulty & Accessibility](#difficulty--accessibility)
- [Flight Log & Save Format](#flight-log--save-format)
- [Multiplayer (Stretch Goal, Post-v1)](#multiplayer-stretch-goal-post-v1)
- [Remaining Open Questions](#remaining-open-questions)

---

## Amendments Status

Decisions recorded from the feature discussion. Items marked *amend* have been agreed; the listed doc edits are pending. Items marked *undecided* remain open.

1. **World scope** -- *undecided*. Two candidates preserved in [World Scope & Map Layout](#world-scope--map-layout): **(A)** one large single map with diverse regional locales, **(B)** ~4 separate regional maps. Procedural generation is a sub-option of both, gated on the cost of retaining flown/discovered state. Do not amend DESIGN.md until this is resolved.
2. **Flight model fidelity** -- *amend, direction (A) confirmed*. Amend [DESIGN.md § Non-Goals](DESIGN.md#non-goals): "Not a full flight sim (no CFD, no engine-management minigames). Aerodynamics are tabulated per-aircraft and faithful; systems are simplified to meaningful toggles. Sim-realism is selectively deep where it affects feel." Liberties for fun are permitted. Player-adjustable dial for wind, shear, hydrodynamics, storms, air temp/humidity, changeable at any time during gameplay.
3. **Missions** -- *amend, direction (A) confirmed*. Amend [DESIGN.md § Non-Goals](DESIGN.md#non-goals) and [PLAN.md § Design Pillars](PLAN.md#design-pillars): "Missions are template-driven and parameterized by world state. Sequential progression increases template difficulty/complexity over time. No authored dialog trees, no branching quest lines. Emergence remains preferred for moment-to-moment situations; missions provide legible scaffolding."
4. **On-foot / character** -- *amend, direction (A) confirmed*. Add a `character/` subsystem to [DESIGN.md § Subsystems](DESIGN.md#subsystems). Minimal character creator authored in Blender by the project owner — Metal Slug-inspired cartoon simplicity rendered in the same cel-shaded + blended volumetric style as the rest of the world; modest personalization. Scope: menu-driven interactions with NPCs/vendors, campfire, disembarked inspection. Gate to Phase 4/5. Tied to wildlife reward unlocks. Detail in [On-Foot / Character](#on-foot--character).
5. **Damage / economy / inventory** -- *amend, confirmed*. Add `damage/`, `economy/`, `inventory/` subsystems to [DESIGN.md § Subsystems](DESIGN.md#subsystems). Damage localized to zones, reduces component performance (including airfoil). Economy leans on a persistent Alaskan wilderness community frame.
6. **Weather expansion** -- *amend, confirmed*. Amend [DESIGN.md § Simulation Architecture](DESIGN.md#simulation-architecture) to enumerate expanded weather state: precipitation regime, storm state, visibility, temperature, humidity, wind shear, water chop, season overlay. Update cadence per tick.
7. **Deterministic seeded soundtrack** -- *ready*, no amendment required. Promote to ROADMAP Phase 3 milestone (Audio). See [Audio / Soundtrack](#audio--soundtrack).
8. **Wildlife as mission source** -- *amend, small*. Add to [PLAN.md § Emergence Vectors](PLAN.md#emergence-vectors): "Wildlife encounters and 'pure wildlife runs' provide non-financial progression: liveries, map fragments, cosmetics, logbook moments. Protected from economy-first pressure."
9. **Difficulty knobs** -- *confirmed*. Knobs tune sim parameters (gust magnitude, stall severity, storm intensity, damage sensitivity, economy harshness, currency decay/inflation). They do *not* swap physics models. Determinism invariant preserved. Adjustable any time in settings; changes take effect at the next tick.

**Resolved open questions:**

- Survival layer -- *post-v1 toggle*. Not in v1 base gameplay. Later, as a difficulty setting, may gate mission acceptance on food/water status; may permit camping anywhere to restore status. Never blocks essential gameplay loops.
- Loan/debt -- *in v1, unlocks as feature delivery*. Debt does not cap the base loop. Engaging with loans opens progression: repairs, engine upgrades (Rotax, turbine), higher-lift mods (winglets, leading-edge slats), prop advances/feathering upgrades, fishing pole (minigame), GPS/autopilot functions, map reveal markers.
- Wildlife rescue -- *humor hook, v1 sketch*. A moose/bear physically extracts the plane from woods or off a boulder after a bad landing. Fun, mysterious, non-financial. Expanded under recovery-by-proximity -- see [Damage & Wear § Recovery by Hub Proximity](#recovery-by-hub-proximity).
- Damage gating -- *non-blocking*. Damage does not stop marginal essential gameplay; repair is required once player reaches a capable hub.
- Multiple aircraft -- *No Man's Sky-style*. Single aircraft is primary; trade or trade+credits to swap. At complex hubs, hangar storage retains individual aircraft with all mods and settings. Barn-find / severely damaged aircraft can be trailered in for cost or time, then repaired -- fuels a gameplay loop. See [Hangar Storage & Fleet](#hangar-storage--fleet).
- Livery/paint -- *in v1, minimal*. Simple rotation + 2D brush on currently viewable texture surface. Few bitmap texture options. Fits cel shading; supports player recreations of real or iconic media liveries.
- Time of day -- *15-20 min day, 5 min night default*. Adjustable slower if visuals justify. Cap gameplay to daylight in v1 but plan night flights and "camp until morning" anywhere parked.
- Night flight -- *post-v1 but with bold aesthetic target*. Night rendering aims at an astrophotography-grade sky (slowly rotating Milky Way band, constellations, deep-sky objects) compatible with cel shading. Aurora as rare weather accent. See [Night Sky & Astrophotography](#night-sky--astrophotography).
- Fourth region identity -- *all four candidates retained; high-desert favored*. Personal affinity (author lives in high desert). Final selection happens during Phase 2 playtesting; others may still ship in post-v1 region packs.
- Pure wildlife runs -- *folded into main mission list* with distinctive iconography and filter. Test in playtesting.
- Seasonal cycle -- *v2, optionally enabled via dev controls in v1*. Static-per-region is the v1 default; cycling becomes a dev/debug toggle and full gameplay feature post-v1.
- Pet / companion -- *acceptable later*. Not v1. Could tie into fishing-pole companion or wildlife-mission progression rewards (e.g., a husky joins at a trapper cabin after a certain wildlife rep threshold). Must not gate flight gameplay.
- Fishing minigame scope -- *simple with light timing and numerical skill progression*. Rare catches gate cosmetic rewards and "message in a bottle" task hooks. See [Fishing & Side Activities](#fishing--side-activities).
- Shared seeds -- *post-v1 feature, possible multiplayer/co-op hook*. If procedural path is chosen, seeded worlds can be shared; same-seed players could fly simultaneously in a future co-op mode. v1 is single-player.
- Shareable flight logs / replays -- *post-v1 feature*. The flight log is the saved game: a human-readable, templated event log of everything contributing to save state. Replay reconstructs state deterministically. See [Flight Log & Save Format](#flight-log--save-format).

**Still open** (not yet decided; listed again at [Remaining Open Questions](#remaining-open-questions)):

- World scope (single large map vs. 4 regional maps vs. procedural vs. fresh-per-session). Prototype and measure in Phase 1. Additional consideration: *if* procedural is consistently good and appealing, persistence may not be necessary at all -- fresh each session could be a design identity, not a limitation.
- Procedural generation cost model / "flight-log memory" feasibility -- prototype and measure.
- Helicopter support -- flagged as probable non-goal, open to revisit with clear justification.

---

## World Scope & Map Layout

**Candidate A: one large single map with diverse regional locales**

- Single contiguous landmass + water covering multiple recognizable seaplane/floatplane/bushplane regions as neighboring biomes.
- Incorporates "interesting features of typical locales" -- Alaskan bays, fjords, boreal lakes, Mediterranean-style archipelagos, mountain lakes, forest corridors, high country -- stitched as neighboring regions rather than separate worlds.
- Pros: one economy, one progression, one mental map, continuous flight possible edge-to-edge, natural support for long delivery routes.
- Cons: large single-map authoring burden; risk of repetitive biome padding; more aggressive streaming/LOD work.

**Candidate B: ~4 separate regional maps**

- **Region 1 -- Alaska-like**: lakes, bushplane strips, coastline, ski-planes, snow as a surface type, floatplanes on deep snow / powder water patches (open question: feasibility of "powder snow as water-like soft surface" for floats -- low-friction plowing, spray shader). Forest Service parallels may apply here too.
- **Region 2 -- Mediterranean-like archipelago**: seaplane-forward, docking-heavy, many small islands, open ocean vs. protected sea. Keep open-water segments interesting -- weather patches, swells, shipping traffic, sea wildlife, distant storm bands.
- **Region 3 -- Mountain / lakes (Rocky Mountain / Alpine-like)**: high-altitude lakes, narrow valleys, density-altitude effects, fire-hopping missions supporting a forest-service analogue. Water-drop/tanker configurations are a strong fit here.
- **Region 4 -- Fourth combination TBD, multiple strong candidates**:
    - **Pacific Northwest**: rain, fog, dense forest, fjords, logging camps, lumber-mill docks, salmon-smoking canneries.
    - **Subarctic / North Atlantic**: basalt coasts, stormy sea, heavy fog, reindeer/puffin wildlife, geothermal steam plumes, midnight sun in summer, aurora in winter.
    - **Tropical island chain**: turquoise reefs (visual + flight-height awareness), squalls, trade winds, coconut-palm atolls, protected lagoons as easy landings, hurricane season.
    - **High Desert / Plateau** (*currently favored; author lives in this region, personal affinity and reference material on hand*): dust devils, heat shimmer, dry-lake landings, thunderstorm anvils on the horizon, dust/leaf debris events, mesa-top strips, sparse remote communities, dark-sky observing sites that tie into the astrophotography night-sky feature.
    - Pick final identity during Phase 2/3 playtesting; others may still ship in post-v1 region packs.

**Procedural-generation sub-option (orthogonal to A vs B)**

- Instead of full hand-authored maps, generate terrain + hubs procedurally from a seed.
- Risk: cost of retaining generated terrain / hub state between sessions and between flights.
- Mitigation idea: **flight-log memory** -- only persist state for regions the player has flown over, landed at, or interacted with. Un-flown regions are regenerated on approach from the same seed. State-size grows with exploration, not with world size.
- Realism check: "may also consider ... and maybe it wouldn't be that bad" -- cost is not prohibitive if persistence is bounded by play, not by world extent.
- Compatibility: flight-log memory could apply to Candidate A (unexplored parts of the big map are dormant) or Candidate B (each region auto-populates procedurally from a seed).

**Candidate C: fresh-per-session, no persistence (roguelike-adjacent)**

- Every new session generates a new world from a new seed; terrain and hub locations reset between play sessions.
- Player identity persists (fleet, money, upgrades, liveries, character, reputation rollover possible).
- Hub *kinds* persist in identity and feel even if their locations change (a "fishing camp near a trout lake" still exists somewhere; finding it is part of the session loop).
- Pros: zero persistence burden, each session feels fresh, naturally supports long-term replay, sidesteps the "is my home airport here?" emotional risk by making exploration the point.
- Cons: loses "your Alaska" familiarity, makes long-term barn-find arcs harder, complicates mission chains that assume persistent geography.
- User position: "maybe it's fine to generate anew every time, if the experience is consistently good and appealing." Preserve as a live candidate.

**Candidate D: hybrid (persistent hubs + procedural surroundings)**

- Hub locations, main routes, and immediate surroundings (~5-10 km radius) are persistent (authored or seed-locked per save).
- Distant terrain regenerates from seed on approach via flight-log memory.
- Pros: player keeps "home base" familiarity while most of the world stays fresh; persistence footprint stays bounded.
- Cons: biome continuity at the persistent/procedural seam needs care.

**Decision is not yet made.** Capture pro/con, prototype the top two candidates in a Phase 1 scratch branch (likely Candidate C and Candidate D), revisit before Phase 2 gate with measurements for:

- Terrain generation cost (CPU, GPU, allocation pressure) per chunk.
- Persistence file size per hour of play.
- Subjective replay appeal after 2-3 "fresh" sessions.
- Mission-chain legibility under each model.

---

## Regional Biomes

Each biome is a bundle of `{palette, flora, fauna, surface types, weather profile, hub archetypes, wildlife, soundtrack flavor}`. Specifically planned, with explicit real-world references where the user has authored a locale target:

- **Coastal Alaska / Canadian boreal** (**baseline** — most of flowPlan evokes this): tundra + forested coast + interior bush + mountainous plains. Tundra tires on gravel strips, ski/float swaps seasonally, grizzly / moose / caribou / wolf wildlife, cold-start effects, long twilights, salmon-run motif. **Stunning alpine lakes, cliff regions, tons of remote recreational landing options** — the bush-pilot ideal.
- **Boreal lake country (Canadian shield / Finnish lakes analogue)**: innumerable small lakes, pine forests, low-density hubs, long shoreline docks, aurora seasonally (visual treat).
- **Mediterranean archipelago + inland seas**: turquoise water, white/terracotta ports, dense island clusters, large coasts, inland ocean crossings, limited hub services between majors, clear visibility baseline punctuated by mistral-like wind events. Blends naturally with coastal Alaska.
- **Pacific Northwest**: dense fog, low cloud, heavy rain, fjords, logging camps, lumber mill docks, salmon-smoking canneries.
- **Rocky / Alpine**: density-altitude, short lake windows, narrow valley flying, fire-hopping, water-bomber drops, glacier-dammed lakes.
- **Subarctic / North Atlantic**: basalt coasts, stormy sea, heavy fog, reindeer/puffin wildlife, geothermal steam plumes, midnight sun in summer.
- **Tropical island chain**: reefs (visual, flight-height awareness), squalls, trade winds, coconut-palm atolls, protected lagoons as easy landings.
- **High Desert / Plateau** (*currently favored Region 4 candidate; author personal affinity — reference locales on hand*): think **Grand Mesa** (isolated flat-top massif rising from the plateau), **Flatirons** (tilted sedimentary slabs against a grassland front), **Arches** (red-rock formations, natural stone spans), scrubby plains, rattlesnake canyons, rivers cutting through rock, **railroads** (iconic industrial signature), rolling dunes / hills, dust devils, heat shimmer, dry-lake landings, thunderstorm anvils on horizon, leaf/dust debris events, **dark-sky observing sites** (ties into astrophotography night-sky feature).

Each biome contributes to the shared feature taxonomy below (surface types, weather regimes, wildlife spawns, hub archetypes).

### Sub-biome feature libraries (pinned architecture)

Biomes are not monolithic. Each parent biome declares a **sub-biome feature library** — specific micro-character feature types that fill the biome's territory as mosaic cells:

- **Coastal Alaska / Canadian boreal** sub-biomes: `lake_cluster | alpine_valley | dense_spruce_forest | rocky_ridge | tundra_patch | river_floodplain | bush_strip_clearing | glacier_tongue | fjord_entrance | salmon_run_river | cliff_coast`.
- **High Desert / Plateau** sub-biomes: `mesa_top | arch_formation | slickrock_canyon | sagebrush_plain | dry_wash | dune_field | dry_lake_bed | rail_corridor | scrub_valley | juniper_stand | slot_canyon`.
- **Mediterranean archipelago** sub-biomes: `protected_cove | rocky_islet | terracotta_port | pine_headland | sandy_beach | small_harbor | island_strait | coastal_plain`.
- **Pacific Northwest** sub-biomes: `rainforest_valley | clear_cut_hillside | logging_camp | fjord | river_delta | old_growth_stand | salmon_cannery_waterfront`.
- **Rocky / Alpine** sub-biomes: `alpine_tarn | ridge_traverse | cirque | narrow_valley | talus_slope | glacier_dammed_lake | fire_burn_area | avalanche_chute`.
- **Subarctic / North Atlantic** sub-biomes: `basalt_coast | geothermal_steam | stormy_headland | reindeer_plain | puffin_rookery | northern_lights_corridor`.
- **Tropical island chain** sub-biomes: `reef_lagoon | coconut_atoll | sandbar_chain | coral_rim | protected_bay | volcanic_cone`.

Each sub-biome feature is a first-class Voronoi-cell unit (see [MAPGEN.md § Chunk model](MAPGEN.md#chunk-model) pinned architecture). It carries its own geometry rules, palette, flora, fauna tables, hub-placement compatibility, and inter-biome adaptation rules. Authoring these libraries is the core design work of Phase 3.6.3.

### Sub-biome feature overlap with palette adaptation

Sub-biome features can **overlap into neighboring parent biomes** — producing organic, natural-feeling transitions without hard biome boundaries:

- A lake-cluster sub-biome (home: Alaskan boreal) might appear in a sheltered high-desert valley with a microclimate water source. Its palette **shifts toward the host biome's atmosphere** — the water stays turquoise but reads against warm red-rock rather than cool evergreen.
- A pine-headland sub-biome (home: Mediterranean) could appear at the dry edge of a Pacific Northwest region with its palette desaturated and greys shifted warm to match.
- A fjord-entrance sub-biome (home: subarctic) could appear in a cold-biased Alaskan region with nearly-home palette (the two biomes are close in climate-gradient space, so adaptation is mild).

**Adaptation mechanism:** each sub-biome feature declares `{home_palette, adaptation_rules_per_host_biome}`. When placed in a host biome, the palette interpolates between home and host-adapted values, weighted by climate-gradient distance from the feature's origin biome. The result is **gradual natural shift** rather than a hard boundary. Detail in [MAPGEN.md § Biome system](MAPGEN.md#biome-system).

### Biome adjacency rule (structural — enforced by the generator)

Biome transitions must be **climate-gradient coherent**. Two biomes can be adjacent iff there's a plausible climate gradient between them.

**Forbidden transitions:**
- snow / tundra ↔ dry desert
- snow / tundra ↔ tropical archipelago
- high desert ↔ temperate archipelago without a buffer biome
- any "Category A ↔ Category C" without a Category B buffer between them

**Acceptable transitions:**
- alpine → tundra → boreal forest → temperate forest → plains
- coastal boreal → coastal temperate → Mediterranean coastal
- Rocky alpine → high-desert plateau (geographically real in many mountain-basin interfaces)

This is a generator-level constraint, not aesthetic polish. Encoded as a biome-adjacency graph; biome placement rejects forbidden-pair adjacencies at bake time. Detail in [MAPGEN.md § Biome system](MAPGEN.md#biome-system).

---

## Map Memory / Cartography

Rendering + data for the map overlay live in [Hubs & Landing Areas § 2D hub map](#2d-hub-map-hotkey-accessible-navigation-overlay). This section covers the **progression + memory + discovery** side — what's visible and when.

- **[sketch]** Flight log: every flight recorded as seed + input trace + events. Enables replay, deterministic "highlight reel" playback, and shareable-route artifacts.
- **[sketch]** Exploration map: regions/hubs/landmarks the player has personally seen are drawn; unexplored regions are fogged or sketchy on the 2D hub map.
- **[sketch]** Map reveal markers: unlocks via debt-funded upgrades (GPS, cartographer contract), mission rewards, wildlife encounters. Each unlock annotates a subset of the world-bake hub catalog with `discovered = true` so the 2D map can render them.
- **[sketch]** Hand-drawn cel-shaded map aesthetic: matches the art direction, not a UI satellite image. Vector-drawn per [UI Direction](#ui-direction) — no bitmap map-tile imagery.
- **[sketch]** Bookmarks: player can pin locations, add annotations. Pinned on the 2D hub map + surfaced as HUD waypoint markers in-flight.
- **[sketch]** Persistence support for procedural generation: only discovery + annotation deltas persist on disk. The hub catalog + hydrology graph regenerate deterministically from master seed; deltas apply on top.
- **[sketch]** Logbook: per-flight summary (route, weather, cargo, incidents, wildlife sighted, photos taken), auto-authored from the flight log, becomes a keepsake. Cross-linked to the 2D hub map (hover a hub → surface that hub's logbook entries).

### Discovery + reveal tiers (progressive unlock)

- **Undiscovered:** hub is **not shown** on the 2D map. Exists in the catalog but invisible to the player.
- **Hinted:** hub shows as a `?` at approximate coords if a mission, radio transmission, or NPC dialog references it.
- **Seen:** flown over or within a tier-dependent observation radius. Tier shows; name may still be omitted.
- **Visited:** landed / taxied. Full hub details — name, services, runway headings, aircraft-parked-here history.
- **Home:** designated as home hub (tier 1 or 2 only). Special iconography + free hangar / free livery edits / loyalty bonus.

### Cartographer-unlockable reveals

- **Regional chart pack (purchase at tier-1 cartographer vendor):** reveals all tier-1 + tier-2 hub locations in a chosen region. Names + tier iconography, services marked but not detailed.
- **Detail chart (per-hub):** upgrades a "seen" hub to "visited"-equivalent detail without actual landing.
- **Terrain contour overlay:** reveals coarse topography + hydrology at higher zoom. Useful for route planning through mountainous regions.
- **Forecast overlay:** shows current weather at discovered hubs (aligns with 3.2 weather forecasts). Decays with distance.

These unlocks are stored as save-format deltas — the underlying hub catalog is unchanged.

---

## Terrain & Static Landscape

### Phase 2 state (implemented)

- **[shipped: Phase 2.2]** Sum-of-sines procedural heightmap (`flowplan-world::Terrain`) with 5 octaves (regional, coarse, mid, fine, detail) at mountain scale (±275 m max). Beach flattening post-process (`raw * smoothstep²(|raw|/120)`) compresses shore heights. 5-tier color ramp: sand / field / forest / rock / snow.

### Phase 2 known limitations (to resolve via Phase 3+ terrain rebuild)

- **Sum-of-sines cannot independently tune amplitude and gradient.** Shores rise steeply because the regional octave has large vertical amplitude over short horizontal distances. Height-based flattening can squash vertical extent but cannot stretch horizontal extent, so gentle beach flats with tall mountain backdrops (per ref1/2/3 photos) aren't achievable without a different representation.
- **No real hydrology.** Water sits at a single fixed y=0 mean level; no lakes at different altitudes, no rivers, no drainage patterns. Valleys never collect water realistically; a lake basin is just "area where terrain happens to dip below zero".
- **No shore classification.** Beach, cliff, rocky headland, estuary, delta all look the same (whatever height+color the sum-of-sines produces). Hub placement can't distinguish safe float landing zones from surf-prone exposed coasts.
- **Repetitive look at distance.** 4 km patch tiled procedurally reveals the sinusoidal basis when flown over for long.

### Future terrain design (Phase 3.6 target)

Phase 3.6 Terrain rebuild + hydrology is the target milestone. Full design detail is in [MAPGEN.md](MAPGEN.md). Summary of what lands there:

- **[ready: Phase 3.6a]** Proper terrain representation that decouples amplitude and gradient — ridged noise plus domain warping, biome-parameterized. Enables genuine beach flats adjacent to mountain backdrops.
- **[ready: Phase 3.6b]** Hydrological realism via a world-bake hydrology graph: rivers carved along drainage-network gradients, lake basins at local minima, coastlines at sea level, river-continuity invariants at chunk boundaries, rivers grow into lakes. Waterfalls at elevation jumps. Each water body has its own surface level, chop behavior, palette.
- **[ready: Phase 3.6c]** Shore classification: `sand_beach | gravel_beach | rock | cliff | mudflat | delta | ice_edge`, driving hub placement, landing feasibility, spray / foam / splash behavior, and wildlife spawn tables. Exposes `world::surface_at(x, z) -> SurfaceClass` which **unblocks [Milestone 2.5b.7 per-surface friction](ROADMAP.md#milestone-25-ground--water-contact-physics)**.
- **[ready: Phase 3.6c]** Heightmap base with vector overlays for rivers, lakes, coastlines, ridges (per MAPGEN.md baseline representation).
- **[ready: Phase 3.6c]** Placed landmarks for dead-reckoning: mountains, towers, lighthouses, wrecks, bridges, unusual rock formations (**arches, mesas, flatirons** for high-desert per biome-specific references), logged cut-blocks, river bends, waterfalls.
- **[ready: Phase 3.6d]** Biome-specific flora density and palette that feed cel shading.
- **[ready: Phase 3.6b]** Erosion-shaped terrain features near water (alluvial fans, river deltas, sandbars) supporting bush landings on suitable sandy surfaces. Large sandbar / sand-beach coastline aesthetics explicitly in scope.
- **[ready: Phase 3.6b]** Water-body classification: `lake_small | lake_large | river | protected_bay | open_coastal | open_ocean | alpine_tarn | glacial`. Drives chop behavior, temperature, visual palette, and wildlife spawns. Small aesthetic water features (creeks, streams, waterfalls, sandbars, inland ocean crossings) emerge from graph + shore classification as feature types.
- **[ready: Phase 3.6d]** Snow-covered terrain variants per season for applicable biomes (altitude-gated; respects biome adjacency rule so transitions stay climate-gradient coherent).
- **[sketch]** Caves, gorges, canyons — limited, for low-pass exploration and mission routing. Mesh vs heightfield-hole representation TBD.
- **[sketch]** Build-time baking of all static meshes; runtime streaming only for distant LOD swap. Chunk model (square vs hex vs Voronoi) is an [open question in MAPGEN.md](MAPGEN.md#chunk-model).

Each sub-feature is potentially a considerable undertaking at its own time; Phase 3.6 is sized accordingly (candidate for splitting if it grows).

---

## Hubs & Landing Areas

Hubs are the narrative and economic anchors of the world. flowPlan uses a **5-tier population model** that drives every other property: layout, runway count + length, hangar capacity, mission count, available aircraft for trade, amenities, and whether the hub supports hangout / multiplayer-chat / emote interactions. Each tier has a distinctive **feel** that the player learns to read at a glance.

### Unified data model

- **[sketch]** Each hub declares `{tier, surfaces, services, vendors, mission sources, wildlife nearby, runways, aircraft parking slots, biome, water-adjacency}` as data-driven RON.
- **[sketch]** **Population tiers:** `major | regional | local | remote | isolated`. Tier drives available services and layout.
- **[sketch]** **Surface types:** `grass_field | gravel_strip | paved_runway | dirt_road | dock | beach | lake_ramp | amphib_combo | snow_strip | ice_runway | floating_platform`.
- **[sketch]** **Service types:** `refuel | repair_minor | repair_major | resupply_basic | resupply_specialty | upgrade_vendor | hangar | mission_board | avionics | paint_shop | barn_find_trailer | cartographer | mooring_manual`.
- **[sketch]** **Runway naming convention:** runways use **real-world degree/360 numbering** (e.g. runway **09/27** aligns with magnetic heading 090°/270°; runway **36** is due north). Parallel runways are suffixed `L` / `R` / `C` (e.g. `18L` / `18R`). **No fictional numbering — this is pilot-facing data and must be authentic.** Runway orientation stored as magnetic heading in degrees; the display digit pair derives from it. Any hub-generator or authoring tool rejects runway data whose numbering doesn't match its heading.
- **[sketch]** **Water/land duality rule:** any hub adjacent to a usable water body (lake, inland sea, river wide enough to land) **must provide water-entry/exit conversion services** — dockside refuel for floatplanes, a ramp or beach for amphibs to transition gear, mooring options for temporary tie-up. The inverse holds: hubs with runways that are also near water expose both surfaces as first-class landing options. The data model prevents the "seaplane base with no way to leave the water" degenerate case.
- **[sketch]** **Hangout / chat / emote support** scales with tier (see per-tier details below). Target: v1 ships single-player hangout affordances (emote at a campfire, sit at a dock, park-and-look-around); live-chat / multiplayer lobby belongs to [Multiplayer stretch phase](#multiplayer-stretch-goal-post-v1).
- **[sketch]** **Mission count** scales with tier: 7–15 (major) → 5–10 (regional) → 3–7 (local) → 1 (remote) → rare-and-special (isolated). Each mission is procedurally parameterized from world state (weather, cargo, season, reputation, time-of-day) per [Missions & Tasks](#missions--tasks). Mix includes `destination | round-trip | one-way | supply-chain-leg`; larger hubs tend toward heavier loads and longer distances.
- **[sketch]** **Aircraft parking slots** are hub features: each has a `{position, size_class, type_constraint, rarity}` — and the slot state (which aircraft currently sits there, or empty) is deterministic per seed + day-of-year + RNG roll. Larger hubs host more diverse + larger aircraft; remote hubs may have nothing parked at all; isolated hubs occasionally hide a barn-find.

### Tier 1 — Major hubs (large, multi-function, social)

- **[sketch]** **Layout:** structured but variable. Baseline: two large square/rectangular hangars, an office building, a medium/small control tower (still remote locale — flowPlan does not ship international airports), ancillary structures (maintenance shed, fuel farm, weather station).
- **[sketch]** **Runways:** 2–3 of varying length. Longest accommodates the largest aircraft in the catalog (flying boats, Twin Otter, Caravan, Pilatus PC-12, water bombers). Mix of paved and gravel / grass for character variety. Parallel runway option (suffixed L/R) when airspace / terrain allows.
- **[sketch]** **Water access:** typically large lake or inland sea adjacent, depending on biome. Full seaplane base tier (docks, wet hangar, mooring buoys, beach / ramp for amphibs).
- **[sketch]** **Services:** full — refuel, repair_minor, repair_major, resupply_basic, resupply_specialty, upgrade_vendor, hangar (owned-aircraft storage, home-hub designation), mission_board, avionics, paint_shop, barn_find_trailer, cartographer.
- **[sketch]** **Aircraft parked / available:** RNG-gated each day. **Always** 2–3 small aircraft present (sometimes for trade, sometimes just as ambient detail). **Rarely** 2 medium aircraft. **Very rarely** 1 large aircraft. Parking slots can be blank depending on RNG roll — the world feels alive because the fleet moves.
- **[sketch]** **Missions:** 7–15 available, refreshing on a slow cadence. Supports larger distances and heavier loads. Supply-contract legs (major ↔ remote) originate here. Wildlife-mission category integrated into the board with iconography + filter.
- **[sketch]** **Hangout / social:** the tier with the most social affordances — seating areas near the office, a fuel-pump waiting bench, walkable taxiway edges, dockside. In single-player this enables pause-and-enjoy vignettes; in the Phase 6 multiplayer stretch-goal this is where the text-chat lobby + callsign-overlay experience is richest.
- **[sketch]** **Vendors:** all types present. Paint shop anchored here (see [Livery & Customization](#livery--customization)). Cartographer sells regional chart packs.

### Tier 2 — Regional hubs (medium, working, character-rich)

- **[sketch]** **Layout:** 2–3 quonset-hut hangars (iconic curved-roof bush-pilot architecture), office with antenna / radio tower but **no control tower** (the tier's distinctive feature). Firewatch tower optional where biome calls for it (forested regions, fire-season Rocky).
- **[sketch]** **Runways:** one runway, sometimes dirt; maybe two when parallel geometry works. Length accommodates small and most medium aircraft but not the largest cargo.
- **[sketch]** **Water access:** often adjacent to rivers or large / medium lakes. Dock + float refuel where applicable. Ramp for amphib gear-up/down.
- **[sketch]** **Services:** most services present except `repair_major` (field-repair tier only), `avionics` (have to fly to a major for glass-panel install), `paint_shop` (not here), `barn_find_trailer` (only majors arrange trailer transport).
- **[sketch]** **Aircraft parked / available:** **1–2 small aircraft typically**, 2–3 small occasionally. **Rarely** 1 medium aircraft. No large aircraft. Blank slots acceptable on RNG roll.
- **[sketch]** **Missions:** 5–10 available. Mix leans toward local delivery + regional supply + passenger transfer. Some wildlife runs origin here.
- **[sketch]** **Hangout / social:** less structured than major hubs but still supported — sit near the office, inspect the firewatch tower, dock-side bench.

### Tier 3 — Local hubs (small, rural, approachable)

- **[sketch]** **Layout:** no hangar building; one or two tie-down spots, a small shed / office combo, a fuel pump, sometimes a wind sock and nothing else. Usually adjacent to a dirt road wide and flat enough for small / medium geared aircraft takeoff (the road IS part of the takeoff surface in some designs — classic Alaska / Canadian bush pattern).
- **[sketch]** **Runways:** dirt / grass / gravel primary. Often shares ground with the adjacent road.
- **[sketch]** **Water access:** adjacent to small or medium lakes, rivers. Seaplane operations supported where geometry allows.
- **[sketch]** **Services:** basic — refuel, resupply_basic, mission_board, mooring_manual. No hangar storage, no specialty services.
- **[sketch]** **Aircraft parked / available:** usually **1–2 small aircraft** available for trade or purchase. Very rarely medium. Slots can be blank.
- **[sketch]** **Missions:** 3–7 available. Mostly short-haul delivery, message/mail, livestock / supply drops, occasional rescue.
- **[sketch]** **Hangout / social:** minimal infrastructure but a chair or two near the office; the dirt-road-as-runway gives a distinctive low-ceremony feel.

### Tier 4 — Remote hubs (scenic, exploration-forward, mostly bush/float)

- **[sketch]** **Layout:** single structure or cluster (bait shop lakeside, campground, ranger cabin, fishing camp, logging camp, research station, fire-fighter forward operating point, small cannery). No hangars, no formal runway markings. The **place itself** is the hub.
- **[sketch]** **Landing surface:** clearing, beach, short grass strip, lake, river. Typically bushplane-specific or floatplane-specific. No paved anything.
- **[sketch]** **Water access:** often the primary surface. Manual mooring (dockside or shoreline tie-down) rather than a formal seaplane base.
- **[sketch]** **Services:** none formal. At most a small supply cache (food / water / fuel can) or nothing at all. `mooring_manual` is the hub's defining affordance.
- **[sketch]** **Aircraft parked / available:** **occasionally (rarely)** 1 aircraft moored or parked for trade/purchase. Most days, nothing. The rare aircraft when it appears is often unusual (bushplane variant, vintage airframe).
- **[sketch]** **Missions:** exactly **1 task or mission** available — exploration-driven. Often wildlife-runs (commune with bears at a natural clearing, log an orca pod from a cove, photograph a rookery at dawn), or delivery to the remote camp's occupants.
- **[sketch]** **Discoverability:** **many remote hubs are uncharted** until flown over, referenced by a radio transmission, or unlocked by a mission hint. Cartographer upgrade reveals known remote hubs on the map.
- **[sketch]** **Hangout / social:** **the gameplay point of these destinations.** Must be **scenic** — an interesting reason to fly there. Campground benches, dockside fishing spots, natural clearings with wildlife, lookouts over a fjord, hot springs, abandoned lighthouse. Player lands, manually moors / ties down / taxis to a sensible park spot, disembarks, sits / emotes / explores on foot. In Phase 6 multiplayer, small groups can choose to meet here for the atmospheric payoff.
- **[sketch]** **Wildlife angle:** remote tier has the most concentrated wildlife spawns. Bears at a fishing camp, moose near a salt lick, eagles at a rookery, whales past a headland. Wildlife-run category often culminates at remote hubs.

### Tier 5 — Isolated hubs (uncharted, barn-find territory)

- **[sketch]** **Layout:** nothing formal. Abandoned airstrip overgrown with grass, a derelict fishing cabin, a forgotten logging-road clearing, a pristine cove with no human presence, a high-alpine tarn only reachable in calm weather.
- **[sketch]** **Landing surface:** whatever geometry permits. Often bushplane-only; sometimes float-only; sometimes requires a specific aircraft configuration (STOL, tundra tires, amphib).
- **[sketch]** **Water access:** typically yes — these hubs exist because a remote lake / cove / river segment is usable. Float / amphib is the common access method.
- **[sketch]** **Services:** none.
- **[sketch]** **Aircraft parked / available:** **barn-find tier.** Rare antique / abandoned aircraft may be present — only discoverable by landing here, and sometimes only reachable *by* a specific aircraft (e.g. a Super Cub can reach an alpine tarn that a Beaver cannot). Discovery triggers a barn-find mission: arrange trailer transport (from a complex hub via `barn_find_trailer` service) back to a hangar for restoration.
- **[sketch]** **Missions:** no formal board. The hub itself is the reward — logbook "moment," cosmetic unlock, cartography annotation, rare-aircraft discovery.
- **[sketch]** **Discoverability:** always uncharted until discovered. No radio hint, no mission reference. Pure exploration payoff. May be referenced in rare message-in-a-bottle catches (see [Fishing](#fishing)) or wildlife-mission dialog fragments.
- **[sketch]** **Hangout / social:** pristine environments. The whole tier is a "found this, it's mine now" feeling. Multiplayer sessions visiting an isolated hub create shared-discovery logbook entries.

### Cross-tier conventions

- **[sketch]** **Fire-hopping forward bases** (Rocky / Colorado biome) are tier-3-or-4 in layout but with a specialized `water_retardant_refill` service. Short strip near active fires; used during fire season community events.
- **[sketch]** **Dock variants at tiers 1–4:** private, commercial, municipal, marina, fishing camp, research dock — visual + narrative flavor, not mechanical difference.
- **[sketch]** **Hub-specific markets:** prices for fuel, supplies, repairs vary by tier and remoteness. Tier 1 = lowest fuel prices (volume discount); tier 5 = no market at all; tier 4 may have a single overpriced fuel jerry can.
- **[sketch]** **Inter-hub relationships:** supply contracts between major and remote hubs generate persistent delivery demand. Completing these shifts the community state (grocery stocked, clinic supplied, fire-watch tower reopened).
- **[sketch]** **Home-hub designation:** player picks one tier-1 or tier-2 hub as "home" — free hangar, free livery edits, loyalty bonus, social affordances unlocked first here.
- **[sketch]** **Tier doesn't imply location fixedness under procedural generation.** Tier + biome + hydrology context determine where a hub of a given tier *can* spawn (a tier-1 major requires water adjacency + flat ground for the long runway + population nearby). The generator places hubs at terrain-plausible coordinates; the tier specifies what goes *at* those coordinates.
- **[sketch]** **Hub content authoring vs procedural:** tier-1 and tier-2 hubs are strong candidates for **authored layouts with procedural neighbors** (per MAPGEN.md Option 2–3). Tier-3 and tier-4 hubs are mostly procedural with authored structure libraries (pick a quonset hangar + pick an office + place a firewatch tower). Tier-5 isolated hubs are purely emergent — the hydrology graph + terrain + discovery rules produce them.

### Runway authoring rules (reminder)

Runway numbering is **pilot-facing** data. These rules are non-negotiable:

- Heading stored as magnetic bearing in degrees (0–359).
- Display number = round(heading / 10) with `0` → `36`. E.g. heading 91° → runway `09`; heading 270° → runway `27`.
- Both-ends numbering = heading + 180° (modulo 360) → opposite display. Runway `09/27` is the same physical strip; which display the pilot uses depends on approach direction.
- Parallel runway suffixes `L/R/C` (e.g. `18L` / `18R`) only when parallel runways actually exist.
- Hub RON authoring tool validates that display numbering matches heading. Mismatches are authoring bugs.
- Magnetic variation is biome-dependent at world-bake time (more accurate over vast procedural worlds). Held deterministic per master seed.

### Hub reference & late binding (world-wide navigation)

Missions reference hubs anywhere in the world — often 50–200 km from the aircraft, far outside the streaming disk's 8–22 km range. But Voronoi cells only generate inside the streaming disk, so the destination cell may not exist yet at mission-acceptance time. Design pattern:

- **[sketch]** **World-bake hub catalog.** At world-seed load, a deterministic bake pass produces a full hub catalog (~50–80K hubs for a 1000 × 1000 km world): `{HubId, approx_world_pos, tier, parent_biome, required_surfaces, display_name, region_id}`. Catalog is stable per master seed, ~15 MB in memory, persisted alongside the hydrology graph.
- **[sketch]** **Approximate coordinates ("fuzz").** Hubs commit approximate world-coords at bake time — accurate to Voronoi-cell scale (~2 km) but not meter-precise. Mission systems use these coords for distance + fuel + routing calculations. Great-circle distance between hubs is sufficient for flight planning.
- **[sketch]** **Streaming-time resolution ("resolve").** When a hub's approximate coords fall into the streaming disk, the Voronoi generator snaps the hub to the nearest compatible cell (by `required_surfaces` × sub-biome feature type). If the nearest cell mismatches, the resolver walks Voronoi neighbors to find a compatible one. Exact coords cached once resolved.
- **[sketch]** **Pattern generalizes.** Same fuzz+resolve applies to wildlife zones, mission anchors, barn-find aircraft, landmark references — any global reference to an eventually-generated local feature. See [MAPGEN.md § Hub late-binding (fuzz + resolve)](MAPGEN.md#hub-late-binding-fuzz--resolve) for full detail.

### 2D hub map (hotkey-accessible navigation overlay)

A player-facing 2D map overlay for route-planning, destination awareness, and mission context. Hotkey-toggled (typical: `M`), available both in-flight and on-ground. Unifies the placeholder "route planning UI stub" in Milestone 4.1 + the "hand-drawn cel-shaded map view" in Milestone 4.8.

- **[sketch]** **Hotkey toggle:** `M` opens/closes the map overlay. Held / tapped modes both supported (tap to pin open, hold to peek). Pauses nothing — sim keeps running during map view.
- **[sketch]** **Rendering:** **cel-shaded, hand-drawn aesthetic, vector-drawable** (see [UI Direction](#ui-direction) — no bitmap satellite imagery; same rendering primitives as the debug orientation widget + attitude indicator).
- **[sketch]** **Data source:** the world-bake hub catalog + flight-log-memory discovery flags + cartographer-unlock progression. No streaming-cell dependency — the map works for hubs anywhere in the world whether their Voronoi cells have been generated or not.
- **[sketch]** **Hub display:**
    - Tier-differentiated iconography (big airport symbol for tier-1; dock for seaplane base; simple dot for remote; `?` for hinted-but-undiscovered).
    - Discovered hubs named; undiscovered hubs anonymized or omitted per cartographer-unlock state.
    - Hover / selection surfaces hub details (services, runway headings, tier, services-available icons).
- **[sketch]** **Player + destination display:**
    - Current aircraft position + heading (cel-shaded arrow or small plane icon).
    - Mission destination highlighted with a route line (great-circle from current position, wrapped to world bounds if candidate-D-hybrid procedural surroundings).
    - Active mission waypoints drawn as pinned markers.
    - Fuel-range ring: circle around current position showing how far the aircraft can fly with current fuel + wind.
- **[sketch]** **Pan / zoom:**
    - Mouse / gamepad zoom and pan.
    - Zoom range: region-level overview (~500 km across) down to local-detail (~5 km across showing individual docks and runways at discovered hubs).
    - Snap-to-aircraft hotkey to re-center on the player's position.
- **[sketch]** **Biome shading on the map:**
    - Hand-drawn-feel biome tinting (colored regions at low zoom; palette-adapted per [Voronoi sub-biome-feature overlap](#regional-biomes)).
    - Hydrology graph overlay (rivers as lines, lakes as fills, coastlines as outlines) — drawn directly from the pre-baked hydrology graph data, so the map is accurate even for unflown regions.
    - Terrain contour hints at higher zoom, derived from coarse global heightfield (the same data the hydrology bake used).
- **[sketch]** **Player annotations:**
    - Pin custom waypoints (long-press / right-click). Annotated with text note; shows on the map and as a HUD marker while in flight.
    - Bookmark favorite hubs for quick-return.
- **[sketch]** **Logbook cross-references:**
    - Hover a discovered hub → shows logbook entries captured there (first-landing date, wildlife sightings, mission completions).
    - Highlights recent flight paths (trail of last N flights fading with time).
- **[sketch]** **Performance:** since the map reads from the pre-baked hub catalog + hydrology graph (~15 MB + ~100 MB in memory, already resident), rendering is purely a vector-drawing pass. Per-frame cost at map-open scales with on-screen hub count (culled by zoom level). Off-screen hubs are LRU-culled from the draw list. Expected frame-cost impact: ≤ 1 ms at reference tier.
- **[sketch]** **Determinism-neutral:** the map is purely presentation; no side effects on sim. Save-format only persists discovery + annotation deltas, not the map view state itself.

See also [Map Memory / Cartography](#map-memory--cartography) for the progression angle (what gets revealed when).

---

## Aircraft Catalog

### MVP Reference Aircraft: DHC-2 Beaver analogue

**[ready]** The MVP flight-model teacher aircraft. Beaver-based specs drive Phase 2 flight-model work; other aircraft become parameter-swap exercises once the reference is tuned to feel right.

- **Real-world basis:** de Havilland Canada DHC-2 Beaver (piston variant, Pratt & Whitney R-985 Wasp Junior radial, 450 hp).
- **In-game name:** TBD sanitized placeholder (candidates: "Northlander," "Klondiker," "Driftwood"). Visual homage, no trademarks, no DHC-2 branding; generic livery pool.
- **Why this aircraft:** iconic bush/float archetype, universally recognized, canonical configuration diversity (wheels / floats / amphibs / skis / tundra tires), wealth of public technical reference data, forgiving flight envelope for MVP tuning, distinctive radial audio signature.
- **Core flight-model parameters** (Phase 2 target):
    - Empty weight ~2,900 lb; MTOW 5,100 lb (wheels), 5,370 lb (floats).
    - Wing area ~250 sq ft; aspect ratio ~9.2; mean aerodynamic chord ~1.52 m; wingspan ~14.63 m.
    - Airfoil: NACA 64A416 root tapering to 64A412 tip (verify against published NACA TN data before lock).
    - Power: 450 hp radial at sea level; fuel burn ~27 gph cruise.
    - Vne ~160 mph, cruise ~125 kt (143 mph), Vs (clean) ~52 mph, Vso (flaps, floats) ~47 mph.
    - Slotted flaps, 4 positions (up / climb / takeoff / landing).
    - No retract; tailwheel configuration for wheeled variant.
    - Fuel: 95 US gal main (front + rear tanks; fuel selector modeled simply).
    - **Rotational dynamics (Phase 2.4.4b shipped values; tuning continues through 2.4.3+).** Moments of inertia placeholders: `I_xx=8000, I_yy=12000, I_zz=5000 kg·m²`. Moment coefficients: `Cm_α=-0.25, Cm_δe=0.24, Cm_q=-25` (pitch); `Cl_δa=0.075, Cl_p=-0.5` (roll); `Cn_β=0.08, Cn_δr=0.04, Cn_r=-0.6` (yaw). All currently hardcoded in `crates/sim/src/flight.rs`; migration to `AircraftSpec` is a 2.4.x follow-up so different aircraft can tune independently.
- **Configurations supported at v1 (MVP spans all three):**
    - **Wheeled taildragger** (bush tires optional).
    - **Straight floats** (Edo 4930 analogue geometry; hydrodynamic model at step transition).
    - **Amphibious floats** (retract capability; float-gear-in-hull configuration).
    - **Skis** (added Phase 5 if winter biome lands).
- **Engine / audio:** Pratt & Whitney R-985 radial -- 9 cylinders, low RPM rumble, distinctive exhaust pattern. Glicol patch keyed to RPM, manifold pressure, and load; idle to takeoff power covers ~650 to ~2,300 RPM. The radial is the reason audio modeling will be flagship on this aircraft.
- **Aesthetic / cel-shade fit:** slab sides, rounded cowl, short nose, tall cabin, high wing with struts. Silhouette reads beautifully in banded shading; surface areas invite strong livery expression.
- **Post-MVP turbine variant:** DHC-2T (PT6A conversion) noted as a future unlock -- simpler power model, modern sound profile, common on today's working Beavers.
- **Reference sources to chase:** DHC-2 Aircraft Operating Manual (publicly available facsimiles), Viking Air DHC-2T performance data, NACA TN-1945 and related reports for the 64A-series airfoil, operational accounts from Alaska/Canada bush operators, type certificate data.

### Broader Catalog (post-MVP expansion)

- **[sketch]** Every aircraft homaged, not branded. Sanitized names (e.g. "Northlander 185" style), low-poly cel-shaded models avoid licensing exposure.
- **[sketch]** Bushplane family: light taildragger (Cub-analogue), mid-weight utility taildragger (Beaver-analogue -- the MVP reference above), STOL-modified taildragger (Carbon Cub-analogue), nosewheel utility (Cessna-analogue), large cargo trike (Caravan-analogue).
- **[sketch]** Floatplane family: light single-float seaplane, mid-weight utility floatplane, big-iron float single (Turbine Otter-analogue), twin-float light sport.
- **[sketch]** Amphibious family: amphib floats with retractable gear (any of the floatplanes + amphib config), flying boats with hull + beaching gear (Widgeon / Goose-analogue), unusual configurations (Albatross-analogue for large hull flying boat).
- **[sketch]** Ski-plane configurations applied to bushplanes for winter biomes.
- **[sketch]** A handful of "popular media" homages -- visually recognizable silhouettes without branding (Porco Rosso's Savoia-adjacent flying boat, a Fire Fox-adjacent utility, an Indiana Jones cargo tramp, the plane from Tales of a Golden Eagle, etc.). Treat as unlockable cosmetic tier.
- **[sketch]** Water bomber / tanker variants for fire-hopping (Colorado region).
- **[sketch]** Helicopter -- **not planned**, likely non-goal (handling model too different, engine scope cost). Flag as [amend] if pursued.
- **[sketch]** Light twins for longer missions in post-v1.
- **[sketch]** Per-aircraft spec sheet (data-driven, no per-aircraft code):
    - Empty mass, max mass (gross / water / amphib), CG range, moment arms for fuel/cargo/passengers.
    - Airfoil section designation + tabulated lift coefficient vs AoA, drag polar, stall region.
    - Reference wing area, aspect ratio.
    - Engine: rated power, torque curve (simplified), fuel burn curve.
    - Prop: diameter, fixed/variable/feathering, pitch range.
    - Service ceiling, cruise, Vne, Vs, Vso, Vx, Vy.
    - Fuel capacity (main, aux, tips), fuel selector positions.
    - Electrical, hydraulic (simplified, mostly cosmetic: "retract works if hydraulic system healthy").
    - Landing gear type (fixed / retractable / amphib), tire sizes, skis.
    - Hull/float geometry for hydrodynamic model.
    - Typical liveries / popular color schemes for the paint shop.

---

## Aircraft Configuration & Upgrades

### Design principle — utility-varying tiers, not linear progression

Upgrades in flowPlan are **branching optimizations**, not a strict ladder. Each category offers three tiers (or three distinct options), and **tier 3 is not "better than" tier 1** — they're different optimizations for different mission classes. A STOL loadout and a range loadout are both valid; neither "wins". This is the core design principle for the upgrade system.

Consequences:
- **Fleet diversity is rewarded.** A STOL+floats+radial bush Beaver and a range-mod+wheels+turbine cross-country Beaver serve different mission classes.
- **Upgrade choices feel meaningful.** No single path.
- **Loadout storytelling.** A player's chosen upgrades reveal their playstyle — bush pilot, cross-country operator, specialty (fire bomber, wildlife, SAR).

### The five categories, three tiers each

**1. Gear** (mechanical-compatibility gated per [AIRCRAFT.md Configuration Matrix](AIRCRAFT.md#configuration-matrix)):
- **[sketch]** `wheel / float / ski` (and sub-variants: tundra tires, amphib floats, retractable ski). Aircraft declares which are mechanically possible; some configurations are unavailable on specific airframes. Swap at a hangar-equipped hub — time + cost.
- **[sketch]** Tradeoff: floats enable water ops but add ~120 kg + 8% Cd0; tundra tires enable rough-field landings but add drag + weight; skis enable snow ops but restrict to snow/ice surfaces.

**2. Cargo** (payload handling):
- **[sketch]** `standard cabin / belly pod / heavy-lift reinforce`.
- Standard cabin — baseline.
- Belly pod — adds storage volume + drag; frees cabin for passengers.
- Heavy-lift reinforcement — raises MTOW tolerance + structural cargo-load rating, but costs useful load at baseline mass.
- **[sketch]** Also: ballast slots (forward / aft / wing lockers) for CG trim or aerobatic feel. Orthogonal to tier choice.

**3. Aero / Airfoil** (airframe optimization):
- **[sketch]** `STOL kit / range-efficiency / ceiling-mod` — **three distinct optimization branches**.
- STOL kit: drooped leading edges, vortex generators, Fowler-flap extensions; drops stall speed + shortens takeoff + slows cruise.
- Range-efficiency: winglets + long-range tanks + wax / clean-up mods; improves cruise economy + max range; adds weight + hurts STOL.
- Ceiling mod: turbocharged prop + high-altitude airfoil re-profile; better altitude performance + density-altitude margin; neither tops cruise nor STOL.
- **[sketch]** Leading-edge slats (lower stall speed, better STOL) bundle with STOL kit in the design.
- **[sketch]** Not strictly better → worse: a loaded bush pilot picks STOL; a cross-country operator picks range-efficiency; a fire-hopper-over-Rockies picks ceiling.

**4. Powerplant** (engine + prop):
- **[sketch]** Prop tier: `fixed-pitch / constant-speed / feathering`.
    - Fixed-pitch: baseline; simplest.
    - Constant-speed: better climb + cruise efficiency (prop pitch matches airspeed).
    - Feathering: CS + emergency-feather for engine-out glide range.
- **[sketch]** Engine tier: `piston radial (baseline) / piston re-rate (more power) / turbine conversion`.
    - Turbine = significant power + altitude performance + more fuel burn + different audio signature (radial → PT6A whine).
- **[sketch]** Combined powerplant choices: 3 prop × 3 engine = up to 9 configurations per airframe; mechanical compatibility may restrict (not every airframe accepts turbine).

**5. Electronics** (avionics + autopilot ladder):
- **[sketch]** `analog panel / digital panel / GPS + moving map / GPS-coupled autopilot`.
- Electronics is more incremental than the other categories — each tier adds capability rather than trading one for another. But each rung **unlocks different mission classes**:
    - Analog: all daytime-VFR missions.
    - Digital panel: adds night-VFR readability (post-v1 night flight).
    - GPS + moving map: nav-reliable missions in low-visibility weather.
    - GPS-coupled autopilot: long-haul missions with autopilot-managed waypoint routing; reduces pilot workload on commercial cross-country flights.
- **[sketch]** Radio tier (analog → digital → satellite) runs in parallel — affects comm clarity and range with remote hubs; tied loosely to electronics but upgradeable separately.

### Loadout composition

- **[sketch]** A complete loadout selects one option from each category (where applicable / compatible). Loadouts are hangar-swappable: same airframe can be reconfigured between sessions at a hangar-equipped hub (time + cost for major changes like gear + engine; fast for minor like cargo pod on/off).
- **[sketch]** **Capability vector.** Each loadout resolves to a capability vector: `{service_ceiling, range_nm_with_reserve, stall_speed_low, cargo_capacity_kg, surfaces_supported, electronics_tier, best_cruise_kt, power_to_weight}`. Missions check against this vector for the [grayed-mission UX](#grayed-mission-capability-display).

### Cargo / storage layout (mechanical)

- **[sketch]** Discrete cargo bays with position (fore / mid / aft / wing) and max mass per bay.
- **[sketch]** Loading plan UI: drag-drop cargo into bays; CG indicator moves with each placement.
- **[sketch]** Passenger seating: passengers are cargo with fixed mass; seating preference "as far forward of cargo as possible" (real POH rule, simplified to UI).
- **[sketch]** Aircraft-specific layouts: bush singles may have limited aft baggage; cargo versions of the same airframe may omit rear seats for more cargo bays.
- **[sketch]** Visual representation: actual boxes / crates / passengers render inside the aircraft (through windows or during walk-around inspection).

### Landing lights, night-ops (post-v1)

- **[sketch]** Landing lights unlock post-v1 night-flight capability. Category-agnostic; a simple add-on rather than a tier.

---

## Cockpit Controls & Autopilot

- **[sketch]** Primary axes: aileron, elevator, rudder, throttle. Analog or keyboard-ramp.
- **[sketch]** Secondary axes: flaps (discrete notches, per aircraft), trim (continuous, pitch + optional roll/yaw), mixture (simple).
- **[sketch]** Switches: parking brake, water rudder (float aircraft), gear up/down (retract-capable), starter, magnetos, fuel selector, carb heat, pitot heat, landing light, taxi light, strobe, beacon.
- **[sketch]** Prop feathering: dedicated control for constant-speed/feathering props. Emergency feather for engine-out glide extension.
- **[sketch]** Tire pressure switch (ground ops only): raise for hard surface, lower for soft/tundra. Affects rolling drag, suspension stiffness, damage thresholds.
- **[sketch]** Ballast shift control (cockpit lever or a pre-flight setting at hangar).
- **[sketch]** Radio: frequency dial, PTT, auto-scan.

**Autopilot / assist (toggle/cycle):**

Phase 2.4.9c shipped the `ControlLaw` module at `crates/sim/src/control.rs` — a shared PID-based control-law library used by both the scenario bench (closed-loop turn scenarios like `beaver_coordinated_turn_30`) AND the in-game autopilot below. Each AP mode maps to a `ControlLaw` variant already implemented. This means:
- Bench scenarios validate the AP control laws by construction (if the bench's `BankHold` holds bank in a scenario, the AP's "hold roll" mode does too — same code path).
- Phase 4.7 AP work is mostly UI + session-state plumbing, not new control laws.
- Gain tuning that happens in bench scenarios carries directly to AP feel.

- **[sketch]** Hold altitude — `ControlLaw::AltitudeHold` (shipped).
- **[sketch]** Hold vertical speed / climb rate — `ControlLaw::VerticalSpeedHold` (shipped).
- **[sketch]** Hold roll (wing-leveler) — `ControlLaw::BankHold { target: 0.0 }` (shipped).
- **[sketch]** Hold turn (standard-rate) — `ControlLaw::BankHold { target: X }` with X from rate target (shipped).
- **[sketch]** Hold heading — `ControlLaw::HeadingHold` (stub in place, implementation pending autopilot milestone).
- **[sketch]** Hold all (straight and level) — compose `BankHold { 0 } + AltitudeHold + AirspeedHold` (all shipped).
- **[sketch]** Full auto to destination and circle — waypoint nav law pending (new ControlLaw variant).
- **[sketch]** Coordination assist (auto-rudder) — `ControlLaw::CoordinationHold` (shipped). Phase 4.9 accessibility assist uses this directly.
- **[sketch]** AP assumes the upgrade slot is filled; early aircraft lack AP at all; later aircraft have full GPS-coupled AP.
- **[sketch]** AP gracefully fails under severe turbulence (disengages with a cue) -- does not fight the aircraft past its authority.
- **[sketch]** Gain scheduling / adaptive control — post-v1 refinement. Current PIDs use fixed gains tuned for cruise.
- **[sketch]** State for AP laws lives in `SimState` so it's part of the determinism hash; PIDs persist integral / prev-error across ticks. Maintains replay-identical AP behavior.

---

## Camera & Views

Camera is a first-class control, not a setting buried in a menu. Switching is fluid and always available so the player can pick the frame that fits the moment.

- **[sketch]** **First-person cockpit view** -- head position inside the cockpit, per-aircraft reference (yoke/stick/throttle visible, instrument panel at realistic angle). Primary view for instrument flying, formation, IFR-like weather work, and immersion.
- **[sketch]** **Third-person chase view** -- behind and slightly above the aircraft, configurable follow distance and height, smooth damping. Primary view for sightseeing, taxi in tight quarters, and low-speed docking.
- **[sketch]** **Exterior orbit / cine view** -- free-orbit around the aircraft, pauseable rotation. Used for pre-flight inspection, photo framing, and debug.
- **[sketch]** **Fluid, easy switching** at any time (flight, taxi, parked, disembarked). No "camera cooldown," no locked transitions. A dedicated button cycles views; modifiers or direct bindings jump to a specific view.
- **[sketch]** Critical use case: **docking and tight water/ground maneuvering**. Third-person is often essential for float aircraft at crowded docks or bushplanes between trees. Switching must feel instant.
- **[sketch]** Short tween (60-120 ms) on camera transitions -- enough to avoid disorientation, fast enough to be a utility. Tween is skippable (hold the switch button to hard-cut).
- **[sketch]** **Additional view options:**
    - Tail / 6-o'clock view (look back).
    - Wing-tip views (left / right), useful for formation and wingtip clearance at tight docks.
    - Close-up cowl / engine view (pre-flight, damage inspection).
    - Cargo-bay view (loading, verifying passenger seating).
    - Hangar-walk view (when parked, inside a hangar, on foot).
- **[sketch]** **Look-around within a view:**
    - In first-person: POV-hat (gamepad right stick with modifier, or dedicated hat on HOTAS) swivels pilot's head. Center snap on release (configurable).
    - In third-person: right-stick (with modifier) orbits the camera around the aircraft without breaking chase frame.
    - Mouse-based look where appropriate (mouse-look modifier key, returns to default on release).
- **[sketch]** **Per-view camera inertia** tuned individually (first-person head sway under turbulence, third-person damping against abrupt attitude changes).
- **[sketch]** **Free-camera debug mode** (Phase 3.6+, `#[cfg(debug_assertions)]`): detach camera from aircraft, WASD / mouse-look / arrow-key 6-DOF movement, toggleable overlays for Voronoi cell boundaries + hub catalog + hydrology + weather cells + other world-gen layers. Built primarily to inspect procedural world output in-engine during 3.6 development. In-engine complement to the static PNG artifacts emitted by the world-gen visual-bench (see [TEST_PLAN.md § World-Gen Visual Bench](TEST_PLAN.md#world-gen-visual-bench)).
- **[sketch]** **Photo mode** (post-v1, Phase 5.5): same free-camera primitive, production-polished — pause sim, free-orbit, long-exposure / star-trail under night sky, framing grid, export. Evolution of the Phase 3.6 dev tool.
- **[sketch]** **Multiplayer spectator** (Phase 6 stretch): same free-camera for downed-aircraft or spawn-waiting players. Observation-only. Third evolution of the same primitive — **build once, expose to more contexts over time**.
- **[sketch]** **Camera for on-foot mode** -- default third-person character cam with first-person toggle. Consistent switching paradigm with in-aircraft views.
- **[sketch]** **FOV settings** per view (default + player override).
- **[sketch]** Visual cue (brief on-screen indicator) when a view changes, especially after a tween skip.

---

## Control Bindings & Mapping

All controls are fully remappable. Default bindings are curated in v1, but the mapping system supports any recognized input device from day one.

### Principles

- **Fully mappable by v1 ship.** Every action that affects gameplay has a binding slot (flight, taxi, ground ops, character, UI, camera).
- **Curated defaults, opinionated.** Keyboard, gamepad (twin-stick), and mouse+keyboard hybrid profiles ship with sensible bindings. HOTAS profiles shipped when HOTAS support lands post-v1.
- **No capability is binding-only.** Every action must also be accessible from a menu or alternative binding to avoid lock-out if a device fails.
- **Determinism preserved.** Raw input events flow into the sim input queue per [DESIGN.md § Threading Model](DESIGN.md#threading-model); binding translation happens at the main-thread boundary.

### Mapping system

- **[sketch]** In-game binding UI with live capture: press the action prompt, press the key/button/axis, done.
- **[sketch]** Supported input kinds: keyboard key, mouse button, mouse axis, gamepad button, gamepad axis, gamepad hat, analog trigger, joystick axis / button / hat (post-v1 HOTAS).
- **[sketch]** **Modifier layers:** `Mod1` (typically `LB`/`Shift`/left alt), `Mod2` (typically `RB`/`Ctrl`/right alt). Bindings may be `(input)` or `(mod + input)`. Up to two modifier layers in v1 (more post-v1 if HOTAS demands).
- **[sketch]** **Chord bindings** (e.g., `LT + RT` + click) supported for rare but meaningful actions (emergency gear down, feather engage).
- **[sketch]** **Per-axis tuning**: inversion, deadzone inner / outer, response curve (linear / mild / aggressive / custom), sensitivity, saturation, trim offset.
- **[sketch]** **Axis-to-button and button-to-axis** emulation (hold a key to ramp an axis; axis crosses threshold to fire a button action).
- **[sketch]** **Multi-device composition**: fly with a joystick for pitch/roll, a throttle quadrant for power, a keyboard for secondary toggles, and a gamepad for the camera. Devices coexist cleanly.
- **[sketch]** **Per-aircraft overlays**: twin-engine variants get separate left/right throttle bindings; retract-equipped aircraft get a gear binding; aircraft without retracts silently ignore it.
- **[sketch]** **Profile management**: named profiles (e.g., "Keyboard default", "Gamepad sim-lite", "HOTAS full"). Switchable at runtime. Profiles saved per user, not per aircraft.
- **[sketch]** **Device-disconnect handling**: if a device unplugs mid-flight, continue with remaining bindings, show a persistent warning, allow graceful landing.
- **[sketch]** **Haptic feedback** where supported (gamepad / force-feedback stick): stall buffet, touchdown thump, terrain-proximity rumble, gear-contact, engine stress. Per-source intensity sliders; entirely disable haptics as an accessibility option.
- **[sketch]** **Binding conflicts** flagged in the UI; resolved per last-wins with a warning, never silently.
- **[sketch]** **Reset to defaults** per profile or per category.

### Default binding sketches (subject to Phase 4/5 tuning)

Illustrative only. Final defaults land during Phase 4 controls milestone and Phase 5 polish.

**Gamepad (twin-stick, Xbox-layout reference):**

- Left stick -- rudder (X) + throttle (Y, with hold-modifier)
- Right stick -- aileron (X) + elevator (Y)
- Right stick + `Mod1` -- POV-hat look-around (first-person) / camera orbit (third-person)
- Triggers -- wheel brakes / water rudder (configurable)
- Bumpers -- flaps up / down (tap), `Mod1` / `Mod2` layer (hold)
- Face buttons -- view switch (A: cycle), gear toggle (B), trim reset (X), autopilot master (Y)
- D-pad -- quick camera select (up: cockpit, down: chase, left: left wing, right: right wing)
- Start/back -- pause / menu

**Keyboard + mouse:**

- `WASD` -- aileron + elevator (continuous or discrete depending on mode)
- `Q` / `E` -- rudder
- Mouse -- aileron / elevator (mouse-fly mode)
- `Shift` / `Ctrl` -- throttle up / down
- `F` -- flaps cycle
- `G` -- gear
- `T` -- trim (hold + up/down)
- `V` -- cycle view; `1`/`2`/`3`/`4` -- specific views
- Mouse + `Mod1` -- look-around in current view
- Backtick -- dev overlay

**Mouse + keyboard hybrid (for casual pilots):**

- Mouse-fly profile with keyboard for secondary functions, mouse-look on `Mod1`.

### Accessibility overlaps

- One-handed profiles (all essential flight actions reachable from a single hand).
- Toggleable-vs-held inputs (toggle gear, toggle flaps, toggle camera look, etc.).
- Input-remap UI readable with colorblind palettes and high-contrast mode.
- Configurable prompt size.

### Scope note

v1 ships with fully-working keyboard and gamepad profiles plus the mapping UI. HOTAS / joystick support and curve editors are deferred post-v1 but the mapping infrastructure is designed to accept them without a rewrite -- the raw-input layer already speaks all input kinds.

---

## Flight Model

### Phase 2 state (implemented)

- **[shipped: Phase 2.4.1]** Flight-model skeleton at `sim/src/flight.rs`: pure `compute_forces(state, spec, dt) -> FlightOutputs` function. `AircraftSpec` parameter plumbed. Per-force breakdown exposed to the debug HUD.
- **[shipped: Phase 2.4.2]** Real airfoil translational forces: lift `0.5 ρ V² S Cl(α)` with NACA 64A416-shaped `CL_TABLE` (whole-wing `Cl_max ≈ 1.65` at α=16°), drag polar `Cd = Cd0 + Cl²/(π·AR·e)` with post-stall parasite spike, α and β computed from body-frame velocity.
- **[shipped: Phase 2.4.2b, in test]** Lift direction perpendicular to velocity (not body-+Y). Fixes the stall-cascade on pitch-up that resulted from spurious `L·sin(α)` backward force. Coordinated banked turns feel realistic -- lift correctly tilts into the turn as the plane rolls. Full throttle climbs; full pitch-up does not bleed into stall at reasonable airspeed.
- **[shipped: Phase 2.4.2b]** Tuning: `THRUST_ACCEL_MAX` 2.0 → 3.5 m/s² for climb margin (proper power curve is 2.4.3); spawn velocity 50 → 55 m/s for ~2.4× Vs stall margin.
- **[shipped: Phase 2.4.1 / refreshed 2.4.4c]** Dynamics debug HUD (top-right egui panel): airspeeds, α / β / Cl / Cd, per-force vectors, total accel, actual angular velocity + angular acceleration, per-axis moment breakdown (control / stability / damping), active aircraft spec. Includes an airspeed-based heuristic stall alert (NORMAL / APPROACH / STALL vs `spec.vs_clean_mps`) with known false-positives in dives and accelerated stalls, pending the real 2.4.4d α-based model.
- **[shipped: Phase 2.4.2c]** Telemetry plumbing (`flowplan_render::Telemetry`) as a single-source derivation from `SimSnapshot + FlightOutputs`. Both debug HUD panels and the orientation widget read from it; no consumer reaches into `SimState` directly.
- **[shipped: Phase 2.4.2c]** Bottom-right cel-shaded orientation widget. Unit-length direction vectors for body axes (+X/+Y/-Z = RGB), velocity (white), thrust (yellow), lift (cyan), drag (orange), gravity (grey), total accel (magenta) in a 180×180 px panel. Always on; direction-only display (magnitudes live in the top-right dynamics HUD). Arrows are cel-shaded (dark outline quad behind, colored fill quad on top) with the outline intentionally a hair thinner than the main scene's sobel outline. A fixed isometric display rotation ensures all three body axes are visible at any attitude. Rendered via a new `TriangleList` pipeline with a dedicated screen-space ortho projection, reused in Phase 4 for hand-rolled HUD ladders.
- **[shipped: Phase 2.4.4e]** Bottom-left attitude indicator / glide-slope widget. Standard-AI layout: yellow aircraft symbol fixed in display, white horizon line rotates with bank and translates with pitch, grey pitch-ladder ticks at ±5°/±10°/±20°, green flight-path-vector crosshair offset from the aircraft symbol by `(β, -α) × scale` so the wedge between aircraft and FPV is angle-of-attack-plus-sideslip made visible. Text readouts (FPA, pitch, α, β, VS, L/D ratio) in a bottom-left-anchored egui panel above the widget. Same `DebugLinesRenderer` pipeline as the orientation widget; per-widget scissor rect clips horizon overflow at extreme pitch.

### Phase 2 open items (each carries a `TODO(phase-N):` at `sim/src/flight.rs`)

- **[shipped: Phase 2.4.4a]** Keyboard input slew-rate ramp. Pitch / roll / yaw axes slew toward held-key targets at 1.5 full-deflection/s (full travel ~0.67 s, symmetric on release). Throttle keeps its per-tick delta. Analog sources (gamepad, HOTAS) bypass when added.
- **[shipped: Phase 2.4.4b]** Moment-based rotational dynamics. Each control surface produces an aerodynamic moment `M = q·S·L·(Cm_δ·δ + Cm_stab·α_or_β + Cm_damp·ω·L/(2V))`; angular acceleration = moment / inertia; Euler-integrated into angular velocity. Control authority scales with dynamic pressure, weathervane stability restores toward velocity under sideslip, α-stability pitches back toward trim, damping opposes rates. Coefficients hardcoded Beaver placeholders; migration to `AircraftSpec` is a 2.4.x follow-up.
- **[shipped: Phase 2.4.4d]** Real stall physics + α-based stall alert. Post-stall nose-down pitching moment (aft shift of center of lift), wing-drop via non-linear Cl differential across the span past stall, accelerated-stall reading correctly because the test is directly on α. HUD stall alert (Normal / Approach / Stall) driven by `StallState` enum from the flight model.

- **[sketch]** Sim-like, sim-realistic in selected ways. Feel-first; accept liberties where they preserve fun.
- **[sketch]** Airfoil-based lift per aircraft: tabulated `Cl(alpha)` curve, drag polar, stall region characteristics. No CFD.
- **[sketch]** Angle of attack as a first-class state -- the angle between body-chord (approx -Z) and relative wind (negated body-frame velocity). All lift, drag, and stall computations key off α.
- **[sketch]** Sideslip angle β and its effects: rudder authority, asymmetric wing loading, adverse coupling to roll.
- **[sketch]** Stall: power-on vs power-off stall speed differential, wing-drop tendency per aircraft, recoverable in configured time, stall buffet as audio cue. Stalled wing produces a nose-down pitching moment (center-of-lift shift) that real aircraft use to auto-recover; placeholder has none.
- **[sketch]** Accelerated / G-loaded stall: `Vs_accelerated = Vs × sqrt(load_factor)`. Tight turns stall at higher airspeed; the airspeed-only HUD heuristic misses this.
- **[sketch]** Spin dynamics: autorotation after asymmetric stall. PARE recovery (Power idle, Ailerons neutral, Rudder opposite, Elevator through stall). Post-2.4 if not earlier.
- **[sketch]** Control authority scales with airspeed: slow-flight has weak authority; near-Vne authority is heavy and approaches limits. Post-stall, ailerons lose authority while elevator and rudder retain it (hence PARE works).
- **[sketch]** Ground effect near surface (reduced induced drag within ~1 wingspan of ground/water).
- **[sketch]** P-factor, torque, adverse yaw, gyroscopic effect -- tunable per aircraft, dampen for accessibility modes.
- **[sketch]** Slipstream / prop wash over rudder and elevator at high power + low airspeed: their authority exceeds what freestream airspeed suggests.
    - **[partially shipped: Phase 2.5a / 2.5b.4]** Elevator slipstream q-boost (`q_local_at_elevator = q_dyn + slipstream_q_boost`, ungated) and rudder slipstream q-boost (`RUDDER_SLIPSTREAM_Q_PER_N = 0.15`, airspeed-gated to match real Beaver rudder ineffectiveness at taxi speed) shipped with 2.5a / 2.5b.4. Delivers the pilot-reported real effect at full power + low airspeed.
    - **[open]** Rudder asymmetry: testing reports that **right rudder in particular feels underpowered** relative to left. This may match the real DHC-2 Beaver (right-rotating prop produces left-yawing moments that the rudder normally offsets — hence right rudder "does less work" in straight flight). Continued dynamics work may want to apply additional slipstream/propwash authority to rudder on the side where prop-induced yaw demands it, similar to the elevator boost. Worth a targeted investigation when Phase 3.2 weather / 3.5 coefficient-tune pass revisits rudder balance.
- **[sketch]** Cross-controls / slip / skid: intentional slip for short-field approaches; unintentional skid in uncoordinated turns as a classic stall-spin setup.
- **[sketch]** CG shift: fuel burn and cargo movement shift CG over time, affecting pitch trim and stall characteristics.
- **[sketch]** Wind response: gusts perturb attitude; crosswind demands rudder/aileron coordination on landing; wind shear near terrain and thunderstorm gust fronts.
- **[shipped: Phase 2.4.5]** Flap model: 4-position discrete handle (UP / CLIMB / TAKEOFF / LANDING) with per-notch `Δ Cl`, `Δ Cd0`, `Δ Cm` deltas. Landing flap drops stall speed from Vs ≈ 23 m/s to Vso ≈ 21 m/s (POH match), adds ~89% to zero-lift drag, and produces a manageable nose-down pitching moment. Keys F / C. Aircraft-specific tuning (per-spec overrides) is a later per-aircraft follow-up.
- **[sketch]** Trim: continuous, persistent between control inputs; full-trim setting per-aircraft (elevator, rudder, aileron).
- **[sketch]** Engine failure modes: fuel starvation (tank empty / wrong selector), carb ice (in certain conditions, carb heat mitigates), overheat (sustained max power + low airspeed), mechanical (damage state). Glide model meaningful; feather prop for max glide.
- **[sketch]** Density altitude: temperature and altitude combine to reduce performance. Hot-day / high-altitude strips are notably harder.
- **[sketch]** Humidity affects power (weak, but present). Exposed as difficulty dial.
- **[sketch]** Ice accretion in specific conditions (below freezing + visible moisture) -- reduces lift, increases drag. Pitot heat and carb heat mitigate.
- **[sketch]** Load-dependent handling: heavy aircraft roll sluggishly, stall earlier, take longer to decelerate. Light aircraft with aft CG become pitch-sensitive.
- **[sketch]** Hydrodynamics model (floats/hull): step transition (hump speed), porpoising risk, water rudder steering, weather-vaning in wind, spray obstructing forward view.

**Player-adjustable difficulty dials (changeable any time in settings):**

- **[sketch]** Wind magnitude multiplier.
- **[sketch]** Wind gust variance.
- **[sketch]** Wind shear intensity.
- **[sketch]** Storm frequency and intensity.
- **[sketch]** Hydrodynamics realism (porpoising on/off, spray impact, chop transmission to airframe).
- **[sketch]** Air temperature / humidity variance (density altitude effects).
- **[sketch]** Stall severity (wing-drop amount, recovery speed).
- **[sketch]** Damage sensitivity.
- **[sketch]** Economy harshness.
- **[sketch]** Autopilot authority (how much wind it can handle before disengaging).

**Invariant note:** Dials tune *parameters*, they do not swap physics *models*. Determinism is preserved: `(seed, input trace, dial state) -> bit-identical result`.

---

## Telemetry & Instrumentation Data Flow

Every display surface on the plane -- debug overlays, HUD, 3D cockpit gauges, autopilot readouts, flight-log entries -- reads the *same* derived telemetry, computed once per render frame from `SimSnapshot + FlightOutputs`. Design-for-reuse: debug plumbing written in Phase 2 is the same plumbing that drives Phase 4 hand-rolled HUD and Phase 4+ 3D cockpit instruments.

See [DESIGN.md § Telemetry derivation (render side)](DESIGN.md#telemetry-derivation-render-side) for the architectural boundary and invariants.

The **flight test bench** (Phase 2.4.8, [TEST_PLAN.md § Flight Test Bench](TEST_PLAN.md#flight-test-bench)) is a second consumer of the telemetry pipeline: it captures a `BenchSample` (CSV-serializable subset of Telemetry) per sim tick during scripted scenarios, enabling per-aircraft POH validation and offline plotting. The Phase 5+ flight-log feature (below) inherits this capture format -- a captured player flight is a replayable scenario.

### The plumbing

A single `Telemetry` struct (lives in `render` or `app`, not `sim`) is derived once per frame from the latest snapshot. Fields are SI internally; display-unit conversions happen at the consumer's format call. Every instrument / gauge / debug widget reads from this struct -- no reaching into `SimState`, no duplicated compute.

### Consumer progression

Each phase adds one consumer of the same telemetry layer; the layer itself grows by adding fields as more instruments need them.

- **[shipped: Phase 2.4.1]** Debug HUD (egui top-left and top-right). Formats numbers to text. Current reader of `SimSnapshot.vehicle` + `last_flight`; this is the de-facto first consumer of the "render-side derivation" pattern.
- **[ready: Phase 2.4.2c]** **3D debug gizmo** -- body-frame axes, velocity vector, force vectors (thrust / lift / drag / total accel) rendered as colored 3D lines at the aircraft position. First visual consumer of the new telemetry plumbing; proves the `LineList` pipeline infrastructure that later phases reuse for HUD ladders and instrument face lines. Toggled with a debug keybind. Details in the dedicated sub-section below.
- **[sketch: Phase 2.4.4]** G-load / load-factor readout in HUD. Telemetry adds `load_factor`, `vertical_g`, `lateral_g`, `longitudinal_g`.
- **[sketch: Phase 3-ish]** Crosswind / wind-direction indicator in debug HUD as weather lands. Telemetry adds `wind_world`, `wind_relative_to_heading`, `crosswind_component`.
- **[sketch: Phase 4 HUD]** Hand-rolled in-game HUD (the player-facing HUD, not the debug overlay). Airspeed tape, altitude tape, artificial horizon, heading tape, vertical speed indicator, turn-coordinator ball, AoA/stall margin indicator, engine cluster, fuel cluster, flap/gear/trim state. All read the same `Telemetry` struct.
    - Colorblind palette, sizing, anchoring, and animation smoothing live in the HUD renderer; the *data* is unambiguous and shared.
- **[sketch: Phase 4+ cockpit]** 3D physically-modeled cockpit instruments -- needles and drums driven by telemetry values. The same `airspeed` that drives the HUD tape drives the ASI needle rotation. Panel lighting, reflections, glass depth all live in the renderer; instrument *readings* are one float per dial.
- **[sketch: Phase 4+ autopilot]** Autopilot target / error readout (HDG bug, ALT capture, VS target, NAV deviation). Autopilot commands live in sim (they're inputs); autopilot *readouts* are telemetry.
- **[sketch: Phase 4+ damage / engine]** CHT, oil temp, oil pressure, manifold pressure, RPM, fuel flow, fuel remaining per tank, total fuel, range-to-empty at current burn. Telemetry picks these up as the engine model (2.4.3+) exposes them in `FlightOutputs`.
- **[sketch: Phase 5+ logbook / replay]** Flight-log entries capture telemetry at key moments (takeoff, landing, max altitude, touchdown G, stall events). Replay visualization consumes the same fields.

### Fields the `Telemetry` struct will carry (progressive)

**Phase 2 (shipped):** `airspeed_total`, `airspeed_horizontal_fwd`, `airspeed_vertical`, `altitude_msl`, `heading_rad`, `bank_rad`, `pitch_rad`, `alpha_rad`, `beta_rad`, `vertical_speed_mps`, force vectors (`thrust`, `lift`, `drag`, `gravity`, `linear_accel`), angular state (`actual_angular_vel`, `angular_accel`), per-axis moment breakdowns (`pitch_moment`, `roll_moment`, `yaw_moment` each with `control / stability / damping`), control positions (`throttle`, `pitch_input`, `yaw_input`, `roll_input`), `cl`, `cd`, `tick`.

**Phase 2.4.4d+:** `load_factor`, `stall_margin` (α-based, not airspeed heuristic), `slip_skid_ball`, `control_surface_deflections` (post-input-ramp -- distinguishing commanded from deflected).

**Phase 3+:** `altitude_agl` (needs terrain sampler plumbed through `SimSnapshot`).

**Phase 3:** `wind_world`, `wind_relative`, `crosswind_component`, `headwind_component`, `density_altitude`.

**Phase 4 (engine & secondary):** `rpm`, `manifold_pressure_inhg`, `fuel_flow_gph`, `fuel_remaining_gal`, `cht`, `oil_temp_c`, `oil_pressure_psi`, `flap_position`, `gear_position`, `trim_positions`.

### Invariants (restated at code site for anyone adding a field)

- Pure function of `(SimSnapshot, FlightOutputs, dt)`; no hidden state in telemetry itself.
- SI units internally; conversions at consumer format edges.
- Lives in `render` or `app`, not `sim`.
- One struct, many consumers -- no parallel re-derivation from sim state anywhere.
- Determinism-safe: presentation-side smoothing (needle lag, averaging) is allowed here; never in sim.

### Orientation Widget (Phase 2.4.2c, first visual consumer)

- **[shipped]** Render module `debug_lines.rs` + `debug_lines.wgsl`: position + color vertex, `wgpu::PrimitiveTopology::TriangleList` pipeline, no depth attachment. Each segment emits two triangulated quads -- a wider dark-grey outline behind a narrower colored fill -- drawn in two separate draw calls within the same pass so all outlines land behind all fills regardless of per-segment push order. The widening happens in widget-local XY because the widget uses an ortho projection down +Z; a future depth-aware world-space variant (for wireframe collision debug or flight-path vector) will need a clip-space vertex shader for widening.
- **[shipped]** `DebugLineBatch { outline_vertices, fill_vertices }` allocated once, cleared+repopulated per frame. Separate vecs so draw ordering is guaranteed regardless of push order.
- **[shipped]** `crates/app/src/debug_gizmo.rs` builds the batch from `Telemetry` each frame in **widget-local `[-1, 1]` space**:
    - No perimeter frame (design feedback: the frame competed with the arrow visuals).
    - Body axes: +X red (right wing), +Y green (up), -Z blue (nose), each rotated by the aircraft's world orientation then by a fixed widget display rotation (≈25° pitch + 32° yaw) so all three axes always have a visible screen-plane projection -- a level-flight nose pointing straight into the screen would otherwise vanish.
    - Force + velocity vectors: unit-length each, direction only (magnitudes already in the top-right dynamics HUD). Thrust yellow, lift cyan, drag orange, gravity grey, velocity white, total accel magenta.
    - Zero-magnitude vectors skipped so the widget stays readable when an engine is off or the aircraft is at rest.
    - Cel outline / fill thicknesses (widget-local units, ≈4 px / ≈2.5 px at the default 180 px widget size) tuned to read a hair thinner than the main scene's sobel outline.
- **[shipped]** Renderer maps the widget-local cube to a fixed screen-space rectangle via a dedicated ortho projection (`widget_view_proj` in `renderer.rs`) written to a separate camera-shaped uniform buffer each frame. Anchored 20 px from the bottom-right, 180×180 px square. Scales naturally with window resize.
- **[shipped]** No keybinding -- the widget is always on. The widget is small and cheap (13 line segments per frame), and it's what drives future HUD design; hiding it behind a toggle would add complexity with no payoff.
- **[shipped]** Render order: after the outline pass, before the egui debug HUD. The widget overlays the scene without being picked up by the outline sobel.
- **[shipped]** Determinism: app-side `gizmo_batch: DebugLineBatch` only; no sim-side state.
- **Infrastructure dividend (in progress):** the same `LineList` pipeline + widget-ortho pattern will carry Phase 4 HUD ladders (pitch, bank), artificial-horizon tick marks, airspeed/altitude tape ticks, and cockpit instrument face lines. A second depth-aware pipeline variant lands alongside the first consumer that needs 3D world-space wireframes (collision volume debug, flight-path vector).

---

## Ground & Water Surface Interaction

- **[sketch]** Surface types: `paved | gravel | grass_short | grass_tall | field | rocky | sand | mud | snow | ice | powder_snow | dry_lake`.
- **[sketch]** Per-surface effects: rolling friction, suspension damping, damage probability on poor touchdown, required ground speed for rotation, nose-over risk.
- **[sketch]** Tall grass hides rocks / obstacles (risk).
- **[sketch]** Sand soft-field technique: power-on, aft stick, shorter takeoff.
- **[sketch]** Powder snow as a soft surface for appropriately equipped floats/skis (plowing, spray, slow deceleration).
- **[sketch]** Tundra tires reduce damage risk on rough surfaces; standard tires increase it.
- **[sketch]** Float water rudder for low-speed taxi; weather-vaning in wind when rudder retracted.
- **[sketch]** Tail strike, prop strike, wingtip strike detection -- damage consequences + visible scuffs.
- **[sketch]** Short-field / soft-field technique rewards in physics: flaps + aft stick + power-on for short; power-on + aft stick for soft.
- **[sketch]** Water surface: chop state (`calm | ripple | moderate | heavy | breakers`), protected vs exposed water, spray intensity, porpoise risk at hump speed.
- **[sketch]** Step transition on floats / hull: clear feel/audio moment when aircraft comes "on step" and planes across water.
- **[sketch]** Surf/breaker zones: beaching a hull flying boat requires approach from the right side.

---

## Weather

Weather is a first-class emergent systems driver. It influences feel, ambience, difficulty, and mission design.

- **[sketch]** Precipitation regimes: `none | drizzle | light_rain | moderate_rain | heavy_rain | squall | light_snow | heavy_snow | blizzard | hail`.
- **[sketch]** Storm state: `clear | building_cumulus | scattered_thunderheads | squall_line | organized_storm | frontal_passage`. Storms spawn lightning, extreme turbulence, downbursts, hail.
- **[sketch]** Fog / mist states: `none | ground_mist | wing-tip_mist | layer_fog | dense_fog`. Morning mist curling off wingtips during water takeoff is a signature mood beat.
- **[sketch]** Visibility in meters/miles; varies by moisture, precipitation, dust, leaves, smoke.
- **[sketch]** Air temperature variable (seasonal + diurnal), feeds density altitude and icing.
- **[sketch]** Humidity variable, feeds density altitude and ice accretion probability.
- **[sketch]** Wind field: base direction + magnitude + variability per region, gust cycles, terrain-induced turbulence on lee side.
- **[sketch]** Wind shear at terrain edges and thunderstorm gust fronts.
- **[sketch]** Water surface chop driven by wind fetch + duration. Landing feasibility gated by chop + surface roughness.
- **[sketch]** Debris events: high dust in dry biomes, leaf storms in autumn, blowing snow in winter, ash/smoke in fire events.
- **[sketch]** Season overlay: rain vs snow balance, temperature baseline, daylight length, flora state.
- **[sketch]** Weather forecasting at hubs: multi-region outlook, accuracy decays with range and distance from forecast time. Remote hubs have less reliable forecasts.
- **[sketch]** Weather reward coupling: certain mission templates pay bonuses for operating in adverse weather (rescue, emergency supply, urgent medevac). Sequential progressions may always include an intended weather event.
- **[sketch]** Pleasant-weather beats: morning mist curling off wingtips, sun breaking after rain, alpenglow, aurora (subarctic), heat shimmer on desert runway, golden-hour landings.
- **[sketch]** Adverse-weather beats: approaching storm wall, gust-front rotor on short-final, whiteout on snow takeoff, icing buildup, unexpected wind shift.
- **[sketch]** Weather patches are spatial and move: pilot can fly around a squall, over a low layer, or under a ceiling.

### Visual target — volumetric, not billboarded

- **[amend]** Clouds target **volumetric in-game**, not 2D billboards. The [PLAN.md](PLAN.md) entry about "2D billboards near camera, optional volumetric only in cutscenes" is out of date / was never audited in detail. Phase 3.2x delivers the volumetric implementation. Cel-shading compatibility via banded alpha steps + hard-edge thresholding on volumetric sampling — the art direction is a constraint, not a reason to fall back to billboards.
- **[amend]** **Fly-through is a first-class requirement.** Players will invariably fly through low clouds. Billboard-based rendering fundamentally cannot handle camera-inside-cloud (the billboard is back-facing from inside, so the view shows nothing). Raymarched volumetric handles camera-inside natively — this is a **functional** reason for volumetric, not just aesthetic.
- **[sketch]** Storm visuals (cumulonimbus anvils, squall-line walls, gust-front rolls, lightning flashes, distant rain curtains) benefit directly from volumetric cloud primitives. Layered cloud regimes (low stratus below higher cumulus) are natural in a volumetric system.
- **[sketch]** Atmospheric optics in scope: sun halos, sun dogs, rainbows, crepuscular rays through broken decks. These are inexpensive when the cloud system is volumetric; expensive bolt-ons when it isn't.
- **[sketch]** Performance discipline: cloud rendering sits inside the DESIGN.md frame-budget ceiling (60% GPU utilization at 240 Hz reference tier). Forward-thinking constraint on render-cost per pass: ≤ 0.6 ms on reference at target quality.

### Cloud-interior flight semantics (fly-through effects)

The cloud density field is **CPU-samplable** (deterministic `world::cloud_density(x, y, z, t)` using the same Perlin-Worley formula the shader samples — bit-identical by construction, same pattern as `world::water_height(x, z, t)` from Phase 2.2). The sim consumes density each tick for three effects:

- **[sketch]** **Turbulence intensity scales with density-gradient magnitude.** Smooth cloud interior is relatively calm; cloud edges + convective cells are rough. Storm state modulates — cumulonimbus = worst turbulence, stratus = mildest. Physics effect: attitude perturbation, control-surface buffeting, stall-margin reduction.
- **[sketch]** **Visibility reduction inside clouds.** Aircraft's effective visibility drops from fog-limited 6-8 km to a few hundred meters inside dense cloud. Instruments-only flight. Couples with the electronics-tier utility-varying upgrade ladder ([Aircraft Configuration & Upgrades](#aircraft-configuration--upgrades)): GPS / autopilot / IFR avionics become mission-enabling rather than convenience.
- **[sketch]** **Icing accretion** in cold + moist cloud interior at appropriate altitudes. Already sketched in the weather feature list above; cloud-density sampling makes it concrete. Reduces lift + increases drag + raises stall speed. Pitot heat + carb heat mitigate.

These effects compound into a real "weather flying is its own skill" gameplay loop — flying under a cloud deck, choosing to punch through a gap vs detour around, learning which aircraft loadouts give comfortable margin. Mission capability requirements for adverse-weather missions (medevac-in-storm, urgent-mail-in-squall) consume these semantics naturally.

---

## Time of Day & Seasons

- **[sketch]** Day/night length default: ~15-20 minute day, ~5 minute night. Adjustable slower; effective real-time day for slow-immersion setting.
- **[sketch]** v1 gameplay capped to daylight. Night flight deferred to post-v1.
- **[sketch]** "Camp until morning" action available anywhere the aircraft can park / disembark.
- **[sketch]** Dawn/dusk visuals and palette shift via sky model + cel-shading banding.
- **[sketch]** Seasonal cycle (optional -- enable in v1 or defer?): spring / summer / autumn / winter affect flora colors, snow coverage, weather profile, daylight length in northern biomes.
- **[sketch]** Seasonal events: aurora in subarctic winter, salmon run in summer Alaska, monsoon in tropics, fire season in Rocky / Colorado.
- **[sketch]** Holiday / special-date flavor (soft): subtle holiday lights at civilian hubs in December; celebratory signage at hubs during regional festivals.
- **[sketch]** Seasonal cycle implementation toggle in v1 via dev controls (useful for playtesting winter/summer variants). Full player-visible seasonal cycle is v2.

---

## Night Sky & Astrophotography

v1 gameplay caps to daylight, but the renderer and sky model are designed from the start so that night looks *extraordinary* the moment night flight is enabled (post-v1). Targets an astrophotography-grade night sky rendered compatibly with cel shading. This is the single strongest deferred-aesthetic commitment in the project.

- **[sketch]** Celestial sphere model: physically-based positions for stars and planets keyed to an in-world date (simplified Julian date), latitude, longitude, and time-of-day. Slowly rotates as in-game time advances.
- **[sketch]** Star catalog: at minimum the Hipparcos-class bright catalog (~few thousand brightest stars) with accurate position, magnitude, and color.
- **[sketch]** Constellations: classical constellation line art as an optional overlay (beginner-friendly); stylized constellation art also available as a setting.
- **[sketch]** Milky Way band: the galactic plane rendered as a stylized, cel-shaded, slowly-rotating band across the sky. Features the galactic bulge, dust-lane silhouettes, and bright star-clouds around Sagittarius / Cygnus as stylized patches. Should read as the Milky Way without photographic noise.
- **[sketch]** Deep-sky objects: a curated catalog of recognizable DSOs (Andromeda, Orion Nebula, Pleiades, LMC/SMC if visible from the region) rendered as cel-shaded shapes and glows, recognizable to amateur astronomers without overwhelming the frame.
- **[sketch]** Planets: the visible planets (Venus, Mars, Jupiter, Saturn) with approximate positions, appropriate brightness, and subtle color differentiation.
- **[sketch]** Moon: modeled with phases matching in-world date, cel-shaded with banded terminator. Full moon affects ambient light meaningfully; new moon = darker night, better sky.
- **[sketch]** Moonlight affects scene lighting: ambient shift, soft shadows under a full moon, cooler palette.
- **[sketch]** Light pollution varies with hub proximity: dark skies over remote lakes and high-desert plateaus, washed-out skies near major hubs. Player-facing mechanic: remote destinations reward with better skies.
- **[sketch]** Cel-shading compatibility:
    - No noise-texture stars; each star is a cel-shaded point-sprite with a few brightness tiers and optional diffraction-spike hint.
    - Milky Way band uses banded value steps rather than a photo texture.
    - Atmospheric scattering near horizon gives warm-to-cool gradient at dusk/dawn and faint twilight arch post-sunset.
- **[sketch]** Aurora:
    - Stylized cel-shaded aurora shader with undulating color bands (green-dominant with magenta/pink peaks).
    - Triggered by: high-latitude region (subarctic / Alaska biomes) + appropriate season (winter/equinox) + weather state (clear-ish skies).
    - Can also appear rarely as a "weather event" in any northern region during solar-activity spikes -- an emergent treat, not scripted.
    - Audio pairing: soft Glicol ambient pad patch when aurora is visible.
- **[sketch]** Meteor showers: tied to in-world date; Perseids / Geminids / Leonids analogues produce occasional streaks. A "right place, right time" emergent payoff.
- **[sketch]** Star trails (post-v1 photo mode): long-exposure equivalent visualization when photo-mode shutter is extended.
- **[sketch]** Night visibility balance: sufficiently illuminated for the "visually pleasant" pillar (moonlight, aurora, starlight tint) without removing the "nighttime difficulty" part of the gameplay. Visibility under a full moon on snow is plausibly high; visibility on a new moon over ocean is plausibly low.
- **[sketch]** Astrophotography night missions (post-v1 mission template): fly to a dark-sky site at a specified date/time to capture a transient event (eclipse, conjunction, meteor shower peak). Non-financial reward tier (fits the wildlife/exploration protected progression).
- **[sketch]** Dev / debug overlay for sky: identify stars, constellations, planets, DSOs by hover. Educational side benefit.
- **[sketch]** Night-sky visuals also viewable during twilight as stars emerge progressively at the correct magnitudes.

Note: the renderer should include the sky components from the start (they cost little once implemented) even though night-flight *gameplay* is post-v1. Building the sky late is harder than building it early alongside the sun/atmosphere model.

---

## Damage & Wear

- **[sketch]** Damage is zone-localized, reducing the affected component's performance rather than using an abstract HP bar.
- **[sketch]** Damage zones:
    - **Rear control** -- elevator, rudder, tail wheel / tail gear, tail-cone structure.
    - **Lower / forward** -- main gear, nose gear, floats, hull, skis.
    - **Propeller** -- blade bend, tip strike, hub damage.
    - **Wing structure left / right** -- spar, ribs, skin, strut.
    - **Wing tip left / right** -- visible scuffs, aero penalty.
    - **Canopy** -- cracks, broken glass (cosmetic + visibility in rain/bugs).
    - **Fuselage** -- structural dings, dents.
    - **Powerplant** -- engine wear, oil leak, overheat history.
- **[sketch]** Airfoil damage reduces lift coefficient, raises stall speed, extends takeoff/landing distance, reduces maneuverability.
- **[sketch]** Control-surface damage reduces control authority on the affected axis.
- **[sketch]** Gear damage reduces suspension travel and raises damage probability on subsequent landings.
- **[sketch]** Prop damage causes vibration (cockpit shake, audio hum), loss of thrust, risk of engine mount failure at high power.
- **[sketch]** Canopy damage lets wind/rain/bugs in, reduces visibility, may cause cabin pressure oddities (not modeled for unpressurized aircraft).
- **[sketch]** Damage accumulates; never blocks the base gameplay loop.
- **[sketch]** Wear (vs damage) accumulates with hours: slow performance drift, increased failure probability if neglected, cheap to address at any repair-capable hub.
- **[sketch]** Visible damage rendered: cel-shaded scuffs, bent prop, torn canopy, asymmetric float damage, wing-skin wrinkles. Hand-shaded, not decal-sheet.

### Recovery by Hub Proximity

After a forced landing or write-off, recovery quality is determined by distance to the nearest capable hub. Closer hubs mean better lift and less secondary damage; remote incidents mean uglier, slower, more costly extractions. This turns hub proximity into a live strategic concern on every cross-country flight.

- **Near a complex / major hub** (within reasonable ground-recovery radius):
    - **Professional lift** -- flatbed recovery vehicle, crane at the hub.
    - Minimal additional damage; repair begins immediately at standard rates.
    - Highest cost, fastest turnaround.
- **Near a simple / minor hub** (local airstrip, fishing camp, general store with a truck):
    - **Pickup truck drag-out** -- rope or chain, improvised, bumpy ride.
    - Moderate additional wear and cosmetic damage (scuffs, paint scrape, suspension stress).
    - Lower cost, slower turnaround.
- **Truly remote (no hub within ground-recovery radius)**:
    - **Self-extraction**: player uses a winch or manual hoist (inventory items, or improvised with cargo straps). Significant additional damage (fuselage drag, prop contact, gear scraping), significant time cost.
    - **Wildlife help** (alternative to self-extraction): summon a moose to shoulder the plane off a boulder, a bear to roll it from trees, a whale to nudge a stranded floatplane off a sandbar. Still damaging but typically less than pure self-extraction.
        - Wildlife help is **gated by wildlife-mission reputation**: more wildlife runs completed -> higher chance of help arriving -> less additional damage incurred.
        - Wildlife help scales from "silly but costly rescue" at low rep to "nearly-professional tow" at high rep, with the wildlife layer acting as an alternative long-term investment for pilots who prefer exploration over cash missions.
        - Humor-forward and charming, never gruesome.
- Recovery options across proximity tiers:
    - Field repair (limited, uses supplies) -- independent of proximity, just enough to limp to a hub.
    - Tow / trailer to a hub (cost + time) -- applies to near-simple and near-complex tiers.
    - Write-off + barn-find replacement -- applies if damage exceeds repair thresholds; player takes a loan and awaits trailer delivery.
    - "Critter assist" call-in -- the remote-area option; see above.
- Recovery cost formula (design target, not final): `base_cost + proximity_multiplier * damage_severity - wildlife_rep_discount`. Numbers tuned for the "relaxing" pillar -- painful but not punitive.
- Near-complex-hub recoveries feel routine; near-simple recoveries have a scrappy, local-flavor feel (someone shows up with their truck, offers coffee); remote recoveries are memorable in either direction (self-rescue grit or wildlife-assist whimsy). This feature is designed to produce stories.

---

## Economy & Community

- **[sketch]** Single currency ("credits" placeholder name).
- **[sketch]** Persistent Alaskan / wilderness community framing: hubs represent real-feeling settlements (ranger stations, fishing villages, logging camps, research outposts) with resupply demand driven by population, season, and events.
- **[sketch]** Earn: mission completion, exploration bonuses, salvage recovery, scientific data return, wildlife-spotter bounties, urgent delivery premiums.
- **[sketch]** Spend: fuel, repair, upgrades, new aircraft (via trade + credits), supplies (food / water / tackle / first-aid), hangar fees, paint shop, cartographer, unlock-markers.
- **[sketch]** Loans: large one-time advance; passive interest; mission rewards apply to balance.
- **[sketch]** Loan-unlocked features (explicitly gameplay-positive; never pay-to-win since there is no PvP):
    - Repairs beyond minor.
    - Engine upgrades (Rotax, turbine).
    - Higher-lift mods (winglets, slats, VGs, STOL kit).
    - Prop upgrades (static advance, feathering, 3/4-blade).
    - Fishing pole + simple fishing minigame (restores food/water).
    - GPS upgrade (moving map with own-ship).
    - Autopilot modules (tier by capability).
    - Map reveal markers (hubs / exploration / missions).
    - More to add.
- **[sketch]** Market variation: hub-tier-based prices. Remote hubs charge more for fuel, less for local crafts. Supply and demand: if the player floods a remote hub's demand, prices drop; scarce goods spike.
- **[sketch]** Persistent supply contracts: major hubs contract with remote hubs for standing orders (fuel, food, medical, mail). The player fulfilling these over time shifts community state (grocery stocked, clinic supplied).
- **[sketch]** Community events: salmon run (demand for transport from remote canneries), fire season (demand for water-bomber support), winter freeze-up (ski-plane only for some hubs), spring break-up (float-only), festival (passenger/spectator transport).
- **[sketch]** Event-driven payloads: larger-than-normal deliveries during crises (weather, injury), higher pay + bonus rep.
- **[sketch]** Reputation per hub / region: unlocks higher-paying mission types, cheaper services, discounted fuel, friendlier paint-shop terms.
- **[sketch]** Survival layer (deferred post-v1, difficulty toggle): food/water drain over time; at low status, mission acceptance restricted; any landing allows camping to restore. Never blocks base gameplay.
- **[sketch]** Fuel economy per aircraft tuned to be neither trivial nor punishing; flight planning matters but defaults err toward relaxed.

---

## Fishing & Side Activities

Light side-activities that add texture without becoming mini-games in the negative sense. Unlocked via loan-funded upgrades or progression.

### Fishing

- **[sketch]** Unlocked by purchasing a fishing pole at an equipped vendor.
- **[sketch]** Cast at suitable spots: docks, stream mouths, alpine lake shores, protected bays, ice holes (winter biomes).
- **[sketch]** Simple interaction with light timing: click to cast, wait for a nibble, time the set. No elaborate minigame.
- **[sketch]** Numerical skill progression: more hours fished -> higher skill -> improved nibble rate, wider catch variety, rare-catch probability growth.
- **[sketch]** Catch variety: common species restore food; rare catches grant cosmetic rewards (paint options, livery unlocks, character outfit items, soundtrack stems) or progression hooks.
- **[sketch]** Message-in-a-bottle tasks: rare pulls include corked bottles with cryptic notes that hook into exploration missions (coordinate fragments, hub hints, wildlife-location tips).
- **[sketch]** Catch-and-release is the default; no inventory management of fish stacks.
- **[sketch]** Logbook entries for notable catches, auto-captured.
- **[sketch]** Cosmetic/minor utility framing -- never required to progress the main game.

### Other side activities (sketch level, less committed)

- **[sketch]** Campfire cooking (if survival layer enabled): fish + foraged ingredients -> restore status faster.
- **[sketch]** Photography (photo mode): scenic shots, wildlife shots, astrophotography shots (post-v1 night flight feature). Unlocks cosmetic LUTs and logbook captures.
- **[sketch]** Logbook journaling: occasional auto-generated reflection entries during camps, surfacing noteworthy flights.
- **[sketch]** Pet / companion (post-v1 consideration): could arrive via a trapper cabin or fishing camp storyline, tie into wildlife-rep milestones. Must not gate flight gameplay.

---

## Hangar Storage & Fleet

- **[sketch]** Single-aircraft baseline (NMS-like): one plane is the primary "current" aircraft. Can be exchanged at vendors (trade + credits adjustment).
- **[sketch]** Hangar storage unlocks at major / regional hubs with hangar service: store individual aircraft with all modifications, livery, CG settings, fuel, cargo, maintenance state retained.
- **[sketch]** Swap active aircraft at a hub-hangar where the target plane is stored; short animation/time cost.
- **[sketch]** Barn-find acquisitions: severely damaged / abandoned aircraft can be discovered (missions, exploration) and trailered to a hangar hub for a fee or delay, then restored.
- **[sketch]** Barn-finds feed a long-tail gameplay loop: discovery -> trailer -> restore -> fly.
- **[sketch]** Trailer contracts: pay a flat fee for a fast trailer, or wait in-game days for a cheaper delivery.
- **[sketch]** Hangar fee per aircraft per in-game period (trivial at owned-home hub; non-trivial elsewhere).
- **[sketch]** Fleet view UI: list all owned aircraft with last-hub, status, paint, loadout.
- **[sketch]** Custom home hub: player can designate one hub as "home" -- free hangar, free livery edits, loyalty bonus.

---

## Missions & Tasks

- **[sketch]** Template-driven missions, parameterized by world state (weather, available cargo, time-of-day, season, reputation).
- **[sketch]** Mission types:
    - Delivery (cargo from A to B).
    - Passenger transport (passenger as fixed-mass cargo, forward seating required).
    - Aerial survey / mapping (fly a route, maintain altitude/heading).
    - Supply drop (no landing required at destination; drop package on target).
    - Message / mail routing.
    - Search & rescue (find a beacon / signal / visual target and land / drop supplies).
    - Medevac (urgent passenger transport with penalty for rough handling).
    - Scientific data collection (fly through / near weather / wildlife / geology, sensors onboard).
    - Fire support (water bomber / retardant drop, Rocky / Colorado region).
    - Livestock/supply airdrop to remote communities.
    - Cartography run (fly unexplored region for the cartographer).
    - Photo run (capture specific landmark at specific time-of-day / weather).
- **[sketch]** Sequential template progression: completing a template unlocks a harder variant -- increasing load, longer distance, multiple destinations, time pressure, adverse weather, night (post-v1).
- **[sketch]** Chained missions: a delivery chain may require picking up multiple items, returning, then routing through several hubs.
- **[sketch]** Mission acceptance via hub mission board (HUD or at-hub screen).
- **[sketch]** Mission log: accepted, active, completed. Waypoints drawn on map.
- **[sketch]** Time-of-day / weather-specific missions (dawn mail run, stormy-weather medevac, post-v1 night ops).
- **[sketch]** Reputation-gated mission tiers.
- **[sketch]** Systemic-parameterization inputs to templates:
    - Current weather regime (storm -> harder mission pays more).
    - Current cargo availability at source hub.
    - Regional demand from the persistent community model.
    - Season (salmon / fire / aurora).
    - Time-of-day.
- **[sketch]** Optional mission objectives as bonuses: on-time delivery, no-damage delivery, fuel under budget, specific-route completion.
- **[sketch]** Mission failure outcomes: no money (but no hard penalty on damage/progression; only rep loss). Catastrophic loss (crash, cargo destroyed): loan call-in.
- **[sketch]** Wildlife runs as a distinct mission category -- non-financial reward, see [Wildlife](#wildlife).

### Mission capability requirements

Every mission declares a **capability-requirements record** that the mission board checks against the player's current aircraft + loadout before acceptance. Not every mission can be flown by every aircraft — and the mission board surfaces this visually rather than hiding incompatible missions.

Capability-requirements fields:
- **[sketch]** `service_ceiling_m` — minimum aircraft service ceiling. Computed from the heightfield maxima along great-circle + optimal-altitude route between source + destination hubs.
- **[sketch]** `range_nm_required` — total mission distance including a reserve margin (typical 30 min reserve at cruise).
- **[sketch]** `required_gear: SurfaceClass[]` — surfaces the mission requires at landing sites. Floatplane-only destinations require `[Dock | Lake | Beach]`; paved-major-hub destinations require `[Paved]`; soft-field delivery points require `[Grass | Gravel | Sand | Tundra]`.
- **[sketch]** `cargo_capacity_kg` — heaviest single payload item. Passenger-transport missions translate passenger count + baggage into kg.
- **[sketch]** `electronics_min: ElectronicsTier` — some missions need IFR-capable (weather-medevac), nav-coupled (cross-country with waypoint routing), or night-flight-rated (post-v1 night ops).
- **[sketch]** `aircraft_class_hint` (optional) — fire-bomber missions require a water-scooper / tanker variant; SAR missions benefit from hull amphibs or bush STOL; rarely a mission constrains to a specific airframe class.

### Grayed-mission capability display

Mission-board UI renders every available mission at a hub with **color-coded capability match**:

- **[sketch]** **Green / available** — current aircraft + loadout meets all requirements. Accept freely.
- **[sketch]** **Yellow / marginal** — meets requirements but without margin (range covers mission + reserve but nothing extra; ceiling clears heightfield by < 500 m; only one qualifying aircraft in fleet). Acceptance allowed with an awareness cue.
- **[sketch]** **Grayed / "outside aircraft capabilities"** — one or more requirements fails. Title rendered dim; mission cannot be accepted in current state.

Grayed entries surface **specific failure reasons**:
- "Requires float gear" (gear mismatch)
- "Range insufficient (needs 350 nm; current 280 nm)" (range)
- "Ceiling insufficient (needs 4,000 m; aircraft 3,800 m)" (service ceiling)
- "Requires GPS-coupled autopilot" (electronics tier)
- "Cargo exceeds capacity (needs 1,800 kg; aircraft 1,200 kg)" (cargo)
- "Requires water-bomber class" (aircraft_class_hint)

And **hint lines** offer paths forward:
- **Fleet swap hint:** "Your [Beaver on floats] stored at [Hub X hangar] qualifies" — surfaces fleet value, links to the 2D hub map to route there.
- **Upgrade hint:** "Long-range tanks upgrade ($X) would enable this mission" — surfaces economy hook; links to [Vendors & Progression § Parts/upgrades](#vendors--progression).
- **Swap + upgrade combination:** for complex capability gaps, the UI shows the cheapest path — fleet-swap if the player already owns a qualifying aircraft, otherwise the shortest upgrade chain.

**Why grayed + reason beats hidden:**
- Player discovers capability progression organically — sees what's possible before they can reach it.
- Upgrade + fleet gameplay hooks surface naturally; the player's curiosity pulls them toward progression.
- No "why isn't this mission showing up?" confusion; the system's capability-gating is transparent.

### Aircraft-aware mission generation (at hub)

- **[sketch]** Mission templates at a hub are **generated** based on the hub's mission-source pool. *Every* generated mission gets a capability-requirements record; capability-gating filters nothing at generation time — it only affects the **display** (green / yellow / grayed).
- **[sketch]** Some templates always generate even if no player-owned aircraft qualifies. Intentional — the board reflects the hub's actual demand, not just the player's capability window. Encourages fleet + upgrade expansion.
- **[sketch]** Special case: a hub may host a **"featured upgrade-suggestion" mission** that's intentionally out of current capability, prominently grayed, with the hint line pointing to a specific upgrade path. Essentially an in-world advertisement for progression. Used sparingly.

---

## Wildlife

- **[sketch]** Wildlife is primarily an atmospheric / observational system; not a combat/hunt system.
- **[sketch]** Biome-specific spawns: grizzlies / moose / caribou / wolves (boreal), whales / orcas / otters / seals (coastal), eagles / ospreys / flocks of geese (many), dolphins / turtles (tropical), reindeer / puffins (subarctic), bison / deer (high desert), alligators (tropical rivers).
- **[sketch]** Flocking flocks (boids-style for birds, simple state machines for mammals).
- **[sketch]** Wildlife encounter missions ("pure wildlife runs"): mysterious, non-financial.
    - Examples: "follow the orca pod and log their path," "respond to a wolf chorus at dusk," "circle a rookery at dawn for the count," "track a moose migration."
    - Rewards: unlock liveries, color-grading LUTs, map annotations, logbook "moments" entries, unique paint-shop options, cosmetic plane parts, soundtrack stems.
- **[sketch]** Wildlife rescue (humor hook): after a bad landing that traps the aircraft in unreachable terrain, a local creature "rescues" it -- a moose shoulders the plane out of a stand of trees, a bear rolls the plane off a boulder, a whale nudges a floundered seaplane off a sandbar. Always humor-forward, never gruesome. Unlockable after specific wildlife-rep thresholds; otherwise the plane requires trailer recovery.
- **[sketch]** Wildlife may react to player behavior -- low passes scatter herds, engine start may clear a parking area of deer.
- **[sketch]** "Impactful wildlife moments" logged automatically: your first orca breach, your first eagle strike near-miss, first caribou herd.
- **[sketch]** No hunting, no harm, no wildlife damage to aircraft beyond strike near-misses as mechanical hazards (bird strike is a weather-like hazard, not a targeted gameplay element).
- **[sketch]** Wildlife-sourced progression is protected: even if the economy skews toward payoffs, wildlife rewards remain non-financial and complementary.

---

## On-Foot / Character

### Art direction — Metal Slug-inspired, integrated with the cel-shaded world

- **[sketch]** **Art target: Metal Slug-level cartoon simplicity.** Appeal comes from **simple shapes and style**. The character lives inside the same visual language as the rest of flowPlan — cel-shaded with banded diffuse, bold outlines, flat colors, and the advanced blended / volumetric atmospheric style the scene uses. Outfits, face features, and body silhouettes all render under the same shader family as aircraft and world geometry, so the pilot reads as part of the world, not pasted onto it.
- **[sketch]** Character art is **authored by the project owner in Blender**. Rigging + base mesh + outfit modeling are art-role deliverables; implementation work supports the rig rather than specifying it in code. Art-direction iteration belongs to the project owner; the systems support whatever aesthetic the art lands on.

### Character creator — body sliders

- **[sketch]** **Height slider** — overall character height within a reasonable player-pilot range.
- **[sketch]** **Body shape sliders — independent, non-mutually-exclusive axes** (so the creator is **gender-agnostic** by construction rather than by opt-out):
    - Muscle tone / boxy-figure slider (lean → bulky frame).
    - Slender / curvaceous slider (reed-thin → fuller shape).
    - These two axes coexist — any combination is valid. There is **no male↔female axis**; the sliders operate independently and collectively describe the character's silhouette.
- **[sketch]** **Per-region shape sliders** (2–3 per area) — shoulders / torso / hips / arms / legs can each be nudged for fine-tuning within the Metal-Slug-simple register. Not anatomically detailed; style-preserving.
- **[sketch]** **Face sliders** — simple face shape (round / long / square), simple eye shape (the Metal-Slug-simple register — dots, crescents, slight variations), skin tone, hair option (small library of cel-shaded hair cards).
- **[sketch]** **Skin palette** — a small library of tones rather than a full color picker; each tone is cel-shade-compatible with pre-authored shading bands.

### Outfit system

#### Slot taxonomy

Outfits are modular across eight slots. Each slot carries its own mesh, rig-attachment, primary color, optional secondary color, and unlock status.

- **[sketch]** **Hat** — bush-pilot cap, beanie, ushanka, straw hat, earmuffs, headband, bare-head (none).
- **[sketch]** **Goggles / eyes** — aviator goggles, sunglasses, prescription glasses, eyepatch (bonus), clear (none). Sits on top of face features.
- **[sketch]** **Lowerface / mouth** — dust scarf, balaclava, moustache, beard, neckerchief pulled up, bare (none).
- **[sketch]** **Shoulders / neck** — shoulder holster (cosmetic only — no weapons), suspenders, scarf drape, cape (bonus / remote-unlock), plain (none).
- **[sketch]** **Torso** — flight jacket, vest, shirt, parka, polo, tank, cargo vest. This is the primary color-coordination slot.
- **[sketch]** **Gloves / arms** — fingerless gloves, thick leather gloves, bracers, sleeve variants, bare (none).
- **[sketch]** **Pants / shorts** — cargo pants, jeans, cargo shorts, work pants, slacks, snow pants.
- **[sketch]** **Shoes / boots** — bush boots, sneakers, waders, mukluks, sandals (bonus / tropical).

#### Color system

- **[sketch]** **Primary color per item: overlaid on the item's shading map.** This is the color-coordination-friendly default. Each outfit item has an authored cel-shaded band set; the primary color tints the band palette while preserving contrast and readability. Easy for the player to get a cohesive look.
- **[sketch]** **Secondary color per item (optional):** available where the item's authoring exposes a secondary region (e.g. stripe on a jacket, cuff on a glove, band on a hat). Not every item has a secondary region — that's authoring-intentional.
- **[sketch]** **Color picker:** the same HSV dial + recent-colors tray used in the [paint shop](#livery--customization). Consistent UI tool across livery + character color work.
- **[sketch]** **Color coordination hint:** the customizer surfaces suggested complementary colors when the player picks a primary, nudging toward cel-compatible palettes. Not a hard constraint — player can choose anything — but default suggestions keep the average result harmonious.

#### Rig + animation requirements (underlying systems support)

- **[sketch]** **Outfit meshes are rigged to the character's base rig** so they follow body deformation correctly during:
    - Walk cycles (locomotion on foot).
    - Jog / idle animations (standing around, looking around).
    - Emote animations (wave, sit, point, stretch, shrug, tip-hat, thumbs-up, looking-at-watch, lean-on-aircraft).
    - State transitions (disembark, climb-into-cockpit, crouch-to-inspect).
- **[sketch]** **Outfit scaling under body sliders:** when the player dials up muscle tone or torso breadth, equipped outfit meshes must scale / stretch sensibly with the body. Blend-shape or rig-driven morphs are the likely mechanism; the exact technique is a Blender-rigging decision owned by the project owner.
- **[sketch]** **No clipping in normal ranges:** the customizer's slider ranges are authoring-bounded to prevent outfits from clipping through the body at extremes. Extreme combinations are either clamped or silently smooth out.
- **[sketch]** Implementation side: the character / outfit system must accept a rigged base mesh + a collection of rigged outfit slot meshes, with per-slot primary/secondary color uniforms. No per-slot custom shader logic; one shared character shader with color-overlay inputs.

### On-foot mechanics

- **[sketch]** Third-person default camera; first-person optional. Consistent switching paradigm with in-aircraft views (see [Camera & Views](#camera--views)).
- **[sketch]** Walking / jogging movement; no sprint, no combat.
- **[sketch]** Interaction targets:
    - Parked aircraft (open door, inspect damage, manual load/unload, preflight walk-around).
    - NPCs (mission providers, vendors) -- menu-based dialog, not branching quest trees.
    - Campfire site (collect wood, light fire, sit, "camp until morning").
    - Fishing spots (if fishing pole upgrade unlocked).
    - Vendor kiosks / shop fronts.
    - Hub hangout benches / docks / lookouts — sit, emote, observe (ties into Phase 6 multiplayer hangout affordances).
- **[sketch]** Hub walkability scales with tier (see [Hubs & Landing Areas](#hubs--landing-areas)): major hubs have a small walkable footprint with multiple interaction points; remote hubs allow circling the aircraft and interacting with one or two NPCs / structures.
- **[sketch]** Minimal NPC animation budget: static poses, idle loops, one-arm acknowledgment gesture. Dialog is menu/card based.
- **[sketch]** Disembark / embark state machine: `in_flight | taxi | parked_cockpit | parked_walkaround | on_foot_free`. Each state enables a distinct control set.
- **[sketch]** Camera transitions between states smoothed.

### Progression integration — cosmetics as reward

- **[sketch]** **Player opt-in cosmetic reward:** when completing a delivery / mission, the reward screen offers a choice — take financial reward (credits) or take a rare/special cosmetic outfit piece or accessory instead. Not both. Player decides per mission. The "non-financial wildlife-run reward" category from PLAN.md generalizes here to all mission types.
- **[sketch]** **Rarity scales with remoteness + difficulty:**
    - Tier-1 major-hub missions → common cosmetic options (baseline outfit pieces, stock color swatches).
    - Tier-2 regional missions → uncommon (distinctive items, minor unique shapes).
    - Tier-3 local missions → rare (bush-pilot-specific kit, region-flavored items).
    - Tier-4 remote missions → very rare (one-of-a-few cosmetic items, hub-themed accessories — the fishing-camp operator's distinctive hat, the fire-watch-tower ranger's jacket).
    - Tier-5 isolated-hub discoveries → vanishingly rare cosmetics (antique pilot kit, heirloom accessories matching a found barn-find aircraft). Exploration-earned.
- **[sketch]** **Wildlife-run cosmetics** remain the protected non-financial-progression tier (bear-country jacket, orca-sighting pin, aurora-chaser scarf). These stack additively on the general cosmetic reward pool.
- **[sketch]** **Unique pre-designed outfits** (wildlife / exploration / seasonal-event-unlocked) are apply-only, not editable. Players can apply without color-picking — the authored palette is the reward. Editable outfits come from the general slot system.
- **[sketch]** **Seasonal / event cosmetics:** community events (salmon run, fire season, winter freeze-up, festival) unlock themed items during their active window. Preserved permanently once earned.

### Open questions for the customization system

- **[sketch]** Character customization anchor point — at the home hub only, at any hub with a mirror / camp, or anywhere at rest? Leaning: home hub + any camp.
- **[sketch]** Outfit save slots — how many named presets (morning-run set, stormy-day set, formal set)?
- **[sketch]** Pets (deferred, optional): small companion at a home hub? (kicked to open questions.) If pursued, pets get their own minimal slot taxonomy (collar color, posture variant) subordinate to the character system.

---

## Camping & Status Restoration

- **[sketch]** Any landing that is survivable (even off-airport) allows "camp until morning" as an action.
- **[sketch]** Campfire: collect wood at suitable biomes (forests, beaches with driftwood). Light via interaction. Sit and rest.
- **[sketch]** Consumes supplies (food / water) if survival layer active; restores status.
- **[sketch]** Time skip: "camp until morning" advances in-game time to a safe pre-dawn.
- **[sketch]** Weather can interrupt (e.g., storm forces early wake, leaves aircraft exposed to wind).
- **[sketch]** Wildlife may visit camp (atmospheric; no danger in base layer).
- **[sketch]** Journal entries sometimes written automatically during camps -- observations, reflections, logbook flavor.

---

## Vendors & Progression

- **[sketch]** Vendor types:
    - **Supply** -- food, water, tackle, fuel jerry cans, first-aid, firewood bundles.
    - **Parts / upgrades** -- engines, props, floats, tires, avionics, airframe mods.
    - **Aircraft vendor** -- sell/trade/buy planes (majors only). Buy includes trade-in credit for current plane.
    - **Paint shop** -- livery editing, bitmap texture slots, cel-shade-compatible.
    - **Cartographer** -- map reveal markers, regional overlays, mission-related charts.
    - **Barn-find broker** -- arranges trailer of a discovered derelict aircraft to the home hangar.
    - **Fishing camp operator** -- fishing-pole access, bait, local tips (mission leads).
- **[sketch]** Vendor availability gated by hub tier, region, and reputation.
- **[sketch]** Cosmetic progression:
    - Liveries, paint jobs, decals (wildlife-sourced unlocks + purchase-able basics).
    - Character clothing.
    - Soundtrack stems (unlocked via exploration / wildlife / mission chains).
    - LUTs / color grading presets.
    - Paint-shop bitmap texture options.
- **[sketch]** Gameplay progression:
    - Aircraft upgrades (performance, range, capability).
    - Hangar expansion (more stored aircraft slots).
    - Map reveal markers.
    - Autopilot tiers.
    - GPS tiers.
    - Fishing-pole and other side activities.

---

## Livery & Customization

### Initial scope (v1 target — Phase 5.3 Paint Shop)

Simple but satisfying. Get painting feel + decal placement working; defer advanced compositing.

- **[sketch]** Rotate aircraft in the paint-shop view (orbit / pitch / yaw controls) to expose the surface the player wants to paint.
- **[sketch]** **Brush tools:** size-scalable brush shapes (round, square, star, stripe, custom — seeded catalog), all parameterizable by size and rotation. Click to place a shape at the cursor location; mouse-drag to apply as a brush stroke. Raycast from camera through cursor to the front-facing surface hit (non-culled surfaces only).
- **[sketch]** **Color picker:** HSV dial + saturation / value sliders (vector-drawable, no bitmap picker assets) + hex input for precise color. Small recently-used-colors tray.
- **[sketch]** **Layer model (v1):** single bitmap layer per aircraft in the simplest shipping version. Each brush / shape stroke rasterizes into that layer. This is the minimum that lets the player "spend time creating complex or simple arrangements of decals" without any compositing complexity.
- **[sketch]** **Undo history:** per edit session. Undo / redo stack maintained while the paint-shop screen is open; **discarded** when the player exits and returns. Keeps per-aircraft save metadata small — the saved livery is the rasterized output, not the edit history.
- **[sketch]** **Preloaded bitmap texture options:** small set of stylized overlays (rivets, canvas weave, polished metal, weathered finish) that apply as an additive layer. Cel-shade-compatible — banded alpha, hard thresholds.
- **[sketch]** **Save:** named livery slot per aircraft; swap between saved liveries at any paint shop.
- **[sketch]** **Share:** screenshot + human-readable paint code export (color + stroke list for regeneration on import). Post-v1 livery marketplace hook preserved by making the code format stable.

### Extended scope (post-v1 / progression tier — hybrid layers)

Once v1 ships and the initial paint workflow is in play, expand toward a richer compositor without breaking back-compat.

- **[sketch]** **Multi-layer model:** base color + procedural-texture slot (one of the bitmap overlays) + decal layer for stripes/logos/shapes placed with the brush tools. The decal layer handles the "perspective-based shape placement on non-culled visible surfaces" — each decal is raycast from camera + projected onto surface normal at the hit point.
- **[sketch]** **Layer operations:** flatten layers down to the base bitmap (saves on storage space when the player is done composing). Must preserve the ability to **return to edit with layers intact** — at the cost of slightly larger per-aircraft save payload for work-in-progress liveries. Flattened liveries are smaller and final; editable liveries are larger and resumable.
- **[sketch]** **Session-scoped undo persists across the multi-layer session** (same rule as v1 — discarded on exit).
- **[sketch]** **Symmetry / mirror toggle** for simultaneous left/right application.
- **[sketch]** **Snap-to-edge / parallel-to-surface-feature** helper for stripes.

### Cel-shading compatibility

- **[sketch]** Cel-shaded compatibility: banded base + bold accent stripes read well; intricate detail is discouraged by the aesthetic but not forbidden.
- **[sketch]** Anti-aliasing in UV paint must avoid blurring the banded look — paint rasterizes with hard edges.
- **[sketch]** Decal projection uses crisp alpha thresholds compatible with the banded-diffuse shader.

### Unlockable liveries

- **[sketch]** Iconic homage liveries: player-recreations of real or iconic media liveries encouraged, not bundled by default (to avoid licensing).
- **[sketch]** Livery persists per aircraft (lives in its save slot); can be restored after repaint.
- **[sketch]** Liveries earned via wildlife / exploration are unique (pre-designed) -- can be applied but not modified.

### Data format & save-format implications

- **[sketch]** Per-aircraft livery save: rasterized PNG (for flattened v1-style) OR PNG + layer metadata (for extended work-in-progress). Bounded size; commits to the save-file budget in [Flight Log & Save Format](#flight-log--save-format).
- **[sketch]** Detailed representation decision (UV-space vs decal projection vs hybrid) is pending formal design doc (candidate: `LIVERY.md` or expanded section here). Pre-Phase-4.10 lock so save-format plumbing knows what it's persisting.

---

## Audio / Soundtrack

- **[sketch]** Fully seeded deterministic soundtrack via Glicol graphs: `(master_seed, season, weather, biome, time-of-day, mission_state, player_mood_proxy) -> deterministic Glicol param schedule`.
- **[sketch]** Additions and subtractions to the graph are deterministic functions of state -- changing context adds or removes layers predictably.
- **[sketch]** Core Glicol graph shapes:
    - Ambient pad layer (always on, per biome).
    - Motive / melodic layer (adds during cruise, climb highlights, dawn/dusk).
    - Rhythm / percussive layer (adds during weather or urgent mission states).
    - Environmental layer (rain, wind, wildlife).
- **[sketch]** Cross-fades between layers are smoothed in the Glicol graph, not step changes.
- **[sketch]** Radio voices (sampled) filtered via Glicol (EQ, squelch, static) -- optional, toggleable.
- **[sketch]** Engine synth per aircraft: RPM + load + airspeed + damage-hum parameters drive a Glicol patch. Different aircraft sound different.
- **[sketch]** Wildlife audio: flocks, splashes, distant calls, localized with stereo panning.
- **[sketch]** Landing / takeoff highlight beats: short musical flourishes in the score at key milestones (first flight with new plane, longest single flight, personal best, mission completion).
- **[sketch]** Shared musical key per biome -- maintaining tonal cohesion across zones.
- **[sketch]** No pre-recorded music stems; everything is generative.
- **[sketch]** Soundtrack stems unlock as cosmetic progression (new layer textures, new biome motifs).

---

## UI Direction

flowPlan's UI is intentionally thin right now. The current debug overlay (dynamics HUD, orientation widget, attitude indicator, all drawn via `DebugLinesRenderer` and cel-shaded screen-space widgets in `crates/render/`) is the reference example of the target style. Proper UI lands in Phase 4+ — but the aesthetic and implementation direction is captured here so the implementing milestones build from a shared target rather than inventing one.

### Principles

- **[sketch]** **Prefer vector / math / shader-drawn UI over bitmap assets.** The current debug widgets demonstrate what's possible without loading PNG/BMP/texture files: signed-distance fields, outlined polygons, procedural gradients, banded alpha. "**It would be impressive if the whole UI didn't require any persistent files in the path to maintain.**" This is the explicit aesthetic target.
- **[sketch]** Bitmap assets are permitted only when genuinely necessary (paint-shop preloaded overlays for rivets / canvas / polished / weathered; aircraft exterior textures; logos the player imports for liveries). Bitmap use is a decision, not a default.
- **[sketch]** Cel-shading compatibility: banded fills, bold outlines, hard edges. Fine serif typography and soft gradients are anti-patterns.
- **[sketch]** Determinism-neutrality: UI renders at display rate from telemetry + game state. Never writes back into sim.

### Debug overlay preservation

- **[sketch]** The current debug HUD stays in the codebase indefinitely. **Toggle-able in-game** so development + iteration on sim / flight-model work remains accessible even after proper player-facing UI lands. Bind key TBD at the Phase 4 UI work; today it's always on.
- **[sketch]** The debug overlay is developer-facing; it has no localization, accessibility, or polish requirement. Player-facing UI is separate code path.

### UI framework (PENDING)

- **[sketch]** Framework decision is open. `egui` is banned outside debug overlays per [DESIGN.md § Non-Goals](DESIGN.md#non-goals). Candidates for the settings / menus / dialog / vendor / mission-board UI:
    - Hand-rolled immediate-mode renderer using the existing `DebugLinesRenderer` / `TriangleList` pipeline. Maximum consistency with debug overlay; maximum effort per screen.
    - `egui` behind a feature flag for non-debug menus specifically (requires DESIGN.md amendment).
    - Hybrid: hand-rolled for in-world HUD + instruments, `egui` for flat settings/menu screens.
- **[sketch]** Decision target: the `UI_ARCHITECTURE.md` design doc, pre-Phase-4.8.

### Scope (by phase)

- **Phase 2 (shipped):** debug HUD (egui top-right/top-left), orientation widget, attitude indicator (both vector-drawn).
- **Phase 3 (sketched):** additional debug readouts as weather / autopilot / engine systems land — all via the same debug channel.
- **Phase 4:** player-facing HUD (Milestone 4.8), graphics panel (4.8.5), difficulty panel (4.9), control bindings (4.7b), save/load (4.10). Plus: main menu, pause menu, settings shell, aircraft selector UI, NPC dialog cards, hub walk-around prompts, logbook viewer, 2D map view, preflight walk-around UI — **each of these is a latent gap identified during the Phase 2.5 close-out stock-take**.
- **Phase 5:** paint shop (5.3), polished visuals, platform verification.

---

## Display & Graphics Settings

Traditional video-game settings for accommodating the full hardware-tier matrix. Respects [DESIGN.md § Non-Goals](DESIGN.md#non-goals) (no TAA/DLSS/FSR, no deferred rendering, cel-shading is the art direction — settings tune the implemented systems, never add a fundamentally different rendering path). All adjustable in-game without restart unless noted; changes take effect on the next frame (or next pipeline rebuild for pipeline-level changes like MSAA).

### Display

- **[sketch]** **Display mode**: exclusive fullscreen / borderless windowed / windowed. Default borderless windowed (best compatibility + quick alt-tab).
- **[sketch]** **Monitor selector** (multi-monitor systems).
- **[sketch]** **Resolution**: detected list + custom. Default = monitor native. Borderless windowed auto-matches desktop.
- **[sketch]** **Refresh rate**: detected list from the selected monitor. 30 / 60 / 90 / 120 / 144 / 165 / 240 Hz and any custom modes the display advertises. Sim tick stays at 120 Hz regardless (see [DESIGN.md § Display-refresh independence](DESIGN.md#display-refresh-independence)).
- **[sketch]** **Present mode**:
    - **VRR / Mailbox** (default; tearing-free, lowest latency at display cap, plays well with G-Sync / FreeSync).
    - **VSync / FIFO** (tearing-free, one extra frame of latency).
    - **No VSync / Immediate** (tearing, lowest latency, highest GPU utilization).
- **[sketch]** **Frame rate cap**: unlimited / 30 / 60 / 90 / 120 / 144 / 165 / 240 / custom. Independent from VSync mode — lets the user limit GPU utilization and thermals even with Mailbox/Immediate.
- **[sketch]** **Render scale**: 50% / 75% / 100% / 125%. Simple nearest-neighbor or bilinear upsample at the final composite. Explicitly *not* DLSS/FSR/TAA (non-goal). Useful for Steam Deck / minimum-tier hardware.
- **[sketch]** **HDR**: off / on (HDR10 if the display reports support). Post-v1 nicety; skip if complexity doesn't pay off against cel shading.

### Quality presets

One-click tiers that set every subsystem at once. Presets are the default entry point; individual sliders override per-subsystem.

- **[sketch]** **Low** — Minimum tier / Steam Deck. 60 Hz target, 75% render scale, MSAA off, low water tessellation, minimal particles, reduced view distance, bloom off.
- **[sketch]** **Medium** — default on first launch if hardware detected as "minimum" class. 120 Hz target, 100% render scale, MSAA 2×, medium water, mid particles, full view distance, bloom on.
- **[sketch]** **High** — default on reference hardware. 144 or 165 Hz target (depending on display), 100% render scale, MSAA 4×, high water tessellation, full particles, view distance + 1 LOD tier, bloom on.
- **[sketch]** **Ultra** — high-tier hardware. 240 Hz+ target (uncapped Mailbox or user cap), all visuals at max, full particle budget, view distance maxed.
- **[sketch]** **Custom** — persists when the user diverges from a preset.
- **[sketch]** First-launch: detect GPU tier, pick preset, show confirmation. No silent override.

### Per-subsystem quality (override individual presets)

- **[sketch]** **Water tessellation**: low / medium / high / ultra. Controls vertex density of the Gerstner grid.
- **[sketch]** **Water distant LOD transition distance**: slider. Closer = higher detail, higher GPU cost.
- **[sketch]** **Anti-aliasing**: off / MSAA 2× / MSAA 4× / MSAA 8×. (No TAA per non-goals.)
- **[sketch]** **Shadow quality** (when shadows land): off / low / medium / high.
- **[sketch]** **View distance**: slider. Affects terrain LOD and object culling radius.
- **[sketch]** **Foliage density**: low / medium / high.
- **[sketch]** **Particle density**: low / medium / high. Controls water splash, wake, exhaust, weather particles.
- **[sketch]** **Cel outline width**: off / subtle / standard / bold. Part of the art direction; exposed for player preference.
- **[sketch]** **Rim light intensity**: slider 0-1.
- **[sketch]** **Dithered bloom**: on / off / intensity slider.
- **[sketch]** **Color grading LUT**: list (default + unlockables from wildlife / exploration progression).
- **[sketch]** **Fog density**: biome default / slider override. (Affects visibility but not weather state — purely visual tuning.)
- **[sketch]** **Motion blur**: off / subtle. Heavy motion blur conflicts with crisp cel shading; only "subtle per-object blur on fast-moving prop" is appropriate.

### Field of view

- **[sketch]** **FOV per view**: first-person cockpit / chase / orbit each get their own FOV (matches [Camera & Views](#camera--views) intent). Range typically 60°-110°.
- **[sketch]** **Head-bob / camera inertia scale**: slider per view.

### HUD & UI scale

- **[sketch]** **HUD scale**: 75% / 100% / 125% / 150%.
- **[sketch]** **UI scale**: menu and dialog scaling, separate from HUD.
- **[sketch]** **Subtitle size**: small / medium / large (accessibility; cross-referenced with radio comms captions).
- **[sketch]** **Reduced motion**: on/off. Disables or softens non-essential UI animations (panel slide-ins, button bounces).

### Accessibility cross-refs

The following already live in [Difficulty & Accessibility](#difficulty--accessibility) and are surfaced from the graphics panel for discoverability:

- Colorblind palettes (Deuteranopia / Protanopia / Tritanopia / High-contrast).
- Audio-only cues for critical events.
- Pause-anywhere in single-player.

### Platform adaptations

- **[sketch]** **Steam Deck profile**: detected on launch, loads Low preset with 60 Hz handheld / 90 Hz docked, 75% render scale, reduced water tier, smaller default HUD scale.
- **[sketch]** **Integrated-GPU detection**: if iGPU is detected, default to Low preset with a warning that performance may be limited; no block.
- **[sketch]** **Small-window mode**: when window is <720 p, warn about text legibility and auto-offer 125% HUD scale.

### Invariants for graphics settings

- Sim determinism is preserved regardless of every setting above. Physics is identical at every refresh rate, present mode, render scale, MSAA level, and quality preset.
- No graphics setting triggers a code-path swap that changes rendering semantics (e.g., no "PBR at Ultra, cel at Low"). Cel shading is universal.
- No graphics setting unlocks post-processing that violates non-goals (TAA, DLSS, FSR, ray tracing, dynamic resolution scaling).
- Every setting is explicitly listed above or in Difficulty & Accessibility. No hidden graphics options.

### Config persistence

- **[sketch]** Settings persist to `config.toml` at a platform-appropriate location (XDG on Linux, %APPDATA% on Windows, `~/Library/Application Support/flowPlan/` on macOS). Human-readable and hand-editable (advanced users).
- **[sketch]** Reset-to-default per section and global.

---

## Difficulty & Accessibility

- **[sketch]** Overall pillar: relaxing, pick-up-and-play; rewarding for pilots.
- **[sketch]** Difficulty dials (adjustable in-game, any time, effective next tick):
    - Wind magnitude.
    - Wind gust variance.
    - Wind shear intensity.
    - Storm frequency.
    - Storm intensity.
    - Hydrodynamics realism (porpoising on/off, spray impact).
    - Air temperature variance.
    - Humidity variance.
    - Stall severity.
    - Adverse yaw / P-factor strength.
    - Damage sensitivity.
    - Economy harshness (price multipliers, loan interest).
    - Autopilot authority.
    - Fuel consumption multiplier.
    - Missions time-pressure multiplier.
- **[sketch]** Assists (toggleable independently):
    - Auto-rudder coordination.
    - Auto-trim.
    - Takeoff / landing guide visualization.
    - Wind indicator HUD.
    - Terrain-proximity warning.
    - Stall warning.
    - Flight path vector.
    - Ground-speed / glide-range overlays.
- **[sketch]** Input profiles: keyboard, gamepad (twin-stick), mouse-aileron. HOTAS/joystick deferred to post-v1.
- **[sketch]** Presets offered (Arcade / Relaxed / Sim-lite / Simulator) but dials individually adjustable under or over any preset.
- **[sketch]** Colorblind-friendly HUD palettes (Deuteranopia / Protanopia / Tritanopia / High-contrast).
- **[sketch]** Audio-only cues for critical events (stall buffet, over-speed, ground prox, engine stress, gear warning).
- **[sketch]** Subtitles / captions for radio comms, ambient dialog.
- **[sketch]** Pause-anywhere (single-player): freezes sim cleanly.
- **[sketch]** Photo mode (post-v1): pause, orbit camera, snap, export.

**Invariant note:** difficulty dials preserve determinism. Dials tune parameters, never swap physics models.

---

## Flight Log & Save Format

The flight log *is* the saved game. Rather than serializing the entire live state, flowPlan saves the **event log** that *generated* the state; determinism makes replay reconstruct the state exactly. This fits the existing determinism invariant and opens long-term features.

- **[sketch]** Save format: a human-readable, templated event log covering every feature's contributing events -- input traces, mission acceptances, damage events, economy transactions, weather transitions, discovery events, dialog/interaction choices, dial changes.
- **[sketch]** Save = `(seed, world_scope_choice, input_trace, world_event_log, dial_state_history)`. Replaying reconstructs all derived state.
- **[sketch]** Templated per-feature sections for legibility: `[flight]`, `[economy]`, `[damage]`, `[missions]`, `[wildlife]`, `[weather]`, `[character]`, `[fleet]`, etc. Each event is one line with a timestamp and structured fields.
- **[sketch]** Save file format: compact textual (RON, TOML, or a custom line-record format optimized for append + diff). Binary compression applied only to input traces where size dominates.
- **[sketch]** Determinism requirement: replaying a save produces bit-identical derived state. Any change in engine behavior that would break replay is a versioned save-format change.
- **[sketch]** Save versioning: each save has a game version, a save-schema version, a seed, and a world-scope designation. Migrations between schema versions are documented and supported.
- **[sketch]** Append-only during play (new events appended each tick or each sim event); compact at save time.
- **[sketch]** Human inspection: a player can open their save, see their flight history, identify specific events, and even redact entries if desired. This is part of the "relaxed and trustworthy" ethos.
- **[sketch]** Shared-seed compatibility: two players with the same seed + same world-scope + same difficulty dial state experience the same weather, wildlife spawns, mission parameterizations, community events. Their input traces diverge; the world state converges on initialization.
- **[sketch]** Post-v1 shared gameplay hook: asynchronous ghosts (other-player flights replayed as planes in your sky) require nothing beyond sharing their flight logs on the same seed.
- **[sketch]** Post-v1 co-op hook: two live players on the same seed share weather and world events; their input traces are interleaved in a combined log.
- **[sketch]** Implementation note: the sim crate must already expose all state-changing events as structured records (for the event log), which aligns with the deterministic tick design. No special layer needed.
- **[shipped: Phase 2.4.8 framework]** The flight test bench (`crates/sim/src/flight_bench.rs`, see [TEST_PLAN.md § Flight Test Bench](TEST_PLAN.md#flight-test-bench)) already implements the core replay primitive: a `FlightScenario` bundles initial state, aircraft spec, scripted `(tick, InputEvent)` trace, and assertions; the runner produces a per-tick CSV telemetry log via the same `SimState::tick` production code path; `--scenario <name>` plays a scenario visually. Phase 5+ flight log extends this format with non-flight event categories (economy / missions / damage / weather / wildlife transitions) and exposes it as a player-facing save file.

---

## Multiplayer (Stretch Goal, Post-v1)

flowPlan's design methodology — invariant-first, determinism as first-class concern, fixed-timestep sim independent of render — means multiplayer is a *natural extension* rather than a retrofit. The work to build a testable, deterministic, single-player-replayable sim is most of the work needed for lockstep multiplayer. This pays off the design discipline in a way most games can't replicate.

Tracked as [ROADMAP.md § Phase 6 (Stretch Goal): Multiplayer](ROADMAP.md#phase-6-stretch-goal-post-v1-multiplayer). Entry criteria: v1 shipped, playtesting confirms complete single-player experience, community signal warrants investment. All items here are `[sketch]` until Phase 6 is opened.

### Architectural fit (why this works for flowPlan specifically)

- **[sketch]** Lockstep networking: peers exchange only *inputs*, not state. Every peer runs the same deterministic sim and regenerates identical state from identical inputs. Bandwidth is tiny (~200 bytes/sec/player).
- **[sketch]** Determinism invariant (DESIGN.md Invariant #8) is already the lockstep prerequisite — validated by the flight-test bench proving `(seed, inputs, dials, dt, tick) → bit-identical state`.
- **[sketch]** Fixed-timestep sim (Invariant #1) gives every peer a clear sync boundary per tick.
- **[sketch]** Snapshot architecture (Invariant #3) naturally extends from cross-thread to cross-peer: peers already read snapshots, not live sim state.
- **[sketch]** Input queue (Invariant #7) is the sync channel: "broadcast this queue to peers" is the network layer.

### Session model

- **[sketch]** 2-4 players per session. Hard cap at 4 for scope reasons (NOT an MMO).
- **[sketch]** Friends-list / trust-based. No public matchmaking, no anti-cheat beyond desync detection.
- **[sketch]** Peer-to-peer, no dedicated server. Host-migration on host disconnect.
- **[sketch]** Session types (proposed):
    - **LAN**: zero-config broadcast discovery, lowest latency (< 5 ms RTT typical).
    - **Wide network**: lobby-code pairing, NAT traversal via STUN.
- **[sketch]** Session config locked at lobby: seed, world scope, difficulty dials, aircraft selections. All peers must agree before session starts.

### Async ghosts (smallest MP feature — ships independently if desired)

- **[sketch]** Player exports a flight log (seed + input trace + scenario config) as a shareable file.
- **[sketch]** Other players import it; the trace plays back as an NPC plane in their session deterministically.
- **[sketch]** Multiple ghosts per session, each independently replayed.
- **[sketch]** Uses same visual pipeline as live-MP remote players — aircraft rendered from snapshot, no sim coupling.
- **[sketch]** Natural prerequisite for live co-op (proves the N-vehicle render path and replay-from-trace machinery).

### Progress migration (SP ↔ MP boundary)

Single-player progression carries into multiplayer selectively. Design goal: multiplayer is a *social celebration* of completed single-player content, not an alternate grind path.

- **[sketch]** Carries SP → MP:
    - Owned fleet (aircraft + liveries + hangar customizations).
    - Pilot character.
    - Accumulated wildlife / exploration unlocks (cosmetic).
    - Paint-shop creations.
- **[sketch]** Does NOT carry SP → MP:
    - Credits / money (MP world uses its own economy if any).
    - Mission progress (MP missions are session-scoped).
    - Hub discoveries (MP world is seeded separately — discoveries per-world).
    - Damage state (everyone starts session with chosen aircraft in airworthy condition).
- **[sketch]** Carries MP → SP:
    - Cosmetic unlocks (liveries earned for shared moments apply to the SP fleet too).
    - Logbook "moments" (first-time-together sightings).
- **[sketch]** Does NOT carry MP → SP:
    - Credits / money (prevents "farm MP to upgrade SP" anti-pattern).
    - Reputation (SP reputation is per-NPC-hub; MP doesn't have NPC hubs).

### Sim-rate policy for MP

Wide-network RTT means lockstep at 120 Hz needs a multi-tick input buffer — perceptible input lag. Two candidate policies:

- **[sketch, candidate A]** Keep 120 Hz sim; exchange inputs every 2 sim ticks (60 Hz "network tick"). Physics determinism preserved exactly; network frequency halved; single tick of "relaxation" in the sync cadence.
- **[sketch, candidate B]** Drop sim to 60 Hz in MP sessions (SP remains 120 Hz). Simpler networking; requires either coefficient re-tune at 60 Hz or explicit acceptance of coarser physics in MP. SP scenarios cannot be replayed in 60 Hz MP sessions.
- **[sketch]** Decision deferred to Phase 6.4; choice depends on playtesting perception of input-lag tradeoffs.

### Session state — shared vs per-player

- **[sketch]** Shared (world-state): weather, wind, time-of-day, wildlife spawns, water chop, terrain, hubs. All deterministic per seed; naturally identical across peers.
- **[sketch]** Shared (mission-board-level): community events, seasonal demand, mission templates at hubs.
- **[sketch]** Per-player: aircraft state, fuel, damage, cargo, mission-acceptance log, logbook, voice comms channel.

### Networking layer

- **[sketch]** Transport: UDP-reliable-ordered via `renet` or `laminar` (both Rust-native). Requires DESIGN.md tech-stack amendment.
- **[sketch]** NAT traversal: STUN for NAT discovery; relay fallback where symmetric NAT blocks direct connection. Steam Networking SDK considered but adds platform coupling (conflicts with "no cloud" non-goal).
- **[sketch]** Handshake: protocol version match, scenario-config hash match, seed match, dial-state match before session starts.
- **[sketch]** State-hash exchange: every 60 ticks (~1 s), all peers exchange `state_hash()`. Mismatch triggers desync-with-reason disconnect. Protects against float-divergence bugs.

### Anti-cheat posture

- **[sketch]** Friends-list trust model. No real anti-cheat beyond state-hash desync detection.
- **[sketch]** A peer who modifies their client to deviate from deterministic sim will desync and disconnect. A peer who modifies only their *input stream* can cheat in limited ways (e.g., clip through terrain on their local view, but the world state all peers see is computed from their reported inputs, so macro-cheating is bounded by input-event space).
- **[sketch]** Users who want competitive anti-cheat should host private sessions with known friends. This is consistent with the single-player-first ethos.

### UI

- **[sketch]** Session browser: LAN-discovered list + "enter lobby code" text input.
- **[sketch]** Pre-flight lobby: player list, each player's selected aircraft, session config preview, "ready" toggle per player.
- **[sketch]** In-session HUD additions:
    - Remote player callsigns overlaid on their aircraft (distance-faded).
    - Player list with pings.
    - Chat box (text; voice via third-party like Discord / TeamSpeak is explicitly the assumed channel).
- **[sketch]** Post-session summary: per-player stats, shared moments captured (coordinated landings, wildlife encounters witnessed together).

### Not in scope even in Phase 6

- Matchmaking service.
- Leaderboards / ranks / seasonal events.
- In-game voice chat.
- More than 4 players.
- PvP / competitive modes.
- Dedicated server hosting.
- DRM / anti-piracy.
- Modding interface for MP (mod-compatibility is its own design problem; deferred separately).

---

## Remaining Open Questions

Narrowed since the last pass. Each will close into an amendment or a deferral by phase end.

1. **World scope** -- Candidates A (single large map) / B (4 regional maps) / C (fresh-per-session, no persistence) / D (hybrid persistent hubs + procedural surroundings). Prototype C and D in Phase 1 scratch branch; measure performance, persistence size, and subjective appeal. Locked before Phase 4. The bigger philosophical question *"does it matter that the region is the same across sessions if the experience is consistently good?"* is preserved here and will be answered by the prototype subjective-appeal check.
2. **Procedural cost** -- flight-log-memory feasibility for Candidates B and D. Prototype and measure.
3. **Fourth region identity** in Candidate B -- all four candidates retained; high-desert currently favored (author affinity, reference material, and natural pairing with the astrophotography night-sky aesthetic). Pick during Phase 2. Others may ship as post-v1 region packs.
4. **Biome continuity at the procedural/persistent seam** (Candidate D specific) -- how to blend without visible seam. Phase 1/2 prototype concern.
5. **Helicopter support ever** -- probable non-goal; reconsidered only with clear justification. Not a roadmap blocker.
