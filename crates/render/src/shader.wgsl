// flowPlan Phase 2 cel-shaded forward pipeline (Milestone 2.1, sub-step 1).
//
// Vertex layout matches `flowplan_assets::Vertex`:
//   @location(0) position : vec3<f32>
//   @location(1) normal   : vec3<f32>
//   @location(2) uv       : vec2<f32>
//   @location(3) color    : vec4<f32>
//
// Camera uniform at @group(0) @binding(0).
// Per-draw model uniform at @group(1) @binding(0).
//
// Fragment shading: banded diffuse (cel) with per-material band count + ambient floor,
// plus a rim-light term. Screen-space outline and hot-reload land in later sub-steps of
// ROADMAP § 2.1.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos:  vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct ModelUniform {
    model:       mat4x4<f32>,
    normal_mat:  mat4x4<f32>,
    tint:        vec4<f32>,
    // x = band count, y = ambient floor, z = rim strength, w = rim exponent.
    shading:     vec4<f32>,
};
@group(1) @binding(0) var<uniform> model: ModelUniform;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) uv:       vec2<f32>,
    @location(3) color:    vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos:    vec4<f32>,
    @location(0)       world_pos:   vec3<f32>,
    @location(1)       world_normal:vec3<f32>,
    @location(2)       vert_color:  vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let world = model.model * vec4<f32>(in.position, 1.0);
    let n = normalize((model.normal_mat * vec4<f32>(in.normal, 0.0)).xyz);
    var out: VsOut;
    out.clip_pos = camera.view_proj * world;
    out.world_pos = world.xyz;
    out.world_normal = n;
    out.vert_color = in.color * model.tint;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Hard-coded scene light until Milestone 2.6 introduces a sun / sky uniform.
    let light_dir = normalize(vec3<f32>(0.45, -0.9, 0.3));
    let n = normalize(in.world_normal);

    let lambert = max(dot(n, -light_dir), 0.0);
    let bands = max(model.shading.x, 1.0);
    let ambient = clamp(model.shading.y, 0.0, 1.0);
    // Banded diffuse: quantize lambert into `bands` steps, then remap [0, 1] into
    // [ambient, 1] so shadowed facets read as shaded, not black.
    let banded = floor(lambert * bands) / bands;
    let diffuse = mix(ambient, 1.0, banded);

    // Rim light: brightens grazing-angle fragments. `pow` on `1 - n.v` sharpens the band.
    let view_dir = normalize(camera.view_pos.xyz - in.world_pos);
    let rim_edge = 1.0 - max(dot(view_dir, n), 0.0);
    let rim = pow(rim_edge, max(model.shading.w, 0.01)) * model.shading.z;

    let shaded = in.vert_color.rgb * diffuse + rim;
    return vec4<f32>(shaded, in.vert_color.a);
}

