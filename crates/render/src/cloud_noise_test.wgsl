// Compute shader for the CPU-GPU bit-identity gate (Phase 3.6.3c.6 /
// 3.2x B4). Samples worley_3d + cloud_perlin_3d at N input points and
// writes results to a readback buffer. The test
// `cloud_noise_bit_identity_cpu_gpu` runs the same math on the CPU
// side and asserts bit-equality.
//
// Composed with cloud_noise.wgsl at shader-module creation time:
// `format!("{}\n{}", CLOUD_NOISE_WGSL_SOURCE, CLOUD_NOISE_TEST_WGSL_SOURCE)`.
//
// Per `docs/WEATHER.md` § Worley noise primitive → CPU-GPU bit-identity:
// failure here means the WGSL math has diverged from the Rust math —
// fix the divergence before shipping anything that consumes either.

struct TestInput {
    pos: vec3<f32>,
    _pad: f32,
}

struct TestOutput {
    worley_f1: f32,
    worley_f2: f32,
    worley_blended: f32,
    perlin: f32,
}

@group(0) @binding(0) var<storage, read> noise_inputs: array<TestInput>;
@group(0) @binding(1) var<storage, read_write> noise_outputs: array<TestOutput>;

// Test parameters — must match the Rust-side constants in
// `cloud_noise_bit_identity.rs`. Any change here requires a matching
// CPU update or the test correctly fails.
const TEST_SEED_HI: u32 = 0x112210F4u;
const TEST_SEED_LO: u32 = 0x7DE98115u;
const TEST_WORLEY_OCTAVE: u32 = 240u;
const TEST_PERLIN_OCTAVE: u32 = 241u;
const TEST_WORLEY_SPACING: f32 = 2000.0;
const TEST_PERLIN_SPACING: f32 = 600.0;
const TEST_WORLEY_K: u32 = 1u;

@compute @workgroup_size(64)
fn cs_cloud_noise_test(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if (idx >= arrayLength(&noise_inputs)) {
        return;
    }
    let p = noise_inputs[idx].pos;
    let w = worley_3d(
        TEST_SEED_HI, TEST_SEED_LO,
        TEST_WORLEY_OCTAVE,
        p.x, p.y, p.z,
        TEST_WORLEY_SPACING, TEST_WORLEY_K,
    );
    let perlin = cloud_perlin_3d(
        TEST_SEED_HI, TEST_SEED_LO,
        TEST_PERLIN_OCTAVE,
        p.x, p.y, p.z,
        TEST_PERLIN_SPACING,
    );
    var out: TestOutput;
    out.worley_f1 = w.f1;
    out.worley_f2 = w.f2;
    out.worley_blended = w.f_blended;
    out.perlin = perlin;
    noise_outputs[idx] = out;
}
