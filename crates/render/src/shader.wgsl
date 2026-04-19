// flowPlan Phase 1 flat-shaded pipeline.
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
// Fragment shading: one directional light for a depth cue, then modulate vertex color.
// Phase 2 replaces this with banded cel lighting + outline pass.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos:  vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct ModelUniform {
    model:       mat4x4<f32>,
    normal_mat:  mat4x4<f32>,
    tint:        vec4<f32>,
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
    let light_dir = normalize(vec3<f32>(0.45, -0.9, 0.3));
    let lambert = max(dot(in.world_normal, -light_dir), 0.0);
    let shade = 0.35 + 0.65 * lambert;
    return vec4<f32>(in.vert_color.rgb * shade, in.vert_color.a);
}
