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
// (32 bytes, 8 × f32 / u32). Unused slots have density = 0 so the
// per-cell early-out skips them.
struct GpuWeatherCell {
    seed_pos: vec2<f32>,        // world XZ of the cell's seed position
    base_altitude_m: f32,       // cloud base altitude
    top_altitude_m: f32,        // cloud top altitude
    density: f32,               // peak density [0, 1]; 0 = unused / clear
    storm_phase: f32,           // life-cycle phase [0, 1)
    cell_type: u32,             // CELL_* discriminant
    _pad: u32,
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

// Per-cell footprint radius (m). C2 ships a single global footprint;
// future polish can vary per cell type (storms wider than fair-weather
// cumulus, stratus = full Voronoi cell). 6 km gives meaningful visible
// area at typical viewing distances (5-30 km) without overlapping
// neighboring cells (W2 spacing ~25 km).
const CELL_FOOTPRINT_M: f32 = 6000.0;

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

// Footprint falloff — smoothstep from full at center to 0 at footprint
// edge. B3 single-cell approximation; C1+C2 use Voronoi-cell membership
// (no falloff needed because the partition tessellates the plane).
fn footprint_falloff(world_xz: vec2<f32>, seed_xz: vec2<f32>, radius_m: f32) -> f32 {
    let d = length(world_xz - seed_xz);
    let t = clamp(1.0 - d / radius_m, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Combined cloud density at a world-space sample point for a specific
// W2 cell. Composes:
//   cell.density × altitude profile × footprint falloff × Perlin-Worley blend
//
// The Perlin-Worley blend per cell type (WEATHER.md § Sub-cell detail
// noise). For Cumulus / Nimbus / Squall: 0.7 Worley + 0.3 Perlin.
// Stratus / Frontal use a different blend ratio (deferred — currently
// all cell types use the same blend; future polish per-type weights).
fn sample_cloud_density(world_pos: vec3<f32>, cell: GpuWeatherCell) -> f32 {
    let alt_factor = altitude_profile(
        cell.cell_type, world_pos.y,
        cell.base_altitude_m, cell.top_altitude_m,
    );
    if (alt_factor <= 0.0) {
        return 0.0;
    }
    let foot_factor = footprint_falloff(
        world_pos.xz, cell.seed_pos, CELL_FOOTPRINT_M,
    );
    if (foot_factor <= 0.0) {
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
    let cloud_factor =
        CUMULUS_WORLEY_WEIGHT * worley_struct +
        CUMULUS_PERLIN_WEIGHT * perlin_norm;
    return cell.density * alt_factor * foot_factor * cloud_factor;
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
// Per WEATHER.md the High-preset target is 32 near. The adaptive
// sampler picks the number of steps based on actual band length so
// short bands (vertical view through 1 km slab) use ~4-6 steps and
// long bands (grazing view through 5+ km) approach this cap.
const RAYMARCH_STEPS_MAX: i32 = 32;

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

    // Multi-cell raymarch (C2). Iterate the active W2 cells; per cell,
    // compute ray-cylinder ∩ ray-altitude-band intersection, march
    // through that, accumulate alpha + color via standard pre-multiplied
    // alpha-over composition. Cells that don't intersect the ray skip
    // immediately at the band/cylinder check (no Worley calls).
    //
    // Iteration order: storage-buffer order. Pre-multiplied alpha-over
    // is associative + commutative for non-overlapping density, so
    // unsorted iteration is correct for the typical "scattered cumulus"
    // pattern. Future polish (sorted iteration for storm overlap or
    // dense Stratus decks) can z-sort the active set on the CPU side
    // before upload.
    var accumulated_alpha: f32 = 0.0;
    var accumulated_color: vec3<f32> = vec3<f32>(0.0);

    let cell_count = arrayLength(&weather_cells);
    for (var cell_idx: u32 = 0u; cell_idx < cell_count; cell_idx = cell_idx + 1u) {
        let cell = weather_cells[cell_idx];
        // Empty / clear-air slot: density 0 → skip (no Worley calls).
        if (cell.density <= 0.0 || cell.cell_type == CELL_CLEAR_AIR) {
            continue;
        }
        let band = ray_altitude_band(
            cam_pos, view_dir,
            cell.base_altitude_m, cell.top_altitude_m,
            MAX_RAY_DISTANCE_M,
        );
        if (band.x < 0.0) {
            continue;
        }
        let cyl = ray_vertical_cylinder(
            cam_pos, view_dir, cell.seed_pos, CELL_FOOTPRINT_M, MAX_RAY_DISTANCE_M,
        );
        if (cyl.x < 0.0) {
            continue;
        }
        let t_enter = max(band.x, cyl.x);
        let t_exit  = min(min(band.y, cyl.y), t_terrain);
        if (t_enter >= t_exit) {
            continue;
        }
        // Adaptive step count: short bands → fewer steps; long bands
        // capped at RAYMARCH_STEPS_MAX. Per-pixel Worley cost scales
        // with actual step count.
        let band_length = t_exit - t_enter;
        let want_steps = ceil(band_length / RAYMARCH_TARGET_STEP_M);
        let actual_steps = i32(min(f32(RAYMARCH_STEPS_MAX), max(want_steps, 4.0)));
        let step_dist = band_length / f32(actual_steps);

        for (var i: i32 = 0; i < actual_steps; i = i + 1) {
            let t = t_enter + step_dist * (f32(i) + 0.5);
            let sample_pos = cam_pos + view_dir * t;
            let density = sample_cloud_density(sample_pos, cell);

            let absorption = 1.0 - exp(-density * step_dist * ABSORPTION_SCALE);

            // Altitude-gradient lighting stand-in (WEATHER.md § Lighting
            // model — proper shadow march reserved for E1 polish).
            let altitude_t = clamp(
                (sample_pos.y - cell.base_altitude_m)
                    / max(cell.top_altitude_m - cell.base_altitude_m, 1.0),
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

        // Stop iterating cells once fully opaque — no benefit to more.
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
