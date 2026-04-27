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

struct SceneUniform {
    sun_dir:   vec4<f32>,
    sun_color: vec4<f32>,
    ambient:   vec4<f32>,
    fog:       vec4<f32>,
    time:      vec4<f32>,
    // weather.x = precip_intensity (mm/hr)
    // weather.y = snow_fraction in [0, 1] — 0 rain, 1 snow
    weather:   vec4<f32>,
};
@group(1) @binding(0) var<uniform> scene: SceneUniform;

// Rain particle shape (snow_fraction = 0): tall thin streak, ~6 cm
// wide × 60 cm tall. Reads as motion-blur raindrop.
const RAIN_HALF_WIDTH_M: f32 = 0.03;
const RAIN_HALF_HEIGHT_M: f32 = 0.30;

// Snow particle shape (snow_fraction = 1): small round flake, equal
// width + height. Smaller than rain because individual flakes drift
// rather than streak.
const SNOW_HALF_WIDTH_M: f32 = 0.06;
const SNOW_HALF_HEIGHT_M: f32 = 0.06;

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
    // Phase E.3: lerp particle dimensions between rain streak (tall +
    // thin) and snow flake (small + round) by snow_fraction.
    let snow = scene.weather.y;
    let half_w = mix(RAIN_HALF_WIDTH_M, SNOW_HALF_WIDTH_M, snow);
    let half_h = mix(RAIN_HALF_HEIGHT_M, SNOW_HALF_HEIGHT_M, snow);
    let offset = right * (in.quad.x * half_w)
               + world_up * (in.quad.y * half_h);
    let world = in.world_pos + offset;
    let camera_relative = world - camera.view_pos.xyz;
    var out: VsOut;
    out.clip_pos = camera.view_proj * vec4<f32>(camera_relative, 1.0);
    out.local_uv = in.quad;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Phase E.3: lerp shape between vertical streak (rain) and round
    // flake (snow) by `scene.weather.y`. Both shapes fade from a bright
    // center toward transparent edges; rain is anisotropic in y, snow
    // is isotropic.
    let snow = scene.weather.y;
    let ax = abs(in.local_uv.x);
    let ay = abs(in.local_uv.y);
    // Rain streak: thin in x, long in y.
    let rain_x = 1.0 - smoothstep(0.0, 0.7, ax);
    let rain_y = 1.0 - smoothstep(0.0, 0.95, ay);
    let rain_intensity = rain_x * rain_y;
    // Snow flake: round radial falloff.
    let radial = sqrt(in.local_uv.x * in.local_uv.x + in.local_uv.y * in.local_uv.y);
    let snow_intensity = 1.0 - smoothstep(0.0, 0.85, radial);
    let intensity = mix(rain_intensity, snow_intensity, snow);
    // Snow reads slightly brighter — opaque white flakes. Rain stays
    // slightly translucent so the cloud color reads through.
    let alpha = intensity * mix(0.55, 0.80, snow);
    let color = vec3<f32>(
        mix(0.88, 0.96, snow),
        mix(0.91, 0.98, snow),
        mix(0.95, 1.00, snow),
    );
    return vec4<f32>(color, alpha);
}
