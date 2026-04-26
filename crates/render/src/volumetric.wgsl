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

// ──────────────────────────────────────────────────────────────────
// B3 cloud-density model
// ──────────────────────────────────────────────────────────────────

// Cell types — match Rust `weather::cell_state::CellType` discriminant.
const CELL_CLEAR_AIR: u32 = 0u;
const CELL_CUMULUS: u32   = 1u;
const CELL_NIMBUS: u32    = 2u;
const CELL_STRATUS: u32   = 3u;
const CELL_SQUALL: u32    = 4u;
const CELL_FRONTAL: u32   = 5u;

// Hardcoded test cell for B3 — single cumulus at world origin XZ.
// Pinned values match what the user can fly over from canonical spawn.
// C2 replaces this with a per-cell storage buffer populated by
// ChunkedWeather streaming.
const TEST_CELL_TYPE: u32 = CELL_CUMULUS;
const TEST_CELL_SEED_X: f32 = 0.0;
const TEST_CELL_SEED_Z: f32 = 0.0;
const TEST_CELL_BASE_M: f32 = 1500.0;
const TEST_CELL_TOP_M: f32 = 2500.0;
const TEST_CELL_DENSITY: f32 = 0.6;
// Footprint radius (m). Within this XZ distance from seed_pos, the cell
// is "in" — outside, density falls to 0. Real W2 cells are Voronoi
// regions; this is a circular approximation for the single-cell B3 demo.
const TEST_CELL_FOOTPRINT_M: f32 = 3000.0;

// Worley noise spacing for cumulus sub-cell detail (m).
// 2000 m at 3 km footprint gives ~1-2 sub-puffs per cumulus — reads as
// "cumulus with single internal puff" rather than cellular patchwork.
// Allows the F1 distance to span more of the ray, so density is more
// stable as camera moves. Tunable per cell type once W2 streaming lands.
const CLOUD_WORLEY_SPACING_M: f32 = 2000.0;
const CLOUD_NOISE_OCTAVE: u32 = 240u;
// `features_per_cell = 1` halves the per-Worley-call distance-compute
// cost vs k=2 (27 vs 54 distance computes in 3D). Single-feature pure
// F1 distance is sufficient for the visual character we want once the
// density-modulation curve is calibrated; the F1+F2 blend is reserved
// for higher-quality presets (Ultra) when we expose them.
const CLOUD_FEATURES_PER_CELL: u32 = 1u;

// Master seed for the cloud-noise field. Matches WORLD_SEED in
// `crates/app/src/main.rs` (decimal 1234567890123456789). Hardcoded
// for B3; B4+ will plumb through camera/scene uniform. Split as
// (high32, low32) to match WGSL's u32-only PCG hash.
const CLOUD_SEED_HI: u32 = 0x112210F4u;
const CLOUD_SEED_LO: u32 = 0x7DE98115u;

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

// Combined cloud density at a world-space sample point. Composes:
//   peak density × altitude profile × footprint falloff × Worley sub-cell
fn sample_cloud_density(world_pos: vec3<f32>, t: f32) -> f32 {
    let alt_factor = altitude_profile(
        TEST_CELL_TYPE, world_pos.y, TEST_CELL_BASE_M, TEST_CELL_TOP_M,
    );
    if (alt_factor <= 0.0) {
        return 0.0;
    }
    let foot_factor = footprint_falloff(
        world_pos.xz, vec2<f32>(TEST_CELL_SEED_X, TEST_CELL_SEED_Z), TEST_CELL_FOOTPRINT_M,
    );
    if (foot_factor <= 0.0) {
        return 0.0;
    }
    // Worley f_blended ∈ [0, ~1.0]. Lower values = closer to feature
    // center = denser cloud puff. We use a SUBTLE Worley contribution
    // rather than gating: a base density that's always present within
    // the cell's footprint+altitude range, modulated by Worley to add
    // sub-cell structure without producing the "cluster of micro-puffs"
    // artifact pure Worley produces.
    //
    // Formula: base 0.6 + Worley modulation in [0.0, 0.4].
    // - At feature center (f_blended ≈ 0): density factor = 1.0
    // - At cell corner (f_blended ≈ 0.7): density factor = 0.6
    // The cloud reads as a coherent puff with internal density
    // variation, not separate puffs glued together. Perlin-Worley
    // blend (B3c) will further smooth this; pure Worley with subtle
    // contribution is the v0 stability fix.
    let w = worley_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_NOISE_OCTAVE,
        world_pos.x, world_pos.y, world_pos.z,
        CLOUD_WORLEY_SPACING_M, CLOUD_FEATURES_PER_CELL,
    );
    let worley_density = clamp(1.0 - w.f_blended * 0.6, 0.6, 1.0);
    return TEST_CELL_DENSITY * alt_factor * foot_factor * worley_density;
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

// Density-to-absorption scale. Tuned so density = 0.6 produces
// visible-but-not-opaque cloud at the typical step distance.
const ABSORPTION_SCALE: f32 = 0.0015;

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
    // geometry (terrain, water, vehicle, instances). depth_ndc ∈ [0, 1];
    // 1.0 = sky / cleared (no scene hit, no termination needed).
    // Anything < 1.0 = solid pixel.
    //
    // Camera-relative rendering caveat: per Phase 3.6.3c.5d the
    // inverse_view_proj used here is `(proj * rotation_only_view).inverse()`
    // — it does NOT include the camera translation. So `unproject(ndc,
    // depth)` returns the position **relative to the camera**, not
    // absolute world. Distance from camera = `length(unprojected)` —
    // do NOT subtract cam_pos again. (Subtracting cam_pos was a bug
    // that made the aircraft appear to not write depth: t_terrain came
    // out wildly wrong, so the clamp didn't bite.)
    let frag_pixel = vec2<i32>(in.clip_pos.xy);
    let depth_ndc = textureLoad(scene_depth, frag_pixel, 0);
    var t_terrain = MAX_RAY_DISTANCE_M;
    if (depth_ndc < 1.0) {
        let depth_pos_cam_rel = unproject(in.ndc, depth_ndc);
        t_terrain = length(depth_pos_cam_rel);
    }

    // Two early-outs to skip Worley evaluation for the vast majority of
    // pixels (sky above clouds, terrain below, side-views missing the
    // cell entirely). The cell's volume is the intersection of:
    //   - altitude band [base, top]
    //   - vertical cylinder centered on (seed_x, seed_z) with radius footprint_m
    //   - ray distance ≤ scene-depth distance (B3b)
    let band = ray_altitude_band(
        cam_pos, view_dir, TEST_CELL_BASE_M, TEST_CELL_TOP_M, MAX_RAY_DISTANCE_M,
    );
    if (band.x < 0.0) {
        return vec4<f32>(0.0);
    }
    let cyl = ray_vertical_cylinder(
        cam_pos, view_dir,
        vec2<f32>(TEST_CELL_SEED_X, TEST_CELL_SEED_Z),
        TEST_CELL_FOOTPRINT_M,
        MAX_RAY_DISTANCE_M,
    );
    if (cyl.x < 0.0) {
        return vec4<f32>(0.0);
    }
    let t_enter = max(band.x, cyl.x);
    let t_exit  = min(min(band.y, cyl.y), t_terrain);
    if (t_enter >= t_exit) {
        return vec4<f32>(0.0);
    }
    // Adaptive step count: shorter bands (vertical view through cloud
    // slab) get fewer steps, longer bands (grazing view) up to the
    // RAYMARCH_STEPS_MAX cap. Per-pixel Worley cost scales with actual
    // step count — far cheaper than fixed 32 steps regardless of
    // intersection length.
    let band_length = t_exit - t_enter;
    let want_steps = ceil(band_length / RAYMARCH_TARGET_STEP_M);
    let actual_steps = i32(min(f32(RAYMARCH_STEPS_MAX), max(want_steps, 4.0)));
    let step_dist = band_length / f32(actual_steps);

    var accumulated_alpha: f32 = 0.0;
    var accumulated_color: vec3<f32> = vec3<f32>(0.0);

    for (var i: i32 = 0; i < actual_steps; i = i + 1) {
        let t = t_enter + step_dist * (f32(i) + 0.5);
        let sample_pos = cam_pos + view_dir * t;
        let density = sample_cloud_density(sample_pos, scene.time.x);

        // Beer-Lambert single-scatter accumulation along the view ray.
        let absorption = 1.0 - exp(-density * step_dist * ABSORPTION_SCALE);

        // Cheap directional shading via altitude gradient. Top of cloud
        // band reads as sun-lit; bottom reads as shadowed. When sun is
        // overhead (sun_dir.y ≈ 1) the gradient is strongest; at low
        // sun the shading flattens.
        //
        // **Stand-in for real lighting**: the locked design (WEATHER.md
        // § Cloud rendering technique → Lighting model) calls for a
        // Beer-Lambert shadow-march from the sample toward the sun,
        // accumulating density along that secondary ray. This costs an
        // extra 6 density samples per primary step (~50% per-pixel
        // overhead). Deferred to E1 polish where the shading model
        // gets tuned alongside the Perlin-Worley blend (B3c). The
        // altitude gradient below is the cheap interim — gives the
        // cloud directional character without the shadow-march cost.
        let altitude_t = clamp(
            (sample_pos.y - TEST_CELL_BASE_M) / (TEST_CELL_TOP_M - TEST_CELL_BASE_M),
            0.0,
            1.0,
        );
        let sun_height = clamp(scene.sun_dir.y, 0.0, 1.0);
        let lit_factor = mix(0.4, altitude_t, sun_height);

        // Sun-lit warm + ambient-cool shadow. Brighter / more saturated
        // sun_lit boosts contrast against terrain backgrounds.
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
