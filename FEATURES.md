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
- [Difficulty & Accessibility](#difficulty--accessibility)
- [Flight Log & Save Format](#flight-log--save-format)
- [Remaining Open Questions](#remaining-open-questions)

---

## Amendments Status

Decisions recorded from the feature discussion. Items marked *amend* have been agreed; the listed doc edits are pending. Items marked *undecided* remain open.

1. **World scope** -- *undecided*. Two candidates preserved in [World Scope & Map Layout](#world-scope--map-layout): **(A)** one large single map with diverse regional locales, **(B)** ~4 separate regional maps. Procedural generation is a sub-option of both, gated on the cost of retaining flown/discovered state. Do not amend DESIGN.md until this is resolved.
2. **Flight model fidelity** -- *amend, direction (A) confirmed*. Amend [DESIGN.md § Non-Goals](DESIGN.md#non-goals): "Not a full flight sim (no CFD, no engine-management minigames). Aerodynamics are tabulated per-aircraft and faithful; systems are simplified to meaningful toggles. Sim-realism is selectively deep where it affects feel." Liberties for fun are permitted. Player-adjustable dial for wind, shear, hydrodynamics, storms, air temp/humidity, changeable at any time during gameplay.
3. **Missions** -- *amend, direction (A) confirmed*. Amend [DESIGN.md § Non-Goals](DESIGN.md#non-goals) and [PLAN.md § Design Pillars](PLAN.md#design-pillars): "Missions are template-driven and parameterized by world state. Sequential progression increases template difficulty/complexity over time. No authored dialog trees, no branching quest lines. Emergence remains preferred for moment-to-moment situations; missions provide legible scaffolding."
4. **On-foot / character** -- *amend, direction (A) confirmed*. Add a `character/` subsystem to [DESIGN.md § Subsystems](DESIGN.md#subsystems). Minimal character creator (cel-shaded style, not Minecraft blocky; modest personalization). Scope: menu-driven interactions with NPCs/vendors, campfire, disembarked inspection. Gate to Phase 4/5. Tied to wildlife reward unlocks.
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

Each biome is a bundle of `{palette, flora, fauna, surface types, weather profile, hub archetypes, wildlife, soundtrack flavor}`. Specifically planned:

- **Coastal Alaska (tundra + forested coast + interior bush)**: tundra tires on gravel strips, ski/float swaps seasonally, grizzly/moose wildlife, cold-start effects, long twilights, salmon-run motif.
- **Boreal lake country (Canadian shield / Finnish lakes analogue)**: innumerable small lakes, pine forests, low-density hubs, long shoreline docks, aurora seasonally (visual treat).
- **Mediterranean archipelago**: turquoise water, white/terracotta ports, dense island clusters, limited hub services between majors, clear visibility baseline punctuated by mistral-like wind events.
- **Pacific Northwest**: dense fog, low cloud, heavy rain, fjords, logging camps, lumber mill docks, salmon-smoking canneries.
- **Rocky / Alpine**: density-altitude, short lake windows, narrow valley flying, fire-hopping, water-bomber drops, glacier-dammed lakes.
- **Subarctic / North Atlantic**: basalt coasts, stormy sea, heavy fog, reindeer/puffin wildlife, geothermal steam plumes, midnight sun in summer.
- **Tropical island chain**: reefs (visual, flight-height awareness), squalls, trade winds, coconut-palm atolls, protected lagoons as easy landings.
- **High Desert / Plateau**: dust devils, heat shimmer, dry-lake landings, thunderstorm anvils on horizon, leaf/dust debris events.

Each biome contributes to the shared feature taxonomy below (surface types, weather regimes, wildlife spawns, hub archetypes).

---

## Map Memory / Cartography

- **[sketch]** Flight log: every flight recorded as seed + input trace + events. Enables replay, deterministic "highlight reel" playback, and shareable-route artifacts.
- **[sketch]** Exploration map: regions/hubs/landmarks the player has personally seen are drawn; unexplored regions are fogged or sketchy.
- **[sketch]** Map reveal markers: unlocks via debt-funded upgrades (GPS, cartographer contract), mission rewards, wildlife encounters.
- **[sketch]** Hand-drawn cel-shaded map aesthetic: matches the art direction, not a UI satellite image.
- **[sketch]** Bookmarks: player can pin locations, add annotations.
- **[sketch]** Persistence support for procedural generation: only generated and visited regions persist on disk; others regenerate from seed.
- **[sketch]** Logbook: per-flight summary (route, weather, cargo, incidents, wildlife sighted, photos taken), auto-authored from the flight log, becomes a keepsake.

---

## Terrain & Static Landscape

### Phase 2 state (implemented)

- **[shipped: Phase 2.2]** Sum-of-sines procedural heightmap (`flowplan-world::Terrain`) with 5 octaves (regional, coarse, mid, fine, detail) at mountain scale (±275 m max). Beach flattening post-process (`raw * smoothstep²(|raw|/120)`) compresses shore heights. 5-tier color ramp: sand / field / forest / rock / snow.

### Phase 2 known limitations (to resolve via Phase 3+ terrain rebuild)

- **Sum-of-sines cannot independently tune amplitude and gradient.** Shores rise steeply because the regional octave has large vertical amplitude over short horizontal distances. Height-based flattening can squash vertical extent but cannot stretch horizontal extent, so gentle beach flats with tall mountain backdrops (per ref1/2/3 photos) aren't achievable without a different representation.
- **No real hydrology.** Water sits at a single fixed y=0 mean level; no lakes at different altitudes, no rivers, no drainage patterns. Valleys never collect water realistically; a lake basin is just "area where terrain happens to dip below zero".
- **No shore classification.** Beach, cliff, rocky headland, estuary, delta all look the same (whatever height+color the sum-of-sines produces). Hub placement can't distinguish safe float landing zones from surf-prone exposed coasts.
- **Repetitive look at distance.** 4 km patch tiled procedurally reveals the sinusoidal basis when flown over for long.

### Future terrain design (Phase 3+ target)

- **[sketch]** Proper terrain representation that decouples amplitude and gradient -- e.g., ridged noise plus domain warping, or a hand-authored heightmap with procedural detail overlaid. Enables genuine beach flats adjacent to mountain backdrops.
- **[sketch]** Hydrological realism: elevation lakes (ice-dammed, alpine tarns, oxbow lakes), rivers carved along gradient lines, drainage basins, deltas and alluvial fans at river mouths, waterfalls at elevation jumps. Each water body has its own surface level, chop behavior, and palette.
- **[sketch]** Shore classification: `sand_beach | gravel_beach | rock | cliff | mudflat | delta | ice_edge`, driving hub placement, landing feasibility, spray / foam / splash behavior, and wildlife spawn tables.
- **[sketch]** Heightmap base with vector overlays for rivers, lakes, coastlines, ridges, tree-line, field/forest polygons (original sketch, still the likely baseline representation).
- **[sketch]** Build-time baking of all static meshes; runtime streaming only for distant LOD swap.
- **[sketch]** Placed landmarks for dead-reckoning: mountains, towers, lighthouses, wrecks, bridges, unusual rock formations, logged cut-blocks, river bends, waterfalls.
- **[sketch]** Biome-specific flora density and palette that feed cel shading.
- **[sketch]** Erosion-shaped terrain features near water (alluvial fans, river deltas, sandbars) supporting bush landings on suitable sandy surfaces.
- **[sketch]** Water-body classification: `lake_small | lake_large | river | protected_bay | open_coastal | open_ocean | alpine_tarn | glacial`. Drives chop behavior, temperature, visual palette, and wildlife spawns.
- **[sketch]** Snow-covered terrain variants per season for applicable biomes.
- **[sketch]** Caves, gorges, canyons -- limited, for low-pass exploration and mission routing.

---

## Hubs & Landing Areas

- **[sketch]** Unified hub feature model: each hub declares `{surfaces, services, vendors, mission sources, wildlife nearby, population tier}` as data.
- **[sketch]** Population tiers: `major | regional | local | remote | isolated`. Tier drives available services.
- **[sketch]** Surface types per hub: `grass_field | gravel_strip | paved_runway | dock | beach | lake_ramp | amphib_combo | snow_strip | ice_runway | floating_platform`.
- **[sketch]** Service types: `refuel | repair_minor | repair_major | resupply_basic | resupply_specialty | upgrade_vendor | hangar | mission_board | avionics | paint_shop | barn_find_trailer | cartographer`.
- **[sketch]** Full-service airfield -- paved/gravel strip, hangar, repair, refuel, upgrade vendor, mission board, paint shop, cartographer.
- **[sketch]** Full-service seaplane base -- docks, wet hangar, repair, refuel, upgrade vendor, mission board, mooring buoys.
- **[sketch]** Mixed amphibious hub -- both surfaces, supports amphibs and swap configurations.
- **[sketch]** Minor resupply hubs -- bush gas station (taildragger on the taxiway of a dirt road), dock near a baitshop (floatplane for tackle), farm strip next to a barn (grass with a sock), general store on the waterfront.
- **[sketch]** Remote hubs (mission-only) -- clearing with a radio tower, ranger cabin, research station, fish cannery, logging camp, fire-fighter forward operating point.
- **[sketch]** Isolated hubs -- unmarked on maps until discovered, may be abandoned, may have special mission reward.
- **[sketch]** Fire-hopping forward bases -- short strip near active fires with water/retardant refill (Colorado-analogue region).
- **[sketch]** Dock variants: private, commercial, municipal, marina, fishing camp, research dock.
- **[sketch]** Discoverability: many remote hubs are uncharted until flown over or referenced by a mission; cartographer upgrade can annotate known hubs on the map.
- **[sketch]** Hub-specific markets: prices for fuel, supplies, repairs vary by tier and remoteness.
- **[sketch]** Inter-hub relationships: supply contracts between major and remote hubs generate persistent delivery demand.

---

## Aircraft Catalog

### MVP Reference Aircraft: DHC-2 Beaver analogue

**[ready]** The MVP flight-model teacher aircraft. Beaver-based specs drive Phase 2 flight-model work; other aircraft become parameter-swap exercises once the reference is tuned to feel right.

- **Real-world basis:** de Havilland Canada DHC-2 Beaver (piston variant, Pratt & Whitney R-985 Wasp Junior radial, 450 hp).
- **In-game name:** TBD sanitized placeholder (candidates: "Northlander," "Klondiker," "Driftwood"). Visual homage, no trademarks, no DHC-2 branding; generic livery pool.
- **Why this aircraft:** iconic bush/float archetype, universally recognized, canonical configuration diversity (wheels / floats / amphibs / skis / tundra tires), wealth of public technical reference data, forgiving flight envelope for MVP tuning, distinctive radial audio signature.
- **Core flight-model parameters** (Phase 2 target):
    - Empty weight ~2,900 lb; MTOW 5,100 lb (wheels), 5,370 lb (floats).
    - Wing area ~250 sq ft; aspect ratio ~9.2.
    - Airfoil: NACA 64A416 root tapering to 64A412 tip (verify against published NACA TN data before lock).
    - Power: 450 hp radial at sea level; fuel burn ~27 gph cruise.
    - Vne ~160 mph, cruise ~125 kt (143 mph), Vs (clean) ~52 mph, Vso (flaps, floats) ~47 mph.
    - Slotted flaps, 4 positions (up / climb / takeoff / landing).
    - No retract; tailwheel configuration for wheeled variant.
    - Fuel: 95 US gal main (front + rear tanks; fuel selector modeled simply).
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

- **[sketch]** Configuration swap at a hangar-equipped hub: `floats <-> tundra_tires <-> standard_tires <-> skis <-> amphib_floats <-> amphib_wheels_retracted`. Time + cost.
- **[sketch]** Upgrade slots:
    - **Engine** -- Rotax (light, efficient), piston upgrade (more power), turbine conversion (more power, more fuel burn, higher cost). Affects rated power, fuel burn curve, reliability profile.
    - **Prop** -- fixed pitch, constant-speed, feathering; static advance settings; aftermarket 3- or 4-blade options (marginal performance, visual).
    - **Wing enhancements** -- winglets (reduced induced drag, higher ceiling), leading-edge slats (lower stall speed, better STOL), vortex generators (lower stall speed marginally), STOL kit (flap/slat combo + wingtip mods).
    - **Tires** -- standard, tundra (low pressure capable), BFG. Tire pressure adjustable via cockpit switch (ground ops only).
    - **Floats** -- standard, amphib, heavy-duty (rougher water capable).
    - **Avionics** -- basic, moving map, GPS, autopilot module, weather radar (post-v1), radio tier (affects comm clarity and range).
    - **Auxiliary fuel tank** -- extends range at cost of useful load.
    - **Cargo pod** -- belly pod adds storage, adds drag, frees cabin for passengers.
    - **Landing lights** -- unlocks night flight capability (post-v1 feature gate).
    - **Skis retract** -- optional for certain ski-equipped bushplanes allowing dry-surface landings.
- **[sketch]** Ballast slots: forward, aft, wing lockers. Shift CG deliberately for trim or aerobatic feel.
- **[sketch]** Cargo/storage layout (visual + mechanical):
    - Discrete cargo bays with position (fore / mid / aft / wing) and max mass.
    - Loading plan UI: drag-drop cargo into bays, see CG indicator move.
    - Passenger seating: passengers are cargo with fixed mass; seating preference "as far forward of cargo as possible."
    - Aircraft-specific layouts: bush singles may have limited aft baggage, cargo versions of the same airframe may omit rear seats for more cargo.
- **[sketch]** Visual representation of stowage: seeing actual boxes/crates/passengers inside the aircraft (through windows or on walk-around inspection).

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

- **[sketch]** Hold altitude (AP captures current altitude).
- **[sketch]** Hold vertical speed / climb rate (AP maintains current climb or set rate).
- **[sketch]** Hold roll (wing-leveler; AP maintains 0 roll).
- **[sketch]** Hold turn (AP maintains standard-rate turn).
- **[sketch]** Hold heading (AP captures current heading).
- **[sketch]** Hold all (altitude + heading + level -- "straight and level" autopilot).
- **[sketch]** Full auto to destination and circle -- point at a hub/waypoint, AP flies there, circles on arrival until disengaged.
- **[sketch]** AP assumes the upgrade slot is filled; early aircraft lack AP at all; later aircraft have full GPS-coupled AP.
- **[sketch]** AP gracefully fails under severe turbulence (disengages with a cue) -- does not fight the aircraft past its authority.

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
- **[sketch]** **Photo mode** (post-v1) -- pause sim, free-orbit, long-exposure / star-trail under night sky, framing grid, export.
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
- **[shipped: Phase 2.4.1]** Dynamics debug HUD (top-right egui panel): airspeeds, α / β / Cl / Cd, per-force vectors, total accel, commanded vs actual angular velocity, active aircraft spec. Includes an airspeed-based heuristic stall alert (NORMAL / APPROACH / STALL vs `spec.vs_clean_mps`) with known false-positives in dives and accelerated stalls, pending the real 2.4.4 α-based model.
- **[shipped: Phase 2.4.2c]** Telemetry plumbing (`flowplan_render::Telemetry`) as a single-source derivation from `SimSnapshot + FlightOutputs`. Both debug HUD panels and the orientation widget read from it; no consumer reaches into `SimState` directly.
- **[shipped: Phase 2.4.2c]** Bottom-right cel-shaded orientation widget. Unit-length direction vectors for body axes (+X/+Y/-Z = RGB), velocity (white), thrust (yellow), lift (cyan), drag (orange), gravity (grey), total accel (magenta) in a 180×180 px panel. Always on; direction-only display (magnitudes live in the top-right dynamics HUD). Arrows are cel-shaded (dark outline quad behind, colored fill quad on top) with the outline intentionally a hair thinner than the main scene's sobel outline. A fixed isometric display rotation ensures all three body axes are visible at any attitude. Rendered via a new `TriangleList` pipeline with a dedicated screen-space ortho projection, reused in Phase 4 for hand-rolled HUD ladders.

### Phase 2 open items (each carries a `TODO(phase-N):` at `sim/src/flight.rs`)

- **[ready, major: Phase 2.4.4]** **Rotational dynamics are fiat, not aerodynamic.** Stick input → body-frame angular velocity directly, with no moments, no q-scaling, no sideslip generation from rudder, no damping. Symptom reported in flight testing: rudder yaws the aircraft identically at any airspeed and generates no β. The 2.4.4 refactor replaces the fixed-rate model with moment-based rotation:
    - Rudder δ_r + β + q → side force on vertical stabilizer → yaw moment N → angular acceleration
    - Elevator δ_e + α + q → pitch moment M
    - Aileron δ_a + q → roll moment L via differential wing lift
    - Each surface adds damping proportional to (rate × q) so the plane self-stabilizes at altitude but mushes at low speed
    - Real stall / wing-drop / accelerated-stall behavior falls out of the same frame naturally
- **[ready: Phase 2.4.4, UX sub-step]** **Keyboard input slew-rate clamp.** Keyboard is 0%/100% step; real control pedals/sticks are analog and rate-limited by human motor capability. Apply a linear slew-rate clamp at the `Controls` input-layer boundary: target value - current value, max Δ per tick bounded by `slew_rate × dt`, full-deflection reach in ~0.7 s (slew rate ~1.5 / s). Linear chosen over progressive to match the physical behavior of a human pushing a pedal at bounded speed; progressive ease would feel mushy near neutral. Apply to pitch / yaw / roll. Throttle gets its own rate (~0.4 / s on hold). Analog sources (gamepad, HOTAS) bypass the ramp.

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
- **[sketch]** Cross-controls / slip / skid: intentional slip for short-field approaches; unintentional skid in uncoordinated turns as a classic stall-spin setup.
- **[sketch]** CG shift: fuel burn and cargo movement shift CG over time, affecting pitch trim and stall characteristics.
- **[sketch]** Wind response: gusts perturb attitude; crosswind demands rudder/aileron coordination on landing; wind shear near terrain and thunderstorm gust fronts.
- **[sketch]** Flap model: per-notch lift boost + drag penalty + stall-speed reduction, aircraft-specific.
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

**Phase 2 (initial):** `airspeed_total`, `airspeed_horizontal_fwd`, `airspeed_vertical`, `altitude_agl`, `altitude_msl`, `heading_rad`, `bank_rad`, `pitch_rad`, `alpha_rad`, `beta_rad`, `vertical_speed_mps`, `forces: ForceBreakdown { thrust, lift, drag, gravity, total }`, `commanded_angular_vel`, `actual_angular_vel`.

**Phase 2.4.4+:** `load_factor`, `stall_margin` (α-based, not airspeed heuristic), `slip_skid_ball`, `control_surface_deflections` (after keyboard-ramp lands).

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

- **[sketch]** Minimal character creator:
    - Cel-shaded style, avoiding Minecraft-blocky or early-3D shapes.
    - Modest personalization: body type (a few options), skin tone, hair, face, clothing palette.
    - One or two outfit sets (bush pilot, coastal, winter).
    - Gender-inclusive, non-committal defaults.
- **[sketch]** Third-person default camera; first-person optional.
- **[sketch]** Walking / jogging movement; no sprint, no combat.
- **[sketch]** Interaction targets:
    - Parked aircraft (open door, inspect damage, manual load/unload, preflight walk-around).
    - NPCs (mission providers, vendors) -- menu-based dialog, not branching quest trees.
    - Campfire site (collect wood, light fire, sit, "camp until morning").
    - Fishing spots (if fishing pole upgrade unlocked).
    - Vendor kiosks / shop fronts.
- **[sketch]** Hub walkability: major hubs have a small walkable footprint; remote hubs may allow circling the aircraft and interacting with one or two NPCs / structures.
- **[sketch]** Minimal NPC animation budget: static poses, idle loops, one-arm acknowledgment gesture. Dialog is menu/card based.
- **[sketch]** Disembark / embark state machine: `in_flight | taxi | parked_cockpit | parked_walkaround | on_foot_free`. Each state enables a distinct control set.
- **[sketch]** Camera transitions between states smoothed.
- **[sketch]** Wildlife rewards tie into character customization (new clothing items, accessories).
- **[sketch]** Pets (deferred, optional): small companion at a home hub? (kicked to open questions).

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

- **[sketch]** Livery editor at paint-shop vendors:
    - Rotate aircraft, paint on currently viewable texture (2D brush onto visible surface).
    - Layer system: base color + pattern / stripe / spot decals + logos.
    - Few bitmap texture options preloaded (rivets, canvas, polished, weathered).
    - Save named schemes; share via screenshot/code export.
- **[sketch]** Cel-shaded compatibility: banded base + bold accent stripes read well; intricate detail is discouraged by the aesthetic but not forbidden.
- **[sketch]** Iconic homage liveries: player-recreations of real or iconic media liveries encouraged, not bundled by default (to avoid licensing).
- **[sketch]** Livery persists per aircraft (lives in its save slot); can be restored after repaint.
- **[sketch]** Liveries earned via wildlife / exploration are unique (pre-designed) -- can be applied but not modified.

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

---

## Remaining Open Questions

Narrowed since the last pass. Each will close into an amendment or a deferral by phase end.

1. **World scope** -- Candidates A (single large map) / B (4 regional maps) / C (fresh-per-session, no persistence) / D (hybrid persistent hubs + procedural surroundings). Prototype C and D in Phase 1 scratch branch; measure performance, persistence size, and subjective appeal. Locked before Phase 4. The bigger philosophical question *"does it matter that the region is the same across sessions if the experience is consistently good?"* is preserved here and will be answered by the prototype subjective-appeal check.
2. **Procedural cost** -- flight-log-memory feasibility for Candidates B and D. Prototype and measure.
3. **Fourth region identity** in Candidate B -- all four candidates retained; high-desert currently favored (author affinity, reference material, and natural pairing with the astrophotography night-sky aesthetic). Pick during Phase 2. Others may ship as post-v1 region packs.
4. **Biome continuity at the procedural/persistent seam** (Candidate D specific) -- how to blend without visible seam. Phase 1/2 prototype concern.
5. **Helicopter support ever** -- probable non-goal; reconsidered only with clear justification. Not a roadmap blocker.
