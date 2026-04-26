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
fn altitude_profile(cell_type: u32, y_alt_m: f32, base_m: f32, top_m: f32) -> f32 {
    if (y_alt_m < base_m || y_alt_m > top_m) {
        return 0.0;
    }
    let depth = top_m - base_m;
    if (depth < 1.0) {
        return 0.0;
    }
    let t = (y_alt_m - base_m) / depth;
    if (cell_type == CELL_CLEAR_AIR) {
        return 0.0;
    } else if (cell_type == CELL_STRATUS) {
        // Flat horizontal layer — uniform density across thin span.
        return 1.0;
    } else {
        // Convective / frontal — parabolic peak at mid-depth.
        let centered = 1.0 - (2.0 * t - 1.0) * (2.0 * t - 1.0);
        return max(centered, 0.0);
    }
}

// Find the cell whose seed_pos is nearest to `world_xz` — implements
// W2 Voronoi tessellation by brute-force scan over the buffered active
// set. Returns the cell's index in the storage buffer.
//
// **Critical**: includes ClearAir cells in the search. A clear-air
// cell legitimately "owns" its Voronoi region; the caller (sample
// density) handles density=0 by returning zero contribution. Skipping
// clear-air would cause a single cumulus cell to "fill the world" out
// to the next cumulus cell (since the nearest non-empty would be
// returned for samples actually in clear-air regions). 2026-04-26 bug
// fix discovered during in-game playtest — "clouds vanish when
// leaving a cell" symptom.
//
// Empty / unfilled buffer slots (density=0 AND seed_pos at origin)
// are skipped via the `is_empty_slot` test below — these are
// uninitialized tail slots, not real clear-air cells. Real clear-air
// cells have a valid `seed_pos` populated by ChunkedWeather.
//
// Cost: ~50-64 distance checks per call. At ~24 ray-steps per pixel,
// ~1.5-2k checks per pixel — modest GPU compute. Still the dominant
// per-pixel cost; future optimization options documented in
// `docs/WEATHER.md § Tessellation discipline`.
//
// Per WEATHER.md § "No-weather is a type of weather": every XZ point
// owns one cell; clear-air cells render zero density, but they still
// participate in the tessellation so cumulus cells stay bounded to
// their actual Voronoi regions.
fn find_nearest_active_cell(world_xz: vec2<f32>) -> u32 {
    var best_idx: u32 = 0u;
    var best_dist_sq: f32 = 1.0e30;
    let cell_count = arrayLength(&weather_cells);
    for (var i = 0u; i < cell_count; i = i + 1u) {
        let cell = weather_cells[i];
        // Empty buffer-tail slots are filled with a sentinel `seed_pos`
        // (e.g. 1e9, 1e9) by `Renderer::update_weather_cells` so the
        // distance check naturally excludes them — no special check
        // required. Real ClearAir cells have valid seed_pos and DO
        // participate in nearest-cell lookup (per tessellation rule).
        let d = world_xz - cell.seed_pos;
        let dsq = d.x * d.x + d.y * d.y;
        if (dsq < best_dist_sq) {
            best_dist_sq = dsq;
            best_idx = i;
        }
    }
    return best_idx;
}

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
    switch cell_type {
        case 1u: { return vec2<f32>(500.0, 200.0); }   // Cumulus — puffy
        case 2u: { return vec2<f32>(1500.0, 400.0); }  // Nimbus — deep convection
        case 3u: { return vec2<f32>(50.0, 30.0); }     // Stratus — flat layer
        case 4u: { return vec2<f32>(700.0, 250.0); }   // Squall — textured
        case 5u: { return vec2<f32>(400.0, 150.0); }   // Frontal — moderate
        default: { return vec2<f32>(50.0, 50.0); }     // ClearAir / haze — subtle
    }
}

// Spacing for the long-wavelength altitude-perturbation Perlin field.
// ~3 km wavelength → bumps a few cells wide, suitable for "convective
// top" mental model. Independent from CLOUD_PERLIN_SPACING_M (which
// drives sub-cell density-noise structure at ~800 m).
const ALT_PERTURB_SPACING_M: f32 = 3000.0;

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

// Cloud density at a world-space sample point.
//
// Tessellated W2 model (C2-polish, 2026-04-26):
// 1. Find which cell owns this XZ via nearest-cell lookup.
// 2. If clear-air or empty slot, density = 0.
// 3. Sample altitude_profile against the cell's [base, top] band.
// 4. Apply Perlin-Worley blend per cell type — produces continuous
//    cloud structure that flows naturally across cell boundaries when
//    neighbors share a type, transitions visibly when types differ.
//
// The Worley + Perlin noise field is sampled at the absolute world
// position (not cell-relative), so adjacent same-type cells produce a
// single continuous cloud structure across their boundary — the
// "front" or "deck" reads as one body of weather rather than two
// separate puffs.
fn sample_cloud_density(world_pos: vec3<f32>) -> f32 {
    let cell_idx = find_nearest_active_cell(world_pos.xz);
    let cell = weather_cells[cell_idx];
    // PINNED 2026-04-26: gate on coverage_strength, not density. ClearAir
    // cells with nonzero margin (W1 approaching a cloud-bearing
    // threshold) carry haze parameters and SHOULD render → wispy
    // transitions. True clear sky has coverage_strength == 0 and is
    // skipped here.
    if (cell.coverage_strength <= 0.0 || cell.density <= 0.0) {
        return 0.0;
    }
    // CM-5: Per-sample altitude perturbation. Sample a long-wavelength
    // Perlin field at the sample's (x, z) — fixed y=0 keeps the
    // perturbation a function of horizontal position only, so the cloud
    // top/base form a coherent bumpy surface rather than per-altitude
    // jitter. Per-type amplitude × coverage_strength means high-margin
    // cumulus / nimbus cells get bumpy convective tops, marginal cells
    // and stratus stay flat. Adds one Perlin-3D call per ray sample.
    let amp = altitude_perturbation_amplitude(cell.cell_type);
    let perturb = cloud_perlin_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_PERLIN_OCTAVE,
        world_pos.x, 0.0, world_pos.z,
        ALT_PERTURB_SPACING_M,
    );
    let perturb_scaled = perturb * cell.coverage_strength;
    let effective_base = cell.base_altitude_m + perturb_scaled * amp.y;
    let effective_top = cell.top_altitude_m + perturb_scaled * amp.x;
    let alt_factor = altitude_profile(
        cell.cell_type, world_pos.y,
        effective_base, effective_top,
    );
    if (alt_factor <= 0.0) {
        return 0.0;
    }
    let w = worley_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_WORLEY_OCTAVE,
        world_pos.x, world_pos.y, world_pos.z,
        CLOUD_WORLEY_SPACING_M, CLOUD_FEATURES_PER_CELL,
    );
    let worley_struct = clamp(1.0 - w.f_blended, 0.0, 1.0);
    let perlin_raw = cloud_perlin_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_PERLIN_OCTAVE,
        world_pos.x, world_pos.y, world_pos.z,
        CLOUD_PERLIN_SPACING_M,
    );
    let perlin_norm = clamp((perlin_raw + 0.7) / 1.4, 0.0, 1.0);
    let weights = perlin_worley_weights_for_type(cell.cell_type);
    let raw_factor = weights.x * worley_struct + weights.y * perlin_norm;
    // Continuous-margin modulation — LINEAR REMAP of [coverage_threshold,
    // 1.0] to [0, 1] so noise above the threshold passes through with
    // preserved gradient. The earlier smoothstep formulation
    // (smoothstep(threshold, threshold+0.10, raw_factor)) saturated to
    // ~1.0 for any raw_factor > threshold + 0.10, eliminating the soft
    // density falloff at cloud edges that the banded-alpha cel-shading
    // relies on. Linear remap preserves identity at cov=1.0 (no
    // modulation; full Beer-Lambert + cel-shaded rim) and still gates
    // the bottom of the noise field for marginal cells (gaps in
    // coverage where noise < threshold). Per
    // `docs/WEATHER.md § Continuous qualification margin`.
    //
    // - cov=1.0: cloud_factor = raw_factor (identity).
    // - cov=0.5: cloud_factor = max(0, (raw_factor - 0.5) * 2.0) — gates
    //   bottom half, preserves linear ramp above.
    // - cov=0.1: cloud_factor = max(0, (raw_factor - 0.9) * 10.0) — only
    //   top 10% of noise survives, but with soft falloff.
    // - cov=0: skipped by early-out above; division by zero never
    //   reached.
    let coverage_threshold = 1.0 - cell.coverage_strength;
    let cloud_factor = clamp(
        (raw_factor - coverage_threshold) / cell.coverage_strength,
        0.0,
        1.0,
    );
    return cell.density * alt_factor * cloud_factor;
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

    var accumulated_alpha: f32 = 0.0;
    var accumulated_color: vec3<f32> = vec3<f32>(0.0);

    for (var i: i32 = 0; i < actual_steps; i = i + 1) {
        let t = t_enter + step_dist * (f32(i) + jitter);
        let sample_pos = cam_pos + view_dir * t;
        let density = sample_cloud_density(sample_pos);
        if (density <= 0.0) {
            continue;
        }
        let absorption = 1.0 - exp(-density * step_dist * ABSORPTION_SCALE);

        // Altitude-gradient lighting stand-in (WEATHER.md § Lighting
        // model — proper shadow march reserved for E1 polish). Uses
        // the global band rather than per-cell base/top so all cells
        // share a consistent gradient — Stratus low + Cumulus high
        // both read with the same overhead-sun behavior.
        let altitude_t = clamp(
            (sample_pos.y - GLOBAL_CLOUD_BASE_M)
                / (GLOBAL_CLOUD_TOP_M - GLOBAL_CLOUD_BASE_M),
            0.0,
            1.0,
        );
        let sun_height = clamp(scene.sun_dir.y, 0.0, 1.0);
        let lit_factor = mix(0.4, altitude_t, sun_height);

        let sun_lit_color = scene.sun_color.rgb * 1.4;
        let shadow_color = scene.ambient.rgb * 0.85;
        let sample_color = mix(shadow_color, sun_lit_color, lit_factor);

        accumulated_color = accumulated_color
            + sample_color * absorption * (1.0 - accumulated_alpha);
        accumulated_alpha = accumulated_alpha
            + absorption * (1.0 - accumulated_alpha);

        if (accumulated_alpha > 0.99) {
            break;
        }
    }

    // Banded-alpha cel-shading (LOCKED in WEATHER.md). 6 bands; hard
    // edge at low alpha keeps silhouettes crisp.
    let banded_alpha = floor(accumulated_alpha * 6.0) / 6.0;
    let final_alpha = select(banded_alpha, 0.0, banded_alpha < (1.0 / 6.0));

    return vec4<f32>(accumulated_color, final_alpha);
}
