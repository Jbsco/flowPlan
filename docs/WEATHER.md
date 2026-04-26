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
- [Atmospheric character per cell type](#atmospheric-character-per-cell-type)
- [Fog + visibility coupling](#fog--visibility-coupling)
- [Atmospheric optics (architectural hooks in 3.2x; visual polish in 5.5)](#atmospheric-optics-architectural-hooks-in-32x-visual-polish-in-55)
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
| **Atmospheric character per cell type** (mood-driven fog/sun/sky modulation) | missing | (this doc § [Atmospheric character per cell type](#atmospheric-character-per-cell-type)) |
| **Fog + visibility weather-coupling** | missing | (this doc § [Fog + visibility coupling](#fog--visibility-coupling)) |
| Atmospheric optics architectural prep (sun halos, sun dogs, rainbows, crepuscular rays, lightning) | architectural hooks in Phase 3.2x; visual polish in Phase 5.5 | (this doc § [Atmospheric optics](#atmospheric-optics-architectural-hooks-in-32x-visual-polish-in-55)) |
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

### Tessellation discipline (PINNED 2026-04-26)

Per WEATHER.md § "No-weather is a type of weather" + MAPGEN.md § W3:
**every world XZ point belongs to exactly one W2 cell** (the closest seed in
the power-diagram metric). That cell's TYPE provides the parameters; the
noise field provides cloud structure within. Across cell boundaries,
parameters interpolate (clear → cumulus, cumulus → stratus, etc.) for
organic transitions rather than step discontinuities.

**Wrong (a tempting shortcut):** circular footprints centered on each cell's
seed_pos, density falls to 0 outside the footprint radius. Produces
"scattered identical puffs with gaps between cells." This was the C2 v1
shortcut; corrected at C2-polish.

**Right:** nearest-cell lookup per ray sample. Every sample owns one cell;
cloud density is `cell.density × altitude_profile × Perlin-Worley(world_pos)`.
Same-type adjacent cells share the same noise field and parameters → the
boundary is invisible (continuous body of weather). Different-type adjacent
cells have different parameters but the same noise → smooth visual
transition where parameters lerp across the edge zone (boundary-blending,
implementation-pending).

**Implementation cost**: per-ray-step nearest-cell scan over the active set
(~25-50 cells × distance check). Adds ~30% to per-pixel cost vs the
shortcut. Future optimization options: 2D texture lookup of pre-baked
cell-index grid (~1 read per step instead of ~50), sorted active set +
early-exit (~50% reduction).

**Boundary blending** (implementation-pending): use `distance_to_edge` from
the W2 power-diagram lookup; within ~1 km of edge, sample both nearest +
second-nearest cell, smoothstep-blend the parameters. Required for visually
correct cumulus → stratus / cumulus → clear-air transitions. Cells of the
same type already blend naturally because the noise field is continuous.

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

**Status (2026-04-26)**: B3 ships an **altitude-gradient stand-in** (top of cloud reads as sun-lit, bottom as shadowed; gradient strength scales with `sun_dir.y` so low-sun gives flatter shading). This is the cheap interim until the proper shadow march lands — the cloud has directional character without the per-step secondary march cost. Real fix below queues for E1 polish where the shading model is tuned alongside the Perlin-Worley blend (B3c).

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

**[LOCKED]** — **Multi-feature-per-cell with F1+F2 distance-weighted blend**, brute-force 3×3 (2D) / 3×3×3 (3D) feature-grid neighborhood. k=2-3 jittered feature points per cell. Pattern parallels biome edge-blending — soft transitions between feature dominance instead of hard Voronoi boundaries.

```rust
pub fn worley_2d_blended(
    seed: u64, octave: u32, x: f32, z: f32, spacing_m: f32, features_per_cell: u32,
) -> WorleyResult {
    let x = wrap_coord(x);
    let z = wrap_coord(z);
    let cell_x = libm::floorf(x / spacing_m) as i32;
    let cell_z = libm::floorf(z / spacing_m) as i32;
    let n = (WORLD_EXTENT_M / spacing_m) as i32;
    let mut d2_sorted = [f32::INFINITY; 4];   // top-2 nearest; size for k_max=3
    for dx in -1..=1 {
        for dz in -1..=1 {
            let cx = wrap_cell_index(cell_x + dx, n);
            let cz = wrap_cell_index(cell_z + dz, n);
            for k in 0..features_per_cell {
                let (jx, jz) = feature_jitter(seed, octave, cx, cz, k);
                let fx = (cx as f32 + jx) * spacing_m;
                let fz = (cz as f32 + jz) * spacing_m;
                let d2 = (x - fx).powi(2) + (z - fz).powi(2);
                insert_sorted(&mut d2_sorted, d2);
            }
        }
    }
    let f1 = libm::sqrtf(d2_sorted[0]) / spacing_m;
    let f2 = libm::sqrtf(d2_sorted[1]) / spacing_m;
    // Smoothstep blend at feature boundary — when f1 ≪ f2 (one clear winner)
    // returns f1; when f1 ≈ f2 (boundary region) returns weighted blend.
    // Visual effect: soft cellular structure, no hard edges.
    let blend_t = ((f2 - f1) / f2.max(1e-6)).clamp(0.0, 1.0);
    let smooth_blend = blend_t * blend_t * (3.0 - 2.0 * blend_t);
    let f_blended = f1 + (f2 - f1) * (1.0 - smooth_blend) * 0.5;
    WorleyResult { f1, f2, f_blended }
}
```

3D variant: 27 cells × k features. At k=2: 54 distance computes per sample; at k=3: 81. Bench shows GPU cost ~15-25 ns at k=2 in modern hardware — fits inside the cloud-pass budget at 32 steps × early-out factor.

Returning all three of `f1`, `f2`, `f_blended` lets the cloud shader pick per cell type:
- **Cumulus**: use `f_blended` for puffy soft-cellular look.
- **Nimbus**: use `f_blended` with low octave + `f2` high octave for bulky-with-detail.
- **Stratus**: use `f1` only (sharp cells); Perlin dominates anyway, Worley contribution is small.
- **Squall / Frontal**: use `f2 - f1` for wispy striated detail at secondary octave.

### Distance metric + feature output

**[LOCKED]** — **F1, F2, F1+F2 smooth-blend, L2 Euclidean.** All three returned per sample so the shader picks per use case.

`F2 - F1` (second-nearest minus nearest) gives wispy striated patterns; available as a derived output without a separate noise call.

Manhattan (L1) and Chebyshev (L∞) metrics produce diamond / square edges respectively — wrong feel for clouds. Rejected.

### Multi-feature parallel to biome blending

**[LOCKED]** — Per the user-pinned direction (2026-04-25), the multi-feature approach with F1+F2 smooth-blend mirrors the biome-edge-bleed-over pattern. Just as biome boundaries soften with smoothstep weights between adjacent regions to produce gradient transitions, Worley feature boundaries soften with smoothstep weights between F1 and F2 to produce gradient cellular structure. **This is not a future enhancement — it's the v1 default.** Pure-feature-per-cell with hard F1 boundaries is rejected as visually too uniform.

### Per-cell feature count variation

**[RECOMMENDED]** — Cell density (k features per cell) varies regionally via a coarse Perlin-driven density field, in the same spirit as the variable Voronoi cell-size design:

```rust
fn features_per_cell_at(seed: u64, cell_x: i32, cell_z: i32) -> u32 {
    // Coarse Perlin sampling at ~10× cell scale yields regional density variation.
    let density_field = perlin_2d(density_seed, cell_x_world, cell_z_world, spacing * 10);
    let normalized = (density_field + 0.707) / 1.414;  // → [0, 1]
    if normalized > 0.66 { 3 }
    else if normalized > 0.33 { 2 }
    else { 1 }
}
```

Some regions have dense cumulus (k=3, lots of small puffs), others have spread-out patterns (k=1, fewer larger clouds). Matches real atmospheric behavior — convective regions cluster while subtropical highs have isolated cumulus.

### Octave-offset slot

`[240, 280)` per the table above. v1 uses `[240, 244)` for 4 octaves of Worley-3D + Perlin-3D blend.

### CPU-GPU agreement

WGSL implementation is hand-rolled in `crates/render/src/cloud_noise.wgsl`, mirrors the Rust formula exactly (same constants, same hash chain, same noise formula). Test rig in `crates/render/tests/cloud_noise_bit_identity.rs` (run via `./flowplan test-cloud-bit-identity`) samples N reference points on both sides, asserts CPU and GPU agree within tolerance.

**Tolerance reality (revised 2026-04-26)**: the original spec called for **bit-identity** between CPU and GPU. After implementation, this proved unachievable in practice on modern GPU drivers without compilation-level controls we don't have access to:

- HASH outputs match exactly (verified empirically — when feature-point coordinates agree CPU/GPU, downstream math agrees to ~1 ULP).
- ARITHMETIC chains (dot products, multi-add accumulation, sqrt + divide) diverge at 1-100 ULPs because GPU drivers fuse multiply-add operations into FMA instructions (single rounding instead of two). naga / wgpu does not currently expose a way to disable this.
- When FMA-induced 1-ULP differences land at the d² comparison that picks the closest Worley feature, cascade through `f1`, `f2`, `f_blended` can grow to 100s of ULPs (~0.1-0.3% relative error at typical magnitudes).

**Practical gate**: per-component absolute difference ≤ **5e-3**. Catches algorithm bugs (wrong constants, broken hash, wrong loop bounds — these produce 0.1-1.0 magnitude divergence) while tolerating FMA-level driver math (observed max 1.1e-3 abs in current implementation). Strict bit-identity becomes a Phase 6.1 cross-platform-determinism-hardening goal alongside soft-float WGSL math.

Cross-platform GPU-side variation across drivers is a superset of within-platform FMA divergence and remains deferred to Phase 6.1 per `CODING_RULES.md § Determinism Discipline` and `determinism.yml` workflow's `continue-on-error` posture.

---

## Particle system architecture

Currently zero infrastructure. Locking the architecture here means each particle effect (rain, snow, wingtip mist, wind debris, water spray) lands as a small additive change rather than re-debating the foundation.

### Pool model

**[LOCKED]** — **Preallocated arena per particle kind** with **instanced billboards** for distance-visible rendering. Fixed-size pool sized for worst-case density (e.g. heavy-rain regime fills ~100K rain streaks). Per frame: render iterates active slice, never allocates.

```rust
pub struct ParticlePool<P: Particle> {
    storage: Box<[P]>,      // preallocated, never resized
    active_count: usize,    // tracks high-water mark
    free_list: Vec<u32>,    // indices reclaimable
}
```

One pool per particle kind keeps cache-line layout dense. `Vec<P>` with capacity-pinning works equivalently — choose whichever fits ergonomics.

### Instanced billboards over fullscreen pass — locked per user direction (2026-04-25)

Particle pool architecture wins over fullscreen-pass uniform-driven rain because **distance visibility is a primary requirement** — atmosphere must read at distance (per § Atmospheric character per cell type). A storm with heavy rain visible from 30 km away comes from per-cell density variation in the pool; a fullscreen rain-pass renders the same density everywhere uniformly and loses the spatial structure.

Tradeoff: pool is more expensive per frame, but the visual payoff is decisive. The fullscreen-pass approach reserved as a windshield-near-overlay (separate post-process for raindrops on the canopy) — a complementary technique, not a replacement.

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

## Atmospheric character per cell type

**[LOCKED]** — User-pinned direction (2026-04-25): the visual experience IS the gameplay. In-engine renders are the producer; vivid sunsets and misty morning sunrise from the lake are primary eyecatches. Architecture must support these atmospheres natively, not as post-processing afterthoughts. Distance visibility matters — a viewer should see a storm front from 30 km away and feel a different mood than seeing a sunshower from the same distance.

### Mood derivation

The presentation-side `AtmosphericMood` is **derived per camera frame** from local cell state + sun visibility + precipitation visibility + time of day. Not a per-cell property — it's what the camera *sees* given the local weather context. Mood determines fog tint, sun-color modulation, particle visual character, sky desaturation, cloud shading bias.

```rust
#[derive(Copy, Clone, Debug)]
pub enum AtmosphericMood {
    Bright,    // sunny, fair-weather cumulus, midday clear
    Serene,    // light snow, mist, dawn, sunshower without sun
    Vivid,     // sunshower with sun visible, golden hour, sunset
    Cool,      // overcast stratus, blue hour, twilight
    Dark,      // approaching storm, gust front, dense rain
    Dramatic,  // inside or near nimbus core, squall passage
}

pub fn atmospheric_mood(
    camera: &CameraState,
    weather: &WeatherSnapshot,   // local cell state + nearby cells
    sun: &SunState,
    time: TimeOfDay,
) -> AtmosphericMood {
    let local = weather.cell_at_camera;
    let sun_visible = sun.is_above_horizon
        && sun.cloud_density_along_ray < SUNSHOWER_THRESHOLD;
    let precip_visible = weather.any_precipitating_cell_in_frustum;

    match (local.cell_type, sun_visible, precip_visible, time) {
        // Sunshower — locked-in eyecatch case (per user pin)
        (_, true, true, _) => AtmosphericMood::Vivid,
        // Storm core / squall passage
        (Nimbus | Squall, _, _, _) if local.density > 0.6 => Dramatic,
        // Approaching storm / dense overcast
        (Nimbus | Squall, _, _, _) => Dark,
        // Light precip + diffuse light = serene
        (_, _, true, _) if local.precip_rate < 1.5 => Serene,
        // Stratus overcast
        (Stratus, _, _, _) => Cool,
        // Golden hour / sunset
        (_, _, _, GoldenHour | Sunset | BlueHour) => Vivid,
        // Mist + dawn/dusk
        (ClearAir, _, _, Dawn | Dusk) if local.fog_state == FogState::GroundMist => Serene,
        // Default fair-weather
        (Cumulus | ClearAir, _, _, _) => Bright,
        _ => Bright,
    }
}
```

### Mood parameters

Each mood drives a uniform-buffer set the cloud shader, particle renderer, fog shader, and sky shader all consume. Locked architectural slot:

```rust
#[repr(C)]
pub struct AtmosphericMoodParams {
    /// Multiplicative shift to scene fog color. RGB tint applied to baseline.
    pub fog_tint: [f32; 3],
    /// Multiplicative scale on fog density. <1 = clearer, >1 = thicker.
    pub fog_density_scale: f32,
    /// Multiplicative shift to sun_color. Storm = darken+desaturate;
    /// Vivid = warm enrich; Serene = cool soften.
    pub sun_color_modulation: [f32; 3],
    /// Multiplicative scale on ambient sky-color. Cool/Dark reduce; Vivid boost.
    pub ambient_modulation: f32,
    /// Cloud sun_lit_tint override (pre-modulation).
    pub cloud_lit_tint: [f32; 3],
    /// Cloud shadow_tint override.
    pub cloud_shadow_tint: [f32; 3],
    /// Sky desaturation amount [0..1]. Storm = high; Vivid = low.
    pub sky_desaturation: f32,
}
```

**[RECOMMENDED]** — initial parameter table (tune during E1 polish):

| Mood | fog_tint | fog_density_scale | sun_color_mod | ambient_mod | cloud_lit_tint | cloud_shadow_tint | sky_desat |
| --- | --- | ---: | --- | ---: | --- | --- | ---: |
| Bright | [1.0, 1.0, 1.0] | 1.0 | [1.0, 1.0, 1.0] | 1.0 | [1.0, 1.0, 1.0] | [0.5, 0.6, 0.75] | 0.0 |
| Serene | [0.85, 0.92, 1.0] | 1.2 | [0.95, 0.95, 1.0] | 0.95 | [0.95, 0.97, 1.0] | [0.65, 0.7, 0.8] | 0.1 |
| Vivid | [1.1, 0.95, 0.85] | 0.95 | [1.15, 1.0, 0.85] | 1.05 | [1.1, 1.0, 0.85] | [0.55, 0.5, 0.65] | 0.0 |
| Cool | [0.85, 0.9, 1.0] | 1.3 | [0.85, 0.9, 1.0] | 0.85 | [0.9, 0.92, 1.0] | [0.5, 0.55, 0.7] | 0.3 |
| Dark | [0.6, 0.65, 0.75] | 1.6 | [0.55, 0.55, 0.6] | 0.6 | [0.7, 0.7, 0.75] | [0.25, 0.27, 0.32] | 0.5 |
| Dramatic | [0.4, 0.42, 0.5] | 2.0 | [0.4, 0.4, 0.45] | 0.4 | [0.55, 0.55, 0.6] | [0.15, 0.17, 0.22] | 0.7 |

Lerps between moods on transition (~2 s constant) prevent pop. Mood blending is just per-channel lerp.

### Sunshower detection

Special architectural case for the highest-eyecatch atmosphere. Detected when:

1. `precipitation_rate_at(camera_pos)` returns nonzero AND
2. `cloud_density along sun_ray from camera` is below `SUNSHOWER_THRESHOLD` (e.g. 0.15) AND
3. Sun is above horizon

When all three hold, mood is `Vivid` — sun-color and fog tints push toward warm and golden, sky desaturation drops to 0, cloud lighting shifts to highlight rim-lit edges. Pairs with the architectural-prep rainbow hook in [Atmospheric optics](#atmospheric-optics-deferred) section. Provides the "sun + rain + rainbow" beat without scripting.

### Distance visibility

Per user-pinned direction — the atmosphere should read at distance. Storm vs sunshower must be distinguishable from 30 km. Mood-driven sky desaturation + cloud-cell type reading ensures this:

- **Dark / Dramatic** moods desaturate distant sky toward gray + push fog tint to slate. Storm-front silhouettes read as dark walls.
- **Vivid** mood pushes warm tints into the distant fog. Sunshowers read as bright bands beneath dark cloud.
- **Serene** mood softens sky to pastel + lifts ambient. Light snow scenes feel high-key from afar.

This is the core architectural payoff of weather-driven atmospheric character — no post-processing or palette overrides needed; the atmosphere *is* the W3 cell-state expressed visually.

### Mood + photo mode (Phase 5.5 forward-prep)

`AtmosphericMoodParams` is a uniform buffer the renderer already updates per frame. Photo mode (Phase 5.5) consumes it as-is — no special photo-mode rendering path needed for atmosphere. Player-driven captures of vivid sunsets and misty morning sunrises consume the same mood pipeline that real-time gameplay uses. **In-engine renders ARE the producer per the user-pinned direction; the architecture supports this natively.**

---

## Fog + visibility coupling

### Current state

Scene fog is a uniform exponential `SceneParams.fog_density` set at construction (currently `0.000_08` /m → ~55% fog at 10 km). Not weather-coupled.

`world::weather::visibility_at(seed, terrain, x, y, z, t)` is shipped, returns effective visibility in meters reduced by cloud density and precipitation rate.

### Proposed coupling

**[RECOMMENDED]** — Per-frame, sample `visibility_at` at the camera position. Convert to fog density via `fog_density = -ln(0.5) / (visibility_m * 0.5)` (exponential half-life at half visibility). Override `SceneParams.fog_density` for this frame, modulated by `AtmosphericMoodParams.fog_density_scale` from § Atmospheric character per cell type.

```rust
let cam = self.active_camera.position;
let vis_m = weather::visibility_at(WORLD_SEED, &terrain, cam.x, cam.y, cam.z, t);
let raw_density = (-(0.5_f32).ln()) / (vis_m * 0.5);
let mood_scaled = raw_density * mood_params.fog_density_scale;
scene_params.fog_density = lerp_per_frame(prev_density, mood_scaled, FOG_LERP_RATE_HZ);
scene_params.fog_color = baseline_fog_color * mood_params.fog_tint;
```

Single sample per frame. Lerp toward target prevents pop when crossing cloud boundary. **[RECOMMENDED]** — `FOG_LERP_RATE_HZ` is **framerate-relative**: expressed as fraction-of-display-rate per frame (e.g. `0.05` = 5% per frame → ~95% convergence in 60 frames at any display rate). Tune during E1 polish. If frame budget allows, near-instant response (single-frame snap) is acceptable for sudden cloud-boundary entry — atmosphere feels more responsive. Per user-pinned direction (2026-04-25): fog response should approach framerate where budget allows. Keep configurable as `WeatherTuning::fog_response_factor` for player tuning at seed-creation UI.

Lerp lives on render side (not sim) — visibility presentation, not physics.

### Layer fog + ground mist + wingtip mist

State ladder per FEATURES.md § 938: `none | ground_mist | wingtip_mist | layer_fog | dense_fog`. Maps to fog modes:

- **ground_mist** — horizontal fog plane at altitude `[0, 50] m`. Render via dedicated translucent quad strip clipped to AABB; pure shader, no particles needed.
- **wingtip_mist** — particle effect (above table). Triggered by humidity + low-altitude + water-surface conditions.
- **layer_fog** — horizontal fog plane at altitude band `[base_altitude_cell, base_altitude_cell + 200]`; same translucent quad pattern as ground_mist but anchored to the cloud-cell base altitude.
- **dense_fog** — uniform fog override (above) clamped to ~200 m visibility, plus screen-space brightness reduction.

**[RECOMMENDED]** — Defer layer_fog + ground_mist authoring to a 3.2x sub-step after cloud raymarcher lands. The raymarcher already handles dense fog cases (it just looks like cloud) — discrete layers are a polish addition.

---

## Atmospheric optics (architectural hooks in 3.2x; visual polish in 5.5)

**[LOCKED]** — Elevated from "deferred" to "scaffolding now, polish later" per user-pinned direction (2026-04-25): visual experience is gameplay; in-engine renders ARE the producer; vivid sunsets + misty morning sunrise are primary eyecatches. Architectural hooks land during 3.2x (so the work later in 5.5 is visual polish rather than re-architecting); rendering of optics ships in 5.5.

Per PLAN.md § 47 + memory `volumetric_clouds_target` — atmospheric optics become natural to add once cloud rendering is volumetric.

### Architectural prep landing in 3.2x

- **Sun halo + sun dog hook** — already partially scaffolded in `sky.wgsl` (lines 81-82 have a halo term). 3.2x leaves these intact + extends with a sun-dog ice-crystal angle modulator gated by `cloud_density along sun_ray > 0.1` (cirrus presence). No new shader, just an additive term.
- **Rainbow hook** — `AtmosphericMood::Vivid` + sunshower detection (above) is the trigger. 3.2x reserves the rendering slot via a dedicated rainbow-arc shader stub that returns transparent black; 5.5 fills in analytic anti-solar arc + water-droplet attenuation. Shader stub is ~50 LOC, costs ~0.01 ms.
- **Crepuscular rays hook** — already a free dividend of volumetric raymarching: the same shadow-march toward sun used for cloud lighting (§ Cloud rendering technique → Lighting model) gives transmittance along sun ray. 3.2x reuses this transmittance to modulate sun_color when the cloud-pass is active. No new pass. Visual polish in 5.5 tunes the contrast.
- **Aurora hook** — placeholder per cell-type extension. Subarctic biome cells classify as `Aurora` (new variant added in 5.6) at appropriate temperature + season. Mesh-based ribbon shader, separate from cloud raymarcher. Ionospheric, not tropospheric — outside the cloud-density field, sits in its own renderer. Phase 5.6 per ROADMAP. **Architectural prep**: leave room in `AtmosphericMoodParams` for an "aurora-active" flag without breaking the uniform-buffer schema.

### Visual polish in 5.5

When 5.5 polish lands:

- **Sun halo + sun dog** — analytic, full-strength, ice-crystal angles tuned per cloud cell type
- **Rainbow** — visual primary at `Vivid` + sunshower mood; double-rainbow when precip rate is heavy + sun-ray transmittance is high
- **Crepuscular rays** — full contrast tuning, banded for cel-shading consistency
- **Lightning flashes** — `Squall` cell + `storm_phase > 0.7`; line-segment with emissive shader + post-process bloom
- **Alpenglow** — sun at horizon + clear sky + high-altitude terrain; sky shader extension
- **Aurora** — Phase 5.6 (separate mesh ribbons, not cloud system)

The volumetric architecture supports all of these without re-architecting. **The work in 5.5 is purely visual tuning; the architectural plumbing already exists from 3.2x.**

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

**Status (2026-04-26)**:

- **Structural cost-bound test** in `crates/render/tests/volumetric_wgsl_parses.rs::volumetric_wgsl_cost_constants_within_budget` — asserts `RAYMARCH_STEPS_MAX ≤ 64`, `RAYMARCH_TARGET_STEP_M ≥ 200`, `CLOUD_FEATURES_PER_CELL ≤ 2`, and `ray_vertical_cylinder` early-out is present. Catches accidental constant bumps. Runs as part of `cargo test --workspace`.
- **GPU wall-clock perf gate** in `crates/render/tests/cloud_pass_perf.rs::cloud_pass_within_perf_budget` — hardware-gated (`hardware-tests` feature), uses `Features::TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_ENCODERS`. Renders 30 frames at **1920×1080 (1080p)** with cloud + minimal scene, computes median GPU frame time. Resolution locked at 1080p (most desktop dev machines) — measurements directly comparable to in-game frame rate at typical play resolution. Budget **3.0 ms** with **144 Hz target** in mind (144 Hz = 6.94 ms per frame; cloud at 3 ms leaves 4 ms for other passes). Reference hardware lands ~1 ms (989 fps cloud + minimal scene), giving ~3× headroom. Tunable per playtest signal — when in-game shows a regression that the gate missed, tighten the budget; when the gate fails on legitimate work, measure on reference hardware first. Run via `./flowplan test-cloud-perf` (release build, ~30 sec) or `cargo test -p flowplan-render --features hardware-tests --test cloud_pass_perf -- --nocapture`.

**Coverage limits**: the GPU gate measures **frame time** (cloud + sky + scene + outline + widgets) at the test's minimal-scene resolution, not cloud pass alone. Per-pass isolation is a future refinement (would need wgpu render-pass `timestamp_writes`). Frame-level + structural-bound + in-engine playtest cover most regression classes today.

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

## Continuous qualification margin (PINNED 2026-04-26 — design pending implementation)

**The discrete-classification problem**: today `weather::cell_state::select_cell_type` does a **binary** eligibility test:

```rust
let elig_cumulus = if rh >= CLOUD_MIN_HUMIDITY && t_k >= CONVECTIVE_MIN_TEMP_K {
    1.0
} else {
    0.0
};
```

A cell either qualifies for Cumulus or doesn't. When it does, it inherits the type's default density (0.35) and renders Cumulus. When it doesn't, it's ClearAir and renders nothing. Visually this means:

- All Cumulus cells render at the same coverage / density.
- A cell with `rh = 0.41` (barely above 0.40 threshold) renders the same as a cell with `rh = 0.85` (deep into convective conditions).
- ClearAir cells with `rh = 0.39` render zero, even though they're virtually identical to the `0.41` neighbor that renders full Cumulus.

The result is **identical-looking cumulus everywhere it appears, with sharp on/off boundaries** at the threshold. Adjacent cells with very similar inputs may look identical or wildly different depending on which side of the threshold they fall on.

**The continuous-margin proposal**: each cell carries a `coverage_strength: f32 ∈ [0, 1]` derived from how strongly its W1 inputs satisfy the cell-type's preconditions. Visual coverage in the shader is modulated by this strength:

- `coverage_strength = 1.0` (well above threshold): solid coverage, dense puffs everywhere within the cell's altitude band.
- `coverage_strength = 0.5` (moderate qualifier): broken coverage — cumulus puffs at noise peaks, clear gaps elsewhere. Like real "fair-weather cumulus" days where puffs scatter rather than fill.
- `coverage_strength = 0.1` (marginal qualifier): wispy traces only at the strongest noise peaks. Approaches "almost cloudless."
- `coverage_strength = 0.0` (no qualification): empty. ClearAir without margin.
- **`coverage_strength` for ClearAir cells**: NOT zero. Even ClearAir gets a nonzero strength when humidity *approaches* cloud-formation threshold — produces the rare-wisp / haze effect you'd get on a "mostly-clear day with a few thin clouds." Continuous transition from clear sky → scattered cumulus over the threshold zone, no on/off step.

The mechanism: smoothstep-based qualification scoring instead of binary eligibility.

```rust
// Before (binary):
let elig_cumulus = if rh >= 0.40 && t_k >= 280.0 { 1.0 } else { 0.0 };

// After (continuous):
let rh_score = smoothstep(0.35, 0.55, rh);          // 0 below 0.35, 1 above 0.55, smooth in between
let temp_score = smoothstep(275.0, 285.0, t_k);
let coverage_strength = rh_score * temp_score;       // 0..1, both factors must contribute
```

The cell still classifies discretely (most-qualified type wins for type-specific behavior — altitude band, color tint, life-cycle duration). But `coverage_strength` is continuous and used to modulate visual density:

```wgsl
// Shader-side (sample_cloud_density):
let raw_factor = worley_w * worley_struct + perlin_w * perlin_norm;
let coverage_threshold = 1.0 - cell.coverage_strength;
let cloud_factor = smoothstep(coverage_threshold, coverage_threshold + 0.1, raw_factor);
return cell.density * alt_factor * cloud_factor;
```

So:
- `coverage_strength = 1.0`: threshold = 0, all noise above 0 produces cloud → solid.
- `coverage_strength = 0.5`: threshold = 0.5, only the upper half of noise values produces cloud → broken.
- `coverage_strength = 0.0`: threshold = 1.0, no noise produces cloud → empty.

**Cross-cell continuity**: adjacent cells with similar inputs have similar `coverage_strength` → similar visual density. The Worley/Perlin noise field is continuous across cell boundaries (already verified by tessellation discipline) → cloud structure flows naturally. Cells differing in coverage_strength produce a **gradient of cloud cover** across the world, like real synoptic-scale weather where humidity and temperature vary smoothly.

**Visual impact**:
- Resolves "all cumulus cells look identical" — strength varies, coverage varies.
- Adjacent cells produce continuous gradients (high → moderate → marginal → clear).
- ClearAir-adjacent-to-cumulus regions get the wispy haze we'd expect at the threshold.
- Storm-adjacent regions get progressive density buildup as you fly into the storm.

**Cost**:
- CPU: ~5-10 extra ALU ops per cell-state compute (small smoothstep chain). Already amortized over 5 sim-second refresh.
- GPU: 1 extra smoothstep per ray sample. Negligible vs the ~50-cell scan cost.
- Memory: 1 extra f32 per `GpuWeatherCell` (32 → 36 bytes; pack into existing padding slot, no struct-size change needed).

**Implementation plan** (~80-100 LOC across world + render):

1. **`crates/world/src/weather/cell_state.rs`** — add `coverage_strength: f32` to `CellState` struct. Compute via per-type smoothstep chain in `select_cell_type` / `parameterize`. Continuous score [0, 1] for each type's eligibility; use the highest-scoring type as classification, retain the score as coverage_strength.
2. **`crates/world/src/weather/cell_state.rs`** — for ClearAir cells, compute "approach margin" coverage_strength when rh / temp / pressure approach a cloud-bearing type's threshold but don't meet it. Capped at e.g. 0.15 so clear cells don't render full clouds.
3. **`crates/render/src/renderer.rs::GpuWeatherCell`** — replace `padding: u32` with `coverage_strength: f32`. struct stays 32 bytes; std430 alignment unchanged.
4. **`crates/app/src/chunked_weather.rs`** — pack `state.coverage_strength` into the GPU layout via `WeatherCellSnapshot` → game_app.
5. **`crates/render/src/volumetric.wgsl::sample_cloud_density`** — apply coverage-strength modulation to cloud_factor via smoothstep.
6. **Tests** — add unit tests for `coverage_strength` formula, regression test for "ClearAir cells near threshold get nonzero coverage."

**Risks / unknowns**:
- Tuning the smoothstep edge-zones (where on the input range does cloud start fading in?). Per-type tuning required. First-pass plausible bounds: humidity smoothstep over ±0.05 around threshold; temperature ±5K; etc. Iterate via in-game observation.
- ClearAir-margin semantics: at what rh do we start showing "approach wisps"? Current CLOUD_MIN_HUMIDITY = 0.40; maybe wisps start at 0.30 → ramp to 0.40 → ramp to 0.55 (full coverage). Three-band smoothstep.
- Determinism: same inputs → same coverage_strength. Already deterministic per master_seed via the existing W1 fields → maintained.

**Does this break any existing tests?**

- `clear_air_dominates`: still passes — ClearAir is still the plurality classification; we just add a small visible-coverage component to a subset of ClearAir cells.
- `nimbus_deeper_than_cumulus`: still passes — type-specific structural parameters (altitude band) unchanged.
- `cell_state_deterministic`: still passes — coverage_strength is a deterministic function of inputs.
- `cloud_bearing_cells_have_positive_altitudes`: passes — non-clear cells still have altitude bands.

**Encoded in**: `weather_continuous_coverage_margin` memory pin (PINNED 2026-04-26).

**Queued in priority ladder**: was item #4 (per-cell-type Worley spacing); now both items merge — coverage-strength refactor is the larger architectural piece, per-cell-type Worley spacing is a tuning sub-item that lands inside the same pass.

## Implementation priority ladder (post-C2-polish, 2026-04-26)

Ordering of remaining work, roughest first → most polished. Effort
estimates assume single-developer focused work; visual impact is the
"how much does this change what the player sees" axis.

| # | Item | Effort | Visual impact | Notes |
|---|---|---|---|---|
| 1 | Periodic ChunkedWeather refresh (cells re-sample state every N sim seconds) | small | high — clouds visibly evolve | **shipped 2026-04-26 C-polish** at 5 sim-second cadence |
| 2 | Per-cell-type Perlin-Worley blend weights | small | medium — varied texture per type | **shipped 2026-04-26 C-polish** via `perlin_worley_weights_for_type` |
| 3 | Tessellation correction (replace circular footprints with nearest-cell) | medium | high — continuous weather, no gaps | **shipped 2026-04-26 C-polish** |
| 4 | **Continuous qualification margin** (per-cell `coverage_strength` derived from W1 inputs; threshold smoothstep instead of binary; ClearAir gets nonzero strength near cloud-formation margin) | medium | very high — fixes "identical cumulus everywhere"; produces organic coverage gradients; cells with marginal qualification show breaks in cover; design above | **NEW**, design pinned, implementation pending |
| 4b | Per-cell-type Worley spacing (Stratus smooth, Squall textured edges, Cumulus puffy) — lands inside the #4 pass | small | medium — visual differentiation | pending |
| 5 | Beer-Lambert shadow march (proper sun lighting toward the sun) | medium | high — dramatic light/shadow on cumulus | E1 polish per WEATHER.md |
| 6 | Boundary blending (smoothstep across cell edges using `distance_to_edge`) | medium | high — fixes "hard cell boundaries" visible at type transitions | implementation-pending; design locked |
| 7 | AtmosphericMood per cell type (fog tint / sun_color / sky desaturation per cell) | medium | high — mood reads at distance (sunset over fair-weather vs squall) | 3.2x scope per WEATHER.md |
| 8 | Fog + visibility coupling (sim's `visibility_at` → render fog density per frame) | small | medium — flying into cloud feels right | 3.2x scope |
| 9 | Particle system architecture (preallocated pool, instanced billboards) | medium | foundation | 3.2x scope |
| 10 | Rain / snow particles (consume `precipitation_rate_at`) | medium | high — wet weather is gameplay | 3.2x scope |
| 11 | Wingtip-mist / wake spray / wind debris | medium-large | atmospheric polish | 3.2x scope |
| 12 | Sim cloud-fly-through effects (turbulence amplification from density gradient, visibility reduction inside cloud) | small | game-feel impact | 3.2x scope; CPU samplers exist already |
| 13 | Adjacency-biased cell evolution (sprawling fronts; cells bias toward neighbor type over sim-time) | medium | weather feels like weather, not random scatter | 3.2.2c scope per WEATHER.md |
| 14 | Cloud shadows on terrain (terrain shader samples cloud density along sun direction at terrain pixel) | small-medium | high — broken-cloud-day light becomes possible | dependent on tessellation being correct ✓ |
| 15 | Icing accretion physics (cold + moist cloud interior accumulates ice on lifting surfaces) | medium | aircraft handling impact | post-3.2x; needs aircraft icing model |
| 16 | Atmospheric optics (sun halo, sun dog, rainbow, crepuscular rays, lightning, alpenglow) | medium-large | dramatic visuals — sunsets, storms feel cinematic | 5.5 polish; architectural prep in 3.2x |
| 17 | Multiple weather layers at different altitudes (low stratus + mid cumulus + high cirrus visible together) | large | very high — realistic atmospheric stacking | late milestone; design memo first |

## Open questions

Resolution lands as amendments to this doc.

1. **~~Async vs sync `ChunkedWeather::update()`~~** — **[RESOLVED 2026-04-25, async]**. Per user-pinned direction: async is the established pattern, and historical sync→async reworks proved costly when later features (LOD, cell-state evolution, neighbor adjacency-bias) demanded it. ChunkedWeather follows ChunkedTerrain's worker-pool template (1-2 workers given cheap per-cell bake; staleness-check + universal-pattern parts intact).

2. **~~Pure-feature-per-cell vs multi-feature-per-cell Worley~~** — **[RESOLVED 2026-04-25, multi-feature with biome-style blending]**. Multi-feature with F1+F2 smooth-blend mirrors biome-edge-bleed-over pattern; pure-feature-per-cell rejected as too uniform. See § Worley noise primitive — Multi-feature parallel to biome blending.

3. **Fog density response rate** — **[REFINED 2026-04-25 to framerate-relative]**. Time constant expressed as fraction-of-frame (e.g. `0.05` = 5% per frame); approaches single-frame snap if budget allows. Tune during E1 polish; may want asymmetric rates for entering vs exiting cloud (entering = sudden, exiting = relief). Configurable in `WeatherTuning::fog_response_factor` for player-facing tuning.

4. **Layer fog rendering technique** — translucent quad strip vs second raymarch pass. **[OPEN, evaluate against cel-shading]** — per user-pinned direction (2026-04-25): cel-shading is the primary aesthetic constraint; pick whichever fits the cel-shaded aircraft + terrain better. Translucent quad strip is cheap and easy to band-quantize for cel-shading; second raymarch reuses existing infrastructure but may need explicit banding. Prototype both during layer-fog implementation, lock at that point.

5. **~~Rain / snow particle geometry~~** — **[RESOLVED 2026-04-25, particle pool with instanced billboards]**. Distance visibility is decisive per user-pinned direction (storm visible from 30 km away requires per-cell density variation only the pool provides). Fullscreen pass reserved as complementary windshield-near-overlay. See § Particle system architecture — Instanced billboards.

6. **Cross-platform Worley sqrt** — **[REAFFIRMED 2026-04-25, defer to 6.1 with portable design]**. Per user direction: cross-platform deferral is fine as long as we're not painted into a corner. Mitigation: stay within libm-discipline on CPU; hand-rolled WGSL avoids built-in `sqrt()` ambiguity by computing distance² and comparing in squared space where possible (avoid sqrt entirely until normalization is needed). Cross-platform GPU divergence handled by 6.1 hardening per `CODING_RULES.md`.

7. **~~Day/night cloud shading~~** — **[ELEVATED 2026-04-25 to v1 architectural priority]**. Per user-pinned direction: visual experience IS gameplay; in-engine renders ARE the producer; vivid sunsets + misty morning sunrise are primary eyecatches. Time-of-day cloud shading is not a post-v1 stretch — it's part of the v1 architectural plumbing, with visual polish landing in Phase 5.5. See § Atmospheric character per cell type and § Atmospheric optics. Architectural-prep checklist:
   - `AtmosphericMoodParams` includes `cloud_lit_tint` and `cloud_shadow_tint` for time-of-day modulation
   - Sun position drives mood derivation (golden hour, blue hour, sunset, dawn)
   - Cloud shader consumes mood params; no new shader needed for time-of-day
   - Photo mode (Phase 5.5) consumes the same pipeline — in-engine renders are the producer

8. **`AtmosphericMood` enum extensibility** — **[NEW]**. v1 ships 6 moods (Bright, Serene, Vivid, Cool, Dark, Dramatic). Future additions (Aurora for subarctic, Twilight for deep dusk, Whiteout for blizzard) extend the enum. Mood transitions are 2-channel lerps, so adding new moods doesn't break consumers — they default to Bright if unknown. Reserve enum slots when extending so save-format compatibility holds.

---

## References

- **Design**: [MAPGEN.md § Three-layer weather composition](../MAPGEN.md#three-layer-weather-composition-second-application-of-the-pattern), [MAPGEN.md § Weather Voronoi layer](../MAPGEN.md#weather-voronoi-layer-second-partition), [MAPGEN.md § Consumer API surface](../MAPGEN.md#consumer-api-surface)
- **Memory pins**: `volumetric_clouds_target`, `cloud_flythrough_requirement`, `weather_voronoi_layer`, `weather_scale_posture`, `chunked_voronoi_layers_universal`
- **Roadmap**: [ROADMAP.md § Milestone 3.2](../ROADMAP.md#milestone-32-weather-state) (W1/W2/W3 — shipped), [ROADMAP.md § Milestone 3.2x](../ROADMAP.md#milestone-32x-weather-visuals--particle-system) (volumetric clouds + particles — pending), [ROADMAP.md § Cloud rendering research memo (pre-3.2x)](../ROADMAP.md#cloud-rendering-research-memo-pre-32x) (was scoped as `docs/CLOUDS.md`; this doc supersedes)
- **Code**: `crates/world/src/weather/{mod,pressure,temperature,humidity,wind,turbulence,orographic,frontal,voronoi,cell_state,deltas}.rs`
- **Visual-bench**: `crates/world/tests/weather_visual_bench.rs` (15 PNG artifacts, opt-in `--ignored`)
- **Performance**: [DESIGN.md § Performance Budget](../DESIGN.md#performance-budget)
- **Determinism**: [CODING_RULES.md § Determinism Discipline](../CODING_RULES.md#determinism-discipline)
