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

struct SceneUniform {
    // xyz = unit vector TOWARD the sun (i.e., "up" when the sun is overhead). w reserved.
    sun_dir:   vec4<f32>,
    // rgb = sun-light color multiplied into the diffuse term. a reserved.
    sun_color: vec4<f32>,
    // rgb = ambient fill (reserved for Milestone 2.6 sub-step 3). a reserved.
    ambient:   vec4<f32>,
    // rgb = fog color, a = fog density per meter (both reserved for sub-step 3).
    fog:       vec4<f32>,
};
@group(2) @binding(0) var<uniform> scene: SceneUniform;

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
    let sun_dir = normalize(scene.sun_dir.xyz);
    let n = normalize(in.world_normal);

    let lambert = max(dot(n, sun_dir), 0.0);
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

    let lit = in.vert_color.rgb * diffuse * scene.sun_color.rgb + rim;

    // Exponential distance fog with sun-bias. Fog density is per-meter; the sun-aligned
    // view scatters forward light, so fragments in the sun quadrant of the view tint
    // warmer (mixed toward sun_color). Milestone 2.6 sub-step 3; full height-scaled fog
    // lands when altitude gameplay requires it.
    let dist = length(camera.view_pos.xyz - in.world_pos);
    let fog_density = max(scene.fog.a, 0.0);
    let fog_factor = clamp(1.0 - exp(-fog_density * dist), 0.0, 1.0);
    let view_to_pixel = normalize(in.world_pos - camera.view_pos.xyz);
    let sun_align = clamp(dot(view_to_pixel, -sun_dir), 0.0, 1.0);
    let fog_col = mix(scene.fog.rgb, scene.sun_color.rgb, pow(sun_align, 6.0) * 0.5);
    let final_color = mix(lit, fog_col, fog_factor);
    return vec4<f32>(final_color, in.vert_color.a);
}

