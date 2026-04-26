# Weather — Implementation Reference

> **Role in the docs stack:** companion to [MAPGEN.md § Three-layer weather composition](../MAPGEN.md#three-layer-weather-composition-second-application-of-the-pattern). MAPGEN is the **design** doc (the *what* and *why* — three-layer composition, public API contract, real-world-possible gate). This is the **implementation** reference (the *how* — algorithms, signatures, technique decisions, performance, sim hookups). Read MAPGEN first for the architectural rationale; read this when you need to pick a raymarch step count or wire a cloud-density consumer.
>
> **Status:** W1 + W2 + W3 sampling shipped (Phase 3.2.1 / 3.2.2 / 3.2.3, ~3500 LOC). Cloud rendering + particle system + sim cloud-fly-through consumption pending — Phase 3.2x / Phase 3.6.3c.6. Updated 2026-04-25.

## Contents

- [Status snapshot](#status-snapshot)
- [Architecture reference](#architecture-reference)
- [Cloud rendering technique](#cloud-rendering-technique)
- [Worley noise primitive](#worley-noise-primitive)
- [Particle system architecture](#particle-system-architecture)
- [Fog + visibility coupling](#fog--visibility-coupling)
- [Atmospheric optics (deferred)](#atmospheric-optics-deferred)
- [Sim consumption — cloud-fly-through effects](#sim-consumption--cloud-fly-through-effects)
- [Performance budget](#performance-budget)
- [Determinism + cross-platform](#determinism--cross-platform)
- [Open questions](#open-questions)

Each technique decision below is marked as either:

- **[LOCKED]** — pinned in MAPGEN, memory, or already shipped code; not up for re-debate without amending the source.
- **[RECOMMENDED]** — proposed by this doc, open to push-back during 3.6.3c.6 / 3.2x kickoff. Resolution lands here as an amendment.

---

## Status snapshot

| Subsystem | State | Reference |
| --- | --- | --- |
| W1 — pressure / temperature / humidity / wind / turbulence / orographic / frontal-graph | **shipped** | `crates/world/src/weather/{pressure,temperature,humidity,wind,turbulence,orographic,frontal}.rs` |
| W2 — Voronoi partition + weighted power-diagram + altitude-stratified drift | **shipped** | `crates/world/src/weather/voronoi.rs` (452 LOC) |
| W2 — per-cell state machine (6 types, lifecycle phase, `WeatherTuning`) | **shipped** | `crates/world/src/weather/cell_state.rs` (887 LOC) |
| W3 — `cloud_density_at`, `precipitation_rate_at`, `visibility_at`, composed wind + turbulence | **shipped** | `crates/world/src/weather/deltas.rs` (530 LOC) |
| Sim wind + turbulence consumption | **shipped** | `crates/app/src/game_app.rs::tick_with_weather` |
| Visual-bench artifacts (15 PNGs incl. cloud-density slice, frontal graph, voronoi partition) | **shipped** | `crates/world/tests/weather_visual_bench.rs` |
| **Worley noise primitive** | missing | (this doc § [Worley noise primitive](#worley-noise-primitive)) |
| **Cloud raymarcher (CPU + GPU)** | missing | (this doc § [Cloud rendering technique](#cloud-rendering-technique)) |
| **`ChunkedWeather` streaming manager** | missing | universal pattern, ref `chunked_voronoi_layers_universal` memory |
| **Particle system (rain/snow/wingtip-mist/wind-debris)** | missing | (this doc § [Particle system architecture](#particle-system-architecture)) |
| **Sim cloud-density consumption (turbulence amplification, visibility, icing)** | missing | (this doc § [Sim consumption](#sim-consumption--cloud-fly-through-effects)) |
| **Fog + visibility weather-coupling** | missing | (this doc § [Fog + visibility coupling](#fog--visibility-coupling)) |
| Atmospheric optics (halos, sun dogs, rainbows, crepuscular rays) | deferred to Phase 5.5 | (this doc § [Atmospheric optics (deferred)](#atmospheric-optics-deferred)) |
| Icing accretion physics | deferred (post-3.2x) | flag with `TODO(post-3.2x)` markers when sim hooks land |
| Cross-platform CPU↔GPU determinism | deferred to Phase 6.1 | per `CODING_RULES.md § Determinism Discipline` + `determinism.yml` workflow |

The implementation gap from "shipped W1/W2/W3 samplers" to "weather is visible + felt in flight" is rendering + sim hookups, **not** more design — every public API in MAPGEN's contract is already implemented.

---

## Architecture reference

Three-layer composition (parallels terrain L1/L2/L3 — see MAPGEN.md):

```
W1 atmospheric base         W2 Voronoi partition      W3 cell deltas
─────────────────────       ────────────────────       ─────────────────
pressure(x,y,z,t)           cell_at(x,z) ───┐         cloud_density(x,y,z,t)
temperature(x,y,z,t)        cell_state(id,t)│ ──┐     wind_at_with_weather()
humidity(x,y,z,t)           drift offset    │   │     turbulence_at_with_weather()
wind_at(x,y,z,t)            (wind aloft)    │   │     visibility_at()
turbulence_at(x,y,z,t)                      │   │     precipitation_rate_at()
orographic_modifier(x,z)                    │   │
frontal_segments(...)                       │   │
                                            │   │
        continuous, deterministic           │   └──> consumed by sim (today: wind + turb;
        per master_seed                     │        future: cloud_density driving
                                            │        turbulence amplification, visibility,
                                            │        icing per cloud_flythrough_requirement)
                                            │
                                            └──> consumed by cloud raymarcher
                                                 (via ChunkedWeather GPU buffer)
                                                 + sim cloud-fly-through queries
```

**Public API contract** (MAPGEN.md § Consumer API surface, all signatures shipped):

```rust
world::pressure_at(x, y, z) -> f32
world::temperature_at(x, y, z, t) -> f32
world::humidity_at(x, y, z, t) -> f32
world::wind_at(x, y, z, t) -> Vec3                                // W1 baseline
world::turbulence_at(x, y, z, t) -> f32                           // W1 baseline
world::cloud_density(x, y, z, t) -> f32                           // CPU-GPU shared
world::precipitation_at(x, y, z, t) -> PrecipRate
world::weather_cell_at(x, z, y, t) -> WeatherCellId
```

**Determinism contract**: every sampler is a pure function of `(master_seed, world_coord, time)`. No hidden state, no wall-clock reads. Within-platform bit-identical (verified). Cross-platform deferred to 6.1.

**Octave-offset convention** (extends WORLD_GEN.md table for cloud noise):

| Subsystem | Octave range | Notes |
| --- | --- | --- |
| Terrain regional | `[0, 100)` | shipped |
| Terrain fine | `[100, 200)` | shipped |
| Weather temperature | `[200, 220)` | shipped |
| Weather humidity | `[220, 230)` | shipped |
| Weather pressure | `[230, 240)` | shipped |
| **Cloud noise (Worley + Perlin sub-cell detail)** | **`[240, 280)`** | **[RECOMMENDED]** new |
| Voronoi cell-weight (terrain L2) | `[400, 500)` | shipped |
| Biome | `[500, 700)` | shipped |

The `[240, 280)` slot is the recommended reservation; 40 octaves gives headroom for 2-3 Worley-3D octaves + 2-3 Perlin-3D detail octaves + future weather-driven noise (gust eddies, density gradients). Pin during 3.6.3c.6 implementation.

---

## Cloud rendering technique

### Render pass placement

**[RECOMMENDED]** — Cloud raymarcher runs **after** the scene pass with **depth read-only** access. Reasoning:

- Sky pass writes the far-plane background.
- Scene pass writes terrain + water + instances and populates the depth buffer.
- Cloud pass reads scene depth to terminate rays at terrain (cloud doesn't render *inside* a mountain). Composites over the rendered scene.
- This is the same ordering the existing `instances` pipeline uses — pattern is already in place.

Alternative considered: pre-scene cloud pass would let scene-pass clouds occlude terrain through gaps, but requires raymarching to far-plane every pixel and double-renders the shared atmosphere → simpler pre-scene placement is **rejected**.

### Density storage

**[RECOMMENDED]** — **Storage buffer of per-cell state structs**, indexed by cell ID. ChunkedWeather populates active cells; shader iterates the active set per ray-step.

```rust
#[repr(C)]
struct GpuWeatherCell {
    seed_pos: [f32; 2],         // cell center, world-XZ
    base_altitude_m: f32,       // cloud deck bottom
    top_altitude_m: f32,        // cloud deck top
    density: f32,               // peak density [0..1]
    cell_type: u32,             // ClearAir | Cumulus | Nimbus | Stratus | Squall | Frontal
    storm_phase: f32,           // [0..1] lifecycle position
    drift_offset: [f32; 2],     // accumulated wind-aloft drift at sample time
    _padding: [f32; 4],         // pad to 64 B for std140 alignment
}
```

Active set ~80-200 cells × 64 B = ~6-13 KB. Trivially under any storage-buffer cap.

Alternatives considered:
- **Uniform buffer** (16 KB cap, ~64 B/cell × 200 = 12.8 KB — fits but tight) — rejected; storage buffer indexing is cleaner and we have no UBO-only constraint
- **2D atlas texture** (1 row per cell, density profile encoded) — rejected; storage buffer with shader-side density formula is equivalent and avoids texture-allocation churn
- **3D density texture** (whole world cloud density baked) — rejected; ~5000 km × 5000 km × 10 km cube too large to bake

### Step-count strategy

**[RECOMMENDED]** — **Adaptive uniform**: 32 steps near (within 5 km of camera), 24 steps mid (5-15 km), 16 steps far (>15 km). Quality preset multiplies (Low 0.5×, Medium 1.0×, High 1.5×, Ultra 2.0×).

Per-step distance is computed from total ray length / step count, so far rays naturally have larger step distances. Camera-inside-cloud takes the near-tier path automatically.

Alternatives:
- **Pure uniform** (e.g. 32 everywhere) — wastes far-pixel budget on subpixel cloud detail
- **Density-adaptive** (early-out at saturation, dense sampling near edges) — better quality per step but adds shader complexity. **Defer to E2 polish if uniform tier fails frame budget.**

### Sub-cell detail noise

**[LOCKED]** — Perlin-Worley blend (MAPGEN § W2 cloud rendering consumption + ROADMAP § Cloud rendering research memo). Blend ratio per cell type:

| Cell type | Worley weight | Perlin weight | Visual feel |
| --- | ---: | ---: | --- |
| Cumulus | 0.7 | 0.3 | puffy, distinct cells |
| Nimbus | 0.5 | 0.5 | bulky, with detail |
| Stratus | 0.2 | 0.8 | smooth, layered |
| Squall | 0.6 | 0.4 | textured, with edges |
| Frontal | 0.4 | 0.6 | banded, with detail |
| ClearAir | n/a | n/a | density = 0, skip |

Worley provides the cellular puffiness; Perlin provides smooth fill. Octave count: 2 octaves of each per sample (4 noise calls/step). At 32 steps × ~1.3M screen pixels (1080p) × 4 calls = 165M noise calls/frame; needs the noise functions to be cheap (~5-10 ns each on GPU = 1.5 ms gross — within budget if the early-out for empty space drops most pixels).

### Lighting model

**[LOCKED]** — Banded alpha cel-shading is the art-direction constraint (PLAN.md, FEATURES.md § 953-959, `volumetric_clouds_target` memory). Quantize accumulated transmittance into N bands (`[RECOMMENDED]` N=6 for v1) before output.

**[RECOMMENDED]** — Single sun-lit / shadow-side shading via **one-bounce Beer-Lambert toward sun**:

```
density_at_sample = sample_density(p);
transmittance_to_sun = exp(-density_to_sun_through_cloud_column);
sample_color = lerp(shadow_tint, sun_lit_tint, transmittance_to_sun);
```

`shadow_tint` and `sun_lit_tint` are scene uniforms; `density_to_sun_through_cloud_column` uses 6-8 cheap "shadow march" steps from sample point toward sun direction, accumulating density along that secondary ray.

Alternatives:
- **Henyey-Greenstein phase function** — more physically correct (forward-scattering peak, side-scatter shading) but adds shader complexity + per-sample atan2-equivalent. Rejected for v1; banded alpha makes the difference invisible. Ship if future cinematic mode needs it.
- **No secondary ray** (uniform shading per cell) — tempting for performance but loses the sun-side highlight that makes cumulus tops "pop". Rejected; secondary-ray cost is small (~6 extra noise calls per primary step ÷ early-out factor).
- **Multi-scatter approximation** — way out of v1 scope; defer indefinitely.

### Composite + cel-shading

**[RECOMMENDED]** — Composite cloud over scene with `src_alpha * src + (1 - src_alpha) * dst`. Accumulated alpha is banded into N=6 levels via `floor(alpha * 6.0) / 6.0` before composite. Hard-edge thresholding at low-alpha boundary (anything < 1/6 → 0) keeps cel-shading silhouettes crisp.

---

## Worley noise primitive

### Algorithm

**[RECOMMENDED]** — **Brute-force 3×3 (2D) / 3×3×3 (3D) feature-grid neighborhood** with one feature point per grid cell jittered by deterministic hash. Same wrap discipline as `perlin_2d`:

```rust
pub fn worley_2d(seed: u64, octave: u32, x: f32, z: f32, spacing_m: f32) -> f32 {
    let x = wrap_coord(x);
    let z = wrap_coord(z);
    let cell_x = libm::floorf(x / spacing_m) as i32;
    let cell_z = libm::floorf(z / spacing_m) as i32;
    let n = (WORLD_EXTENT_M / spacing_m) as i32;
    let mut min_d2 = f32::INFINITY;
    for dx in -1..=1 {
        for dz in -1..=1 {
            let cx = wrap_cell_index(cell_x + dx, n);
            let cz = wrap_cell_index(cell_z + dz, n);
            let (jx, jz) = feature_jitter(seed, octave, cx, cz);  // deterministic hash → [0,1)²
            let fx = (cx as f32 + jx) * spacing_m;
            let fz = (cz as f32 + jz) * spacing_m;
            let d2 = (x - fx).powi(2) + (z - fz).powi(2);
            min_d2 = min_d2.min(d2);
        }
    }
    libm::sqrtf(min_d2) / spacing_m  // normalize to [0, ~1)
}
```

3D variant is the same with 27 cells instead of 9. Cost: 9 distance computes (2D) / 27 (3D) per sample. ~50-100 ns on CPU at 3D — fine for sim sampling rate; bench at GPU before committing if it shows up in frame budget.

Alternatives:
- **Spatial-hash with feature points stored in a buffer** — cheaper per sample but requires upload; same brute-force trick is what `voronoi.rs` already does at world scale, and it's fine
- **Voronoi-style with multiple feature points per cell** — adds memory + complexity; pure-feature-per-cell is sufficient for the cloud-noise role

### Distance metric + feature output

**[RECOMMENDED]** — **F1 (closest-feature distance), L2 Euclidean.** Standard cellular noise.

`F2 - F1` (second-nearest minus nearest) gives wispy striated patterns; useful as a secondary sub-octave for cirrus or squall detail. **Add `worley_2d_f2_minus_f1` variant when needed**, not v1.

Manhattan (L1) and Chebyshev (L∞) metrics produce diamond / square edges respectively — wrong feel for clouds. Rejected.

### Octave-offset slot

`[240, 280)` per the table above. v1 uses `[240, 244)` for 4 octaves of Worley-3D + Perlin-3D blend.

### CPU-GPU bit-identity

WGSL implementation is hand-rolled in `crates/render/src/cloud_noise.wgsl`, mirrors the Rust formula exactly. Test rig in `crates/render/tests/cloud_noise_bit_identity.rs` samples N reference points on both sides, hashes outputs, asserts identical. **Hard gate** — bit-identity test failure blocks the milestone.

Cross-platform GPU-side variation across drivers may force tolerance on the WGSL side; document any divergence in `CODING_RULES.md § Determinism Discipline` and route through the `continue-on-error` cross-platform CI. v1 ships with within-platform bit-identity verified; cross-platform is 6.1 work.

---

## Particle system architecture

Currently zero infrastructure. Locking the architecture here means each particle effect (rain, snow, wingtip mist, wind debris, water spray) lands as a small additive change rather than re-debating the foundation.

### Pool model

**[RECOMMENDED]** — **Preallocated arena per particle kind**. Fixed-size pool sized for worst-case density (e.g. heavy-rain regime fills ~100K rain streaks). Per frame: render iterates active slice, never allocates.

```rust
pub struct ParticlePool<P: Particle> {
    storage: Box<[P]>,      // preallocated, never resized
    active_count: usize,    // tracks high-water mark
    free_list: Vec<u32>,    // indices reclaimable
}
```

One pool per particle kind keeps cache-line layout dense. `Vec<P>` with capacity-pinning works equivalently — choose whichever fits ergonomics.

### Lifecycle

**[RECOMMENDED]** — Three operations on the render thread:

1. **Spawn**: weather sampler decides per-frame how many of which kind. Reads `precipitation_rate_at` for rain/snow density; reads aircraft state + cell `wingtip_mist` flag for wingtip mist; reads cell debris regime for wind-blown leaves/dust.
2. **Update**: per-frame integrate position with wind + gravity; decrement TTL; reclaim expired into free list.
3. **Render**: instanced billboards for streak / sprite particles; tiny-mesh-per-particle for wingtip mist; geometry shader expansion (if WGPU exposes) for streaks.

**No allocations on render hot path.** Free-list reclaim is O(1) push; spawn pulls from free-list O(1).

### Determinism (per-chunk RNG)

**[LOCKED]** — Per-chunk deterministic spawn from a weather-RNG stream so replay works (ROADMAP.md § 669). Chunk-keyed seed = `hash(master_seed, chunk_xz, weather_chunk_tick)`. Same seed → same particles → bit-identical replay.

### Particle kinds (ladder, deferred to per-kind milestones)

| Kind | Weather trigger | Authoring target | Phase |
| --- | --- | --- | --- |
| Rain streaks | `precipitation_rate > 0.5 mm/hr`, temp > 0°C | world-space streak billboards far + screen-space windshield overlay near | 3.2x |
| Snow particles | as above, temp ≤ 0°C | softer snowflake sprites; slower fall | 3.2x |
| Wingtip mist | high humidity + low altitude + water surface beneath | thin-column emitter at wingtip positions | 3.2x (signature mood beat per PLAN.md) |
| Wind debris | dry biome dust / autumn leaves / winter snow / fire ash | per-biome textured sprite; chunked spawn | 3.2x |
| Water spray | float / wheel contact velocity above threshold | already deferred from 2.5a polish | 3.2x |
| Lightning | `Squall` cell + storm_phase > 0.7 | line-segment with emissive shader, post-process bloom | 5.5 polish |

**[LOCKED]** — All kinds consume the existing W3 sampler outputs. None of them need new world-gen state.

---

## Fog + visibility coupling

### Current state

Scene fog is a uniform exponential `SceneParams.fog_density` set at construction (currently `0.000_08` /m → ~55% fog at 10 km). Not weather-coupled.

`world::weather::visibility_at(seed, terrain, x, y, z, t)` is shipped, returns effective visibility in meters reduced by cloud density and precipitation rate.

### Proposed coupling

**[RECOMMENDED]** — Per-frame, sample `visibility_at` at the camera position. Convert to fog density via `fog_density = -ln(0.5) / (visibility_m * 0.5)` (exponential half-life at half visibility). Override `SceneParams.fog_density` for this frame.

```rust
let cam = self.active_camera.position;
let vis_m = weather::visibility_at(WORLD_SEED, &terrain, cam.x, cam.y, cam.z, t);
let fog_density = (-(0.5_f32).ln()) / (vis_m * 0.5);
scene_params.fog_density = fog_density;
```

Single sample per frame. Lerp toward target with ~0.2 s time constant to avoid pop when crossing cloud boundary. Lerp lives on render side (not sim) — visibility presentation, not physics.

### Layer fog + ground mist + wingtip mist

State ladder per FEATURES.md § 938: `none | ground_mist | wingtip_mist | layer_fog | dense_fog`. Maps to fog modes:

- **ground_mist** — horizontal fog plane at altitude `[0, 50] m`. Render via dedicated translucent quad strip clipped to AABB; pure shader, no particles needed.
- **wingtip_mist** — particle effect (above table). Triggered by humidity + low-altitude + water-surface conditions.
- **layer_fog** — horizontal fog plane at altitude band `[base_altitude_cell, base_altitude_cell + 200]`; same translucent quad pattern as ground_mist but anchored to the cloud-cell base altitude.
- **dense_fog** — uniform fog override (above) clamped to ~200 m visibility, plus screen-space brightness reduction.

**[RECOMMENDED]** — Defer layer_fog + ground_mist authoring to a 3.2x sub-step after cloud raymarcher lands. The raymarcher already handles dense fog cases (it just looks like cloud) — discrete layers are a polish addition.

---

## Atmospheric optics (deferred)

Per PLAN.md § 47 + memory `volumetric_clouds_target` — atmospheric optics (sun halos, sun dogs, rainbows, crepuscular rays) become natural to add once cloud rendering is volumetric. Out of v1 scope (Phase 3.2x); land in Phase 5.5 polish.

**[LOCKED] design pre-commitment** — when these land:

- **Sun halo + sun dog** — analytic in the sky shader (dot view-dir to sun-dir + ice-crystal angle table). Already partially implemented (sky.wgsl line 81-82 has a halo term).
- **Rainbow** — analytic anti-solar arc, modulated by water-droplet / rain regime presence. Single shader pass after cloud composite.
- **Crepuscular rays** — light-shaft post-process via cloud density along sun direction. Free dividend of volumetric raymarching: shoot a primary "shadow ray" from each pixel toward sun, accumulate cloud density, modulate sun_color by `exp(-density_along_ray)`. Adds 1 secondary march (~6 steps) but only needs running on pixels where sun is visible above horizon.
- **Aurora** — rare weather accent for subarctic biome. Mesh-based ribbons with emissive shader; outside W2 cell-state model since aurora is ionospheric, not tropospheric. Phase 5.6 per ROADMAP.

The volumetric architecture supports all of these without re-architecting; placeholder hooks land naturally during 3.2x and get wired during 5.5/5.6.

---

## Sim consumption — cloud-fly-through effects

The `cloud_flythrough_requirement` memory pins three effects the sim must consume from `world::cloud_density`:

### 1. Turbulence amplification

**[RECOMMENDED]** — Density-gradient magnitude additively boosts `WeatherInputs.turbulence_intensity`. Per tick:

```rust
let density = weather::cloud_density_at(seed, &terrain, p.x, p.y, p.z, t);
let dx = weather::cloud_density_at(seed, &terrain, p.x + 50.0, p.y, p.z, t);
let dy = weather::cloud_density_at(seed, &terrain, p.x, p.y + 50.0, p.z, t);
let dz = weather::cloud_density_at(seed, &terrain, p.x, p.y, p.z + 50.0, t);
let grad = ((dx - density).powi(2) + (dy - density).powi(2) + (dz - density).powi(2)).sqrt() / 50.0;
turbulence += grad * CLOUD_GRADIENT_TURB_GAIN;  // capped; tune via flight-bench
```

Smooth cloud interior is calm; cloud edges produce buffeting (matches real flying). Cell-type modulates `CLOUD_GRADIENT_TURB_GAIN` (cumulonimbus = high gain; stratus = low gain) — already encoded in W3 cell-type table.

3 extra `cloud_density_at` calls per tick = small (~few µs) given the lookup is `voronoi::cell_at` + cached cell state.

### 2. Visibility reduction

Already shipped sampler `visibility_at` — wire into render-side fog (above § Fog + visibility coupling). Sim doesn't need separate consumption; fog reduction *is* the visibility effect from the player perspective.

For instrument-reliant missions (Phase 4.5 mission capability), the sim queries `visibility_at` at aircraft position to gate "VFR mission can complete in IFR conditions" — tier-gated by electronics upgrade per `utility_varying_upgrades` memory. Defer to Phase 4.5 hookup.

### 3. Icing accretion

**[DEFERRED]** to post-3.2x. Cold + moist cloud interior accumulates ice on leading edges, reducing lift + increasing drag + raising stall speed. Pitot heat + carb heat mitigate. Sketched in FEATURES.md § 942-967.

When it lands:
- Sim queries `temperature_at` + `cloud_density_at` per tick at aircraft position.
- Below freezing + inside cloud → accumulate ice mass on `IcingState`.
- Ice mass scales lift coefficient down + drag coefficient up + stall speed up.
- Pitot/carb heat toggles drain ice mass at fixed rate when active.
- Flight-bench scenarios for icing onset + recovery.

Block on aircraft icing model existing first; flag with `TODO(post-3.2x)` markers in cloud-density consumption sites.

---

## Performance budget

Per DESIGN.md § Performance Budget — reference tier 240 Hz, 60% GPU utilization cap, full frame ≤ 4.2 ms.

**[LOCKED]** — Cloud rendering pass: **≤ 0.6 ms / pass at reference tier**.

Allocation across cloud-pass cost centers (rough budget for v1 implementation):

| Cost center | Budget (ms) | Notes |
| --- | ---: | --- |
| Primary raymarch (32 near steps × 1080p × early-out factor ~0.3) | 0.30 | dominant |
| Secondary shadow march (6 steps × 1080p × early-out × visible-sun factor) | 0.10 | sun-lit shading |
| Cell-state buffer fetch (~80-200 cells, indexed lookup) | 0.05 | trivial |
| Composite + banded-alpha quantization | 0.05 | screen-space pass |
| Sky + scene depth read | 0.05 | shared infra |
| Headroom | 0.05 | margin |

Quality presets scale step count + secondary march:

| Preset | Primary steps | Secondary steps | Resolution scale |
| --- | ---: | ---: | --- |
| Low | 16 | 4 | 0.5× |
| Medium | 24 | 6 | 0.75× |
| High (default) | 32 | 6 | 1.0× |
| Ultra | 48 | 8 | 1.0× |

**[RECOMMENDED]** — Frame-bench gate added in 3.6.3c.6 / 3.2x: assert cloud-pass cost ≤ 0.6 ms at High preset on reference hardware. Failure = revisit step count or cell-lookup strategy before merge. Pattern matches existing `crates/world/tests/perf_budgets.rs` discipline.

CPU-side cloud-density sampling (sim consumption) cost — bench at the `world` crate level:

| Operation | Budget | Notes |
| --- | ---: | --- |
| `cloud_density_at(x, y, z, t)` | < 200 ns | one Voronoi lookup + cell-state derivation + altitude profile + Perlin-Worley sub-cell detail |
| `world::cloud_density(...)` (full public API) | < 250 ns | wraps above with consistent signature |

Add to `crates/world/tests/perf_budgets.rs` when sampler ships.

---

## Determinism + cross-platform

Per CODING_RULES.md § Determinism Discipline:

- **Within-platform**: bit-identical CPU↔CPU (verified by existing flight-bench infrastructure). When cloud sampler lands, add it to the bit-identity test battery.
- **CPU↔GPU**: bit-identical *within* a platform (Linux↔Linux). Hard gate; cloud-noise WGSL implementation must match Rust formula bit-for-bit. Test rig samples N reference points, hashes both sides, asserts identical.
- **Cross-platform GPU**: deferred to Phase 6.1 per `determinism.yml` + `continue-on-error` posture. WGSL trig functions vary across drivers; soft-float WGSL math is 6.1 work.

**[LOCKED]** — `libm` for all CPU transcendentals in cloud-density code path. No `f32::sqrt`, no `f32::sin`, no `f32::cos` — every transcendental routes through libm. Mirror the existing pattern in `voronoi.rs`, `wind.rs`, `humidity.rs`.

**[RECOMMENDED]** — Pin the Worley + Perlin-Worley blend formula in code comments + this doc. Any change to the formula requires a coordinated CPU + WGSL update + bit-identity re-baseline. Treat as a load-bearing protocol.

---

## Open questions

Resolution lands as amendments to this doc.

1. **Async vs sync `ChunkedWeather::update()`** — per-cell state bake is ~1 µs; the universal-pattern worker pool may be overkill. Sync inside `update()` saves complexity (~120 LOC). Decision deferred to C1 implementation; revisit if frame-bench shows main-thread spike.

2. **Pure-feature-per-cell vs multi-feature-per-cell Worley** — pure (one feature/cell, jittered) is simpler + sufficient for cumulus. Multi (k=2-3 features/cell) gives finer-grained cellular structure, useful for nimbus textures. Defer to E1 polish if v1 cumulus reads as too uniform.

3. **Fog density time-constant** — 0.2 s lerp is a guess. Tune during E1 polish; may want different constants for fog-density-up (entering cloud) vs fog-density-down (exiting) since entering should feel sudden but exiting should feel relieving.

4. **Layer fog rendering technique** — translucent quad strip vs second raymarch pass. Translucent quad is cheap + doesn't need full volumetric; raymarch reuses existing infrastructure. Defer to layer-fog implementation post-3.2x.

5. **Rain / snow particle geometry** — instanced billboards (one streak quad per particle) vs single fullscreen rain-pass shader (no per-particle state, uniform-driven). Particle pool gives gameplay-driven density variation; fullscreen pass is cheaper but uniform. Probably particle pool for rain (per-cell density variation matters); fullscreen pass for heavy-rain windshield overlay. Lock during 3.2x rain-particle implementation.

6. **Cross-platform Worley sqrt** — likely fine within libm-discipline, but flag if cross-platform CI surfaces divergence. Defer to 6.1 with the rest of cross-platform work.

7. **Day/night cloud shading** — sun-lit / shadow-side shading assumes daytime. Night cloud rendering (moonlit silhouettes, lightning illumination) is post-v1. Architectural prep: shadow_tint + sun_lit_tint are scene uniforms, so swapping them out via time-of-day code is additive.

---

## References

- **Design**: [MAPGEN.md § Three-layer weather composition](../MAPGEN.md#three-layer-weather-composition-second-application-of-the-pattern), [MAPGEN.md § Weather Voronoi layer](../MAPGEN.md#weather-voronoi-layer-second-partition), [MAPGEN.md § Consumer API surface](../MAPGEN.md#consumer-api-surface)
- **Memory pins**: `volumetric_clouds_target`, `cloud_flythrough_requirement`, `weather_voronoi_layer`, `weather_scale_posture`, `chunked_voronoi_layers_universal`
- **Roadmap**: [ROADMAP.md § Milestone 3.2](../ROADMAP.md#milestone-32-weather-state) (W1/W2/W3 — shipped), [ROADMAP.md § Milestone 3.2x](../ROADMAP.md#milestone-32x-weather-visuals--particle-system) (volumetric clouds + particles — pending), [ROADMAP.md § Cloud rendering research memo (pre-3.2x)](../ROADMAP.md#cloud-rendering-research-memo-pre-32x) (was scoped as `docs/CLOUDS.md`; this doc supersedes)
- **Code**: `crates/world/src/weather/{mod,pressure,temperature,humidity,wind,turbulence,orographic,frontal,voronoi,cell_state,deltas}.rs`
- **Visual-bench**: `crates/world/tests/weather_visual_bench.rs` (15 PNG artifacts, opt-in `--ignored`)
- **Performance**: [DESIGN.md § Performance Budget](../DESIGN.md#performance-budget)
- **Determinism**: [CODING_RULES.md § Determinism Discipline](../CODING_RULES.md#determinism-discipline)
