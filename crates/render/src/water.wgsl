// flowPlan Phase 2 water pipeline (Milestone 2.2.2: Gerstner waves).
//
// Vertex shader displaces each water-grid vertex by a sum of Gerstner wave components,
// per GPU Gems 2 Ch. 1 (Finch). Three components tuned for calm-to-moderate sea state:
// amplitudes 0.1-0.25 m, wavelengths 7-18 m. Normals are recomputed per-vertex from the
// wave derivatives so cel shading responds to surface slope. Fragment shader mirrors the
// scene cel model + sun-aligned specular spike (sunglint) + fog.
//
// Bind groups match the scene pipeline: @group(0)=camera, @group(1)=model, @group(2)=scene.

struct CameraUniform {
    view_proj:         mat4x4<f32>,
    view_pos:          vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct ModelUniform {
    model:       mat4x4<f32>,
    normal_mat:  mat4x4<f32>,
    tint:        vec4<f32>,
    shading:     vec4<f32>,
};
@group(1) @binding(0) var<uniform> model: ModelUniform;

struct SceneUniform {
    sun_dir:   vec4<f32>,
    sun_color: vec4<f32>,
    ambient:   vec4<f32>,
    fog:       vec4<f32>,
    time:      vec4<f32>,
};
@group(2) @binding(0) var<uniform> scene: SceneUniform;

// Wave definitions. Each vec4 packs (dir.x, dir.z, wavelength_m, amplitude_m).
// Steepness + speed in the companion array as (speed_mps, steepness_Q, _, _).
// Sum(steepness * k * amplitude) should stay < 1.0 to avoid crest loops. With the values
// below: k1*A1*Q1 + k2*A2*Q2 + k3*A3*Q3 ~= 0.06 + 0.04 + 0.03 = 0.13, well under.

const WAVE_COUNT: u32 = 3u;

fn wave_dir_and_params(i: u32) -> vec4<f32> {
    // Separate function avoids the array<vec4> const init that some wgsl backends
    // rejected at Phase 2.2 test. Explicit branching is equally fast at 3 waves.
    if (i == 0u) { return vec4<f32>( 1.00,  0.10, 18.0, 0.25); }
    if (i == 1u) { return vec4<f32>( 0.20,  1.00, 11.0, 0.12); }
    return             vec4<f32>(-0.70,  0.50,  7.0, 0.08);
}

fn wave_speed_and_steep(i: u32) -> vec2<f32> {
    if (i == 0u) { return vec2<f32>(3.2, 0.7); }
    if (i == 1u) { return vec2<f32>(2.0, 0.5); }
    return             vec2<f32>(1.5, 0.4);
}

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

const TAU: f32 = 6.283185307179586;

@vertex
fn vs_water(in: VsIn) -> VsOut {
    let world0 = (model.model * vec4<f32>(in.position, 1.0)).xyz;
    let t = scene.time.x;

    var offset = vec3<f32>(0.0, 0.0, 0.0);
    // Normal accumulator per GPU Gems 2 Ch. 1 simplification:
    //   N = normalize(-sum(Dx * k * A * cos), 1 - sum(Q * k * A * sin), -sum(Dz * k * A * cos))
    var n_acc = vec3<f32>(0.0, 1.0, 0.0);

    for (var i: u32 = 0u; i < WAVE_COUNT; i = i + 1u) {
        let dp = wave_dir_and_params(i);
        let sp = wave_speed_and_steep(i);
        let dir = normalize(dp.xy);
        let wavelength = dp.z;
        let amp = dp.w;
        let speed = sp.x;
        let steep = sp.y;
        let k = TAU / wavelength;
        let omega = k * speed;
        let phase = k * (dir.x * world0.x + dir.y * world0.z) + omega * t;
        let c = cos(phase);
        let s = sin(phase);
        let wa = k * amp;

        offset.x = offset.x + steep * amp * dir.x * c;
        offset.z = offset.z + steep * amp * dir.y * c;
        offset.y = offset.y + amp * s;

        n_acc.x = n_acc.x - dir.x * wa * c;
        n_acc.z = n_acc.z - dir.y * wa * c;
        n_acc.y = n_acc.y - steep * wa * s;
    }

    let displaced = world0 + offset;
    let n = normalize(n_acc);

    var out: VsOut;
    out.clip_pos = camera.view_proj * vec4<f32>(displaced, 1.0);
    out.world_pos = displaced;
    out.world_normal = n;
    out.vert_color = in.color * model.tint;
    return out;
}

@fragment
fn fs_water(in: VsOut) -> @location(0) vec4<f32> {
    let sun_dir = normalize(scene.sun_dir.xyz);
    let n = normalize(in.world_normal);

    let lambert = max(dot(n, sun_dir), 0.0);
    let bands = max(model.shading.x, 1.0);
    let ambient = clamp(model.shading.y, 0.0, 1.0);
    let banded = floor(lambert * bands) / bands;
    let diffuse = mix(ambient, 1.0, banded);

    let view_dir = normalize(camera.view_pos.xyz - in.world_pos);
    let rim_edge = 1.0 - max(dot(view_dir, n), 0.0);
    let rim = pow(rim_edge, max(model.shading.w, 0.01)) * model.shading.z;

    // Specular highlight -- sun reflection on wave crests, gives a "glint" band.
    let view_reflect = reflect(-view_dir, n);
    let spec_align = max(dot(view_reflect, sun_dir), 0.0);
    let spec = pow(spec_align, 64.0) * 0.35;

    // Whitecap foam: wave crests above a threshold get a cream-white tint. Cheap and
    // reads as "water has surface texture" without needing a depth-compare pass for
    // shoreline foam (that's deferred to 2.5 with proper contact physics).
    let crest = smoothstep(0.12, 0.28, in.world_pos.y);
    let foam_color = vec3<f32>(0.94, 0.96, 0.98);

    let base_lit = in.vert_color.rgb * diffuse * scene.sun_color.rgb
        + rim
        + scene.sun_color.rgb * spec;
    let lit = mix(base_lit, foam_color, crest * 0.55);

    // Matching scene fog so water fades at the same horizon as terrain.
    let dist = length(camera.view_pos.xyz - in.world_pos);
    let fog_density = max(scene.fog.a, 0.0);
    let fog_factor = clamp(1.0 - exp(-fog_density * dist), 0.0, 1.0);
    let view_to_pixel = normalize(in.world_pos - camera.view_pos.xyz);
    let sun_align = clamp(dot(view_to_pixel, -sun_dir), 0.0, 1.0);
    let fog_col = mix(scene.fog.rgb, scene.sun_color.rgb, pow(sun_align, 6.0) * 0.5);
    let final_color = mix(lit, fog_col, fog_factor);

    return vec4<f32>(final_color, in.vert_color.a);
}
