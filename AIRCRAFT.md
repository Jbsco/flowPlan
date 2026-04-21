<!-- L0: living catalog of real-world and media-homage aircraft for flowPlan, with design-model spec sheets -->
# flowPlan -- Aircraft Catalog

Living catalog of aircraft considered for flowPlan. Each entry is a design-model spec sheet aligned with the per-aircraft data expected by [FEATURES.md § Aircraft Catalog](FEATURES.md#aircraft-catalog) and [DESIGN.md § Simulation Architecture](DESIGN.md#simulation-architecture). When implementation begins, each entry becomes a `data/aircraft/<name>.ron` file authored from these specs.

**Role in the docs stack:** AIRCRAFT.md is a *content* document -- granular design data not yet in the specs. It sits alongside [FEATURES.md](FEATURES.md) as a reference that does not require amendment to grow. Adding an aircraft here is free; promoting an aircraft to `[ready]` implementation status requires a phase slot in [ROADMAP.md](ROADMAP.md).

**All specs below should be verified against the real-world aircraft's POH / type certificate / public reference before implementation.** Figures here are the author's best recollection and are tagged `(verify)` where uncertainty is higher than baseline. Data in the spec should faithfully model the real aircraft; *presentation* in-game uses a sanitized name, neutral livery, and low-poly cel-shaded model to avoid licensing exposure.

## Attribution / licensing (HARD PRE-RELEASE GATE)

flowPlan is currently in a **placeholder-asset phase**. The project owner sources free third-party 3D models (CC-licensed / permissive Blender community sources) as development placeholders for `data/aircraft/*.glb` meshes during Phase 2–3 while gameplay systems iterate. This is a deliberate trade: iterate on sim / flight model / world systems without gating on art authoring.

**Every placeholder mesh is replaced with original work authored by the project owner before v1 ships.** A game release has to be tight and correct concerning attribution and licensing — there is no path where third-party placeholder assets ship in the v1 binary.

Implications:
- The `data/aircraft/<name>.glb` filename and catalog entry (via `AircraftCatalog::scan`) are **stable across the placeholder-→-original swap**; only the mesh file changes. Rigging, CG, float/gear offsets should be re-verified at swap time but the data-path is unchanged.
- Do not commit flight-model tuning or mesh-topology assumptions that are placeholder-mesh-specific (e.g. vertex counts, submesh IDs, material-slot names) without an explicit `TODO(pre-v1-asset-swap)` marker.
- A formal **asset-attribution reckoning milestone** lands in Phase 5 (or as a dedicated pre-release pass). Every committed mesh must be either (a) project-owner-authored or (b) cleared with a permissive license that allows commercial redistribution with compatible attribution requirements.
- Placeholders present today are tracked in this file's entries; new placeholders added to the backlog should note the source and license so the audit is trivially enumerable at swap time.

## Status legend

- **[mvp]** -- Phase 2-4 reference aircraft (currently DHC-2 Beaver only).
- **[v1]** -- Planned for the v1 content pass (Phase 5.1).
- **[post-v1]** -- Expansion tier; gated to post-v1 roadmap.
- **[bonus]** -- Media / pop-culture homage; cosmetic-tier unlock.
- **[sketch]** -- Captured for consideration; not yet tiered.

## Spec sheet template

Each entry follows this structure:

```
### In-game name (real inspiration)
**[tier]** Real-world basis | Era | Configuration family
- MTOW: X lb | Empty: Y lb | Useful load: Z lb
- Engine: type, power (hp or shp), displacement
- Prop: type (fixed / CS / feathering), blades, diameter
- Airfoil: section(s) -- root / tip if tapered
- Wing area: X sq ft | Aspect ratio: Y
- Speeds: cruise | Vne | Vs (clean) | Vso (flaps/floats) | Vx | Vy
- Fuel: capacity (gal) | Range (nm)
- Payload: cargo volume | seats | max passenger count
- Configurations: [wheels, floats, amphibs, skis, hull, retract]
- Hybrid / specialty gear: [amphib floats, hull-with-retract, hull-with-wingtip-floats, tundra tires, etc.]
- Typical upgrades: [engine mods, STOL mods, avionics, etc.]
- Characteristic handling: [distinctive traits]
- Modeling difficulty: simple | moderate | complex
- Audio signature: [engine character, prop note]
- Real-world reference points: POH, type cert, key reports
- Notes: flavor, history, why include
```

## Contents

- [MVP Reference](#mvp-reference)
- [Light Bushplanes -- Taildragger](#light-bushplanes----taildragger)
- [Mid / Heavy Bushplanes -- Taildragger](#mid--heavy-bushplanes----taildragger)
- [Bushplanes -- Nosewheel](#bushplanes----nosewheel)
- [Floatplanes -- Dedicated](#floatplanes----dedicated)
- [Flying Boats -- Hull](#flying-boats----hull)
- [Light Modern Amphibs (LSA tier)](#light-modern-amphibs-lsa-tier)
- [Ultralights & Homebuilt STOL](#ultralights--homebuilt-stol)
- [Military Conversions -- Civilian Use](#military-conversions----civilian-use)
- [Twins and Turbines](#twins-and-turbines)
- [Golden Age / Historic](#golden-age--historic)
- [Fire / Rescue Specialty](#fire--rescue-specialty)
- [Media / Bonus Tier](#media--bonus-tier)
- [Configuration Matrix](#configuration-matrix)
- [Common Upgrade Catalog](#common-upgrade-catalog)
- [References to Chase](#references-to-chase)

---

## MVP Reference

### Northlander (DHC-2 Beaver)

**[mvp]** de Havilland Canada DHC-2 Beaver | 1947-1967, ~1,657 built | Bushplane / Floatplane / Amphibian (piston)

- MTOW: 5,100 lb (wheels) / 5,370 lb (floats) | Empty: ~2,900 lb
- Engine: Pratt & Whitney R-985 Wasp Junior, 450 hp, 9-cyl radial
- Prop: constant-speed, 2-blade, 99 in diameter (typical)
- Airfoil: NACA 64A416 root -> 64A412 tip (verify)
- Wing area: ~250 sq ft | Aspect ratio: ~9.2
- Speeds: cruise ~125 kt | Vne ~160 mph (~139 kt) | Vs ~52 mph | Vso ~47 mph | Vx ~65 mph | Vy ~85 mph
- Fuel: 95 US gal | Range ~455 nm
- Payload: ~2,100 lb useful | 6-7 seats or equivalent cargo
- Configurations: wheels (taildragger), straight floats, amphib floats, skis, tundra tires
- Hybrid / specialty: Edo 4930 straight / amphib floats; Wipaire 5000 options
- Typical upgrades: Viking DHC-2T turbine conversion (PT6A), VGs, STOL kit, long-range tanks, glass panel, wingtip extensions
- Characteristic handling: docile, forgiving, strong low-speed authority, short-field capable, trustworthy
- Modeling difficulty: moderate (radial engine is the main complexity)
- Audio signature: 9-cyl radial rumble, distinctive exhaust pattern, strong low-RPM character, shrill prop note in climb
- Real-world reference: DHC-2 POH, Viking Air turbine data, NACA TN-1945 (verify), Alaska/Canada operator accounts
- Extracurricular suitability (bush-pilot maneuvers): NOT ideal for tail-up-balance / wheelie-standing (demanding and nose-over-prone at 2313 kg / 140 W/kg P/W; real bush pilots can do it but it's aggressive, not a demonstration). Better suited to the `cub` / `husky` / `carbon_cub` ladder when those airframes land in Phase 5.1. Good fit for steep turns, precision glide (L/Dmax), short-field soft-field taxi, water-landing technique variants. See [`skills/flowplan-flight-bench/references/behavior-taxonomy.md` §6b](../skills/flowplan-flight-bench/references/behavior-taxonomy.md) for the execution-quality variants matrix.
- Notes: *The* iconic bush/floatplane. Full entry cross-linked from [FEATURES.md § MVP Reference Aircraft](FEATURES.md#mvp-reference-aircraft-dhc-2-beaver-analogue).

---

## Light Bushplanes -- Taildragger

### Birchling (Piper PA-18 Super Cub)

**[v1]** Piper PA-18 Super Cub | 1949-1994, ~10,000+ including variants | Light bushplane / floatplane

- MTOW: 1,750 lb | Empty: ~930 lb | Useful: ~820 lb
- Engine: Lycoming O-320 (150 hp) or O-360 (180 hp) flat-four
- Prop: fixed-pitch, 2-blade
- Airfoil: USA 35B (modified Clark Y) (verify)
- Wing area: 178 sq ft | Aspect ratio: ~6.2
- Speeds: cruise ~100 kt | Vne ~132 mph | Vs ~38 mph | Vso ~34 mph
- Fuel: 36 gal (std) | Range ~460 sm
- Payload: ~800 lb useful | 2 tandem seats
- Configurations: wheels (taildragger), straight floats, amphib floats (uncommon), skis, tundra tires, bushwheels
- Hybrid / specialty: 31" / 35" bushwheels, PK float options
- Typical upgrades: 180 hp conversion, VGs, extended fuel, STOL kits, big bushwheel mods, bubble doors, heavy-duty gear
- Characteristic handling: light, crisp, very short-field capable, low stall, demanding tailwheel technique
- Modeling difficulty: simple (small light piston)
- Audio signature: simple flat-four buzz, high RPM at cruise
- References: Piper POH, SuperCub.org community data
- Notes: the starter bushplane, canonical tandem taildragger. Good "early game" aircraft.

### Huskling (Aviat A-1 Husky)

**[v1]** Aviat A-1 Husky | 1987-present | Light bushplane (modern Super Cub analogue)

- MTOW: 2,200 lb | Empty: ~1,200 lb
- Engine: Lycoming O-360 (180 hp) or IO-390 (200 hp) flat-four
- Prop: fixed or CS, 2-blade
- Airfoil: USA 35B (verify)
- Speeds: cruise ~120 kt | Vs ~44 mph
- Fuel: 52 gal | Range ~800 sm
- Payload: ~1,000 lb useful | 2 tandem
- Configurations: wheels, floats (EDO 2000), amphibs, skis, tundra tires
- Typical upgrades: Micro VGs, BlackJack props, long-range tanks
- Characteristic handling: Super-Cub feel with modern systems and more power
- Modeling difficulty: simple (shares much with Super Cub; parameter swap)
- Notes: still-built Super Cub successor; pair with Super Cub as "early / mid" ladder.

### Carbon Falcon (CubCrafters Carbon Cub)

**[v1]** CubCrafters Carbon Cub SS / EX-3 | 2009-present | Ultralight STOL bushplane

- MTOW: 1,320 lb (SS LSA) / 1,865 lb (EX) | Empty: ~900 lb
- Engine: CC340 (180 hp) or CC363i (186 hp fuel-injected) flat-four
- Prop: CS 3-blade composite
- Airfoil: USA 35B variant
- Speeds: cruise ~115 kt | Vs ~30 mph
- Fuel: 25-44 gal | Range ~500 sm
- Payload: ~500-900 lb useful | 2 tandem
- Configurations: wheels (35" bushwheels standard), floats, skis
- Typical upgrades: heavy-lift prop, extended tanks, STOL mod already baked in
- Characteristic handling: extreme STOL -- ridiculously short takeoff, very low stall, high climb angle
- Modeling difficulty: simple but with aggressive STOL tuning
- Notes: premier STOL showpiece -- "can it land on a picnic table?" aesthetic.

### Champ (Aeronca 7AC Champ / Citabria / Scout)

**[post-v1]** Aeronca/American Champion family | 1945-present | Light trainer / utility

- MTOW: 1,450-1,950 lb depending on variant
- Engine: Continental C-85 (85 hp) / Lycoming O-235 (115 hp) / O-320 (150 hp) / O-360 (180 hp) flat-four
- Speeds: cruise ~95-115 mph | Vs ~38 mph
- Configurations: wheels, floats, skis
- Notes: budget vintage starter aircraft; 3 variants (Champ / Citabria / Scout) differ in engine/empennage/fuel.

### Stinson 108 (historic utility)

**[post-v1]** Stinson 108 Voyager / Station Wagon | 1946-1950 | Post-war 4-seat tailwheel

- MTOW: 2,400 lb
- Engine: Franklin 6A4, 165 hp flat-six
- Speeds: cruise ~115 mph | Vs ~55 mph
- Configurations: wheels, floats (EDO 2425 on some)
- Notes: classic 4-seat taildragger utility; less iconic but plentiful reference material.

### Taylorcraft BC-12D / F-19

**[bonus]** Taylorcraft BC-12D | 1938-present (intermittent production) | Pre-war light

- MTOW: 1,200 lb
- Engine: Continental A-65/A-75, 65-75 hp
- Notes: very simple early light taildragger; charm aircraft for the vintage tier.

### Rookling (Yakovlev Yak-12 / PZL-101 Gawron)

**[post-v1]** Yakovlev OKB Yak-12 (USSR) / PZL-101 Gawron (Poland) | 1946-1974, ~4,500+ built across variants | Soviet-era light utility STOL taildragger

- MTOW: ~2,650 lb (Yak-12M) | Empty: ~1,900 lb
- Engine: Ivchenko AI-14R 9-cyl radial, 260 hp (Yak-12M/R); earlier M-11FR 145 hp 5-cyl radial (Yak-12A); AI-14RF 300 hp on Gawron
- Prop: 2-blade fixed-pitch wood or metal; constant-speed on later variants
- Airfoil: Clark YH family (verify)
- Wing area: ~247 sq ft | Aspect ratio: ~7
- Speeds: cruise ~78 kt (~90 mph) | Vne ~120 mph | Vs ~37 kt
- Fuel: ~40 gal
- Payload: 4 seats (pilot + 3) + small cargo
- Configurations: wheels (taildragger, tundra optional), skis, floats (rare); Gawron variants adapted for agricultural spray
- Typical upgrades: AI-14RF re-engine, improved avionics, long-range tanks; Gawron ag-spray kit
- Characteristic handling: slow, docile, strong low-speed authority; classic Soviet-era bush / ag feel
- Modeling difficulty: simple (single radial piston, standard taildragger behavior)
- Audio signature: AI-14 9-cyl radial -- Russian radial character, similar tonal family to the R-985 but smaller displacement
- Real-world reference: Yakovlev Design Bureau data, PZL Okecie Gawron documentation, Eastern-Bloc operator histories
- Notes: Soviet analog to the Super Cub / Skywagon niche; widespread in USSR, Warsaw Pact, and ag-aviation. Lineage continues in the SM-92 Finist.

---

## Mid / Heavy Bushplanes -- Taildragger

### Tussock (Maule MX-7 / MT-7)

**[v1]** Maule MX-7, MT-7, M-9 | 1962-present | Mid utility taildragger / STOL

- MTOW: 2,500-2,800 lb | Empty: ~1,500 lb
- Engine: Lycoming O-320 to IO-580, 160-260 hp (piston); M-7-420 Rocket variant
- Airfoil: NACA 4412-mod (verify)
- Speeds: cruise ~135-155 kt | Vs ~35 mph
- Fuel: 74-84 gal
- Configurations: wheels, floats, amphibs, skis, tundra tires
- Typical upgrades: turbine MT conversion, long-range tanks, STOL mods
- Characteristic handling: STOL-capable, versatile, wide engine options
- Modeling difficulty: moderate
- Notes: still-built bush workhorse, fills the gap between Super Cub and Skywagon.

### Courser (Helio H-250 / H-295 Super Courier)

**[post-v1]** Helio Courier family | 1954-1974 (built in small runs since) | Extreme STOL

- MTOW: 3,400 lb
- Engine: Lycoming GO-480 (295 hp) geared flat-six
- Wing: slatted leading edge + full-span flaps (interceptor-like)
- Speeds: cruise ~140 kt | Vs ~30 mph with full flaps (!) | Vso near 27 kt
- Configurations: wheels, floats, skis
- Characteristic handling: leading-edge slats give extraordinary slow-flight capability
- Modeling difficulty: complex (slat behavior, full-span flaps interact with airfoil tables non-trivially)
- Notes: legendary STOL, unique silhouette with the slats and long-span flaps.

### Taiga (Technoavia SM-92 Finist)

**[post-v1]** Technoavia SM-92 Finist | 1994-present, small-series production | Russian single-radial STOL bush utility (Yak-12 successor lineage)

- MTOW: ~3,700 lb (~1,680 kg) | Empty: ~2,700 lb
- Engine: Vedeneev M-14P 9-cyl radial, 360 hp; some M-14PF 400 hp variants
- Prop: 2-blade constant-speed composite or metal
- Airfoil: NACA 23018 (verify)
- Wing area: ~237 sq ft | Aspect ratio: ~8
- Speeds: cruise ~110 kt | Vne ~140 kt | Vs ~40 kt
- Fuel: ~85 gal | Range ~550 nm
- Payload: 6 seats (pilot + 5) or freighter
- Configurations: wheels (taildragger, large low-pressure tundra standard), skis, floats
- Typical upgrades: reinforced gear for rough-field ops, long-range tanks, glass panel; SM-92T turbine conversion on prototype
- Characteristic handling: STOL-capable; radial burbles on power reductions; descendant of Yak-12 / Yak-18T lineage with updated geometry and higher payload
- Modeling difficulty: moderate (M-14 radial has distinctive audio + power-response character)
- Audio signature: Vedeneev M-14 9-cyl radial -- hallmark rasp heard on Russian aerobatic aircraft; slightly higher-RPM character than R-985
- Real-world reference: Technoavia manufacturer data, OKB-92 / Sokol Nizhny Novgorod production notes, Russian Aviation Monitor reporting
- Notes: modern spiritual successor to the Yak-12 / An-2 ethos but lighter; fits the Siberian / subarctic biome visually and narratively.

### Summit (Pilatus PC-6 Porter / Turbo Porter)

**[v1]** Pilatus PC-6 Porter / Turbo Porter | 1959-2019, ~600+ built | Swiss single-engine turbine STOL utility taildragger

- MTOW: ~6,100 lb (PC-6/B2-H4) | Empty: ~2,800 lb
- Engine: Pratt & Whitney Canada PT6A-27 or -34, 550-680 shp turboprop; earlier piston variants used Lycoming GSO-480 (340 hp)
- Prop: Hartzell 3-blade constant-speed reversing feathering; some 4-blade conversions
- Airfoil: Pilatus high-lift section (close to a modified NACA 64-series, verify)
- Wing area: ~324 sq ft | Aspect ratio: ~9.3
- Speeds: cruise ~120 kt | Vne ~151 kt | Vs ~52 kt | Vso ~48 kt
- Fuel: 160 gal standard; up to ~260 gal with tip tanks | Range ~770 nm
- Payload: ~2,800 lb useful | 10-11 seats or cargo / parachute-jump configuration
- Configurations: wheels (fixed taildragger), skis, straight floats, amphib floats (rare conversions)
- Typical upgrades: Raisbeck tip tanks, cargo pod, glass panel, upgraded PT6 variants
- Characteristic handling: legendary STOL -- landing rolls under 200 ft at low weights; bushmaster for mountain, jump, and island operations; strong prop-reverse authority on rollout
- Modeling difficulty: complex (turbine power curve + reverse thrust + mountain-airstrip density-altitude + tailwheel + broad weight range)
- Audio signature: PT6 turbine whine with classic beta-range whoop on reverse; higher pitched than a Beaver's radial, calmer than a twin's scream
- Real-world reference: Pilatus PC-6 POH, PT6A-27/34 installation data, Swiss / Antarctic / SEA operator histories, Air America historical records
- Notes: the definitive single-engine turbine bushplane. Used from Himalayan supply to Papuan missions and Nepal highland strips. Canonical glue between piston bush aircraft and multi-engine turboprops.

---

## Bushplanes -- Nosewheel

### Skyfly 172 (Cessna 172 Skyhawk)

**[post-v1]** Cessna 172 | 1956-present, ~44,000 built (most-produced aircraft ever) | Light 4-seat trainer / utility

- MTOW: 2,450 lb
- Engine: Lycoming O-320 / IO-360, 150-180 hp
- Airfoil: NACA 2412
- Speeds: cruise ~122 kt | Vs ~48 kt | Vne 158 kt
- Fuel: 56 gal (53 usable) | Range ~640 nm
- Payload: ~880 lb useful
- Configurations: wheels (fixed trike), floats (Wipline 2100), amphibs
- Typical upgrades: Penn Yan 180/210 hp, Robertson STOL, Horton STOL, turbo conversion, glass panel, Sportsman cuff
- Characteristic handling: forgiving, predictable, the universal trainer feel
- Modeling difficulty: simple
- Notes: not a pure bushplane, but include because it floats, everyone recognizes it, and it's the universal reference point for GA handling.

### Skywagon 180 (Cessna 180 Skywagon)

**[v1]** Cessna 180 | 1953-1981, ~6,193 built | Classic 4-seat utility taildragger

- MTOW: 2,800 lb
- Engine: Continental O-470, 225-230 hp flat-six
- Airfoil: NACA 2412
- Speeds: cruise ~140 kt | Vs ~50 kt
- Fuel: 65 gal | Range ~800 nm
- Configurations: wheels (taildragger), floats (EDO 2790 / 3430), amphibs, skis
- Typical upgrades: IO-520 conversion, VGs, Sportsman STOL, Wipline floats, cargo pods
- Characteristic handling: strong, stable, mild taildragger
- Modeling difficulty: simple
- Notes: the everyman Alaska/Canada bush utility.

### Skywagon 185 (Cessna 185 Skywagon)

**[v1]** Cessna 185 | 1961-1985, ~4,356 built | Mid-weight utility taildragger

- MTOW: 3,350 lb
- Engine: Continental IO-520-D, 300 hp flat-six
- Airfoil: NACA 2412
- Speeds: cruise ~145 kt | Vs ~49 kt
- Fuel: 84 gal | Range ~920 nm
- Payload: ~1,500 lb useful
- Configurations: wheels, floats (EDO 3430 / Wipline 3450), amphibs, skis
- Typical upgrades: IO-550 conversion, VGs, STOL kits, auxiliary tanks, cargo pod, amphib upgrade
- Notes: the workhorse; many operators prefer 185 over Beaver for owner-ops; include as mid-v1 mainstay.

### Stationair (Cessna 206 / 207)

**[v1]** Cessna 206 Stationair / 207 | 1962-present | 6-seat utility nosewheel

- MTOW: 3,600 lb (206) / 3,800 lb (207)
- Engine: IO-540, 300 hp (turbo variants available)
- Airfoil: NACA 2412 modified
- Speeds: cruise ~145 kt | Vs ~54 kt
- Fuel: 88 gal
- Configurations: wheels (fixed trike), floats (Wipline 3730), amphibs
- Typical upgrades: turbo, IO-550, cargo pod, cam-lock door, STOL
- Notes: roomy cargo hauler; commercial bush operator favorite.

### Caravan (Cessna 208 Grand Caravan)

**[v1]** Cessna 208B Grand Caravan | 1984-present | Turbine utility

- MTOW: 8,000-8,750 lb (EX)
- Engine: P&W PT6A-114/-140, 675-867 shp turboprop
- Speeds: cruise ~186 kt | Vs ~60 kt (clean)
- Fuel: 332 gal | Range ~1,000+ nm
- Payload: ~3,300 lb useful
- Configurations: wheels (fixed trike), amphib floats (Wipaire 8750), straight floats, skis
- Typical upgrades: Texas Turbine Supervan conversion, cargo pod, glass panel
- Notes: modern bush/cargo workhorse. Turbine complements the Beaver's radial as an audio counterpoint.

### Centurion (Cessna 210)

**[post-v1]** Cessna 210 Centurion | 1957-1986 | Retractable-gear utility cruiser

- MTOW: 3,800 lb
- Engine: IO-520 / IO-550, 285-310 hp
- Speeds: cruise ~174 kt | Vne ~200 kt
- Fuel: 89 gal | Range ~850 nm
- Configurations: wheels only (retractable trike), turbo optional
- Notes: not a bushplane; faster cruiser with retracts. Include to cover the retract-training niche.

### Airvan (Gippsland GA8)

**[post-v1]** Gippsland GA8 Airvan | 1995-present | Modern utility workhorse

- MTOW: 4,200 lb
- Engine: Lycoming IO-540, 300 hp (GA8-TC: PT6A turbine)
- Speeds: cruise ~130 kt | Vs ~55 kt
- Fuel: 91 gal
- Payload: ~1,830 lb useful | 8 seats
- Configurations: wheels (fixed trike), limited float availability
- Notes: Australian outback utility; adds a modern GA alternative.

---

## Floatplanes -- Dedicated

### Biggerlander (DHC-3 Otter)

**[v1]** de Havilland Canada DHC-3 Otter | 1951-1967, ~466 built | Large bush / floatplane

- MTOW: 8,000 lb | Empty: ~4,170 lb
- Engine: Pratt & Whitney R-1340 Wasp, 600 hp 9-cyl radial (turbine conversion: PT6A)
- Airfoil: NACA 64A-series (verify)
- Speeds: cruise ~121 kt | Vs ~58 mph
- Fuel: 178 gal | Range ~720 nm
- Payload: ~2,100 lb useful | up to 10 seats or cargo
- Configurations: wheels, floats (EDO 7170), amphibs, skis
- Typical upgrades: Turbine Otter (DHC-3T / Texas Turbine Otter), VGs, pax interior, cargo interior
- Characteristic handling: Beaver feel, scaled up; heavier and slower
- Modeling difficulty: moderate (Beaver parameter swap)
- Notes: the Beaver's big sister. Include as mid-v1 once Beaver is tuned.

### Norseman (Noorduyn Norseman)

**[post-v1]** Noorduyn Norseman | 1935-1959, ~900 built | Golden-age Canadian bushplane

- MTOW: 7,400 lb
- Engine: P&W R-1340 Wasp or Wright R-975, 500-600 hp radial
- Speeds: cruise ~125 kt | Vs ~58 mph
- Configurations: wheels, floats, skis
- Notes: pre-Beaver Canadian bush icon; adds golden-age flavor.

### Kodiak (Daher Kodiak 100)

**[v1]** Quest Kodiak / Daher Kodiak 100 | 2007-present | Modern turbine utility

- MTOW: 7,305 lb
- Engine: P&W PT6A-34, 750 shp turboprop
- Speeds: cruise ~174 kt | Vs ~60 kt
- Fuel: 320 gal | Range ~1,100 nm
- Payload: ~3,700 lb useful
- Configurations: wheels (fixed trike), amphib floats (Aerocet 6650), straight floats
- Notes: modern purpose-built bush/missionary turbine; clean contemporary design.

### Bush Hawk (Found Bush Hawk XP)

**[post-v1]** Found FBA-2C Bush Hawk | 1995-present (limited) | Purpose-built modern bushplane

- MTOW: 3,500 lb
- Engine: Lycoming IO-540, 300 hp
- Configurations: wheels, floats, amphibs
- Notes: niche modern bushplane.

---

## Flying Boats -- Hull

### Gosling (Grumman G-21 Goose)

**[v1]** Grumman G-21 Goose | 1937-1945, ~345 built | Classic amphibious twin flying boat

- MTOW: 8,000 lb | Empty: ~5,425 lb
- Engine: 2x Pratt & Whitney R-985 Wasp Junior, 450 hp each (post-war: Lycoming conversions)
- Airfoil: NACA 23012 (verify)
- Speeds: cruise ~160 kt | Vne ~190 kt | Vs ~65 kt
- Fuel: 220 gal | Range ~640 nm
- Payload: 4-7 pax + cargo
- Configurations: **hull + retractable main gear** (amphib by design) | wingtip floats
- Hybrid / specialty: hull-retract + wingtip-float arrangement (distinctive)
- Typical upgrades: McKinnon G-21C (four-engine conversion), turbine conversions, radome pods
- Characteristic handling: twin-engine amphib feel; both takeoff surfaces regularly used; sequenced gear retraction for water/land
- Modeling difficulty: moderate (twin, hull hydrodynamics distinct from floats)
- Notes: iconic small amphib flying boat. Twin-engine entry point.

### Sprig (Grumman G-44 Widgeon)

**[post-v1]** Grumman G-44 Widgeon | 1940-1955, ~276 built | Small amphibious twin flying boat

- MTOW: 4,525 lb
- Engine: 2x Ranger 6-440C (200 hp each) originally; most re-engined to Lycoming O-540 (260 hp) post-war
- Speeds: cruise ~140 kt
- Configurations: hull + retract, wingtip floats
- Notes: smaller Goose; charming small amphib with distinctive hull; post-war Super-Widgeon conversions are common.

### Mallard (Grumman G-73)

**[post-v1]** Grumman G-73 Mallard | 1946-1951, 59 built | Executive amphibious twin

- MTOW: 12,750 lb
- Engine: 2x P&W R-1340, 600 hp (turbine conversions exist: Frakes G-73T with PT6A)
- Speeds: cruise ~150 kt
- Configurations: hull + retract, wingtip floats
- Notes: elegant post-war amphib, fewer built than Goose/Widgeon but still flying.

### Albatross (Grumman HU-16)

**[post-v1]** Grumman HU-16 Albatross | 1949-1961, ~466 built | Large SAR amphibian

- MTOW: 30,000-37,500 lb
- Engine: 2x Wright R-1820-76A, 1,425 hp each radial
- Speeds: cruise ~125 kt
- Configurations: hull + retract, wingtip floats
- Notes: large, rare, dramatic presence. Premium unlock for "fly anything" completionists.

### Catalina (Consolidated PBY)

**[post-v1]** Consolidated PBY-5A / PBY-6A Catalina | 1936-1945, ~3,300 built | Iconic WWII amphibian

- MTOW: 35,420 lb
- Engine: 2x P&W R-1830 Twin Wasp, 1,200 hp each radial
- Speeds: cruise ~115 kt | Vne ~170 kt
- Configurations: hull + retract, **retractable wingtip floats** (distinctive feature)
- Hybrid / specialty: retracting wingtip floats that fold up to become wingtips in flight -- unique and worth modeling visually
- Notes: iconic aircraft; fits fire-hopping / rescue roles; retractable wingtip floats are a showcase feature.

### Tarn (Republic RC-3 Seabee)

**[v1]** Republic Aircraft RC-3 Seabee | 1946-1947, ~1,060 built | Small single-engine amphibious flying boat (pusher)

- MTOW: ~3,000 lb | Empty: ~2,100 lb | Useful: ~900 lb
- Engine: Franklin 6A8-215-B8F, 215 hp, 6-cyl air-cooled flat-six, pusher configuration
- Prop: fixed-pitch 2-blade pusher (post-war CS 2/3-blade conversions on modified airframes)
- Airfoil: NACA 4415 (verify)
- Wing area: ~192 sq ft | Aspect ratio: ~5.9
- Speeds: cruise ~93 kt (107 mph) | Vne ~120 mph | Vs ~52 mph | Vso ~50 mph
- Fuel: 75 gal | Range ~400 sm
- Payload: 4 seats + baggage
- Configurations: **hull + retractable tricycle gear** (amphib by design) | fixed wingtip floats
- Hybrid / specialty: high-mounted wing over pusher prop; nose-forward cabin with excellent visibility; gear retracts into the hull sides
- Typical upgrades: Lycoming GO-480 "Super Seabee" (~260 hp, the common re-engine), Lycoming IO-540 conversions, rare turbine conversions, gull-wing door mods, Twin Bee (twin-engine conversion, pusher removed), Simuflight and Whitehouse aerodynamic kits
- Characteristic handling: docile and slow; short-field capable on both surfaces; pusher-prop creates distinctive thrust/pitch coupling (power-on pitches nose up); visibility excellent from nose-forward cabin
- Modeling difficulty: moderate (hull hydrodynamics + tricycle retract + pusher prop + wingtip-float coupling)
- Audio signature: flat-six pusher -- engine and prop sit above and behind the cabin, producing a muted-in-cabin but distinctive external note
- Real-world reference: Seabee POH, Seabee Operator's Manual, Percival Spencer design notes, Simuflight service documents, active RC-3 owners' community
- Notes: the "Jeep of the air" personal amphibian. Distinctive silhouette (high-wing + pusher + nose-forward cabin). Strong Alaska-summer / lake-community vibe. Small but dedicated enthusiast community provides ample technical reference.

### Islet (Icon A5)

**[v1]** Icon A5 | 2014-present | Modern light amphibious sport

- MTOW: 1,510 lb
- Engine: Rotax 912iS, 100 hp
- Speeds: cruise ~88 kt | Vs ~39 kt
- Fuel: 20 gal | Range ~400 nm
- Configurations: hull + retractable trike gear, folding wings, wingtip fences (not floats)
- Notes: recreational modern amphib; accessible entry-level flying boat; unique folding-wing logistics.

### Seastar (Dornier Do 24 / Seastar)

**[bonus]** Dornier Do 24 Seastar | proto 1983, Dornier Seawings resumption | Modern trimotor-era flying boat

- MTOW: ~20,000 lb (Do 24)
- Engine: 3x radial (Do 24) or twin turbine (Seastar)
- Configurations: hull with integrated sponsons (no wingtip floats needed -- distinctive)
- Hybrid / specialty: fuselage-side sponsons replace wingtip floats; high-mounted wing
- Notes: distinctive silhouette; golden-age trimotor feel + modern revival. Premium bonus aircraft.

### Strut-Wing (Sikorsky S-38)

**[bonus]** Sikorsky S-38 | 1928-1933, ~111 built | Golden-age amphibious flying boat

- Engine: 2x P&W Wasp, 400 hp each
- Speeds: cruise ~110 kt
- Configurations: parallel-hull "sesquiwing" twin, hull + retract
- Notes: golden-age icon (Lindbergh, Jimmy Stewart film *The Spirit of St. Louis* set design, African Queen echoes). Visual showpiece.

---

## Light Modern Amphibs (LSA tier)

Small, modern, composite amphibious aircraft in the light-sport / ultralight weight classes. Accessible, cheap to operate, visually distinct from the classic flying boats. Pair well with the Icon A5 already in the Flying Boats section -- together they form a "modern hull" sub-tier.

### Gulling (Chaika L-42 / L-44 / L-45 / L-4)

**[post-v1]** Chaika Aircraft Ltd. (Kyiv) L-42M, L-44M, L-45, L-4 variants | 1999-present | Ukrainian light amphibious flying boat family

- MTOW: ~1,720 lb (L-42M, single) / ~2,400 lb (L-44M, twin) / ~3,000 lb (L-45) (verify per variant)
- Engine: L-42M: Rotax 912 ULS, 100 hp single, pusher on pylon (verify); L-44M: 2x Rotax 912, twin push-pull or side-by-side (verify); L-45: Rotax 914 turbocharged or similar (verify)
- Prop: fixed or ground-adjustable composite, 2-3 blade
- Wing area / airfoil: composite high-wing, airfoil TBD (verify from manufacturer)
- Speeds: cruise ~90-110 kt depending on variant | Vs ~38-45 kt | Vne ~130-140 kt
- Fuel: 20-45 gal depending on variant
- Payload: 2-4 seats + modest baggage (utility-limited, per user framing)
- Configurations: hull + retractable trike gear (amphib by design), wingtip floats or sponsons
- Hybrid / specialty: composite hull construction; pusher-prop configuration on L-42
- Typical upgrades: long-range fuel, GPS, glass panel, Rotax variant swap
- Characteristic handling: light amphib feel, pusher-prop pitch coupling, modest cruise speed, excellent slow-flight for tight water landings
- Modeling difficulty: simple-to-moderate (Rotax engine, pusher configuration adds pitch-trim coupling)
- Audio signature: Rotax 912 at high RPM; quieter than radial-era flying boats; pusher prop note distinct from tractor configurations
- Real-world reference: Chaika Aircraft Ltd. manufacturer data (verify all figures; variants differ meaningfully)
- Notes: Ukrainian-designed light amphibs; the L-42/L-44 family adds modern composite small-amphib flavor to the catalog. User preference noted. Provide the L-42 single as the v1-accessible entry and higher variants as unlocks.

### Sea Bear (L-65 SeaBear)

**[post-v1]** LA Sea Bear L-65 | ~2019 first flight, ongoing development | Modern twin-engine V-tail amphibious composite

- MTOW: ~6,600 lb (3,000 kg) (verify)
- Empty: ~3,800 lb (verify)
- Engine: 2x Rotax 915iS, 141 hp each turbocharged flat-four (~282 hp combined)
- Prop: composite, 3-blade constant-speed (verify per variant)
- Wing: high cantilever composite monoplane; wing area and airfoil TBD (verify from manufacturer)
- **Empennage: V-tail** (distinctive; sets silhouette apart from conventional twin amphibs)
- Speeds: cruise ~130 kt | Vs ~50 kt | Vne ~150+ kt (verify)
- Fuel: ~90-110 gal (verify)
- Range: ~800-1,000 nm (verify)
- Payload: 5 seats (pilot + 4 passengers) + baggage
- Configurations: hull + retractable tricycle gear (amphib by design), sponsons (likely) rather than separate wingtip floats (verify)
- Typical upgrades: avionics packages, cabin trim options, long-range fuel (once out of prototype phase)
- Characteristic handling: twin-Rotax turbo at altitude; V-tail response (coupled pitch/yaw); modern composite amphib feel; more capable than LSA class, closer to Twin Comanche amphib territory
- Modeling difficulty: moderate (twin pusher-style thrust lines, V-tail coupling)
- Audio signature: twin Rotax 915iS -- turbocharged flat-four note, notably quieter and smoother than radials; V-tail turbulent air on the control surfaces adds subtle high-frequency texture
- Real-world reference: manufacturer prototype data, type-certification trajectory (verify all figures; aircraft is new-ish and data is not yet stable)
- Notes: genuinely distinctive modern amphib -- twin turbo-Rotax + V-tail + composite hull is a rare combination. Flight model gets to exercise V-tail coupling and twin turbocharged-altitude response. Flag `(verify)` on most figures until manufacturer data is sourced; prototype-class aircraft should be treated as "in active development" both in reality and in this catalog.

---

## Ultralights & Homebuilt STOL

Small, single- or two-seat aircraft intended for short trips, pond-hopping, beach landings, camping, fishing, and the "just go fly for the afternoon" niche. Limited cargo, limited range, low operating cost -- the antidote to the heavy-utility tier. Covers amphibious ultralights (pond hoppers) and non-amphibious STOL homebuilts (backcountry specialists). These are entry-tier and late-game "just for fun" aircraft both.

### Pondhopper (Progressive Aerodyne SeaRey)

**[v1]** Progressive Aerodyne SeaRey / SeaRey LSX / SeaRey Adventure | 1992-present | Iconic light-sport amphibious pusher

- MTOW: 1,430 lb (LSA) / ~1,500 lb (Sport Pilot)
- Empty: ~950 lb
- Engine: Rotax 912 ULS (100 hp) or 912iS (100 hp), pusher configuration
- Prop: composite 2-3 blade pusher
- Wing area: ~166 sq ft | Airfoil: high-wing braced, USA 35B or similar (verify)
- Speeds: cruise ~90 kt | Vs ~39 kt | Vne ~110 kt
- Fuel: 22 gal (some with extended 30+ gal) | Range ~370 nm
- Payload: 2 side-by-side + small baggage
- Configurations: hull + retractable tricycle gear (amphib), sponson-style wingtip stabilizers
- Hybrid / specialty: pusher-prop layout mounted above wing; folding wings for trailer storage
- Typical upgrades: 912iS conversion, long-range fuel, glass panel, Garmin G3X
- Characteristic handling: light, responsive, very low stall; pusher-prop pitch coupling on power changes; exceptional pond-hopping capability; single-engine amphib feel
- Modeling difficulty: simple (small single, Rotax standard behavior)
- Audio signature: Rotax 912 pusher note -- quieter than tractor configs, slightly muffled
- Real-world reference: Progressive Aerodyne manufacturer data, LSA airworthiness certificate
- Notes: the pond hopper par excellence. More built than any other amphibious LSA. Perfect "afternoon flight to a lake, fish, fly home" aircraft -- a gameplay-tier-appropriate entry-level hull aircraft.

### Dragonfly (Aero Adventure Aventura II / Aventura UL)

**[v1]** Aero Adventure Aventura II (Part 23 kit) / Aventura UL (Part 103 single-seat) | 1997-present | Ultralight amphibious pusher flying boat

- MTOW: 1,430 lb (Aventura II LSA) / 550 lb (Aventura UL Part 103)
- Empty: ~750 lb (II) / ~290 lb (UL)
- Engine: Rotax 582 (65 hp 2-stroke) or Rotax 912 (80-100 hp 4-stroke) pusher on most; small variants use simpler 2-strokes
- Prop: composite 2-blade pusher
- Wing: high parasol wing, simple construction
- Speeds: cruise ~65-80 kt | Vs ~35 kt | Vne ~95 kt
- Fuel: 10-18 gal | Range ~200 nm
- Payload: 2 tandem (II) or 1 (UL) + minimal baggage
- Configurations: hull + retractable trike, wingtip sponsons
- Typical upgrades: 4-stroke Rotax conversion on older 2-stroke builds, ballistic chute, glass panel
- Characteristic handling: minimal, direct, barnstormer-like; 2-stroke variants have distinct power-response curves
- Modeling difficulty: simple
- Audio signature: 2-stroke whine (original) or Rotax 912 (modern) -- distinct depending on variant
- Real-world reference: Aero Adventure manufacturer plans, kit-build community
- Notes: budget entry into amphib flying. Good "starter aircraft" gameplay-wise -- cheap, slow, utility-limited, but you can land it on a pond and fish.

### Wildwing (Kitfox Series 7)

**[v1]** Kitfox Aircraft Kitfox Series 7 (plus earlier Series 1-6 variants) | 1984-present (Series 7: 2006-present) | Homebuilt STOL taildragger, floatable

- MTOW: 1,550 lb (LSA) / 1,800 lb (experimental Series 7 SS)
- Empty: ~770 lb
- Engine: Rotax 912 ULS (100 hp) standard; Continental O-200 (100 hp), Titan O-233 (116 hp), UL Power, or Rotax 914/915 upgrade
- Prop: fixed-pitch 2-blade or CS 3-blade composite
- Wing: high-wing braced, folding for trailer storage; wing area ~130 sq ft | Airfoil: proprietary (similar to Clark Y family) (verify)
- Speeds: cruise ~110 kt | Vs ~38 kt | Vne ~120 kt
- Fuel: 27 gal | Range ~500 nm
- Payload: 2 side-by-side + small cargo compartment
- Configurations: wheels (taildragger standard; trike option), floats (EDO 1320, Wipline 1450), tundra tires, skis
- Typical upgrades: Rotax 914/915 turbo, Catto 3-blade, vortex generators, extended baggage, larger tundra tires
- Characteristic handling: light crisp taildragger, STOL-capable, very popular for rough-field and backcountry use; forgiving by STOL standards
- Modeling difficulty: simple
- Audio signature: Rotax 912 ULS typical; distinctive high-RPM note at cruise
- Real-world reference: Kitfox Aircraft build manuals, community forums, FAA experimental records
- Notes: one of the most popular STOL homebuilts ever; the "you can fly it off your driveway" aircraft. Floats-optional makes it versatile. Great mid-tier backcountry aircraft.

### Hopper (Zenith STOL CH 701)

**[v1]** Zenith Aircraft Company STOL CH 701 | 1986-present (design), still available as kit | Ultralight-class kit STOL two-seater

- MTOW: ~1,100 lb (Part 103 ultralight trainer) / ~1,320 lb (LSA-compliant kit) | Empty: ~580 lb
- Engine: Rotax 912 UL / ULS (80-100 hp); earlier builds with Continental O-200 (100 hp), Jabiru 2200 (85 hp), or VW conversions
- Prop: fixed 2-blade composite or wood; ground-adjustable on some builds
- Airfoil: full-span fixed leading-edge slats + Junkers-style slotted flaperons (defining feature -- not a standard NACA section)
- Wing area: ~122 sq ft | Aspect ratio: ~6
- Speeds: cruise ~65-70 kt | Vne ~100 kt | Vs ~25 kt (with flaperons) | Vso ~23 kt
- Fuel: 15 gal (24 gal extended) | Range ~260 nm
- Payload: 2 side-by-side + minimal baggage
- Configurations: tricycle gear standard (taildragger kits exist); straight floats and skis available as kit conversions
- Hybrid / specialty: full-span slats + slotted flaperons give extraordinary low-speed capability; take-off roll as short as ~90 ft
- Typical upgrades: 100 hp Rotax upgrade from 80 hp, larger tundra tires, ballistic chute, composite 3-blade prop
- Characteristic handling: extreme STOL in a featherweight; high drag and low cruise the trade-off; crosswind sensitivity in the landing flare
- Modeling difficulty: moderate (slats + flaperons require custom aero curves -- cannot be captured by a plain NACA table)
- Audio signature: Rotax 912 buzz; very quiet cruise given low prop speed and light airframe
- Real-world reference: Zenith Aircraft Company builder manuals, Chris Heintz design documentation, community build logs
- Notes: "Sky Jeep" of the kitbuild world. Perfect for pond-hopping, beach strips, and "drop in anywhere" backcountry gameplay. Pairs with the larger CH 750 Super Duty below as a progression pair.

### Skyjeep (Zenith CH 750 STOL / Super Duty)

**[v1]** Zenith Aircraft CH 750 STOL / CH 750 Cruzer / CH 750 Super Duty | 2008-present | Extreme STOL homebuilt

- MTOW: 1,320 lb (STOL LSA) / 1,440 lb (Super Duty experimental) / 1,440 lb (Cruzer)
- Empty: ~720 lb
- Engine: Rotax 912 ULS (100 hp), Continental O-200 (100 hp), UL Power 520i (130 hp), Jabiru 3300 (120 hp), or Lycoming O-233
- Prop: fixed-pitch ground-adjustable 2-blade (typical)
- Wing: drooped leading edge with fixed leading-edge slats, full-span flaperons; wing area ~126 sq ft | Airfoil: Reisner modified NACA (verify)
- Speeds: cruise ~90 kt | **Vs ~28 kt** | Vne ~120 kt
- Fuel: 24 gal (standard) / 48 gal (Super Duty)
- Range: ~370 nm standard
- Payload: 2 side-by-side + generous cargo (Super Duty carries 4)
- Configurations: wheels (tricycle standard; tailwheel and bushwheel options), skis, floats (uncommon but available)
- Typical upgrades: Super Duty airframe (4-seat, 180 hp), bushwheels, larger fuel, glass panel
- Characteristic handling: extraordinarily short takeoff and landing; welded-cage construction (forgiving of hard landings); unglamorous but trustworthy
- Modeling difficulty: simple with aggressive STOL tuning (fixed slat changes lift-curve shape vs clean wing)
- Audio signature: depends on engine; Rotax 912 standard; UL520 adds deeper note
- Real-world reference: Zenith Aircraft manuals, community builds
- Notes: the "Sky Jeep" -- minimal frills, extreme STOL. Perfect for remote short-strip operations.

### Backcountry (Just Aircraft SuperSTOL)

**[v1]** Just Aircraft SuperSTOL / SuperSTOL XL / Escapade / Highlander | 2001-present | Modern extreme-STOL backcountry homebuilt

- MTOW: 1,320 lb (LSA SuperSTOL Classic) / 1,550 lb (SuperSTOL XL)
- Empty: ~800 lb
- Engine: Rotax 912, 912iS, 914, 915iS, Titan O-340, Lycoming O-320, O-360 (100-200 hp)
- Prop: composite 2-3 blade, CS optional
- Wing: **automatically-deploying leading-edge slats** (gravity-deploying or spring-loaded), vortex generators, long-span flaps; wing area ~135 sq ft
- Speeds: cruise ~100 kt | **Vs ~26 kt** (slats deployed) | Vne ~120 kt
- Fuel: 24-35 gal | Range ~500 nm
- Payload: 2 side-by-side + cargo
- Configurations: wheels (taildragger standard), bushwheels (Alaskan 26/29/31/33"), floats, skis
- Typical upgrades: Rotax 915iS turbo (high-altitude performance), Titan/Lycoming swap (more power), glass panel, extended baggage, tundra wheel combos
- Characteristic handling: **slats deploy automatically as airspeed slows** -- approaches below 30 kt feel mostly normal; climbing away from stall authority is a defining feel; bush-pilot favorite
- Modeling difficulty: moderate (auto-slat behavior changes lift-curve shape dynamically)
- Audio signature: engine-dependent; Rotax 915iS adds turbo whine
- Real-world reference: Just Aircraft manuals, Draco / Valdez STOL Fly-in videos
- Notes: modern backcountry extreme-STOL. Auto-slats are the signature feature and a rewarding flight-model detail. "The Alaska bush pilot's weekend aircraft."

---

## Military Conversions -- Civilian Use

Famous military seaplanes with historical or ongoing civilian service. Included for flight-model flavor and experience, not as economic workhorses -- cargo and utility are limited by design per user framing. These aircraft expand the flight-model envelope to cover large hulls, unusual configurations, and era-defining silhouettes.

### Windsor (Short Sunderland / Sandringham / Solent)

**[bonus]** Short S.25 Sunderland + civilian conversions (Sandringham, Solent) | 1938-1946 military, Sandringham/Solent civil conversions 1945-1974 | RAF WWII patrol flying boat

- MTOW: ~65,000 lb (Mk V) / similar for Sandringham
- Engine: 4x Bristol Pegasus XVIII (Mk III) or Pratt & Whitney R-1830 Twin Wasp (Mk V), 1,050-1,200 hp each radial
- Prop: 3-blade constant-speed on each engine
- Wing area / airfoil: ~1,487 sq ft, thick cantilever monoplane wing (verify airfoil section)
- Speeds: cruise ~135 kt | Vne ~180 kt | Vs ~75 kt
- Fuel: ~2,550 gal main (huge range aircraft; civilian variants retuned)
- Payload: civilian Sandringham carried 24-45 passengers + crew; freight capacity significant but compartment-limited
- Configurations: hull only (true flying boat, no amphib); retractable beaching gear for ground handling
- Hybrid / specialty: hull-only, no wheels for in-flight; beaching gear used exclusively for slipway transit
- Typical upgrades (civilian conversion era): passenger interior, extended range tanks, modern avionics retrofit
- Characteristic handling: four-engine heavy, slow, stable; significant torque on takeoff; requires coordinated throttle technique
- Modeling difficulty: complex (four engines, large hull hydrodynamics, multi-crew cockpit feel)
- Audio signature: four radials in harmony -- iconic "bomber in the sky" presence; Pegasus vs R-1830 vs Twin Wasp each has a subtly different exhaust note
- Real-world reference: Ansett Flying Boat Services records, Sandringham/Solent POHs, *Short Sunderland: The 'Flying Porcupines' at War* (Norman Franks, reference); Foynes Flying Boat Museum archives
- Notes: The "Flying Porcupine." Ansett operated Sandringhams in Australia until 1974, carrying passengers and freight; the Solent was a BOAC passenger variant. Premium bonus unlock; flight-model showcase for large hull operations.

### Duck (Grumman J2F Duck)

**[bonus]** Grumman J2F Duck + civilian-certified airworthy examples | 1933-1945 military, civilian warbird ownership since | US Navy / Coast Guard / Marines single-engine amphibious utility biplane

- MTOW: 7,700 lb
- Engine: Wright R-1820-54 Cyclone, 950 hp, 9-cyl radial
- Prop: 3-blade constant-speed
- Wing area / airfoil: biplane, upper wing ~409 sq ft total wing area (verify)
- Speeds: cruise ~135 kt | Vne ~200 kt | Vs ~65 kt
- Fuel: 150 gal (military spec; civilian conversions vary)
- Payload: 2 crew + light cargo; limited utility in civilian use
- Configurations: **hull + retractable gear, with the lower wing serving as the central float -- lower-wing-as-float is unique**; optional wingtip floats; amphib by design
- Hybrid / specialty: **biplane + amphibious flying boat hybrid** -- the lower wing fairs directly into a central pontoon structure housing the retractable main gear. Unique in aviation.
- Typical upgrades (civilian): warbird restoration, airshow livery, gentle avionics retrofit
- Characteristic handling: biplane stability with hull water handling; single-engine amphib manageability; radial torque on takeoff
- Modeling difficulty: moderate (biplane lift stacking + hull hydrodynamics + unique lower-wing-float geometry)
- Audio signature: single Wright R-1820 radial -- distinctive from R-985 / R-1340 tone
- Real-world reference: surviving civilian-owned J2F Ducks (Planes of Fame, private collectors), *Grumman JF/J2F Duck* (Squadron/Signal publications)
- Notes: one of the most visually distinctive aircraft ever built. The lower-wing-as-float design makes it a singular flight-model specimen -- useful for pushing the buoyancy + aerodynamics integration. Movie star aircraft (*Murphy's War* / *Flight of the Intruder* set piece).

### Mariner (Martin PBM)

**[bonus]** Martin PBM-3 / PBM-5 Mariner | 1941-1949, ~1,366 built | US Navy WWII patrol flying boat (limited civilian post-war use)

- MTOW: 58,000 lb (PBM-5)
- Engine: 2x Wright R-2600 Twin Cyclone (PBM-3), ~1,700 hp each OR 2x Pratt & Whitney R-2800 Double Wasp (PBM-5), 2,100 hp each radial
- Prop: 4-blade constant-speed on each engine
- Wing area / airfoil: ~1,408 sq ft, gull-wing configuration (verify airfoil, likely NACA 23-series)
- Speeds: cruise ~150 kt | Vne ~215 kt | Vs ~76 kt
- Fuel: ~1,900 gal (civilian range significantly extendable)
- Payload: crew + patrol stores; civilian conversions limited, utility-bounded per user framing
- Configurations: hull only, retractable beaching gear, wingtip floats
- Hybrid / specialty: gull-wing (anhedral inboard, dihedral outboard) planform
- Typical upgrades (civilian/modern): R-2800 conversion for PBM-3 bases, modernized cockpit, long-range tanks
- Characteristic handling: gull-wing twin-radial heavy; coordinated throttle technique; good rough-water capability
- Modeling difficulty: complex (twin + hull + gull-wing aerodynamics)
- Audio signature: twin Double Wasp; deeper note than Twin Wasp, authoritative
- Real-world reference: USN archive records, Martin aircraft company archives, *Martin PBM Mariner* (Squadron/Signal)
- Notes: a handful operated post-war civilian; included primarily for flight-model variety. The gull-wing silhouette is distinctive and rewards careful rendering.

---

## Twins and Turbines

### Duotter (DHC-6 Twin Otter)

**[v1]** de Havilland DHC-6 Twin Otter | 1965-1988, Viking 2010-present, ~960+ built | Twin turbine utility

- MTOW: 12,500 lb
- Engine: 2x P&W PT6A-27 or -34, 620-750 shp each turboprop
- Airfoil: DHC-developed high-lift section
- Speeds: cruise ~160 kt | Vs ~58 kt (clean)
- Fuel: 378 gal | Range ~800 nm
- Payload: ~4,000 lb useful | 19-seat commuter
- Configurations: wheels (fixed trike), floats (Wipline 13000), amphibs, skis, tundra tires (TT version)
- Typical upgrades: Viking Series 400 avionics, cargo interior, winglets
- Notes: extensive bush / Arctic / island service; twin turbine entry point for heavy missions.

### Drover (GAF Nomad)

**[post-v1]** Government Aircraft Factories N22 / N24 Nomad | 1971-1984, ~172 built | Australian twin-turboprop STOL utility

- MTOW: N22B ~8,500 lb | N24A ~9,400 lb | Empty: N22B ~4,800 lb (verify)
- Engine: 2x Allison / Rolls-Royce 250-B17 series, ~420 shp each turboprop
- Prop: Hartzell 3-blade reversing feathering constant-speed
- Airfoil: NACA 64-series high-lift section (verify)
- Wing area: ~324 sq ft | Aspect ratio: ~10 (verify)
- Speeds: cruise ~168 kt | Vne ~200 kt | Vs ~58 kt (verify)
- Fuel: ~274 gal (N22B) | Range ~730 nm
- Payload: 12 pax / freight (N22), 16 pax / freight (N24); military Searchmaster variant for maritime patrol
- Configurations: wheels (fixed tricycle); rare float conversions; no production ski variant
- Hybrid / specialty: full-span leading-edge slats + double-slotted flaps give short-field performance at twin-turboprop weight
- Typical upgrades: refurb avionics packages, re-engine studies (rare, modern -C20 variants), cargo-door mods
- Characteristic handling: STOL-capable for its class; steady twin-turboprop feel with notable prop-reverse authority on rollout; docile in bush ops; historical asymmetric-thrust quirks documented in the type's safety record
- Modeling difficulty: complex (twin turboprop + reversing prop + STOL slats/flaps + two-variant fuselage length)
- Audio signature: pair of Allison 250 turboprops -- higher-pitched whine than the Twin Otter's PT6, distinctive chirp on start
- Real-world reference: GAF Nomad flight manuals, RAAF / PNGDF operator histories, Australian type-certificate data, operator-community technical write-ups
- Notes: rare outside Australia / PNG / Indonesia / Philippines; strong "remote utility" silhouette for the bush and island biomes. Good fit for mid-tier twin-turbine missions when the Twin Otter is overkill.

### Glacier (Pilatus PC-12)

**[post-v1]** Pilatus PC-12 / PC-12 NG / PC-12 NGX | 1991-present, ~2,000+ built | Swiss pressurized single-engine turboprop utility / executive

- MTOW: ~10,450 lb (PC-12/45) to ~10,995 lb (PC-12/47E, NG, NGX) | Empty: ~6,200 lb
- Engine: Pratt & Whitney Canada PT6A-67B (-45), -67P (-47E/NG), -67P variant (NGX), 1,200-1,845 shp rated (derated for type)
- Prop: Hartzell 4-blade (early) / 5-blade (NGX) constant-speed reversing feathering; composite MT on some
- Airfoil: Pilatus high-lift section (verify)
- Wing area: ~277 sq ft | Aspect ratio: ~10.3 | winglets standard since NG
- Speeds: cruise ~280 kt | Vne ~280+ | Vs ~67 kt | Vso ~61 kt
- Fuel: 402 gal | Range ~1,800 nm (NG/NGX)
- Payload: 9-11 pax pressurized cabin (9 executive, 11 combi); cargo-door variants
- Configurations: retractable tricycle gear only (no factory float variant due to weight and pressurization)
- Hybrid / specialty: pressurized cabin for high-altitude cruise over mountains; large aft cargo door for combi operations
- Typical upgrades: NG / NGX retrofits, glass panel upgrades, extended-range tanks, medevac interior, government / special-mission sensor pods
- Characteristic handling: fast cruise for a single-turbine utility; long-range; good rough-field capability for its class though not STOL; forgiving handling with strong single-engine reliability record
- Modeling difficulty: complex (pressurization cabin model, high-speed turbine, autopilot expectations, service-ceiling density-altitude effects)
- Audio signature: PT6A-67 variant -- louder inlet whine than the -27/-34 in the Porter, 5-blade NGX prop quieter on the tip
- Real-world reference: Pilatus PC-12 POH, PT6A-67P installation data, PlaneSense / Surf Air Mobility / government operator histories
- Notes: end-game utility for long-range high-altitude missions. Pairs with the Porter (single-turbine low-and-slow) as the fast-and-high counterpart. Corporate / medevac / high-altitude air-ambulance mission templates fit here.

---

## Golden Age / Historic

### Trimotor (Ford 5-AT)

**[post-v1]** Ford Trimotor | 1925-1933, ~199 built | Golden-age trimotor airliner

- MTOW: 13,500 lb
- Engine: 3x Pratt & Whitney Wasp or Wright Whirlwind, ~300-450 hp each
- Speeds: cruise ~95 kt | Vs ~50 kt
- Fuel: 235 gal
- Configurations: wheels primarily; some floats configurations rare
- Notes: iconic "Tin Goose" golden-age airliner; niche inclusion for vintage missions.

### Vega (Lockheed Vega 5)

**[bonus]** Lockheed Vega | 1927-1934, 128 built | Record-setting monocoque monoplane

- MTOW: 4,265 lb
- Engine: P&W Wasp, 420 hp radial
- Speeds: cruise ~135 kt | Vne ~185 kt
- Configurations: wheels, floats (rare)
- Notes: Earhart / Post icon. Faster monoplane bonus tier.

### Clipper (Sikorsky S-42 or Boeing 314)

**[bonus]** Sikorsky S-42 / Boeing 314 Clipper | 1934-1945 | Giant flying-boat airliner

- MTOW: 38,000 lb (S-42) / 82,500 lb (314)
- Engine: 4x Pratt & Whitney Twin Wasp / Wright Twin Cyclone
- Configurations: hull, wingtip floats (S-42) or fuselage sponsons (314)
- Notes: trans-oceanic golden-age flying boats. Aspirational bonus.

### Cairn (Pilatus SB-2 Pelican)

**[bonus]** Pilatus SB-2 Pelican | 1944 prototype, one built (HB-AEP) | Pioneering Swiss single-engine STOL mountain utility

- MTOW: ~2,900 lb (verify) | Empty: ~2,100 lb (verify)
- Engine: Argus As 10C-3 inverted air-cooled V8, ~240 hp
- Prop: 2-blade wooden fixed-pitch
- Airfoil: high-lift section with fixed leading-edge slats + slotted flaps (pre-war experimental, verify geometry)
- Wing area: ~269 sq ft (verify) | Aspect ratio: ~7
- Speeds: cruise ~90 kt (verify) | Vne ~130 mph (verify) | Vs ~32 kt (verify)
- Fuel: ~40 gal (verify)
- Payload: 4-5 seats
- Configurations: wheels (fixed taildragger); skis documented in period photographs
- Hybrid / specialty: one of the first purpose-designed STOL mountain utility aircraft; conceptual ancestor of the Pilatus PC-6 Porter lineage
- Characteristic handling: slow, docile, alpine-strip capable; pre-war STOL design language made mechanical with slats + slotted flaps
- Modeling difficulty: moderate (slat + slotted-flap interaction, pre-war V8 torque character)
- Audio signature: Argus As 10 inverted V8 -- distinctive pre-war German aero-engine tone, rare in any sim catalog
- Real-world reference: Pilatus company historical archives, Swiss Air Force historical documents, *Pilatus: Seit 1939* (company history), preserved HB-AEP records
- Notes: only one built; a historical curiosity and the seed from which Pilatus's later mountain-utility line grew. Included for catalog completeness and as a rare pre-war STOL single to contrast with the Helio Courier and PC-6 later in the tree.

---

## Fire / Rescue Specialty

### Scooper (Canadair CL-215 / CL-415 / DHC-515)

**[v1]** Canadair CL-415 Super Scooper (now DHC-515) | 1969-present | Purpose-built amphibious water bomber

- MTOW: 43,850 lb (CL-415)
- Engine: 2x PW123AF turboprop, 2,380 shp each (CL-215: R-2800 radial)
- Speeds: cruise ~180 kt | Vs ~76 kt
- Payload: 1,620 gal water tank
- Configurations: hull + retract, wingtip floats, water scooping mechanism
- Hybrid / specialty: water-scoop probes on the hull -- land on water, fill tanks in 12 seconds, climb out, drop
- Typical upgrades: foam injection system, drop-pattern selector
- Notes: fire-hopping specialty for the Rocky Mountain / high-desert / Mediterranean biomes. Iconic firefighting silhouette.

### Fire Boss (Air Tractor AT-802F on Wipline floats)

**[post-v1]** Air Tractor AT-802F "Fire Boss" on Wipline 10000 amphib floats | 1990-present | Single-engine air tanker

- MTOW: 16,000 lb
- Engine: PT6A-67AG, 1,350 shp
- Payload: 800 gal water/retardant
- Configurations: amphib floats (Fire Boss variant), wheels (AT-802 standard)
- Notes: single-engine scoop option; more common at smaller operations.

---

## Media / Bonus Tier

Pop-culture homage aircraft, sanitized and visually distinctive. The user is expected to propose specifics; starter sketches only.

### S.21 Racer (Porco Rosso)

**[bonus]** Savoia S.21 Folgore (fictional, *Porco Rosso*) | Golden-age Italian flying-boat racer

- Single-seat hull, parasol wing, inline engine
- Configurations: hull + wingtip floats
- Notes: the archetypal flying-boat racer from Miyazaki's film. Distinctive red silhouette; tight aerobatic feel.

### Sea Drake (Conwing L-16 Sea Duck, TaleSpin)

**[bonus]** Conwing L-16 "Sea Duck" (fictional, *TaleSpin*, Disney 1990-1991) | Cargo amphibian flying boat, golden-age styling | Closest real-world analogues: Sikorsky S-43, Grumman Goose, Sikorsky S-38

- MTOW: ~9,000-10,000 lb (fictional; Goose/S-43-class baseline)
- Engine: 2x radial, ~450-600 hp each (baseline from R-985 Wasp Junior to R-1340 Wasp)
- Prop: 2-blade constant-speed on each engine
- Wing area / airfoil: high parasol cantilever wing above fuselage; ~375 sq ft (estimate); airfoil a thick classic section
- Speeds: cruise ~150 kt | Vne ~180 kt | Vs ~60 kt
- Fuel: 200 gal main (estimate); additional cargo-bay aux tanks implied by lore
- Payload: 2-4 seats + significant cargo volume (the Sea Duck is a cargo aircraft by narrative design)
- Configurations: hull + retractable main gear (amphib), wingtip floats, distinctive parasol wing mount
- Hybrid / specialty: high parasol wing with engines mounted above; amphib hull with roof-mounted cargo door; rugged cargo-bush flavor
- Typical upgrades (fictional lore): "Sky Pirates"-grade reinforcements, extra fuel tanks in the cargo bay
- Characteristic handling: medium twin amphib, workboat-in-the-sky feel; strong torque on takeoff; water-handling biased toward moderate chop (Khyber-Pass adventurous rather than flat-water serene)
- Modeling difficulty: moderate (twin + hull + distinctive geometry)
- Audio signature: twin radials, slightly offset exhaust for character; cargo-bay rattle accents
- Real-world reference: *TaleSpin* series bible, fan-compiled technical analyses (variously citing S-43 and Goose); publicly drawn fan specs
- In-game name placeholder: "Sea Drake" (candidate -- needed to avoid Disney IP; alternatives: "Bearclaw," "Cargo Goose," "Yellowjacket")
- Notes: sanitize the silhouette slightly from the cartoon original (different cockpit glazing shape, altered vertical stabilizer, unique engine-nacelle detailing) to avoid IP exposure; retain the spirit (yellow-green livery, parasol wing, twin radials, cargo-bay door) as a canonical unlockable livery set. Cargo-amphib character fills a distinct gameplay niche between the Beaver (utility) and Goose (twin amphib).

### Clipper (Pan Am S-43 / S-42 -- Indiana Jones)

**[bonus]** Sikorsky S-43 Baby Clipper | 1935-1944 | Indiana Jones opening / establishing icon

- Twin-engine flying boat, trans-oceanic stages
- Notes: montage / long-range establishing aircraft.

### Further media candidates

User-proposed list expected here. Remaining candidate slots: *Fraggle Rock* airship, *Avatar* Samson, *TaleSpin* Iron Vulture (rival airship -- if airships ever enter scope), *Miyazaki* airships (Castle in the Sky, Nausicaa), *Crimson Skies* Zeppelins / Devastator, *Porco Rosso* Curtiss Pirate bomber, *Sky Captain* P-40s on floats, etc. -- none committed.

---

## Configuration Matrix

Quick lookup of which configurations each planned aircraft supports. `o` = standard, `*` = common optional, `-` = not typical, `?` = TBD.

| Aircraft           | Wheels TD | Wheels NW | Floats | Amphib Floats | Hull | Hull+Retract | Skis | Tundra | Notes                 |
| ------------------ | --------- | --------- | ------ | ------------- | ---- | ------------ | ---- | ------ | --------------------- |
| Northlander (Beaver)| o        | -         | o      | *             | -    | -            | *    | *      | MVP                   |
| Birchling (Super Cub)| o       | -         | *      | -             | -    | -            | *    | *      |                       |
| Huskling (Husky)   | o         | -         | *      | *             | -    | -            | *    | *      |                       |
| Carbon Falcon      | o         | -         | *      | -             | -    | -            | *    | o      | STOL                  |
| Tussock (Maule)    | o         | -         | *      | *             | -    | -            | *    | *      |                       |
| Courser (Helio)    | o         | -         | *      | -             | -    | -            | *    | -      | STOL w/ slats         |
| Skyfly 172         | -         | o         | *      | *             | -    | -            | -    | -      |                       |
| Skywagon 180       | o         | -         | *      | *             | -    | -            | *    | *      |                       |
| Skywagon 185       | o         | -         | *      | *             | -    | -            | *    | *      |                       |
| Stationair (206)   | -         | o         | *      | *             | -    | -            | -    | -      |                       |
| Caravan (208)      | -         | o         | *      | *             | -    | -            | *    | -      | Turbine               |
| Centurion (210)    | -         | o (retract)| -     | -             | -    | -            | -    | -      | Retract               |
| Airvan (GA8)       | -         | o         | *      | -             | -    | -            | -    | -      |                       |
| Biggerlander (Otter)| o        | -         | o      | *             | -    | -            | *    | *      |                       |
| Norseman           | o         | -         | o      | -             | -    | -            | *    | -      |                       |
| Kodiak             | -         | o         | *      | *             | -    | -            | -    | -      | Turbine               |
| Bush Hawk          | -         | o         | *      | *             | -    | -            | -    | -      |                       |
| Gosling (Goose)    | -         | -         | -      | -             | o    | o            | -    | -      | Twin amphib           |
| Sprig (Widgeon)    | -         | -         | -      | -             | o    | o            | -    | -      | Twin amphib           |
| Mallard (G-73)     | -         | -         | -      | -             | o    | o            | -    | -      | Twin amphib           |
| Albatross (HU-16)  | -         | -         | -      | -             | o    | o            | -    | -      | Large twin amphib     |
| Catalina (PBY)     | -         | -         | -      | -             | o    | o            | -    | -      | Retract wingtip floats|
| Islet (Icon A5)    | -         | -         | -      | -             | o    | o            | -    | -      | Light sport amphib    |
| Gulling (Chaika L-42/44/45)| -  | -         | -      | -             | o    | o            | -    | -      | Modern LSA amphib fam.|
| Sea Bear (L-65)    | -         | -         | -      | -             | o    | o            | -    | -      | Twin Rotax V-tail amphib |
| Pondhopper (SeaRey)| -         | -         | -      | -             | o    | o            | -    | -      | Iconic LSA pond hopper|
| Dragonfly (Aventura II)| -     | -         | -      | -             | o    | o            | -    | -      | Budget ultralight amphib |
| Wildwing (Kitfox)  | o         | -         | *      | -             | -    | -            | *    | *      | Floatable STOL homebuilt |
| Skyjeep (Zenith CH 750)| -     | o         | *      | -             | -    | -            | *    | -      | Extreme STOL (fixed slats) |
| Backcountry (SuperSTOL)| o     | -         | *      | -             | -    | -            | *    | o      | Auto-slat STOL, bushwheel |
| Windsor (Sunderland)| -        | -         | -      | -             | o    | -            | -    | -      | Hull only; beaching gear |
| Duck (J2F)         | -         | -         | -      | -             | o    | o            | -    | -      | Biplane + hull + lower-wing-float |
| Mariner (PBM)      | -         | -         | -      | -             | o    | -            | -    | -      | Gull-wing twin hull   |
| Duotter (DHC-6)    | -         | o         | *      | *             | -    | -            | *    | *      | Twin turbine          |
| Scooper (CL-415)   | -         | -         | -      | -             | o    | o            | -    | -      | Water bomber          |
| Fire Boss (AT-802F)| -         | -         | -      | *             | -    | -            | -    | -      | Water bomber (single) |
| Sea Drake (Sea Duck)| -        | -         | -      | -             | o    | o            | -    | -      | Cartoon cargo amphib  |

Configuration columns:
- **Wheels TD** -- conventional taildragger main + tailwheel
- **Wheels NW** -- tricycle (nosewheel) layout
- **Floats** -- straight floats (no gear in floats)
- **Amphib Floats** -- amphibious floats with retractable gear in floats
- **Hull** -- integral flying-boat hull (straight water-only)
- **Hull+Retract** -- hull amphib with retractable gear in hull/sponson
- **Skis** -- ski landing gear for snow / ice
- **Tundra** -- oversize bush tires (31"+ typical)

## Common Upgrade Catalog

Upgrade hooks shared across many aircraft. When implemented, these become inventory items / hangar service options; their exact mechanical effect is per-aircraft in the spec.

### Engine

- **Engine re-rate** -- increased horsepower from same engine (conversion to O-360, IO-520, IO-550, etc.)
- **Turbine conversion** -- PT6A replacement for piston radial (Turbo Otter, Turbine Beaver, Supervan, Texas Turbine Otter). Step change in power, range, reliability, fuel burn, cost, audio signature.
- **Turbocharger** -- for piston aircraft at altitude (Cessna 172RG / 182T / 206T variants).

### Propeller

- **Constant-speed prop** upgrade from fixed-pitch.
- **Feathering prop** for twin / engine-out glide.
- **3-blade or 4-blade** aftermarket (marginal performance, noise, vibration).
- **BlackJack / composite** lightweight props.

### Wing / lift

- **STOL kit** -- drooped leading edge (Horton, Robertson, Wing-X, Sportsman cuff).
- **Vortex generators (VGs)** -- reduce stall speed marginally.
- **Winglets** -- reduce induced drag.
- **Leading-edge slats** -- significant stall improvement (Helio-style).

### Gear

- **Tundra tires** -- 31" / 35" Alaskan Bushwheel options.
- **Amphib float retrofit** -- convert straight floats to amphibious.
- **Ski kit** -- winter operations.
- **Heavy-duty gear legs** -- reinforced for rough-field ops.

### Fuel / range

- **Long-range tanks** -- extended fuel (wing / tip / aux).
- **Cargo pod** -- belly-mounted additional storage (adds drag).

### Avionics / capability

- **Glass panel** upgrade (Garmin G3X / G500-tier placeholder).
- **GPS / moving map**.
- **Autopilot** -- tiered (wing-leveler / 2-axis / full coupled).
- **Weather radar** (post-v1).
- **ADS-B, traffic awareness** (marginal).

### Interior

- **Cargo interior** -- remove seats, add tiedowns.
- **Passenger interior** -- club seating, cabin trim.
- **Convertible interior** -- quick-change (quick-release seat tracks).

---

## References to Chase

Public sources for verifying spec figures during Phase 2+ implementation:

- **Type Certificate Data Sheets (TCDS)** -- FAA TCDS for each aircraft (A-XXX-XX format), publicly available. Gives authoritative MTOW, engine options, fuel capacity, airfoil, etc.
- **Pilot's Operating Handbooks (POH)** -- widely shared PDFs for older aircraft; OEM manuals for in-production.
- **NACA Technical Notes / Reports** -- airfoil Cl/Cd tables (TN-1945 for 64A series, TR-824 and others for common airfoils).
- **Manufacturer technical data** -- Viking Air (Beaver / Otter / Twin Otter), Daher (Kodiak), Aviat (Husky), CubCrafters, Maule.
- **Operator-community knowledge** -- SuperCub.org, Backcountry Pilot forums, Alaska/Canada bush operator retrospectives, Tecnam / Kitplanes articles.
- **Existing flight-sim models** for parameter cross-reference (X-Plane, MSFS, FSX community mods) -- treat as *reference*, not ground truth.
- **Historic references** -- Jane's All the World's Aircraft (dated editions available), AeroFiles, flyingboatmuseum.com.
