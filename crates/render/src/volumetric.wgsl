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
// 600 m gives ~5 sub-cells per 3 km cell footprint = puffy-but-coherent.
const CLOUD_WORLEY_SPACING_M: f32 = 600.0;
const CLOUD_NOISE_OCTAVE: u32 = 240u;
const CLOUD_FEATURES_PER_CELL: u32 = 2u;

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
    // center = denser cloud puff. Invert + scale to get cumulus
    // structure: dense-near-feature, sparse-at-cell-corners.
    let w = worley_3d(
        CLOUD_SEED_HI, CLOUD_SEED_LO,
        CLOUD_NOISE_OCTAVE,
        world_pos.x, world_pos.y, world_pos.z,
        CLOUD_WORLEY_SPACING_M, CLOUD_FEATURES_PER_CELL,
    );
    let worley_density = clamp(1.0 - w.f_blended * 1.5, 0.0, 1.0);
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

// Raymarch step count — locked in WEATHER.md as 32 near / 24 mid /
// 16 far at the High preset. B3 uses uniform 32; B4+ adds the
// distance-tier logic.
const RAYMARCH_STEPS: i32 = 32;

// Maximum ray distance (m). B3b adds depth-based termination at terrain;
// B3 uses a fixed cap.
const MAX_RAY_DISTANCE_M: f32 = 50000.0;

// Density-to-absorption scale. Tuned so density = 0.6 produces
// visible-but-not-opaque cloud at the typical step distance.
const ABSORPTION_SCALE: f32 = 0.0015;

@fragment
fn fs_volumetric(in: VsOut) -> @location(0) vec4<f32> {
    let world_near = unproject(in.ndc, 0.0);
    let world_far  = unproject(in.ndc, 1.0);
    let view_dir = normalize(world_far - world_near);
    let cam_pos = camera.view_pos.xyz;

    let step_dist = MAX_RAY_DISTANCE_M / f32(RAYMARCH_STEPS);

    var accumulated_alpha: f32 = 0.0;
    var accumulated_color: vec3<f32> = vec3<f32>(0.0);

    for (var i: i32 = 0; i < RAYMARCH_STEPS; i = i + 1) {
        let t = step_dist * (f32(i) + 0.5);
        let sample_pos = cam_pos + view_dir * t;
        let density = sample_cloud_density(sample_pos, scene.time.x);

        // Beer-Lambert single-scatter accumulation. B3c will add a
        // shadow-march toward sun; B3 uses uniform sun_color shading.
        let absorption = 1.0 - exp(-density * step_dist * ABSORPTION_SCALE);
        // Shading: sun-lit base color tinted with ambient. Cloud color
        // brightens with sun direction; shadow side picks up ambient.
        // B3c will replace this with secondary-march transmittance.
        let sample_color = mix(
            scene.ambient.rgb * 1.5,    // shadow tint
            scene.sun_color.rgb * 1.2,  // sun-lit tint
            0.7,                         // bias toward sun-lit; tune in E1
        );
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
