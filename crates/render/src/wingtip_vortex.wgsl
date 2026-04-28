// Wingtip vortex ribbon shader (Phase 3.2x.W ribbon revision, 2026-04-27).
//
// Renders a parametric ribbon helix per wingtip — generated CPU-side
// from a ring buffer of spine waypoints. Each vertex carries its own
// world position and alpha (CPU computes both from the parametric
// spiral functions: position = spine + radius(age) × (cos(θ(age)) ·
// perp_a + sin(θ(age)) · perp_b); alpha fades with age).
//
// Camera-relative rendering: subtract `camera.view_pos` before
// applying `view_proj` (matches `perspective_view_proj` convention
// in renderer.rs).

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct VsIn {
    @location(0) world_pos: vec3<f32>,
    @location(1) alpha: f32,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) alpha: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let camera_relative = in.world_pos - camera.view_pos.xyz;
    var out: VsOut;
    out.clip_pos = camera.view_proj * vec4<f32>(camera_relative, 1.0);
    out.alpha = in.alpha;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Translucent white. Per-vertex alpha drives the age fade across
    // the ribbon — old end of ribbon (far from aircraft) is faded out
    // smoothly. Constant 0.45 base reads as wispy condensation rather
    // than opaque cloth.
    return vec4<f32>(0.96, 0.97, 0.99, in.alpha * 0.45);
}
