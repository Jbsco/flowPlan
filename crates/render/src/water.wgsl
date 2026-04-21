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
// Sum(steepness * k * amplitude) should stay < 1.0 to avoid crest loops.
//
// MUST match `WAVES` in `crates/world/src/water.rs` for sim/render consistency.
// Tuning rationale (gentle lake / float-friendly): see the world-side comment.

const WAVE_COUNT: u32 = 7u;
const SWELL_COUNT: u32 = 2u; // first 2 waves are long swells; rest is chop

fn wave_dir_and_params(i: u32) -> vec4<f32> {
    // Separate function avoids the array<vec4> const init that some wgsl backends
    // rejected at Phase 2.2 test. Explicit branching is fast even at 7 waves.
    if (i == 0u) { return vec4<f32>( 1.00,  0.30, 50.0, 0.040); } // long swell A
    if (i == 1u) { return vec4<f32>( 0.70,  1.00, 35.0, 0.030); } // long swell B
    if (i == 2u) { return vec4<f32>( 0.40,  1.00,  6.0, 0.045); } // chop
    if (i == 3u) { return vec4<f32>(-0.80,  0.50,  4.0, 0.045); } // chop
    if (i == 4u) { return vec4<f32>( 1.00, -0.40,  3.0, 0.040); } // chop
    if (i == 5u) { return vec4<f32>(-0.30,  1.00,  2.0, 0.035); } // fine chop
    return             vec4<f32>( 0.50,  0.85,  1.4, 0.025);     // fine chop
}

// 2D value noise: hash at integer grid corners, smooth bilinear interpolation. Gives
// spatially coherent blobs at a chosen scale, unlike `fract(sin(dot))` which produces
// per-pixel static. Used for foam patch masking.
fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn value_noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn wave_speed_and_steep(i: u32) -> vec2<f32> {
    if (i == 0u) { return vec2<f32>(3.5, 0.40); } // long swell A — slow + slight trochoidal
    if (i == 1u) { return vec2<f32>(2.8, 0.40); } // long swell B
    if (i == 2u) { return vec2<f32>(2.0, 0.35); } // chop
    if (i == 3u) { return vec2<f32>(1.7, 0.35); } // chop
    if (i == 4u) { return vec2<f32>(1.4, 0.30); } // chop
    if (i == 5u) { return vec2<f32>(1.1, 0.25); } // fine chop
    return             vec2<f32>(0.85, 0.20);    // fine chop — minimal trochoidal
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
    // Per-component wave height ratios in [-1, +1]. Index 0 = long swell, 1 = chop
    // (sum of wave 1 + wave 2). Lets the fragment shader read foam + color
    // independently per scale: foam appears at LOCAL maxima of either component
    // (chop tip OR swell crest), and color tinting is depth-cued per component
    // so wavelets visually "extrude" out of swell troughs even when the absolute
    // height is still negative.
    @location(3)       wave_t:      vec2<f32>,
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

    // Per-component height accumulators (for foam + color masking in fragment).
    // wave 0 = long swell; waves 1+2 = chop.
    var swell_y: f32 = 0.0;
    var swell_amp_max: f32 = 0.0;
    var chop_y: f32 = 0.0;
    var chop_amp_max: f32 = 0.0;

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

        // Split height per component class. First SWELL_COUNT waves are swells;
        // remaining are chop. Lets fragment shader weight foam + color tinting
        // independently per scale.
        if (i < SWELL_COUNT) {
            swell_y = swell_y + amp * s;
            swell_amp_max = swell_amp_max + amp;
        } else {
            chop_y = chop_y + amp * s;
            chop_amp_max = chop_amp_max + amp;
        }
    }

    let displaced = world0 + offset;
    let n = normalize(n_acc);

    var out: VsOut;
    out.clip_pos = camera.view_proj * vec4<f32>(displaced, 1.0);
    out.world_pos = displaced;
    out.world_normal = n;
    out.vert_color = in.color * model.tint;
    out.wave_t = vec2<f32>(
        swell_y / max(swell_amp_max, 0.001),
        chop_y / max(chop_amp_max, 0.001),
    );
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

    // Multi-scale water shading inspired by stylized open-water games (Sea of
    // Thieves' water as visual reference, not direct port). Three blended
    // techniques:
    //
    //   1. Slope-driven foam — foam follows wave FACES (steep regions about to
    //      break) rather than narrow crest lines. Produces broad foam patches
    //      that move with the wave shape rather than streaks at peaks.
    //   2. Per-component height awareness — `wave_t.x` (swell) and `wave_t.y`
    //      (chop) drive color tinting independently so wavelets visually
    //      "extrude" from swell troughs even when absolute height is still
    //      negative. Multi-scale feel.
    //   3. Subsurface-scattering glow — chop crests near sun get a translucent
    //      warm glow as if light passed through the wave; reads as the curling
    //      glow on the back side of small waves.
    let swell_t = in.wave_t.x;
    let chop_t = in.wave_t.y;

    // Slope = wave-face steepness, derived from the normal's deviation from
    // straight-up. Flat water → 0; steep wave face → larger. Drives foam over
    // a broad region rather than just at the peak.
    let slope = clamp(1.0 - n.y, 0.0, 1.0);

    // Foam contributions: slope (broad), chop near-peak (sharp tips), swell
    // crest (subtle on long rollers).
    let foam_slope = smoothstep(0.06, 0.22, slope);
    let foam_chop_peak = smoothstep(0.55, 0.95, chop_t);
    let foam_swell_peak = smoothstep(0.40, 0.90, swell_t);
    let foam_intensity = clamp(
        foam_slope * 0.7 + foam_chop_peak * 0.5 + foam_swell_peak * 0.25,
        0.0,
        1.0,
    );

    let noise_large = value_noise2(in.world_pos.xz * 0.40);   // ~2.5 m patch
    let noise_detail = value_noise2(in.world_pos.xz * 1.20);  // ~0.8 m detail
    let foam_field = noise_large * 0.55 + noise_detail * 0.45;
    let foam_mask = smoothstep(0.25, 0.60, foam_field);
    let foam_color = vec3<f32>(0.94, 0.96, 0.98);

    // Depth-cued tinting: greener-darker at swell troughs, bluer at chop crests.
    // Two-layer mix so wavelets are visually distinct from the swell rocking.
    let swell_depth = smoothstep(-1.0, 0.5, swell_t);   // 0 at swell trough → 1 at mean+
    let chop_lift = smoothstep(-0.4, 0.8, chop_t);      // 0 at chop trough → 1 at near-peak
    let trough_tint = vec3<f32>(0.55, 0.85, 0.62);      // saturated dark green for deep look
    let crest_tint = vec3<f32>(1.10, 1.08, 1.05);       // brighter, slight cool→neutral push
    let depth_tinted = in.vert_color.rgb * mix(
        mix(trough_tint, vec3<f32>(1.0, 1.0, 1.0), swell_depth),
        crest_tint,
        chop_lift * 0.55,
    );

    // Subsurface-scattering proxy: when looking roughly toward the sun across a
    // chop crest, the wave appears to "glow" translucent (light passing through
    // the thin water at the crest). Adds warm tint that scales with chop_lift +
    // sun-back-lighting alignment.
    let view_to_pixel_h = normalize(in.world_pos - camera.view_pos.xyz);
    let sun_back_align = clamp(dot(view_to_pixel_h, sun_dir), 0.0, 1.0);
    let sss_glow = chop_lift * sun_back_align * 0.35;
    let sss_color = vec3<f32>(0.95, 1.00, 0.85);

    let base_lit = depth_tinted * diffuse * scene.sun_color.rgb
        + rim
        + scene.sun_color.rgb * spec
        + sss_color * sss_glow;
    let lit = mix(base_lit, foam_color, foam_intensity * foam_mask * 0.90);

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
