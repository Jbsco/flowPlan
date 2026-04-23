# World Generation — Implementation Reference

> **Role in the docs stack:** companion to [MAPGEN.md](../MAPGEN.md). MAPGEN is the **design** doc (the *what* and *why*); this is the **implementation** reference (the *how* — actual algorithms, function signatures, parameters, performance notes). Read MAPGEN first for the architectural rationale; read this when you need to know what `Terrain::height` actually does or which octave offsets are reserved for which subsystem.
>
> **Status:** Phase 3.6.0–3.6.1c shipped. Updated 2026-04-23.

## Contents

- [Architecture overview](#architecture-overview)
- [Noise primitives](#noise-primitives)
- [Mipmap-aware sampling](#mipmap-aware-sampling)
- [Octave-offset convention](#octave-offset-convention)
- [Terrain composition](#terrain-composition)
- [Altitude envelope (real-world-possible gate)](#altitude-envelope-real-world-possible-gate)
- [Biome classification + soft weighting](#biome-classification--soft-weighting)
- [Per-biome terrain profiles](#per-biome-terrain-profiles)
- [Hydrology graph (3.6.1d)](#hydrology-graph-361d)
- [Weather field generation](#weather-field-generation)
- [Voronoi cells (variable size)](#voronoi-cells-variable-size)
- [Toroidal world topology](#toroidal-world-topology)
- [Visual-bench artifacts](#visual-bench-artifacts)
- [Performance characteristics](#performance-characteristics)
- [Determinism contract](#determinism-contract)

---

## Architecture overview

flowPlan's world is a **flat square torus** of side `WORLD_EXTENT_M = 5,000,000 m` (5000 km), wrapping continuously on both axes. World content composes from independently-deterministic samplers all driven by a single `master_seed: u64`:

```
┌─────────────────────────────────────────────────────────────────┐
│                         master_seed                             │
└────────────────────────────┬────────────────────────────────────┘
                             │
        ┌────────────────────┼────────────────────────┐
        │                    │                        │
        ▼                    ▼                        ▼
┌──────────────┐    ┌─────────────────┐    ┌──────────────────┐
│   TERRAIN    │    │     WEATHER     │    │     VORONOI      │
│              │    │                 │    │                  │
│ regional fBm │    │ temperature     │    │  weighted-power  │
│ + fine fBm   │    │  + latitude     │    │   diagram cells  │
│ + per-biome  │    │ humidity fBm    │    │  (variable size) │
│   primitives │    │ pressure fBm    │    │                  │
└──────┬───────┘    └────────┬────────┘    └──────────────────┘
       │                     │
       │                     ▼
       │          ┌─────────────────────┐
       │          │  BIOME WEIGHTS      │
       │          │  (smoothstepped     │
       │          │   from temp + hum)  │
       │          └──────────┬──────────┘
       │                     │
       └──────────┬──────────┘
                  │
                  ▼
        ┌─────────────────┐
        │ height_at_scale │  ← public API
        └─────────────────┘
```

All samplers are **pure functions of `(master_seed, x, z, ...)`** — no global state, no wall-clock dependencies, bit-identical on every machine.

Crate map:

| Layer | Module | Responsibility |
|---|---|---|
| Foundation | `flowplan_core::world_topology` | `WORLD_EXTENT_M` constant, `wrap_coord`, `wrap_cell_index`, `quantize_*_frequency` |
| Noise | `flowplan_world::noise` | Perlin-fBm, ridged multifractal, domain warping (mipmap-aware variants) |
| Terrain | `flowplan_world::terrain` | `Terrain::height(x, z)`, biome composition, beach flatten |
| Biome | `flowplan_world::biome` | `BiomeClass` enum, `biome_at`, `biome_weights_at` (soft membership) |
| Weather W1 | `flowplan_world::weather::{pressure, temperature, humidity, wind, frontal, turbulence}` | Atmospheric base fields |
| Weather W2 | `flowplan_world::weather::{voronoi, cell_state}` | Weather-cell partition + per-cell state |
| Weather W3 | `flowplan_world::weather::{deltas, orographic}` | Per-cell wind/turbulence/precip deltas |
| Sky | `flowplan_world::sky` | Sun direction, longitude-varying local time |

---

## Noise primitives

All in `crates/world/src/noise.rs`. Three families, all **toroidal-wrap-continuous by construction** — the gradient grid wraps via `wrap_cell_index(ix, n_cells)` so `sample(x) ≡ sample(x + WORLD_EXTENT_M)`.

### Perlin 2D — `perlin_2d(seed, octave, x, z, spacing_m) -> f32`

Standard Ken Perlin gradient noise. Each grid cell has a pseudo-random gradient direction (hash of `(seed, octave, ix, iz)` via Knuth multipliers + `splitmix64` mixing). At a query point, dot-product the relative position with each of the 4 corner gradients, bilinearly interpolate using Perlin's quintic fade `t³(6t² − 15t + 10)`. Output bounded in `[−1/√2, 1/√2] ≈ [−0.707, 0.707]`.

**Wrap requirement**: `WORLD_EXTENT_M / spacing_m` must be integer (callers enforce by picking `spacing_m` that divides extent evenly).

### Fractional Brownian motion — `fbm_2d(seed, octave_offset, x, z, base_spacing_m, octaves)`

Sum of Perlin octaves at halving spacing (lacunarity 2.0) and halving amplitude (persistence 0.5):

```
fbm(x, z) = Σ_{i=0..N-1}  0.5^i  ×  perlin(x, z, base_spacing / 2^i)
```

Produces natural rolling terrain. Typical raw range `[−1.4, 1.4]` (6 octaves correlated case).

### Ridged multifractal — `ridged_multifractal_2d(seed, octave_offset, x, z, base_spacing_m, octaves, offset, power)`

fBm variant with absolute-value-inversion + power-shaping per octave:

```
ridged(x, z) = Σ_{i=0..N-1}  0.5^i  ×  (offset − |perlin_i|)^power
```

- `offset` (typical 1.0): controls ridge sharpness
- `power` (1.5–2.5): higher = more cliff-like crests

Output is **always non-negative** (returns 0 at valleys, max at ridge-lines). Use case: alpine peaks (high power), mesa formations (moderate power).

### Domain-warped fBm — `domain_warped_fbm_2d(seed, octave_offset, x, z, base_spacing_m, octaves, warp_amp_m, warp_spacing_m, warp_octaves)`

Coordinate perturbation via fBm before sampling another fBm:

```
warp_x = warp_amp × fbm_warp_x(x, z)
warp_z = warp_amp × fbm_warp_z(x + π, z + e)        ← decorrelation offsets
output = fbm_base(x + warp_x, z + warp_z)
```

Produces organic distortion — fjord-like coastlines, varied island shapes. The decorrelation offsets (31415.9 + 27182.8) ensure the two warp components don't correlate, giving 2D twisting rather than diagonal stripes.

---

## Mipmap-aware sampling

Each primitive has an `_at_scale` variant that skips octaves whose spacing falls below `min_spacing_m`. Equivalent to GPU mipmap filtering at the procedural level — band-limits output to features above the floor.

```rust
pub fn fbm_2d_at_scale(seed, octave_offset, x, z, base_spacing_m, octaves, min_spacing_m) -> f32 {
    let mut spacing = base_spacing_m;
    for i in 0..octaves {
        if spacing < min_spacing_m { break; }   // skip sub-Nyquist octaves
        total += amp * perlin_2d(seed, octave_offset + i, x, z, spacing);
        amp *= 0.5;
        spacing *= 0.5;
    }
}
```

**Use cases:**
- Contour-bench wide crops (avoid per-pixel aliasing)
- Phase 3.6.1d hydrology bake at 100 m grid resolution
- Phase 3.6.2 LOD streaming (coarser-LOD chunks skip fine octaves)
- Phase 4.7 in-engine map view (zoom-appropriate detail)
- Phase 5.5 photo mode rendering

**Suggested `min_spacing_m`:** `pixel_size_m × 2` for anti-aliased rendering (Nyquist-inspired). At `min_spacing_m = 0.0` matches non-mipmap version exactly (passes through unchanged).

`Terrain::height_at_scale(x, z, min_spacing_m)` is the public API. `Terrain::height(x, z)` delegates to `height_at_scale(_, _, 0.0)`.

---

## Octave-offset convention

Each subsystem reserves a disjoint octave-offset range to prevent gradient-hash collisions across layers (since they all share the same `master_seed`):

| Range | Subsystem |
|---|---|
| `[0..100)` | Terrain regional baseline fBm |
| `[100..200)` | Terrain fine baseline fBm |
| `[200..220)` | Weather temperature climate + synoptic |
| `[220..230)` | Weather humidity regional + synoptic |
| `[230..240)` | Weather pressure synoptic |
| `[400..500)` | Voronoi cell weights |
| `[500..520)` | Biome HighDesert (primary `[500]`, background `[515]`, warp x/z `[600]/[700]`) |
| `[520..540)` | Biome RockyAlpine |
| `[540..560)` | Biome CoastalBoreal |
| `[560..580)` | Biome MediterraneanArchipelago |

Convention: each biome reserves **20 slots** (primary + background + warp-x + warp-z + future expansion). When adding new biomes (Phase 3.6.2+), use offsets `[580..600)`, `[600..620)`, etc.

---

## Terrain composition

`Terrain::height_at_scale(x, z, min_spacing_m)` composes baseline + per-biome contributions, then enforces the real-world-possible altitude envelope:

```
raw(x, z) = amplitude_scale × (
              regional_baseline_fbm(x, z)                  ← continental trend
              + fine_baseline_fbm(x, z)                    ← small-scale texture
              + Σ biome_weight_i × biome_contribution_i(x, z)  ← per-biome character
            )

clamped(x, z) = soft_clamp(raw, weighted_envelope(x, z))   ← per-biome envelope
                  .clamp(WORLD_HARD_MIN_M, WORLD_HARD_MAX_M)  ← global hard limits

height(x, z) = beach_flatten(clamped, BEACH_WIDTH_M) + base_height
```

**Baseline layers** are biome-independent — they prevent flat-height seams across biome transitions. Tuning:
- Regional: `100 km` spacing, 4 octaves, ±200 m amplitude
- Fine: `2 km` spacing, 6 octaves, ±150 m amplitude

**Biome contribution** soft-blends 4 per-biome primitives by weight (see [next section](#biome-classification--soft-weighting)). Each biome contribution =
`mean_altitude + amplitude × primary_primitive + background_amplitude × fbm_background`

The `background_amplitude × fbm_background` term prevents flat-plateau artifacts where ridged-multifractal valleys collapse to exactly zero contribution.

**Altitude envelope (Phase 3.6.1c, real-world-possible gate)**: `soft_clamp` enforces per-biome `(hard_min, typical_min, typical_max, hard_max)` bounds — most output sits inside the typical band (identity, no transformation), excess gets exponentially compressed asymptotically toward the hard bounds, never exceeding them. C¹-continuous at the soft edges (no visible kink). Then a global `clamp` to `[WORLD_HARD_MIN_M, WORLD_HARD_MAX_M] = [-100 m, 4500 m]` as a final safety net. See [Altitude envelope](#altitude-envelope-real-world-possible-gate) below for formula + per-biome values.

**Beach flatten** compresses heights within `BEACH_WIDTH_M = 20 m` of sea level via `raw × smoothstep(|raw|, 0, BEACH_WIDTH_M)²` so shores rise gently rather than punching directly through the water surface. Pre-Phase-3.6.1d hydrology stopgap.

**`amplitude_scale`** is 1.0 for `Terrain::from_seed(seed)`, 0.0 for `Terrain::flat()` / `Terrain::flat_land()` (used in scenario-bench playback to disable terrain content). The `amplitude_scale = 0` path bypasses biome composition + envelope entirely so the test scaffolds remain bit-exact.

---

## Altitude envelope (real-world-possible gate)

Phase 3.6.1c enforces the design gate from `MAPGEN.md § Altitude envelope`: *"is this possible in the real world? if not, we may have allowed too much."* Three-layer enforcement at terrain sample time:

### 1. World hard limits

```rust
pub const WORLD_HARD_MIN_M: f32 = -100.0;  // Death Valley −86 m + safety margin
pub const WORLD_HARD_MAX_M: f32 = 4_500.0; // Above Mt Elbert 4401 m + Mt Rainier 4392 m
```

No 8000 m Himalayan biomes — within Beaver service ceiling (5500 m density-altitude), turbine conversions reach ~7 km. Terrain never exceeds these bounds regardless of seed or biome composition.

### 2. Per-biome envelope (`BiomeProfile`)

Each biome declares `(hard_min_m, typical_min_m, typical_max_m, hard_max_m)`:

| Biome | Real reference | typical_min | typical_max | hard_min | hard_max |
|---|---|---|---|---|---|
| HighDesert | Colorado Plateau, Uncompahgre 3851 m | 200 m | 2 500 m | −100 m | 3 851 m |
| RockyAlpine | Colorado Rockies, Elbert 4401 m | 800 m | 3 500 m | 0 m | 4 500 m |
| CoastalBoreal | Chugach / Wrangell, Denali rare | −100 m | 1 500 m | −100 m | 4 500 m |
| MediterraneanArchipelago | Aegean / Sardinia, Etna 2706 m | −50 m | 1 200 m | −100 m | 2 706 m |

At biome edges the envelope is `Σ weight_i × profile_i.bound` — smooth transitions match the biome weight smoothing so altitude bands don't show stripes.

### 3. `soft_clamp(value, hard_lo, soft_lo, soft_hi, hard_hi)`

Inside `[soft_lo, soft_hi]`: identity (no transformation — most terrain output passes through untouched).

Outside: exponential compression asymptotically saturating at the hard bound:

```
if value > soft_hi:
    range = hard_hi - soft_hi
    delta = value - soft_hi
    return soft_hi + range × (1 - exp(-delta / range))
```

(symmetric for `value < soft_lo`).

Properties:
- **C¹ continuous at the soft edges** — derivative = 1 at `soft_lo` / `soft_hi`, matching the identity branch. No visible kink in the heightfield.
- **Asymptotically saturates** — `lim value→±∞ soft_clamp(value) = hard_lo / hard_hi`. f32 precision saturates to exact equality at extreme overshoots (~50× the over-band range), which is acceptable since the math result is "infinitesimally close."
- **Monotonic everywhere** — stable for elevation-derived calculations (gradient, hillshade).

### Verification

`heightfield_within_world_hard_bounds` asserts every sample on a 50×50 grid (~2500 km square) lies in `[-100 m, 4500 m]`. `biome_envelope_caps_extreme_amplitudes` does the same on a different seed at 80 km spacing. `soft_clamp_*` tests verify identity-inside-band, asymptotic-outside-band, C¹ continuity at soft edges, monotonicity.

---

## Biome classification + soft weighting

Two functions in `crates/world/src/biome.rs`:

### `biome_at(seed, x, z) -> BiomeClass`

Discrete classification — used for L2 sub-biome feature-type assignment where one biome per cell is needed.

Decision tree using **only climate** (temperature + humidity at sea level — intentionally terrain-independent so 3.6.1b biome terrain doesn't cause circular dependency):

```
if temp_c < ALPINE_TEMP_C (10°C):       RockyAlpine
elif temp_c > HOT_TEMP_C (20°C)
     and hum < DRY_CEILING (0.40):       HighDesert
elif hum > COASTAL_FLOOR (0.55):         CoastalBoreal
else:                                    MediterraneanArchipelago
```

Thresholds are symmetric around the 15°C climate baseline so each biome covers ~20% of world area.

### `biome_weights_at(seed, x, z) -> BiomeWeights`

Soft membership — used by terrain blending. Smoothstep-softened thresholds across ±2°C temperature / ±0.05 humidity transitions. Always normalized to sum to 1.0:

```rust
struct BiomeWeights {
    high_desert: f32,
    rocky_alpine: f32,
    coastal_boreal: f32,
    mediterranean: f32,
}
```

Used by `Terrain::height_at_scale` to blend per-biome primitives without seams. Smooth transitions ensure terrain character morphs gradually across biome boundaries (alpine → coastal transitions show mountain peaks gradually replaced by fjord-like coast over ~20 km).

---

## Per-biome terrain profiles

Defined in `crates/world/src/terrain.rs` as `BiomeProfile` const instances. Current values (noise-shape parameters; envelope bounds in [Altitude envelope](#altitude-envelope-real-world-possible-gate)):

| Biome | Primitive | Spacing | Amplitude | Mean alt | Power | Warp amp | Background |
|---|---|---|---|---|---|---|---|
| HighDesert | Ridged | 20 km | 400 m | 800 m | 1.8 | — | 60 m @ 1 km |
| RockyAlpine | Ridged | 10 km | 900 m | 1500 m | 2.5 | — | 80 m @ 800 m |
| CoastalBoreal | DomainWarped | 10 km | 300 m | −50 m | — | 3 km | 40 m @ 600 m |
| MediterraneanArchipelago | DomainWarped | 5 km | 200 m | 20 m | — | 5 km | 30 m @ 400 m |

**Mean-altitude bias** lifts each biome to its characteristic elevation regardless of baseline noise — alpine sits at ~1500 m baseline, coastal at sea level. Biome weights interpolate these means at boundaries → smooth elevation transitions.

**Background fBm layer** prevents flat-plateau artifacts. Ridged multifractal bottoms out to exactly 0 in valleys, so without the background term you'd see perfectly flat regions at `mean_altitude`. The 30–80 m background variation gives rolling valley floors.

**Altitude envelope** is enforced AFTER primitive composition — see [Altitude envelope (real-world-possible gate)](#altitude-envelope-real-world-possible-gate).

---

## Hydrology graph (3.6.1d)

`crates/world/src/hydrology.rs` bakes a read-only spatial water classification + per-body surface-elevation graph at world-load time. Consumers query via O(1) cell lookup; the bake runs ~2 s on a modern CPU for the 5000 km world at 2.5 km resolution.

### Bake pipeline

1. **Coarse heightfield** at `BAKE_RESOLUTION_M = 2500 m` (`BAKE_GRID_N = 2000`² = 4 M cells). Sampled via `Terrain::height_at_scale(x, z, 2500.0)` — mipmap-aware to suppress sub-cell aliasing.

2. **Priority-flood + D8** (Barnes/Lehman/Mulla 2014). Seeds all sea cells (`raw_elevation ≤ 0`) as sinks, floods outward assigning every non-sea cell a `d8_downstream` pointer + pit-filled elevation. Pit-fill lifts depressions to their spillway level so flow direction is well-defined inside closed pits. Single pass O(N log N). Invariants: `filled_elevation[c] ≥ raw_elevation[c]` everywhere; walking `d8_downstream` from any cell eventually reaches a sea cell.

3. **Flow accumulation** via Kahn's topological sort. Each cell receives upstream contributions via `upstream_count`-decrement; own `precip_index` gets added; total pushes to `d8_downstream`. Elevation-sort doesn't work here — pit-filled flat regions produce ties that a sort handles wrong. Kahn's is O(N) and respects flow topology regardless of elevation.

4. **Wetness equilibrium** replaces "always fill to spillway" with real-world equilibrium: per basin, water surface area `A_w = inflow / evap_index`; binary-search the basin's hypsometric curve for the fill elevation that produces `A_w` cells underwater. Capped at spillway (overflow case). Basins where `A_w < footprint` are partially filled (closed-basin / arid). This is where desert basins stay mostly dry while alpine cirques brim.

5. **Channel classification** via second Kahn's pass computing path-integrated residual flow (inflow minus per-cell evaporation along the downstream path). Cells classified:
   - `River`: residual ≥ `RIVER_FLOW_THRESHOLD` (400)
   - `Creek`: residual ≥ `RIVER_TO_CREEK_THRESHOLD` (40)
   - `DryChannel`: flow_accumulation ≥ `RIVER_TO_CREEK_THRESHOLD` AND residual < it (accumulation existed pre-evap but path evaporation cancelled surface water — desert wadi / wash)
   - Otherwise: dry land

6. **DeltaFan detection**: River/Creek cells whose `d8_downstream` is sea or lake, plus their immediate upstream 1-step footprint, reclassify as `DeltaFan`. Turns the D8 "parallel-flow on flats" visual artifact into a gameplay feature — 3.6.3 renders these as braided sandbar / mudflat geometry.

7. **Coastline polylines** via Marching Squares at `SEA_LEVEL_M = 0.0` on the coarse grid. Reuses the MS pattern from `terrain_visual_bench`.

### Per-biome hydrology priors (`BiomeHydrology`)

Dimensionless precipitation and evaporation indices driving the wetness model:

| Biome | precip_index | evap_index | typical outcome |
|---|---|---|---|
| HighDesert | 0.15 | 1.0 | dry playas + ephemeral wadis; rare filled basin |
| RockyAlpine | 0.7 | 0.4 | tarns brim, perennial rivers, cirque lakes |
| CoastalBoreal | 1.0 | 0.3 | many full lakes, dense river network |
| MediterraneanArchipelago | 0.5 | 0.7 | mid-fill lakes, dry-summer creeks inland |

Channel evaporation per cell: `CHANNEL_EVAP_PER_CELL × evap_index = 0.25 × evap_index`. In high desert (evap 1.0), per-cell loss (0.25) exceeds per-cell precip (0.15) → flow shrinks downstream → DryChannel forms. In boreal (evap 0.3), loss (0.075) << precip (1.0) → flow grows → Rivers form.

### Query API

```rust
HydrologyGraph::bake(master_seed, &terrain) -> Self
HydrologyGraph::water_surface_at(x, z) -> Option<WaterSurface>
HydrologyGraph::lake(id) -> Option<&LakeBasin>
HydrologyGraph::river(id) -> Option<&RiverEdge>
HydrologyGraph::coastline_segments() -> &[CoastlineSegment]
```

`WaterSurface` carries `{class, elevation_m, chop_amplitude_scale, body_id}`. Sea = 0 m; lake = equilibrium fill elevation; river / creek / delta = terrain-flush (render-side offset). `chop_amplitude_scale` is the per-body dispatch parameter that 3.5b FFT ocean will use for wave-model selection (sea → full FFT; lakes → scaled Gerstner; rivers → flow shader).

### Data-layer stats (canonical seed `0xF10_1A11`)

| Class | Coverage | Notes |
|---|---|---|
| OpenOcean (sea) | 23.9 % | matches terrain-below-sea-level fraction |
| Lake | 4.6 % | filled basins (≥ 5 m depth, ≥ 25 km² footprint) |
| River | 1.9 % | perennial named-river-scale channels |
| Creek | 3.6 % | smaller perennial flow |
| DeltaFan | 0.3 % | river-mouth fan footprints |
| DryChannel | 0 % (this seed) | needs HighDesert biome coverage to activate |
| Dry land | 65.7 % | the rest |

### Known limitations (deferred)

- **D8 parallel-flow on flats**: simple D8 produces visibly parallel flow lines across large flat regions (filled-pit plateaus). 3.6.1d accepts this via `DeltaFan` tagging as a feature rather than a bug; a future pass can implement Barnes 2014 "flat direction resolution" to route flat cells toward the spillway for more dendritic visual structure.
- **DryChannel absent in canonical seed**: only High Desert (0.2 % of this seed) activates the desert-evap > desert-precip condition. Will appear more on seeds with higher desert coverage, or if Mediterranean biome constants are tuned drier.
- **Buoyancy wiring**: `world::water_height` currently returns sea-level-only; passing a HydrologyGraph reference into buoyancy so lakes contribute at their own elevations is a small follow-up (SimState owns the graph + buoyancy reads through it).
- **FFT coupling** (3.5b second pass): per-body wave-model dispatch inherits from `WaterClass` + `chop_amplitude_scale` — no hydrology rework needed.

---

## Weather field generation

All in `crates/world/src/weather/*.rs`. Phase 3.2.1–3.6.1a implementation; Perlin-fBm migration completed 2026-04-22.

### Sea-level temperature — `temperature::sea_level_temperature(seed, x, z, t)` → Kelvin

```
temp_k = 288.15 (BASELINE)
       + 8 K × cos(2π · z / WORLD_EXTENT_M)            ← latitude proxy
       + 12 K × fbm_climate(seed, x, z, 500 km, 4 oct)
       + 3 K × fbm_synoptic(seed, x, z, 357 km, 3 oct)
       + 5 K × diurnal(t)
```

**Latitude proxy**: single cos cycle per world z-axis produces synthetic equator-warm / polar-cold belt structure. Wraps cleanly (period divides extent). On a flat torus there are no true poles, but the climate gradient matches real-Earth intuition (fly north → cooler).

**Climate fBm** at 500 km scale: regional warm/cold zones independent of latitude — e.g., a continental dry interior stays cooler than coastal regions at the same latitude.

**Synoptic fBm** at 357 km: storm-scale temperature variation. 357 km is the wrap-quantized version of the original 350 km (`WORLD_EXTENT_M / 14`).

**Diurnal**: simple sinusoid keyed to `t` seconds — peak at noon (`t = 12 h`), trough at midnight.

`temperature_at(seed, x, alt_m, z, t)` adds ISA tropospheric lapse rate `−6.5 K/km` for altitude.

### Sea-level humidity — `humidity::sea_level_humidity(seed, x, z, t)` → `[0, 1]`

```
rh = 0.55 (BASELINE)
   + 0.30 × fbm_regional(seed, x, z, 200 km, 4 oct)
   + 0.10 × fbm_synoptic(seed, x, z, 357 km, 3 oct)
```

Clamped to `[0, 1]`. Regional layer drives biome-scale moisture (coastal vs interior). Synoptic adds storm-system moisture.

`humidity_at_with_terrain(seed, terrain, x, z, t)` adds the orographic modifier (windward boost / lee-side desiccation) per `weather::orographic`.

### Sea-level pressure — `pressure::sea_level_pressure(seed, x, z, t)` → hPa

```
p = 1013.25 (BASELINE_HPA)
  + 25 × fbm(seed, x_drift, z, 357 km, 4 oct)
```

Where `x_drift = x + 10 m/s × t` to simulate westerly synoptic drift. Pressure systems advect at ~10 m/s per real-mid-latitude observation.

### Wind — `wind::wind_at(seed, x, alt, z, t)` → `Vec3`

Derived from pressure gradient via approximate geostrophic balance: wind blows perpendicular to the gradient (north for southward gradient, etc.). Magnitude scales with gradient strength; altitude-stratified (low-altitude wind feels terrain friction; aloft wind matches geostrophic exactly).

### Turbulence — `turbulence::turbulence_at_with_weather(seed, terrain, x, alt, z, t)` → `f32`

Composed from:
- Terrain-induced (lee-side rotors via `terrain.height_at_scale` slope)
- Weather-cell intensity (storm cells contribute extra turbulence)
- Altitude scaling (decreases above ~2000 m AGL)

### Cell state — `cell_state::state_at_time(seed, cell_id, t)` → `CellState`

Per-Voronoi-cell weather classification. At cell birth, samples W1 fields at the cell center to determine type (ClearAir / Cumulus / Nimbus / Stratus / Squall / Frontal). Per-cell life-cycle phase advances with time (`cell_phase_with_tuning`); density + precipitation rate scale with phase via the lifecycle-activity curve (growth → peak → dissipation → dormant).

---

## Voronoi cells (variable size)

`crates/world/src/weather/voronoi.rs` — currently the only Voronoi instance; same pattern lands for terrain L2 in Phase 3.6.2.

### Cell lookup — `cell_at(master_seed, x, z) -> CellLookup`

**Power-diagram metric** with per-cell weights from a coarse Perlin field:

```
weight(seed_pos) = Perlin(weight_seed, seed_pos.x, seed_pos.z, 500 km) × (20 km)²
distance²(query, seed_i) = ||query − seed_pos_i||² − weight(seed_i)
owning_cell = argmin_i(distance²(query, seed_i))
```

Search a 5×5 grid neighborhood (expanded from 3×3 to handle weighted cells reaching further). Mod-N wrap for toroidal cells near seams.

Result: organic cell-size variation at ~2-3× typical / up to 10× extremes. Real-weather-character — compact convective cells alongside sprawling frontal systems.

**Constants:**
- `GRID_SPACING_M = 25_000` (25 km base cell spacing — divides 5000 km world cleanly into 200 cells/side)
- `JITTER_FRACTION = 0.3` (per-cell seed offset within grid)
- `WEIGHT_FIELD_SPACING_M = 500_000` (500 km — region-scale weight character)
- `WEIGHT_AMPLITUDE_M2 = 4.0e8` (~(20 km)² — produces 2–3× typical size variation)

### Drift — `cell_at_drifted(seed, x, alt, z, t)`

Convects cells past stationary observers via wind-displacement: `cell_at(seed, x − wind.x × t, z − wind.z × t)`. Altitude-dependent (different wind layers drift at different rates, matching real weather behavior).

---

## Toroidal world topology

`crates/core/src/world_topology.rs` provides foundation primitives:

- `WORLD_EXTENT_M = 5_000_000.0` — world side length (5000 km)
- `WORLD_EXTENT_M_INV` — reciprocal, exposed as fundamental spatial frequency
- `wrap_coord(x: f32) -> f32` — wraps coordinate into `[0, WORLD_EXTENT_M)`
- `wrap_cell_index(i: i64, n: i64) -> i64` — wraps grid index modulo `n`
- `quantize_angular_frequency(ω: f32)` — rounds angular frequency to nearest integer-cycles-per-world
- `quantize_wavelength(λ: f32)` — convenience for callers reasoning in wavelengths

**Wrap requirement** for clean seam continuity: every noise wavelength λ must satisfy `WORLD_EXTENT_M mod λ ≈ 0`. Terrain octaves are quantized via `quantize_angular_frequency`. Voronoi grids choose `GRID_SPACING_M` such that `WORLD_EXTENT_M / GRID_SPACING_M` is integer.

Weather synoptic wavelengths (originally 350 km) quantized to `WORLD_EXTENT_M / 14 ≈ 357.14 km` (2.04% adjustment, well within natural variation).

---

## Visual-bench artifacts

`crates/world/tests/terrain_visual_bench.rs` and `weather_visual_bench.rs` emit deterministic PNG artifacts per master seed at multiple crop scales:

| Crop name | Extent | Use case |
|---|---|---|
| `player` | 10 km | Pilot-chart scale, single-flight visible area |
| `quadrant` | 250 km | Regional view, multi-biome boundaries |
| `full` | 500 km | Broader regional, mountain-range character |
| `continental` | 2000 km | World-scale climate + biome zones |

### Artifacts emitted (per crop)

**Terrain** (`crates/world/tests/terrain_visual_bench.rs`):
- `heightfield_*.png` — color-ramped elevation (cool→warm)
- `biome_*.png` — discrete-color biome class mask
- `topo_*.png` — full topographic style (elevation color ramp + hillshade + contour lines)
- `contour_*.png` — slate-white background + contour lines (per-pixel method, gradient-aware thickness)
- `contour_ms_*.png` — same but via Marching Squares (50× faster, Bresenham segments)

**Weather** (`crates/world/tests/weather_visual_bench.rs`): pressure, temperature, humidity, wind vectors, turbulence, frontal-boundary graph, Voronoi partition, cell-type mask (initial + evolved at t=3h + phase), composite synoptic / moisture overlays. ~37 PNGs per seed.

### Marching Squares contour algorithm

Classic 1987 algorithm. Sample altitudes on a coarse 1024×1024 grid. For each cell, check which contour levels cross its 4 edges (corners straddle the level). Linear-interpolate the exact crossing position on each edge. Connect 2 (or 4 for saddles) crossing points with line segments. Rasterize segments via Bresenham's line algorithm.

**Why MS over per-pixel:**
- 16× fewer terrain samples (1024² coarse grid vs 4096² fine)
- Crisp 1-pixel-wide lines (no threshold fuzziness)
- Mathematically exact (linear-interpolated endpoints)
- 50× faster wall-clock (7 s vs 6 min at 4096 output × 4 crops)

**Bold sea-level contour**: the 0-altitude contour renders as 3-pixel-thick line via a 3×3 brush stamp along the Bresenham path — topographic-map convention emphasizing the shoreline.

**Contour intervals (size-tiered):**

| Crop | MS | Per-pixel |
|---|---|---|
| ≤ 20 km (player) | 15 m | 15 m |
| ≤ 100 km | 60 m | 60 m |
| ≤ 500 km | 150 m | 150 m |
| ≤ 1000 km | 500 m | 500 m |
| > 1000 km | 1000 m | 1000 m |

Modeled on USGS topographic-map conventions (40 ft = 12 m at 1:24000 quadrangle scale; 100 m at 1:250000 regional; 1000 m at 1:12000000 world).

---

## Performance characteristics

Measured on Linux x86_64 release build:

| Operation | Cost | Notes |
|---|---|---|
| `perlin_2d` single sample | ~80 ns | Hash + 4 corner gradients + bilinear |
| `fbm_2d` 6 octaves | ~500 ns | 6 perlin calls + sum |
| `ridged_multifractal_2d` 5 octaves | ~600 ns | + abs/pow per octave |
| `domain_warped_fbm_2d` | ~1.5 µs | 3 fbm calls (warp-x, warp-z, base) |
| `Terrain::height` | ~3 µs | regional + fine + 4 biome primitives blended |
| `Terrain::height_at_scale` (heavy mipmap) | ~600 ns | At continental zoom, ~1/5 the work |
| `biome_weights_at` | ~200 ns | 2 weather samples + smoothstep arithmetic |
| `weather::cell_at` | ~5 µs | 5×5 neighborhood × jitter hash + power-diagram math |

**Per-frame budget** at runtime: aircraft samples `terrain.height` once per tick (~3 µs), totally negligible (frame budget at 240 Hz reference is 4.17 ms).

**Per-bake budget** at session start: 200×200 mesh = 40K samples × 3 µs = 120 ms. Acceptable one-time cost.

**Per-bench-artifact budget**: 4096×4096 = 16M samples per crop. Per-pixel contour: ~6 minutes per crop. Marching Squares contour: ~7 seconds per crop (50× speedup via 1024×1024 sampling + segment rasterization).

---

## Determinism contract

Every world-gen function is **pure**: same `(master_seed, world_coords, time)` → bit-identical output on every machine. This extends [DESIGN.md § Invariants](../DESIGN.md#invariants) Invariant #8 (deterministic simulation) to world generation.

**Invariant chain:**
1. Hashing uses `splitmix64` (constant-time, deterministic) + Knuth multipliers
2. Float math uses `libm` (pure-Rust, platform-independent transcendentals) where determinism matters
3. Terrain octaves use quantized frequencies (integer cycles per world)
4. Voronoi uses mod-N indexing for wrap

**Tested via:**
- `crates/world/src/biome.rs` — `deterministic_for_same_inputs`
- `crates/world/src/terrain.rs` — `same_seed_same_height`, `height_at_scale_zero_matches_height`
- `crates/world/src/noise.rs` — `perlin_deterministic`, `*_wraps_toroidally`
- `crates/world/src/weather/*.rs` — `*_deterministic` per field
- Cross-platform smoke (Phase 3 prerequisite): pending direct Linux+Windows comparison; planned in Phase 6.1

**Pending** (see `seed_uniqueness_test_plan` memory): full seed-space coverage tests when player-facing seed-creation UI lands at Phase 3.7. Currently determinism is verified per-pair (same seed → same output), not across the seed space (1000 seeds → 1000 distinct worlds without collision).

---

## Design references

- [`MAPGEN.md`](../MAPGEN.md) — full design rationale (the *why* this implementation chose these patterns)
- [`MAPGEN.md § Layer 1 noise primitives`](../MAPGEN.md#layer-1-noise-primitives-reference) — formulas + per-biome application rules
- [`MAPGEN.md § Variable Voronoi cell size`](../MAPGEN.md#variable-voronoi-cell-size-pinned--all-voronoi-applications) — power-diagram math + future application sites
- [`MAPGEN.md § Biome edge blending`](../MAPGEN.md#biome-edge-blending--tri-biome-rare-experiences-pinned) — tri-biome intersection design (Phase 3.6.2)
- [`ROADMAP.md § Phase 3.6`](../ROADMAP.md#phase-36-world-foundations) — milestone-by-milestone plan
- [`TEST_PLAN.md § World-Gen Visual Bench`](../TEST_PLAN.md#world-gen-visual-bench) — bench infrastructure pattern
