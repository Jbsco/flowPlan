// flowPlan Phase 2 procedural sky (Milestone 2.6 sub-step 2).
//
// Full-screen triangle pass rendered BEFORE the scene pass. Computes a cel-shade-friendly
// analytic sky: a zenith-to-horizon gradient, a warm band along the sun bearing, and a
// sun disc + halo. No depth read/write. The sky bind groups are:
//   @group(0) = camera (needs `inverse_view_proj` to reconstruct per-pixel view rays)
//   @group(1) = scene  (sun direction and color)
//
// A full Preetham / Hosek-Wilkie model is deferred; this keeps Phase 2 cheap and
// recognizable. Atmospheric complexity grows in Phase 5 alongside the celestial sphere.

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
};
@group(1) @binding(0) var<uniform> scene: SceneUniform;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       ndc:      vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) idx: u32) -> VsOut {
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

@fragment
fn fs_sky(in: VsOut) -> @location(0) vec4<f32> {
    // Reconstruct world-space view direction from screen NDC.
    let world_near = unproject(in.ndc, 0.0);
    let world_far  = unproject(in.ndc, 1.0);
    let view_dir = normalize(world_far - world_near);

    let sun_dir = normalize(scene.sun_dir.xyz);

    // Zenith-to-horizon gradient. view_dir.y = +1 at zenith, 0 at horizon, negative below.
    let up = clamp(view_dir.y, 0.0, 1.0);
    let zenith = vec3<f32>(0.28, 0.48, 0.82);
    let horizon = vec3<f32>(0.78, 0.88, 0.96);
    let sky_grad = mix(horizon, zenith, pow(up, 0.55));

    // Warm tint along the sun bearing, concentrated near the horizon.
    let sun_horiz = normalize(vec3<f32>(sun_dir.x, 0.0, sun_dir.z));
    let view_horiz_raw = vec3<f32>(view_dir.x, 0.0, view_dir.z);
    let view_horiz = select(
        vec3<f32>(1.0, 0.0, 0.0),
        normalize(view_horiz_raw),
        length(view_horiz_raw) > 1e-4,
    );
    let horizon_align = clamp(dot(view_horiz, sun_horiz), 0.0, 1.0);
    let warmth = vec3<f32>(1.00, 0.78, 0.55);
    let warm_band = pow(1.0 - up, 4.0) * pow(horizon_align, 3.0);
    let sky_col = mix(sky_grad, warmth, warm_band * 0.5);

    // Sun disc + halo. `sun_align` is 1 when looking straight at the sun.
    let sun_align = clamp(dot(view_dir, sun_dir), 0.0, 1.0);
    let sun_disc = smoothstep(0.9995, 0.99995, sun_align);
    let sun_halo = pow(sun_align, 128.0) * 0.6 + pow(sun_align, 16.0) * 0.15;
    let sun_contrib = scene.sun_color.rgb * (sun_disc * 2.0 + sun_halo);

    // Below-horizon fragments darken slightly -- simulates the ground-reflected lower hemisphere.
    let below = clamp(-view_dir.y, 0.0, 1.0);
    let below_col = mix(sky_col, sky_col * 0.35, below);

    return vec4<f32>(below_col + sun_contrib, 1.0);
}
