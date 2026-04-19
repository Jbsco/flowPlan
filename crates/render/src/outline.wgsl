// flowPlan Phase 2 outline pass (Milestone 2.1).
//
// Full-screen triangle (drawn with 3 vertices, no vertex buffer) samples the scene
// depth texture, runs a 3x3 sobel, and emits black with alpha proportional to edge
// strength. Alpha-blended on top of the scene pass. ROADMAP § 2.1: "screen-space sobel
// on depth+normals"; normals are deferred to a later sub-step once an MRT normal
// target is added. Depth alone catches silhouettes and most creases for Phase 1 scenes.

@group(0) @binding(0) var depth_tex: texture_depth_2d;

struct OutlineParams {
    // x = viewport width (px), y = viewport height (px),
    // z = edge threshold (post-projection depth delta), w = outline alpha scalar.
    viewport_and_tunables: vec4<f32>,
};
@group(0) @binding(1) var<uniform> outline: OutlineParams;

@vertex
fn vs_outline(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {
    // Oversized full-screen triangle. NDC mapping:
    //   idx 0 -> (-1, -1)
    //   idx 1 -> ( 3, -1)
    //   idx 2 -> (-1,  3)
    // The excess beyond [-1, 1] is clipped; interior is seamless.
    let x_ndc = f32(i32(idx) & 1) * 4.0 - 1.0;
    let y_ndc = f32(i32(idx) >> 1) * 4.0 - 1.0;
    return vec4<f32>(x_ndc, y_ndc, 0.0, 1.0);
}

fn sample_depth(coord: vec2<i32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(depth_tex));
    let c = clamp(coord, vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    return textureLoad(depth_tex, c, 0);
}

@fragment
fn fs_outline(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let c = vec2<i32>(frag.xy);
    // 3x3 sobel on depth. Horizontal and vertical gradients separately.
    let tl = sample_depth(c + vec2<i32>(-1, -1));
    let t  = sample_depth(c + vec2<i32>( 0, -1));
    let tr = sample_depth(c + vec2<i32>( 1, -1));
    let l  = sample_depth(c + vec2<i32>(-1,  0));
    let r  = sample_depth(c + vec2<i32>( 1,  0));
    let bl = sample_depth(c + vec2<i32>(-1,  1));
    let b  = sample_depth(c + vec2<i32>( 0,  1));
    let br = sample_depth(c + vec2<i32>( 1,  1));

    let gx = (tr + 2.0 * r + br) - (tl + 2.0 * l + bl);
    let gy = (bl + 2.0 * b + br) - (tl + 2.0 * t + tr);
    let edge = sqrt(gx * gx + gy * gy);

    let threshold = outline.viewport_and_tunables.z;
    let strength = outline.viewport_and_tunables.w;
    // Soft step above threshold so outlines fade rather than aliasing at the boundary.
    let alpha = clamp((edge - threshold) * 2000.0, 0.0, 1.0) * strength;
    return vec4<f32>(0.0, 0.0, 0.0, alpha);
}
