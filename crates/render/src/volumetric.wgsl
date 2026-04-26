// Volumetric cloud raymarcher (Phase 3.6.3c.6 / 3.2x B2 — skeleton).
//
// Per `docs/WEATHER.md` § Cloud rendering technique:
//   - Full-screen triangle pass running AFTER scene + outline, BEFORE
//     widgets. Reads scene depth (added in B3) to terminate rays at
//     terrain. Composites alpha-blended over the rendered scene.
//   - Returns transparent in the B2 skeleton — empty raymarch loop
//     produces zero density everywhere. Validates pipeline + bind
//     groups + pass ordering without changing visual output.
//   - B3 adds W2 weather-cell sampling + Perlin-Worley sub-cell detail
//     per ray-step. B4 adds CPU-GPU bit-identity gate vs Rust
//     `cloud_density_at`.
//
// Bind groups:
//   @group(0) = camera (inverse_view_proj for ray reconstruction)
//   @group(1) = scene  (sun direction, ambient, fog, time)
//   @group(2) = depth  (scene-pass depth texture, ray termination — ADDED IN B3)

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

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       ndc:      vec2<f32>,
};

@vertex
fn vs_volumetric(@builtin(vertex_index) idx: u32) -> VsOut {
    // Oversized full-screen triangle at the far plane (clip_z = 1.0).
    // Same trick as sky.wgsl — three vertices cover the full viewport.
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

// Placeholder density sampler. B3 replaces this with W2 weather-cell
// lookup + Perlin-Worley sub-cell detail.
fn sample_density_at(world_pos: vec3<f32>, t: f32) -> f32 {
    // B2 stub: zero density everywhere → fully transparent cloud pass.
    return 0.0;
}

// Raymarch step count — locked in WEATHER.md as 32 near / 24 mid /
// 16 far at the High preset. B3 implements the distance-tier logic;
// B2 uses uniform 32 steps (placeholder).
const RAYMARCH_STEPS_NEAR: i32 = 32;

// Maximum ray distance (m). Beyond this, atmospheric perspective takes
// over and clouds aren't sampled. WEATHER.md performance budget caps
// at ~50 km streaming radius for cloud cells.
const MAX_RAY_DISTANCE_M: f32 = 50000.0;

@fragment
fn fs_volumetric(in: VsOut) -> @location(0) vec4<f32> {
    // Reconstruct world-space view ray from screen NDC.
    let world_near = unproject(in.ndc, 0.0);
    let world_far  = unproject(in.ndc, 1.0);
    let view_dir = normalize(world_far - world_near);
    let cam_pos = camera.view_pos.xyz;

    // Raymarch parameters. Step distance derived from total ray length
    // / step count; near steps are dense, far steps stretch out.
    let step_dist = MAX_RAY_DISTANCE_M / f32(RAYMARCH_STEPS_NEAR);

    // Accumulate density along the ray. B2: stays zero (empty stub).
    var accumulated_alpha: f32 = 0.0;
    var accumulated_color: vec3<f32> = vec3<f32>(0.0);

    for (var i: i32 = 0; i < RAYMARCH_STEPS_NEAR; i = i + 1) {
        let t = step_dist * (f32(i) + 0.5);
        let sample_pos = cam_pos + view_dir * t;
        let density = sample_density_at(sample_pos, scene.time.x);

        // Beer-Lambert single-scatter accumulation. With density = 0
        // throughout, alpha stays 0 and color stays black — net effect
        // is transparent, the B2 skeleton's intended output.
        let absorption = 1.0 - exp(-density * step_dist * 0.001);
        let sample_color = scene.sun_color.rgb;
        accumulated_color = accumulated_color
            + sample_color * absorption * (1.0 - accumulated_alpha);
        accumulated_alpha = accumulated_alpha
            + absorption * (1.0 - accumulated_alpha);

        // Early-out when fully opaque. B3+ tunes the threshold; B2's
        // density is always 0, so this never fires.
        if (accumulated_alpha > 0.99) {
            break;
        }
    }

    // Banded-alpha cel-shading quantization (LOCKED in WEATHER.md).
    // 6 bands; hard-edge thresholding zeros out alpha below the lowest
    // band so silhouettes stay crisp. B2 output is alpha = 0 throughout
    // so quantization is a no-op here.
    let banded_alpha = floor(accumulated_alpha * 6.0) / 6.0;
    let final_alpha = select(banded_alpha, 0.0, banded_alpha < (1.0 / 6.0));

    return vec4<f32>(accumulated_color, final_alpha);
}
