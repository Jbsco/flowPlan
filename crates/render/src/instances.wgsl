// flowPlan feature-instance pipeline (Phase 3.6.3c.5c.1).
//
// Reads per-instance (position, rotation_y, scale, color) from a storage
// buffer indexed by `@builtin(instance_index)`. The primitive mesh is a
// unit-size box uploaded once and reused for every instance. Active
// instance count is set by the draw call, not by the buffer length —
// game_app streams just the cells whose Voronoi footprint is in the
// active set.
//
// Bind groups match the main scene pipeline where layouts overlap:
//   @group(0) = camera (identical to shader.wgsl)
//   @group(1) = instance storage buffer
//   @group(2) = scene (identical to shader.wgsl)

struct CameraUniform {
    view_proj:         mat4x4<f32>,
    view_pos:          vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct InstanceData {
    // xyz = world position; w = rotation around Y axis (radians).
    pos_rot: vec4<f32>,
    // xyz = per-axis scale; w = reserved.
    scale:   vec4<f32>,
    // rgba 0-1 (palette-adapted, baked at world-gen time).
    color:   vec4<f32>,
};
@group(1) @binding(0) var<storage, read> instances : array<InstanceData>;

struct SceneUniform {
    sun_dir:   vec4<f32>,
    sun_color: vec4<f32>,
    ambient:   vec4<f32>,
    fog:       vec4<f32>,
    time:      vec4<f32>,
};
@group(2) @binding(0) var<uniform> scene: SceneUniform;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) uv:       vec2<f32>,
    @location(3) color:    vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos:     vec4<f32>,
    @location(0)       world_pos:    vec3<f32>,
    @location(1)       world_normal: vec3<f32>,
    @location(2)       inst_color:   vec4<f32>,
};

@vertex
fn vs_instance(in: VsIn, @builtin(instance_index) instance_id: u32) -> VsOut {
    let inst = instances[instance_id];
    let cos_y = cos(inst.pos_rot.w);
    let sin_y = sin(inst.pos_rot.w);

    // Scale then rotate (Y only) then translate.
    let scaled = vec3<f32>(
        in.position.x * inst.scale.x,
        in.position.y * inst.scale.y,
        in.position.z * inst.scale.z,
    );
    let rotated = vec3<f32>(
        scaled.x * cos_y + scaled.z * sin_y,
        scaled.y,
        -scaled.x * sin_y + scaled.z * cos_y,
    );
    let world = rotated + inst.pos_rot.xyz;

    // Non-uniform scale → normals need inverse-scale. Still uniform in
    // Y axis because rotation is Y-only; skew-free.
    let n_unscaled = vec3<f32>(
        in.normal.x / max(inst.scale.x, 1e-4),
        in.normal.y / max(inst.scale.y, 1e-4),
        in.normal.z / max(inst.scale.z, 1e-4),
    );
    let n_rotated = normalize(vec3<f32>(
        n_unscaled.x * cos_y + n_unscaled.z * sin_y,
        n_unscaled.y,
        -n_unscaled.x * sin_y + n_unscaled.z * cos_y,
    ));

    var out: VsOut;
    out.clip_pos = camera.view_proj * vec4<f32>(world, 1.0);
    out.world_pos = world;
    out.world_normal = n_rotated;
    out.inst_color = inst.color;
    return out;
}

@fragment
fn fs_instance(in: VsOut) -> @location(0) vec4<f32> {
    let sun_dir = normalize(scene.sun_dir.xyz);
    let n = normalize(in.world_normal);

    let lambert = max(dot(n, sun_dir), 0.0);
    // Cel-shading defaults mirror the terrain band count + ambient
    // floor from shader.wgsl. Instance shader has no per-draw shading
    // uniform — hardcoded to 5c.1 MVP (3 bands, ambient 0.2).
    let bands = 3.0;
    let ambient = 0.2;
    let banded = floor(lambert * bands) / bands;
    let diffuse = mix(ambient, 1.0, banded);
    let lit = in.inst_color.rgb * diffuse * scene.sun_color.rgb;

    // Exponential distance fog (matches shader.wgsl).
    let dist = length(camera.view_pos.xyz - in.world_pos);
    let fog_density = max(scene.fog.a, 0.0);
    let fog_factor = clamp(1.0 - exp(-fog_density * dist), 0.0, 1.0);
    let view_to_pixel = normalize(in.world_pos - camera.view_pos.xyz);
    let sun_align = clamp(dot(view_to_pixel, -sun_dir), 0.0, 1.0);
    let fog_col = mix(scene.fog.rgb, scene.sun_color.rgb, pow(sun_align, 6.0) * 0.5);
    let final_color = mix(lit, fog_col, fog_factor);
    return vec4<f32>(final_color, 1.0);
}
