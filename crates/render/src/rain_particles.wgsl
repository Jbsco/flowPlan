// Rain particle billboard pass (Post-CM-10 Phase E.4 world-space
// particle pool, 2026-04-27). Each particle is rendered as a small
// world-up-aligned quad: vertical raindrop streak that always faces
// roughly toward the camera. Drops are anchored to world coordinates
// (positions managed CPU-side) so motion parallax + rotation
// independence work correctly.
//
// Pipeline shape:
// - Vertex 0..6 forms a quad (two triangles, no index buffer).
// - Per-instance attribute: world_pos (vec3).
// - Vertex shader builds the quad in world space, oriented with world-Y
//   up and camera-perpendicular right axis.
// - Fragment shader writes a soft elliptical streak.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

// Streak dimensions: 6 cm wide × 60 cm tall. Real raindrops are
// of course much smaller; what we render is the motion-blur streak.
// Tuned 2026-04-27: initial 20 cm × 1.5 m read too chunky for ambient
// rain — looked like falling logs more than streaks.
const STREAK_HALF_WIDTH_M: f32 = 0.03;
const STREAK_HALF_HEIGHT_M: f32 = 0.30;

struct VsIn {
    // Per-vertex quad corner offset, in [-1, +1]². Drives the local
    // billboard space.
    @location(0) quad: vec2<f32>,
    // Per-instance world position.
    @location(1) world_pos: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local_uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // World-up-aligned billboard. **Camera-relative rendering**: the
    // engine's `view_proj` matrix is built with translation = 0 (see
    // `perspective_view_proj` in renderer.rs); world positions must be
    // shifted by subtracting `camera.view_pos` BEFORE applying the
    // matrix. Discovered the hard way 2026-04-27 — drops were
    // projecting to clip coords way off-screen because we were using
    // raw world positions directly.
    let world_up = vec3<f32>(0.0, 1.0, 0.0);
    let to_cam = camera.view_pos.xyz - in.world_pos;
    var right = cross(world_up, to_cam);
    let right_len_sq = dot(right, right);
    if (right_len_sq < 0.0001) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    } else {
        right = right * inverseSqrt(right_len_sq);
    }
    let offset = right * (in.quad.x * STREAK_HALF_WIDTH_M)
               + world_up * (in.quad.y * STREAK_HALF_HEIGHT_M);
    let world = in.world_pos + offset;
    let camera_relative = world - camera.view_pos.xyz;
    var out: VsOut;
    out.clip_pos = camera.view_proj * vec4<f32>(camera_relative, 1.0);
    out.local_uv = in.quad;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Soft vertical streak: bright center, fading horizontally fast and
    // vertically gradually. Captures the "raindrop motion-blur streak"
    // appearance.
    let ax = abs(in.local_uv.x);
    let ay = abs(in.local_uv.y);
    let x_shape = 1.0 - smoothstep(0.0, 0.7, ax);
    let y_shape = 1.0 - smoothstep(0.0, 0.95, ay);
    let intensity = x_shape * y_shape;
    let alpha = intensity * 0.55;
    return vec4<f32>(0.88, 0.91, 0.95, alpha);
}
