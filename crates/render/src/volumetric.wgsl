// Volumetric cloud raymarcher (Phase 3.6.3c.6 / 3.2x).
//
// Per `docs/WEATHER.md` § Cloud rendering technique:
//   - Full-screen triangle pass running AFTER scene + outline, BEFORE
//     widgets. Composites alpha-blended over the rendered scene.
//   - B2 (skeleton): empty raymarch, transparent output.
//   - B3 (this revision): single hardcoded test cell sampled per
//     ray-step via altitude-banded Worley density. Visible clouds at
//     world origin XZ around altitude 1500-2500 m.
//   - B3b (follow-up): scene-depth read for ray termination at terrain.
//   - B3c (follow-up): Perlin-Worley blend (currently Worley-only).
//   - B4 (follow-up): CPU-GPU bit-identity gate vs Rust
//     `cloud_density_at`.
//   - C1+C2 (follow-up): replace hardcoded test cell with W2 storage
//     buffer of streamed active cells.
//
// # Shader composition
//
// This file is **not** a complete WGSL module on its own. It depends
// on the noise primitives (Worley + PCG hash + helpers) declared in
// `cloud_noise.wgsl`. The renderer composes them at shader-creation
// time via `format!("{}\n{}", CLOUD_NOISE_WGSL_SOURCE,
// VOLUMETRIC_WGSL_SOURCE)`. The naga parse test exercises the same
// composition so syntax errors in either file fail at test time.
//
// Bind groups:
//   @group(0) = camera (inverse_view_proj for ray reconstruction)
//   @group(1) = scene  (sun direction, ambient, fog, time)

struct CameraUniform {
    view_proj:         mat4x4<f32>,
    view_pos:          vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct SceneUniform {
    sun_dir:   vec4<f32>,
    sun_color: vec4<f32>,
    ambient:   vec4<f32>,
    fog:       vec4<f32>,
    // time.x = seconds since app start
    // time.y = chop_intensity (water shading)
    // time.zw = wind_dir_horiz (Post-CM-10 Phase A.1 cloud tilt)
    time:      vec4<f32>,
};
@group(1) @binding(0) var<uniform> scene: SceneUniform;

// Scene-pass depth texture (B3b). Used to terminate rays at terrain
// so clouds don't render through mountains. Sampled via textureLoad
// at integer pixel coords (no sampler needed).
@group(2) @binding(0) var scene_depth: texture_depth_2d;

// W2 weather cell array (C2). Replaces the hardcoded test cell from
// B3 — multi-cell streamed weather populated by ChunkedWeather.
// Layout matches `crates/render/src/renderer.rs::GpuWeatherCell`
// (32 bytes, 8 × f32 / u32). Unused slots use sentinel
// `seed_pos = (1e9, 1e9)` so the nearest-cell lookup naturally
// excludes them (huge distance never wins min). Real ClearAir cells
// sit with valid seed_pos + density=0 — they DO participate in
// nearest-cell lookup so cumulus cells stay within their Voronoi
// region. Per `docs/WEATHER.md § Tessellation discipline`.
struct GpuWeatherCell {
    seed_pos: vec2<f32>,        // world XZ of the cell's seed position
    base_altitude_m: f32,       // cloud base altitude
    top_altitude_m: f32,        // cloud top altitude
    density: f32,               // peak density [0, 1]; 0 = unused / clear
    storm_phase: f32,           // life-cycle phase [0, 1)
    cell_type: u32,             // CELL_* discriminant
    coverage_strength: f32,     // continuous qualification margin [0, 1]
                                // 1.0 = solid, 0.5 = broken/sparse,
                                // 0.0 = empty. ClearAir near threshold
                                // gets 0..0.15 → wispy haze.
                                // PINNED 2026-04-26 (`docs/WEATHER.md
                                // § Continuous qualification margin`).
    vertical_amp_factor: f32,   // Post-CM-10 Phase B: scales CM-5
                                // altitude perturbation amplitudes
                                // per cell. Strong nimbus → ~5×;
                                // marginal cumulus → ~1×; stratus → ~0.3×.
    profile_kind: u32,          // Post-CM-10 Phase B: PROFILE_* enum
                                // (BELL/TOWER/ANVIL/DECK/LAYERED).
                                // Selects altitude_profile shape.
}
@group(3) @binding(0) var<storage, read> weather_cells: array<GpuWeatherCell>;

// ──────────────────────────────────────────────────────────────────
// Cloud-density model
// ──────────────────────────────────────────────────────────────────

// Cell types — match Rust `weather::cell_state::CellType` discriminant.
const CELL_CLEAR_AIR: u32 = 0u;
const CELL_CUMULUS: u32   = 1u;
const CELL_NIMBUS: u32    = 2u;
const CELL_STRATUS: u32   = 3u;
const CELL_SQUALL: u32    = 4u;
const CELL_FRONTAL: u32   = 5u;

// Profile kinds (Post-CM-10 Phase B). Selects altitude_profile shape.
// Match `weather::cell_state::ProfileKind` discriminant on Rust side.
const PROFILE_BELL: u32    = 0u;  // parabolic peak (fair-weather)
const PROFILE_TOWER: u32   = 1u;  // peak at 70%, narrow base
const PROFILE_ANVIL: u32   = 2u;  // T-shape, broad top above 80%
const PROFILE_DECK: u32    = 3u;  // uniform middle 70%, smooth fades
const PROFILE_LAYERED: u32 = 4u;  // double-peak at 30% and 70%

// Global cloud altitude band (m). Wide enough to cover all cell types:
// cumulus (1200-4000), nimbus (900-12000), stratus (300-1400), squall
// (500-5000), frontal (1000-6000). Per-cell base/top further constrain
// where each cell contributes density. The shader uses these globals
// for the ray-band early-out so a single ray traversal covers all
// possible cloud altitudes; per-sample altitude_profile + cell type
// then decides whether the sample is inside that cell's actual band.
const GLOBAL_CLOUD_BASE_M: f32 = 200.0;
const GLOBAL_CLOUD_TOP_M: f32 = 12500.0;

// Worley noise spacing for cumulus sub-cell detail (m).
// 2000 m at 3 km footprint gives ~1-2 sub-puffs per cumulus — reads as
// "cumulus with single internal puff" rather than cellular patchwork.
const CLOUD_WORLEY_SPACING_M: f32 = 2000.0;
const CLOUD_WORLEY_OCTAVE: u32 = 240u;
// `features_per_cell = 1` halves the per-Worley-call distance-compute
// cost vs k=2. Single-feature pure F1 distance is sufficient for the
// visual character we want once the density-modulation curve is
// calibrated; F1+F2 blend reserved for Ultra preset.
const CLOUD_FEATURES_PER_CELL: u32 = 1u;

// Perlin sub-cell detail (B3c). Spacing tuned so the Perlin
// contribution is visibly distinct from the Worley structure: at
// 600 m the cloud spans ~5 Perlin "wavelengths" → fine cauliflower
// texture inside puffs. Larger spacings (1500+) duplicate Worley's
// scale and make the contribution invisible. Smaller (<300 m)
// produce salt-and-pepper noise. Octave 241 (Worley = 240) per
// WEATHER.md octave-offset convention `[240, 280)`.
const CLOUD_PERLIN_SPACING_M: f32 = 600.0;
const CLOUD_PERLIN_OCTAVE: u32 = 241u;

// Perlin-Worley blend weights per cell type (WEATHER.md § Sub-cell
// detail noise). For Cumulus (current test cell): 0.7 worley + 0.3
// perlin. C2 will store per-cell weights when the streaming buffer
// includes cell type — for B3c the test cell is hardcoded Cumulus.
const CUMULUS_WORLEY_WEIGHT: f32 = 0.7;
const CUMULUS_PERLIN_WEIGHT: f32 = 0.3;

// Master seed for the cloud-noise field. Must match `WORLD_SEED` in
// `crates/app/src/game_app.rs` (`0x1337_CAFE_B0A7_5EAF`). Hardcoded
// for now; future polish will plumb through a uniform buffer when
// seeds become per-save configurable. Split as (high32, low32) for
// the WGSL u32-only PCG hash chain.
const CLOUD_SEED_HI: u32 = 0x1337CAFEu;
const CLOUD_SEED_LO: u32 = 0xB0A75EAFu;

// Altitude profile within a cell's [base, top] band. Returns scalar
// in [0, 1] that scales peak density to local density at altitude `y`.
// Mirrors Rust `weather::deltas::altitude_profile`.
// Altitude profile dispatcher. Post-CM-10 Phase B (2026-04-26): now
// branches on `profile_kind` rather than `cell_type` so the same cell
// type can render different shapes (e.g. cumulus + high-cov + strong
// updraft → TOWER profile; cumulus + low-cov → BELL).
//
// Returns alt_factor in [0, 1] for altitude `y_alt_m` within the
// cell's [base_m, top_m] band. The CPU-side classifier picks the
// profile_kind from cell_type + coverage_strength.
fn altitude_profile(profile_kind: u32, y_alt_m: f32, base_m: f32, top_m: f32) -> f32 {
    if (y_alt_m < base_m || y_alt_m > top_m) {
        return 0.0;
    }
    let depth = top_m - base_m;
    if (depth < 1.0) {
        return 0.0;
    }
    let t = (y_alt_m - base_m) / depth;
    if (profile_kind == PROFILE_DECK) {
        // Stratus deck — uniform middle 70% with smoothstep fades over
        // the bottom and top 15%. Visually a flat-topped trapezoid;
        // matches the "thin stratus layer" mental model.
        let bottom_fade = smoothstep(0.0, 0.15, t);
        let top_fade = smoothstep(1.0, 0.85, t);
        return bottom_fade * top_fade;
    } else if (profile_kind == PROFILE_TOWER) {
        // Towering cumulus — peak shifted to 70% altitude. Narrow base
        // (sharp smoothstep at 30%), broad upper portion. Models
        // strong-updraft cumulus that punches up through the
        // boundary layer.
        let bottom_fade = smoothstep(0.0, 0.6, t);
        // Top fade starts at 90% — cloud peaks high in the band.
        let top_fade = smoothstep(1.0, 0.9, t);
        return bottom_fade * top_fade;
    } else if (profile_kind == PROFILE_ANVIL) {
        // Anvil cumulonimbus — narrow base/column up to 80% altitude,
        // broad spread above (the iconic "anvil top" formed when
        // convection hits the tropopause and spreads laterally).
        // Modeled as a smooth column gating union'd with a top
        // plateau so the upper region has wide alt_factor support.
        let column = smoothstep(0.0, 0.3, t) * smoothstep(0.85, 0.6, t);
        let top_plateau = smoothstep(0.7, 0.85, t) * smoothstep(1.05, 0.95, t);
        return max(column, top_plateau);
    } else if (profile_kind == PROFILE_LAYERED) {
        // Frontal double-peak — two altitude bands within the cell,
        // each producing density. Models stratified frontal cloud
        // (e.g. nimbostratus + altostratus layers).
        let lower = smoothstep(0.05, 0.3, t) * smoothstep(0.55, 0.3, t);
        let upper = smoothstep(0.55, 0.75, t) * smoothstep(0.95, 0.75, t);
        return max(lower, upper);
    } else {
        // PROFILE_BELL (default) — symmetric smoothstep × smoothstep
        // bell curve. Same peak character as parabolic but with
        // C1-continuous fades at edges. (CM-10 iter-3 fix.)
        let bottom_fade = smoothstep(0.0, 0.5, t);
        let top_fade = smoothstep(1.0, 0.5, t);
        return bottom_fade * top_fade;
    }
}

// Three-cell nearest-cell lookup result for V-vertex blending
// (Post-Phase-B 2026-04-27). Original CM-6 used find_nearest_TWO
// which produced visible artifacts at Voronoi vertices (where 3
// cells meet equidistantly) and "instant cloud snap" when a new
// closer cell loaded and displaced the previously-dominant cell
// in the idx1 slot. Returning the top-3 by distance + softmax
// weights gives smooth transitions: when a new cell C becomes
// closest, the previously-dominant cell A doesn't drop out of the
// blend entirely — it still contributes via the 2nd or 3rd slot.
// Invariant: `dsq1 <= dsq2 <= dsq3`.
struct NearestThree {
    idx1: u32,
    idx2: u32,
    idx3: u32,
    dsq1: f32,
    dsq2: f32,
    dsq3: f32,
}

// Find the two cells whose `seed_pos` are nearest to `world_xz` —
// implements W2 Voronoi tessellation with **boundary-blend support**
// (CM-6, 2026-04-26). The caller blends the two cells' contributions
// based on the relative distances so cell boundaries are smooth rather
// than hard cutoffs. CM-1 ensured ClearAir cells participate in the
// scan; CM-6 returns the two best, not just the one best.
//
// **Critical**: still includes ClearAir cells in the search. A
// clear-air cell legitimately "owns" its Voronoi region; per-cell
// gating in `sample_cell_density_inner` handles cov_strength=0 by
// returning zero contribution. Skipping clear-air would cause a
// single cumulus cell to "fill the world" out to the next cumulus
// cell. The 2026-04-26 in-game playtest "clouds vanish when leaving
// a cell" was the natural visible result of strict tessellation;
// CM-6 boundary blending fixes the visible-seam consequence by
// blending across the Voronoi midline.
//
// Empty / unfilled buffer slots use sentinel `seed_pos = (1e9, 1e9)`
// so the distance check naturally excludes them — no special check
// required. Real ClearAir cells have valid seed_pos and DO
// participate.
//
// Cost: same ~50-64 distance checks per call as the single-cell
// lookup; one extra comparison per iteration (tracks two best); ZERO
// sqrts (Option 2 — caller does one sqrt for d1 only; d2-d1 is
// derived algebraically from squared distances).
//
// Per WEATHER.md § "No-weather is a type of weather": every XZ point
// owns one cell (the *nearest*) but blends with the second-nearest in
// the boundary band — adjacent same-type cells fuse into one body of
// weather, different-type cells transition smoothly.
fn find_nearest_three_active_cells(world_xz: vec2<f32>) -> NearestThree {
    var i1: u32 = 0u;
    var d1: f32 = 1.0e30;
    var i2: u32 = 0u;
    var d2: f32 = 1.0e30;
    var i3: u32 = 0u;
    var d3: f32 = 1.0e30;
    let cell_count = arrayLength(&weather_cells);
    for (var i = 0u; i < cell_count; i = i + 1u) {
        let cell = weather_cells[i];
        let d = world_xz - cell.seed_pos;
        let dsq = d.x * d.x + d.y * d.y;
        if (dsq < d1) {
            i3 = i2; d3 = d2;
            i2 = i1; d2 = d1;
            i1 = i; d1 = dsq;
        } else if (dsq < d2) {
            i3 = i2; d3 = d2;
            i2 = i; d2 = dsq;
        } else if (dsq < d3) {
            i3 = i; d3 = dsq;
        }
    }
    return NearestThree(i1, i2, i3, d1, d2, d3);
}

// Width (m) of the smoothstep boundary-blend band centered on the
// Voronoi midline between two adjacent cells. Inside this band the
// per-sample density is a weighted mix of both cells; outside the
// band it's pure nearest-cell.
//
// Sized for ~25 km mean cell spacing (W2 partition).
//
// **Tuning history**:
// - 5 km initial: blend zone half-width.
// - 2026-04-26 bisect: 5 km → 10 km caused +0.5 ms perf hit. Reverted.
// - 2026-04-27 retry: bumped 10 km again, reverted again per user
//   request — the issue was diagnosed as cell-state distribution
//   (clear regions truly clear), not blend zone width. Atmospheric
//   haze is the fix being pursued.
const BOUNDARY_BLEND_DISTANCE_M: f32 = 5000.0;

// Note (2026-04-26): A `BOUNDARY_BLEND_W2_EARLY_EXIT = 0.05` constant
// previously gated cell 2's pre-filter when w2 was small (deep inside
// cell 1). It was removed because the gate produced a *visible
// discontinuity* — at the contour `w2 = 0.05`, cell 2's contribution
// switched on/off abruptly. In world space, that contour is a
// hyperbola whose asymptotes form a V-wedge between two cells (the
// "pie_cut" artifact). The cel-banded alpha amplified the small step
// into a hard line. The math `density += w2 * cell_2_contribution`
// self-attenuates correctly without the gate, at the cost of one
// extra altitude_profile + per-type-amp lookup per sample (~0.03 ms).

// Per-pixel ray-start jitter hash. Returns a deterministic value in
// `[0, 1)` derived from integer pixel coords. Replaces the fixed
// `0.5` step-midpoint offset in the raymarch loop so adjacent pixels
// sample the cloud at scattered `t` offsets — band-edge artifacts from
// the discrete altitude_profile transitions get dithered across pixels
// and read as soft texture rather than visible horizontal rings.
//
// PCG-style integer hash: cheap (~10 ALU), no precision concerns at
// modest screen sizes, no state. For temporal stability with a moving
// camera the hash could mix in a frame counter for TAA-style temporal
// integration; not needed at present (static shots show no banding).
fn pixel_hash_unit(p: vec2<i32>) -> f32 {
    var h: u32 = u32(p.x) * 1597334677u + u32(p.y) * 3812015801u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967296.0);
}

// Per-cell-type altitude-perturbation amplitudes (m). Returns
// `vec2(top_amp_m, base_amp_m)`. Cumulus / nimbus get bumpy convective
// tops; stratus stays nearly flat (its physical signature is a uniform
// layer); squall + frontal get moderate variation. Modulated by
// `coverage_strength` at sample time so marginal cells stay flat
// regardless of type. PINNED amplitudes per
// `docs/WEATHER.md § Cloud altitude variation` and the
// `weather_continuous_coverage_margin` memory pin
// (continuous-margin sub-phase 5, inside project Phase 3.6.3c.6 / 3.2x).
fn altitude_perturbation_amplitude(cell_type: u32) -> vec2<f32> {
    // Tuned 2026-04-26 (Phase B follow-up): doubled cumulus baseline
    // 500/200 → 800/300 because in-game observation showed clouds
    // reading as "very flat layers" even after Phase B's
    // `vertical_amp_factor` scaling. The factor multiplies these
    // baselines, so a larger baseline gives more visible verticality
    // at typical cov values (cov=0.5 → factor 1.25 → top shift
    // ±1000m vs prior ±625m).
    switch cell_type {
        case 1u: { return vec2<f32>(800.0, 300.0); }   // Cumulus — puffy
        case 2u: { return vec2<f32>(1500.0, 400.0); }  // Nimbus — deep convection
        case 3u: { return vec2<f32>(50.0, 30.0); }     // Stratus — flat layer
        case 4u: { return vec2<f32>(900.0, 350.0); }   // Squall — textured
        case 5u: { return vec2<f32>(500.0, 200.0); }   // Frontal — moderate
        default: { return vec2<f32>(80.0, 60.0); }     // ClearAir / haze — subtle
    }
}

// Spacing for the long-wavelength altitude-perturbation Perlin field.
// ~3 km wavelength → bumps a few cells wide, suitable for "convective
// top" mental model. Independent from CLOUD_PERLIN_SPACING_M (which
// drives sub-cell density-noise structure at ~800 m).
const ALT_PERTURB_SPACING_M: f32 = 3000.0;

// CM-8 vertical-distribution: anisotropic y-stretching for density
// noise (Worley + density Perlin). Multiplies world_pos.y by this
// factor before passing to the noise primitives, compressing features
// vertically — within a 1500 m cumulus band, the effective noise
// wavelength becomes ~267 m instead of 800 m, producing 5-6 vertical
// waves per cell instead of 1-2. **Combined with CM-4's linear-remap
// gating**, this gives the design intent of CM-8 directly: marginal
// cells (low coverage_strength) gate the lower noise values → discrete
// vertical lobes with sky-gaps; well-qualified cells let through the
// full noise field → continuous mass with vertical detail.
//
// Breaks up the regular Worley grid pattern visible from above as
// "rows" and from below as "globules" (in-game observation 2026-04-26
// from `banding_from_above.png` + `up_from_under.png`).
//
// **Applied to density-noise calls only**, NOT the CM-5 altitude
// perturbation call (which is sampled at fixed y=0 by design — its
// 2D-surface character is intentional for the "convective top"
// concept). PINNED 2026-04-26.
const VERTICAL_NOISE_STRETCH: f32 = 3.0;

// CM-10 domain warping: pre-perturbs the (x, z) coords for the density
// noise lookup with a low-frequency Perlin field. Breaks the
// axis-aligned regularity of the Worley feature grid that CM-8's
// vertical anisotropy alone doesn't fix — visible as east-west
// "bands" aligned with the X/Z world axes (in-game observation
// 2026-04-26: bands followed sunrise/sunset axis, which happened to
// coincide with the Worley grid's X axis).
//
// Cost: 1 extra Perlin call per sample (the X-axis warp reuses the
// existing CM-5 `perturb` sample at fixed y=0; the Z-axis warp adds
// a new Perlin sample at an offset spatial origin so warp_x and
// warp_z are decorrelated).
//
// Amplitude tuned to ~30% of the smallest Worley spacing
// (Squall = 500m → ~150m warp). At this magnitude, features get
// shifted enough to break grid alignment visibly. Bumped 60 → 150
// on 2026-04-26 after `pattern_from_above.png` showed residual
// diagonal patterning at the lower 60m amplitude — Worley grid was
// still partly visible.
const NOISE_WARP_AMPLITUDE_M: f32 = 150.0;
const NOISE_WARP_SPACING_M: f32 = 4000.0;

// **Post-CM-10 Phase B vertical-carve amplitude** (2026-04-26).
// Margin-coupled amplitude that modulates `alt_factor` by the
// existing `perlin_norm` — high-cov cells barely carve, marginal
// cells carve sky-gaps within the altitude band (3D voids).
// Reuses `perlin_norm` (with its 3D structure from CM-8's y-stretch)
// as the carve signal — saves a dedicated Perlin call (~0.1 ms).
// Lazily aligns carve with density features (carves where Perlin is
// low) — has the side effect of amplifying the noise contrast.
//
// Tuned 0.6 → 0.4 on 2026-04-26 (Phase B follow-up). At 0.6 the
// contrast amplification turned the y-stretched noise into visible
// horizontal-stripe layers (`layers.png`). 0.4 still produces 3D
// voids in marginal cells but with softer contrast.
const VERTICAL_CARVE_AMP: f32 = 0.4;

// **Post-CM-10 Phase A.1: wind-shear cumulus tilt** (2026-04-26).
// Lateral offset (m) applied to the density-noise lookup per unit of
// normalized altitude (`(y - GLOBAL_BASE) / (GLOBAL_TOP - GLOBAL_BASE)`).
// Direction comes from `scene.time.zw` (wind_dir_horiz, sampled at
// camera position once per frame). Tilts cloud features downwind so
// the sample at altitude y maps to lookup-position
// `(x + wind_x × tilt × t, y, z + wind_z × tilt × t)` where t is the
// normalized altitude. Real cumulus tilt 10-30% of cell height —
// 1500m thick cell → 150-450m top shift. We use 400m for a
// noticeable lean. Breaks the X/Z-aligned Worley grid character that
// has produced angular-line / V-cut artifacts in playtest.
const WIND_TILT_AMPLITUDE_M: f32 = 400.0;

// Per-cell-type Worley feature spacing (CM-9, 2026-04-26 — formerly
// priority ladder #4b). Different cloud types have characteristic
// feature scales:
// - Cumulus: medium puffs (current global value).
// - Nimbus: denser internal structure (deep convection has fine detail).
// - Stratus: smooth large-scale variation (low contrast layered look).
// - Squall: small textured features (gust-front turbulent character).
// - Frontal: mid-scale banded structure.
// - ClearAir / haze: soft low-contrast (large spacing → smooth wisps).
//
// Combined with per-type Perlin/Worley blend weights and CM-8
// anisotropic y-stretching, this gives each cell type a distinctive
// visual signature instead of "all clouds same character with
// different colors." Visible particularly when adjacent cells are
// different types — boundary blend transitions across a stylistic
// difference, not just a parameter difference.
//
// Cost: Worley primitive can no longer be hoisted as a single
// shared call across both blend cells; instead computed per-cell
// (with same-type cache reuse). Same-type cells share one call;
// mixed-type cells pay 2 Worley calls. Average ~1.05× Worley cost
// vs CM-8 baseline (~5%, ~0.02 ms).
// Per-cell pseudo-random spacing scale factor. Hash-based, deterministic
// per cell.seed_pos, range [0.7, 1.5]. Multiplied into the per-type
// Worley spacing so cells of the same type still vary in feature size
// across the world. Trig-based hash (`fract(sin(...) × C)`) is the
// classic GPU PRNG — small input range gives ~uniform distribution.
fn cell_spacing_factor(seed_pos: vec2<f32>) -> f32 {
    let h = fract(sin(dot(seed_pos, vec2<f32>(127.1, 311.7))) * 43758.5453);
    return 0.7 + 0.8 * h;
}

fn worley_spacing_for_type(cell_type: u32) -> f32 {
    switch cell_type {
        case 1u: { return 800.0; }   // Cumulus — medium puffs
        case 2u: { return 600.0; }   // Nimbus — denser internal detail
        case 3u: { return 2000.0; }  // Stratus — smooth large layers
        case 4u: { return 500.0; }   // Squall — small textured features
        case 5u: { return 1000.0; }  // Frontal — mid-scale bands
        // ClearAir / haze — bumped 1500 → 4000 on 2026-04-26
        // (Phase B follow-up 2). User in-game observation: clear-air
        // dot clouds appeared too small and too uniform; bumping to
        // 4000m gives features ~2.7× larger (proportional to
        // user-requested 3-4× max). Multi-octave variation deferred —
        // costs ~1.5 ms (Worley is expensive in real-game rendering
        // contexts; ALU estimates underestimate the cost).
        default: { return 4000.0; }
    }
}

// Per-cell-type Perlin-Worley blend weights (WEATHER.md § Sub-cell
// detail noise). Cumulus = puffy distinct cells. Nimbus = bulky
// detailed. Stratus = smooth layered. Squall = textured edges. Frontal
// = banded with detail.
fn perlin_worley_weights_for_type(cell_type: u32) -> vec2<f32> {
    switch cell_type {
        case 1u: { return vec2<f32>(0.7, 0.3); }  // Cumulus
        case 2u: { return vec2<f32>(0.5, 0.5); }  // Nimbus
        case 3u: { return vec2<f32>(0.2, 0.8); }  // Stratus
        case 4u: { return vec2<f32>(0.6, 0.4); }  // Squall
        case 5u: { return vec2<f32>(0.4, 0.6); }  // Frontal
        default: { return vec2<f32>(0.7, 0.3); }  // Cumulus default
    }
}

// ──────────────────────────────────────────────────────────────────
// CM-7 Beer-Lambert sun-direction shading (2026-04-26)
// ──────────────────────────────────────────────────────────────────
//
// Replaces the placeholder altitude-gradient lighting with proper
// physically-motivated shading: each cloud sample's color is the sum
// of (sun light attenuated by the cloud column between sample and
// sun) plus (ambient skylight). Beer-Lambert: light reaching a cloud
// sample = `sun_color * exp(-optical_depth_to_sun * extinction)`.
// Optical depth comes from a short secondary ray toward the sun.
//
// The Henyey-Greenstein phase function adds the characteristic
// forward-scattering halo (sun appears brighter through cloud edges
// when the camera is looking near the sun direction).
//
// Direct enabler of the eyecatch character: morning mist (low stratus
// undersides dim + sun-lit tops), sunset cumulus illumination
// (orange/pink tops via warm sun_color × full transmittance; cool
// undersides via ambient), dramatic storm signature (dark nimbus
// through deep optical depth).

const SUN_RAY_STEPS: i32 = 5;
const SUN_RAY_STEP_M: f32 = 200.0;
// Total secondary ray length: 5 × 200 = 1 km — enough to cover most
// of a typical cumulus column without wasting cost on samples in
// empty sky beyond the cloud top.
const SUN_EXTINCTION_SCALE: f32 = 0.005;
// Tuning parameter. Higher = darker shadow side; lower = bright
// throughout. CM-10 will refine via in-game observation.

const SUN_INTENSITY: f32 = 1.8;
// Brightness multiplier on sun-lit color. Bumped from 1.4 → 1.8 on
// 2026-04-26 after in-game observation showed CM-7 effect was too
// subtle at noon-from-below view (front-to-back compositing makes
// dim cloud-bottom samples obscure bright cloud-top samples). Higher
// SUN_INTENSITY combined with multi-scatter baseline (see below)
// makes the lit side more visible.

const MULTISCATTER_INTENSITY: f32 = 0.4;
// Multi-scatter approximation intensity (CM-7 revision 2026-04-26).
// Single-scattering Beer-Lambert produces dim cloud undersides at
// high-sun + view-from-below because the secondary ray traverses the
// full vertical column. Real clouds brighten via multi-scattering
// (light bouncing internally). Approximate by adding a baseline sun
// term that doesn't depend on sun_transmittance — represents diffuse
// sun light spreading through the cloud body.
// AAA technique (Schneider GDC15, Hillaire 2020). Tunable in CM-10.

const AMBIENT_INTENSITY: f32 = 0.55;
// Brightness multiplier on ambient (skylight) color. Lowered from
// 0.85 → 0.55 on 2026-04-26 to improve lit/shadow contrast — the
// previous value made ambient dominate too much, drowning out the
// CM-7 sun-direction signal.

const PHASE_G: f32 = 0.6;
// Henyey-Greenstein anisotropy. Positive = forward bias (sun appears
// brighter when camera looks near sun direction — characteristic
// "halo" through cloud edges). g=0.6 is the cumulus standard.

const PI: f32 = 3.14159265;

// Henyey-Greenstein single-scattering phase function. Returns the
// directional weight for sun-light scattered through cloud at angle
// `cos_theta = dot(view_dir, sun_dir)`. Computed once per pixel
// (constant along a ray) — moved outside the raymarch loop.
//
// Cost: ~1 sqrt + ~5 ALU. Negligible per-pixel cost.
fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom_base = 1.0 + g2 - 2.0 * g * cos_theta;
    let denom = denom_base * sqrt(denom_base);
    return (1.0 - g2) / (4.0 * PI * denom);
}

// Cheap cloud density approximation for the secondary lighting ray.
// Skips the high-frequency Worley + Perlin detail noise since the
// optical-depth integral averages over a long path; only the
// altitude band thickness × cell density × coverage_strength matters
// for shadowing accuracy. Uses the cell parameters from the main
// ray's lookup — the secondary ray (typically <1 km traversal) stays
// within the same cell horizontally for typical sun angles.
fn cloud_density_cheap_in_cell(world_pos: vec3<f32>, cell: GpuWeatherCell) -> f32 {
    if (cell.coverage_strength <= 0.0 || cell.density <= 0.0) {
        return 0.0;
    }
    let alt_factor = altitude_profile(
        cell.profile_kind, world_pos.y,
        cell.base_altitude_m, cell.top_altitude_m,
    );
    return cell.density * alt_factor * cell.coverage_strength;
}

// Sun transmittance at a sample point: integrates **blended** cloud
// density along a secondary ray toward the sun, returns
// `exp(-optical_depth × scale)`.
//
// **CM-10 revision 2026-04-26**: was previously cell1-only
// (`sun_transmittance_through_cell`). At the Voronoi midline,
// `find_nearest_two_active_cells` swaps idx1↔idx2 — so the cell1
// parameters used for sun_transmittance suddenly switch to the
// other cell's parameters → visible lighting discontinuity at the
// boundary even though CM-6 blended *density* smoothly. See
// `boundary_line.png`. Now we blend cell1+cell2 cheap-densities
// along the secondary ray, weighted by the main sample's w1/w2.
//
// Approximation: w1/w2 captured at main sample's position is reused
// for all secondary samples (which are within ~1 km of main → likely
// same cell membership). Avoids re-running find_nearest_two per
// secondary sample (would be 5x more expensive).
//
// Cost: SUN_RAY_STEPS × 2 cheap density calls = ~5 × 60 ALU = ~300
// cycles per main sample (was ~150). Net ~+0.05 ms.
fn sun_transmittance_blended(
    sample_pos: vec3<f32>,
    cell1: GpuWeatherCell,
    cell2: GpuWeatherCell,
    w1: f32,
    w2: f32,
    sun_dir: vec3<f32>,
) -> f32 {
    var optical_depth: f32 = 0.0;
    for (var i: i32 = 0; i < SUN_RAY_STEPS; i = i + 1) {
        let t = SUN_RAY_STEP_M * (f32(i) + 0.5);
        let s = sample_pos + sun_dir * t;
        let d1 = cloud_density_cheap_in_cell(s, cell1);
        let d2 = cloud_density_cheap_in_cell(s, cell2);
        let blended = w1 * d1 + w2 * d2;
        optical_depth = optical_depth + blended * SUN_RAY_STEP_M;
    }
    return exp(-optical_depth * SUN_EXTINCTION_SCALE);
}

// Per-sample cloud-pass output: density (for absorption + alpha
// accumulation along view ray) and sun_transmittance (for Beer-Lambert
// lighting). Returned together so the shared find_nearest_two_active_cells
// scan happens once per sample.
struct CloudSample {
    density: f32,
    sun_transmittance: f32,
}

// Per-cell density contribution given pre-computed alt_factor + global
// noise samples (CM-6 helper, 2026-04-26). Final cloud_factor + density
// math only — the expensive noise calls happen ONCE in the caller and
// the cheap altitude_profile is also pre-computed.
fn cell_density_contribution(
    cell: GpuWeatherCell,
    alt_factor: f32,
    worley_struct: f32,
    perlin_norm: f32,
) -> f32 {
    let weights = perlin_worley_weights_for_type(cell.cell_type);
    let raw_factor = weights.x * worley_struct + weights.y * perlin_norm;
    // Continuous-margin modulation — LINEAR REMAP of [coverage_threshold,
    // 1.0] to [0, 1]. Identity at cov=1.0 (preserves Beer-Lambert + cel
    // rim); gates noise floor for marginal cells. See
    // `docs/WEATHER.md § Continuous qualification margin`.
    let coverage_threshold = 1.0 - cell.coverage_strength;
    let cloud_factor = clamp(
        (raw_factor - coverage_threshold) / cell.coverage_strength,
        0.0,
        1.0,
    );
    return cell.density * alt_factor * cloud_factor;
}

// Cloud density at a world-space sample point.
//
// **CM-6 boundary blending** (2026-04-26): finds the two nearest
// cells, computes a smoothstep blend weight from the half-difference
// of distances, and weighted-averages each cell's density
// contribution. Adjacent same-type cells fuse into one body of
// weather; different-type cells transition smoothly across the
// Voronoi midline. Earlier single-cell version produced visible hard
// seams ("clouds vanish when leaving a cell" symptom).
//
// Tessellated W2 model with **two-stage cost gating**:
// 1. Find the two cells whose seeds are nearest to this XZ.
// 2. Compute blend weight w1 from `(d2 - d1) / 2`.
// 3. **Cheap pre-filter**: one Perlin perturbation call (CM-5),
//    altitude_perturbation_amplitude lookup + altitude_profile per
//    active cell. If both cells' alt_factor = 0 (sample outside any
//    cloud band), return 0 — skips the expensive density noise.
//    Critical because the raymarch covers 12 km of altitude and most
//    samples are in empty sky.
// 4. **Expensive noise**: Worley_3d + density Perlin (one call each,
//    sampled at world_pos — same for both cells).
// 5. Blend each active cell's density contribution by `w1` / `w2`.
//
// The Worley + Perlin noise fields are sampled at the absolute world
// position (not cell-relative), so adjacent same-type cells produce a
// single continuous cloud structure across their boundary — the
// "front" or "deck" reads as one body of weather. Different-type
// adjacent cells (e.g. Cumulus next to Stratus) get a smoothstep
// transition between their per-type Perlin-Worley blend weights and
// altitude bands.
fn sample_cloud_density(world_pos: vec3<f32>, sun_dir: vec3<f32>) -> CloudSample {
    // 2-cell blend retained — find_nearest_three exists for future
    // V-cut fix (task #328) but adds cost without addressing the
    // primary user-observed "cloud disappears in clear region"
    // symptom. Atmospheric haze is the address for that.
    let nearest3 = find_nearest_three_active_cells(world_pos.xz);
    let cell1 = weather_cells[nearest3.idx1];
    let cell2 = weather_cells[nearest3.idx2];

    // PINNED 2026-04-26: gate on coverage_strength, not density.
    // ClearAir cells with nonzero margin carry haze parameters and
    // SHOULD render → wispy transitions. True clear sky has
    // coverage_strength == 0 and is skipped here.
    let active1 = cell1.coverage_strength > 0.0 && cell1.density > 0.0;
    let active2 = cell2.coverage_strength > 0.0 && cell2.density > 0.0;
    if (!active1 && !active2) {
        return CloudSample(0.0, 1.0);
    }

    // CM-6 blend weight (2-cell). 3-cell blend pinned for V-cut fix.
    let d1 = sqrt(max(nearest3.dsq1, 1.0));
    let half_diff = (nearest3.dsq2 - nearest3.dsq1) / (4.0 * d1);
    let blend_t = smoothstep(0.0, BOUNDARY_BLEND_DISTANCE_M, half_diff);
    let w1 = 0.5 + 0.5 * blend_t;
    let w2 = 1.0 - w1;

    // CM-5 altitude perturbation — one Perlin call shared by both
    // cells. Sampled at fixed y=0 so the perturbation is a function of
    // (x, z) only, producing a coherent bumpy surface for cell
    // tops/bases.
    let perturb = cloud_perlin_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_PERLIN_OCTAVE,
        world_pos.x, 0.0, world_pos.z,
        ALT_PERTURB_SPACING_M,
    );

    // Cheap altitude pre-filter: per-cell altitude_profile (arithmetic
    // + per-type lookup). Sampling alt_factor here lets us skip the
    // expensive density noise below when the sample is outside any
    // cloud band — critical because most ray samples in the 12 km
    // global band are in empty sky.
    //
    // **No early-exit on `w2`** (revision 2026-04-26 — fixes pie_cut.png
    // V-shape artifact): an earlier version skipped cell 2's pre-filter
    // when `w2 < 0.05`, which produced a hard discontinuity at the
    // contour `(d2 - d1) / 2 ≈ 3950 m` in world space — a hyperbola
    // whose asymptotes form a V-wedge converging at the midpoint
    // between cells. The cel-banded alpha amplified the small density
    // step into a sharp band-jump rendered as a visible straight-line
    // cut. The math `density += w2 * cell_2_contribution` already
    // self-attenuates (small w2 → small contribution), so removing the
    // early-exit eliminates the discontinuity at trivial cost (~0.03 ms
    // for the unconditional altitude_profile + per-type-amp lookup
    // when deep inside cell 1).
    // Post-CM-10 Phase B: per-cell vertical_amp_factor multiplies into
    // CM-5 altitude perturbation amplitudes. Strong nimbus (~5×)
    // produces dramatic ±1500m+ top shifts → towering cumulonimbus
    // reaching ~9km. Marginal cumulus (~1×) stays near nominal.
    var alt_factor1: f32 = 0.0;
    if (active1) {
        let amp1 = altitude_perturbation_amplitude(cell1.cell_type)
            * cell1.vertical_amp_factor;
        let p1 = perturb * cell1.coverage_strength;
        alt_factor1 = altitude_profile(
            cell1.profile_kind, world_pos.y,
            cell1.base_altitude_m + p1 * amp1.y,
            cell1.top_altitude_m + p1 * amp1.x,
        );
    }
    var alt_factor2: f32 = 0.0;
    if (active2) {
        let amp2 = altitude_perturbation_amplitude(cell2.cell_type)
            * cell2.vertical_amp_factor;
        let p2 = perturb * cell2.coverage_strength;
        alt_factor2 = altitude_profile(
            cell2.profile_kind, world_pos.y,
            cell2.base_altitude_m + p2 * amp2.y,
            cell2.top_altitude_m + p2 * amp2.x,
        );
    }
    if (alt_factor1 <= 0.0 && alt_factor2 <= 0.0) {
        return CloudSample(0.0, 1.0);
    }

    // Density noise. Only reached when at least one cell's
    // altitude_profile is positive.
    //
    // **CM-8 anisotropic y-stretching** (2026-04-26): multiplies
    // world_pos.y by VERTICAL_NOISE_STRETCH before sampling, giving
    // ~5-6 vertical noise waves per cumulus altitude band instead
    // of 1-2. Breaks up the regular Worley grid pattern visible
    // from above as "rows" and from below as "globules". Combined
    // with CM-4's linear-remap gating, marginal cells naturally
    // produce vertical lobes with sky-gaps. Cost: free.
    //
    // **CM-9 per-cell-type Worley spacing** (2026-04-26): each cell
    // type has its own Worley feature scale via
    // `worley_spacing_for_type` — Stratus smooth (2 km), Squall
    // textured (500 m), Cumulus puffy (800 m), etc. Worley computed
    // per-cell instead of once globally, but **same-type cells
    // share a single call** (the typical case for deep-cell-1
    // pixels and same-type-blend pixels). Mixed-type blend zones
    // pay 2 Worley calls; that's the visual win — different cell
    // types read with distinct stylistic signatures.
    //
    // Perlin still uses the global CLOUD_PERLIN_SPACING_M (per-type
    // Perlin variation could land in CM-10 if needed; for now the
    // smooth-noise component is uniform so cell types differ only
    // in their Worley character + Perlin/Worley blend weights).
    // **Variable y-stretch** (Phase B follow-up tuning 2026-04-26):
    // fixed VERTICAL_NOISE_STRETCH produced a regular vertical
    // wavelength → at fixed (x, z), Perlin/Worley peak/valley fall
    // at predictable y intervals → CM-4 gating + carve amplified
    // these into visible horizontal "stack of layers" stripes
    // (`layers.png`). Modulating stretch by the existing `perturb`
    // sample (3 km wavelength) makes adjacent (x, z) regions have
    // slightly different vertical wavelengths — layers dither across
    // pixels instead of forming uniform horizontal bands. Free
    // (reuses existing Perlin sample). Stretch range: ~2.4× to ~3.6×.
    let local_stretch = VERTICAL_NOISE_STRETCH * (1.0 + 0.3 * perturb);
    let y_stretched = world_pos.y * local_stretch;

    // **CM-10 domain warping** — perturb (x, z) before density-noise
    // lookup to break axis-aligned Worley grid regularity. `perturb`
    // (computed earlier for CM-5 altitude perturbation) is reused as
    // warp_x; warp_z samples Perlin at an offset spatial origin so
    // the two displacement components are decorrelated.
    let warp_z_perlin = cloud_perlin_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_PERLIN_OCTAVE,
        world_pos.x + 7919.0, 0.0, world_pos.z + 6133.0,
        NOISE_WARP_SPACING_M,
    );
    // **Post-CM-10 Phase A.1 wind tilt**: shift density-noise lookup
    // downwind by `wind_dir × WIND_TILT_AMPLITUDE_M × normalized_altitude`.
    // Higher samples shift more → cloud features lean downwind.
    // wind_dir in scene.time.zw is sampled CPU-side once per frame
    // from W1 wind_at(camera_xz, mid_altitude, time).
    let alt_normalized = clamp(
        (world_pos.y - GLOBAL_CLOUD_BASE_M)
            / (GLOBAL_CLOUD_TOP_M - GLOBAL_CLOUD_BASE_M),
        0.0,
        1.0,
    );
    let wind_dir = scene.time.zw;
    let tilt_offset_x = wind_dir.x * WIND_TILT_AMPLITUDE_M * alt_normalized;
    let tilt_offset_z = wind_dir.y * WIND_TILT_AMPLITUDE_M * alt_normalized;
    let warped_x = world_pos.x + perturb * NOISE_WARP_AMPLITUDE_M + tilt_offset_x;
    let warped_z = world_pos.z + warp_z_perlin * NOISE_WARP_AMPLITUDE_M + tilt_offset_z;

    let perlin_raw = cloud_perlin_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_PERLIN_OCTAVE,
        warped_x, y_stretched, warped_z,
        CLOUD_PERLIN_SPACING_M,
    );
    let perlin_norm = clamp((perlin_raw + 0.7) / 1.4, 0.0, 1.0);

    // **CM-10 mixed-spacing Worley** (revision 2026-04-26): instead
    // of computing TWO Worley calls (one per cell) and blending
    // results, mix the *spacings* and compute ONE Worley call. This
    // produces a single coherent noise field whose feature scale
    // smoothly transitions across cell boundaries — the texture
    // CHARACTER changes gradually rather than two distinct textures
    // interfering at the midline (visible as a sharp seam in
    // `blend_line.png`). At deep-cell-1 (w2=0), mixed_spacing =
    // spacing1; at midline (w2=0.5), spacing is halfway between;
    // at deep-cell-2 (w2=1.0), mixed_spacing = spacing2.
    //
    // Per-cell character is preserved in the Perlin/Worley *blend
    // weights* (per_type_for_cell) which still differ per cell —
    // each cell weights worley_struct vs perlin_norm differently.
    //
    // Cost win: always 1 Worley call (was 1-2 with same-type cache
    // reuse). Net negative perf cost vs CM-9.
    // **Per-cell hash-based spacing variation** (Phase B follow-up
    // 2026-04-26): each cell's spacing is `base_spacing × (0.7 +
    // 0.8 × hash)` where `hash ∈ [0, 1)` is derived from cell.seed_pos.
    // Range 0.7-1.5× the per-type baseline → adjacent cells across
    // boundaries get different feature sizes. Free (just hash + mul).
    // The CM-9 mix(spacing1, spacing2, w2) blend handles smooth
    // transition across boundaries; we just pre-jitter each cell's
    // spacing before the mix.
    let spacing1 = worley_spacing_for_type(cell1.cell_type)
        * cell_spacing_factor(cell1.seed_pos);
    let spacing2 = worley_spacing_for_type(cell2.cell_type)
        * cell_spacing_factor(cell2.seed_pos);
    let mixed_spacing = mix(spacing1, spacing2, w2);
    // **Single Worley call**: a multi-octave variant cost +1.5 ms,
    // not the +0.06 ms that simple back-of-envelope ALU math
    // suggested. Worley's full-cost is GPU-context-dependent
    // (register pressure / divergence / memory pipeline interactions),
    // which doesn't reduce to simple ALU counts. Stayed at one
    // Worley call per sample. Feature-size variation achieved via
    // per-cell-hash spacing scaling above.
    let worley_v = worley_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_WORLEY_OCTAVE,
        warped_x, y_stretched, warped_z,
        mixed_spacing, CLOUD_FEATURES_PER_CELL,
    );
    let worley_struct = clamp(1.0 - worley_v.f_blended, 0.0, 1.0);

    // **Post-CM-10 Phase B vertical-carve via perlin_norm reuse**
    // (cost-saving variant 2026-04-26): instead of an extra dedicated
    // Perlin call at full (x, y, z), we reuse the already-computed
    // `perlin_norm` (which has 3D structure via CM-8's y-stretch) as
    // the carve noise. `(1 - perlin_norm)` carves where Perlin is low
    // → reinforces existing density contrast (high-perlin gets even
    // denser; low-perlin gets carved out). Margin-coupled amplitude
    // means strong-cov cells barely carve (continuous mass), marginal
    // cells carve aggressively (3D voids). Saves ~0.1 ms vs the
    // dedicated-Perlin variant.
    let carve_signal = 1.0 - perlin_norm;
    if (alt_factor1 > 0.0) {
        let carve_amp1 = (1.0 - cell1.coverage_strength) * VERTICAL_CARVE_AMP;
        let carve1 = clamp(1.0 - carve_amp1 * carve_signal, 0.0, 1.0);
        alt_factor1 = alt_factor1 * carve1;
    }
    if (alt_factor2 > 0.0) {
        let carve_amp2 = (1.0 - cell2.coverage_strength) * VERTICAL_CARVE_AMP;
        let carve2 = clamp(1.0 - carve_amp2 * carve_signal, 0.0, 1.0);
        alt_factor2 = alt_factor2 * carve2;
    }

    var density: f32 = 0.0;
    if (alt_factor1 > 0.0) {
        density = density + w1 * cell_density_contribution(
            cell1, alt_factor1, worley_struct, perlin_norm,
        );
    }
    if (alt_factor2 > 0.0) {
        density = density + w2 * cell_density_contribution(
            cell2, alt_factor2, worley_struct, perlin_norm,
        );
    }
    if (density <= 0.0) {
        return CloudSample(0.0, 1.0);
    }
    // CM-7 secondary ray toward sun for Beer-Lambert shading. **CM-10
    // revision** blends cell1+cell2 cheap-densities along the
    // secondary ray (was cell1-only, which produced a visible
    // lighting discontinuity at the Voronoi midline because idx1↔idx2
    // swap → CM-6 blended density correctly but CM-7 didn't blend
    // lighting). Uses the main sample's w1/w2 for all secondary
    // samples (approximation — secondary ray is within ~1 km of
    // main, almost always same cell membership).
    let sun_t = sun_transmittance_blended(world_pos, cell1, cell2, w1, w2, sun_dir);
    return CloudSample(density, sun_t);
}

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       ndc:      vec2<f32>,
};

@vertex
fn vs_volumetric(@builtin(vertex_index) idx: u32) -> VsOut {
    // Oversized full-screen triangle at the far plane (clip_z = 1.0).
    let x = f32(i32(idx) & 1) * 4.0 - 1.0;
    let y = f32(i32(idx) >> 1) * 4.0 - 1.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

fn unproject(ndc: vec2<f32>, ndc_z: f32) -> vec3<f32> {
    let clip = vec4<f32>(ndc.x, ndc.y, ndc_z, 1.0);
    let world4 = camera.inverse_view_proj * clip;
    return world4.xyz / world4.w;
}

// Maximum raymarch step count — upper bound for the adaptive sampler.
// 24 steps × 500m target = 12 km global band coverage. With Worley
// spacing 1500m, 500m steps give 3 samples per noise wavelength —
// adequate by Nyquist while keeping per-pixel cost bounded under the
// tessellation cost added by per-step nearest-cell lookup (C2-polish
// 2026-04-26). Was 32 in the per-cell-iteration era.
const RAYMARCH_STEPS_MAX: i32 = 24;

// Target step distance (m). With Worley spacing 2000 m, sampling
// every ~400 m gives 5 samples per feature wavelength — enough for
// stable shape per Nyquist, well below the perceptual flicker
// threshold. Adaptive step count = ceil(band_length / target_step).
const RAYMARCH_TARGET_STEP_M: f32 = 400.0;

// Maximum ray distance (m) for the B3 single-cell test. The
// ray-cylinder intersection caps the actual march distance much
// shorter than this for any single cell; this is the safety net for
// pixels that intersect a cell's footprint at all.
const MAX_RAY_DISTANCE_M: f32 = 15000.0;

// Density-to-absorption scale. Tuned for the W2 cumulus-density range
// (~0.35 peak per `weather::cell_state::parameterize`). At 0.0022 a
// max-density sample contributes ~7% alpha per 100 m step → visible
// but translucent cumulus over typical 16-step ray spans. Earlier
// 0.0015 was tuned for the B3 hardcoded TEST_CELL with density 0.6;
// real W2 cumulus needs slightly higher absorption to read as visible.
const ABSORPTION_SCALE: f32 = 0.0022;

// Compute ray ∩ altitude band [base, top]. Returns `(t_enter, t_exit)`.
// Returns `(-1, -1)` if the ray misses the band entirely.
fn ray_altitude_band(
    cam_pos: vec3<f32>,
    view_dir: vec3<f32>,
    base_m: f32,
    top_m: f32,
    max_dist: f32,
) -> vec2<f32> {
    if (abs(view_dir.y) < 1e-6) {
        // Ray parallel to the slabs (horizontal viewing). Inside the
        // band → march to max distance; outside → miss.
        if (cam_pos.y >= base_m && cam_pos.y <= top_m) {
            return vec2<f32>(0.0, max_dist);
        }
        return vec2<f32>(-1.0, -1.0);
    }
    let t_base = (base_m - cam_pos.y) / view_dir.y;
    let t_top  = (top_m - cam_pos.y) / view_dir.y;
    let t_enter_raw = min(t_base, t_top);
    let t_exit_raw  = max(t_base, t_top);
    if (t_exit_raw < 0.0) {
        return vec2<f32>(-1.0, -1.0);
    }
    let t_enter = max(t_enter_raw, 0.0);
    let t_exit  = min(t_exit_raw, max_dist);
    if (t_enter >= t_exit) {
        return vec2<f32>(-1.0, -1.0);
    }
    return vec2<f32>(t_enter, t_exit);
}

// Compute ray ∩ vertical XZ-disk (infinite-height cylinder). Returns
// `(t_enter, t_exit)`; `(-1, -1)` for miss. Pairs with
// [`ray_altitude_band`] so we march only within the cell's actual
// volume — without this, horizontal rays at cloud altitude would march
// through 15 km of empty space outside the test cell's 3 km footprint.
//
// Performance: this is THE critical early-out. Most pixels see no cell
// at all and return immediately after the band+cylinder test (no Worley
// calls). Pixels that hit the cylinder march only within the
// intersection range — typically 3-6 km along ray, vs 15 km without
// this clamp. ~5-10× per-pixel cost reduction.
fn ray_vertical_cylinder(
    cam_pos: vec3<f32>,
    view_dir: vec3<f32>,
    center_xz: vec2<f32>,
    radius_m: f32,
    max_dist: f32,
) -> vec2<f32> {
    let dx = cam_pos.x - center_xz.x;
    let dz = cam_pos.z - center_xz.y;
    let a = view_dir.x * view_dir.x + view_dir.z * view_dir.z;
    if (a < 1e-6) {
        // Vertical ray. Inside disk → full extent; outside → miss.
        if (dx * dx + dz * dz <= radius_m * radius_m) {
            return vec2<f32>(0.0, max_dist);
        }
        return vec2<f32>(-1.0, -1.0);
    }
    let b = 2.0 * (dx * view_dir.x + dz * view_dir.z);
    let c = dx * dx + dz * dz - radius_m * radius_m;
    let disc = b * b - 4.0 * a * c;
    if (disc < 0.0) {
        return vec2<f32>(-1.0, -1.0);
    }
    let s = sqrt(disc);
    let t0 = (-b - s) / (2.0 * a);
    let t1 = (-b + s) / (2.0 * a);
    if (t1 < 0.0) {
        return vec2<f32>(-1.0, -1.0);
    }
    let t_enter = max(t0, 0.0);
    let t_exit  = min(t1, max_dist);
    if (t_enter >= t_exit) {
        return vec2<f32>(-1.0, -1.0);
    }
    return vec2<f32>(t_enter, t_exit);
}

@fragment
fn fs_volumetric(in: VsOut) -> @location(0) vec4<f32> {
    let world_near = unproject(in.ndc, 0.0);
    let world_far  = unproject(in.ndc, 1.0);
    let view_dir = normalize(world_far - world_near);
    let cam_pos = camera.view_pos.xyz;

    // B3b — sample scene depth so cloud rays terminate at scene
    // geometry. depth_ndc ∈ [0, 1]; 1.0 = sky / cleared.
    let frag_pixel = vec2<i32>(in.clip_pos.xy);
    let depth_ndc = textureLoad(scene_depth, frag_pixel, 0);
    var t_terrain = MAX_RAY_DISTANCE_M;
    if (depth_ndc < 1.0) {
        let depth_pos_cam_rel = unproject(in.ndc, depth_ndc);
        t_terrain = length(depth_pos_cam_rel);
    }

    // Tessellated raymarch (C2-polish 2026-04-26). Single ray traversal
    // through the global cloud altitude band; per-sample nearest-cell
    // lookup determines weather params at that XZ. This produces
    // continuous tessellation per WEATHER.md § "No-weather is a type
    // of weather" — every point belongs to a cell, clear-air cells
    // contribute zero density, like-type adjacent cells appear as one
    // continuous body of weather.
    let band = ray_altitude_band(
        cam_pos, view_dir,
        GLOBAL_CLOUD_BASE_M, GLOBAL_CLOUD_TOP_M,
        MAX_RAY_DISTANCE_M,
    );
    if (band.x < 0.0) {
        return vec4<f32>(0.0);
    }
    let t_enter = band.x;
    let t_exit = min(band.y, t_terrain);
    if (t_enter >= t_exit) {
        return vec4<f32>(0.0);
    }
    let band_length = t_exit - t_enter;
    let want_steps = ceil(band_length / RAYMARCH_TARGET_STEP_M);
    let actual_steps = i32(min(f32(RAYMARCH_STEPS_MAX), max(want_steps, 4.0)));
    let step_dist = band_length / f32(actual_steps);

    // Per-pixel ray-start jitter — replaces the fixed 0.5 step-midpoint
    // with a deterministic-per-pixel offset in [0, 1). Hides the
    // horizontal band-edge artifacts from altitude_profile discrete
    // transitions: adjacent pixels' rays sample at scattered t offsets,
    // so the band boundaries dither across pixels and read as soft
    // texture instead of visible rings (the user-observed "stair-stepping"
    // 2026-04-26).
    let jitter = pixel_hash_unit(frag_pixel);

    // CM-7 Beer-Lambert lighting setup. Sun direction is constant
    // along a single view ray.
    let sun_dir = normalize(scene.sun_dir.xyz);
    // Sun-height fade — drops sun contribution to zero when the sun
    // is below the horizon, keeps it strong at sunset / sunrise.
    let sun_height_factor = clamp(scene.sun_dir.y, 0.0, 1.0);
    // Hoist per-pixel constants OUT of the per-sample loop. These
    // depend only on uniforms (sun_color, ambient, sun_height_factor)
    // → constant across all samples on this ray. Computing them in
    // the loop wastes 5 ALU × 24 samples × 4M pixels × 60 fps ≈ 0.6 ms.
    let multiscatter_per_pixel = scene.sun_color.rgb
        * MULTISCATTER_INTENSITY
        * sun_height_factor;
    let ambient_per_pixel = scene.ambient.rgb * AMBIENT_INTENSITY;
    // **Phase function NOT applied as a per-sample multiplier (CM-7
    // revision 2026-04-26)**: the normalized Henyey-Greenstein
    // function returns ~0.03 at perpendicular view angle, ~0.01 at
    // back-scatter — those values × sun_color × sun_transmittance
    // produce essentially zero sun contribution when looking at a
    // cumulus from anywhere except directly toward the sun. Real
    // clouds get their brightness from multi-scattering, which
    // single-scatter HG doesn't model. Dropping the phase multiplier
    // gives the correct cel-shaded look: cumulus sun-side bright,
    // shadow-side dim, regardless of camera angle. The
    // `henyey_greenstein` function is kept for possible CM-10
    // re-introduction as an *additive halo* (only contributes
    // strongly when looking near the sun direction) rather than a
    // multiplier on the base sun-lit term.

    var accumulated_alpha: f32 = 0.0;
    var accumulated_color: vec3<f32> = vec3<f32>(0.0);

    for (var i: i32 = 0; i < actual_steps; i = i + 1) {
        let t = t_enter + step_dist * (f32(i) + jitter);
        let sample_pos = cam_pos + view_dir * t;
        let cloud = sample_cloud_density(sample_pos, sun_dir);
        if (cloud.density <= 0.0) {
            continue;
        }
        let absorption = 1.0 - exp(-cloud.density * step_dist * ABSORPTION_SCALE);

        // CM-7 Beer-Lambert lighting model (replaces the placeholder
        // altitude-gradient stand-in). Per-sample color is the sum of:
        //   1. Sun light reaching this sample, attenuated by the cloud
        //      column toward the sun (`cloud.sun_transmittance`).
        //      Multiplied by sun_height so contribution fades to zero
        //      at night. PRIMARY visual differentiator — lit side of
        //      cloud reads brighter than shadow side.
        //   2. Multi-scatter approximation — baseline sun-tinted term
        //      that doesn't depend on transmittance. Represents diffuse
        //      sun light bouncing through the cloud body. Without this,
        //      noon-from-below view shows uniformly dim cloud undersides
        //      because front-to-back compositing puts low-transmittance
        //      bottom samples first; bright top samples are obscured.
        //   3. Ambient skylight (constant per-sample; represents
        //      indirect lighting from the sky dome).
        // Sunset character emerges naturally: scene.sun_color shifts
        // warm; ambient stays cool from skylight; the contrast at
        // cloud rim reads as orange/pink-lit tops over cool bases.
        // sun_lit is the only per-sample term (depends on transmittance);
        // multiscatter + ambient are pre-computed per-pixel.
        let sun_lit = scene.sun_color.rgb
            * cloud.sun_transmittance
            * SUN_INTENSITY
            * sun_height_factor;
        let sample_color = sun_lit + multiscatter_per_pixel + ambient_per_pixel;

        accumulated_color = accumulated_color
            + sample_color * absorption * (1.0 - accumulated_alpha);
        accumulated_alpha = accumulated_alpha
            + absorption * (1.0 - accumulated_alpha);

        if (accumulated_alpha > 0.99) {
            break;
        }
    }

    // Banded-alpha cel-shading (LOCKED in WEATHER.md). 6 bands.
    //
    // **CM-10 tuning 2026-04-26**: previously the lowest band had a
    // *hard cutoff* — values below 1/6 snapped to 0, producing a
    // visible discrete contour line at α ≈ 0.166 wherever density
    // smoothly faded out (boundary-blend zones, cell edges, ray
    // glancing exits). The hard line was particularly distracting
    // at CM-6 cell-blend transitions (`boundary_blend.png` artifact).
    // Now the lowest band uses a smoothstep ramp from 0 to 1/6, so
    // alpha fades softly to 0 instead of clipping. Higher bands keep
    // the discrete cel character — silhouettes stay crisp at the
    // visible cloud body, but feathery edges no longer terminate in
    // a sharp line.
    let banded_alpha = floor(accumulated_alpha * 6.0) / 6.0;
    let low_smooth = smoothstep(0.0, 1.0 / 6.0, accumulated_alpha) * (1.0 / 6.0);
    let final_alpha = max(banded_alpha, low_smooth);

    return vec4<f32>(accumulated_color, final_alpha);
}
