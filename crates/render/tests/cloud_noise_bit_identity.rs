//! CPU-GPU agreement gate for cloud-noise primitives (Phase 3.6.3c.6 /
//! 3.2x B4).
//!
//! Tier-3 hardware-gated test (`hardware-tests` feature). Boots a wgpu
//! compute pipeline, samples `worley_3d` + `cloud_perlin_3d` at N
//! reference points on the GPU, runs the same math on the CPU side
//! via `flowplan_world::noise::*`, asserts max absolute difference
//! stays within tolerance.
//!
//! # Why approx-equality, not bit-identity
//!
//! `docs/WEATHER.md` § Worley noise primitive originally specified
//! **bit-identity** — same constants, same hash chain, same noise
//! formula on both sides. After implementation:
//!
//! - The HASH outputs match exactly (verified empirically — when a
//!   hashed feature point's coordinates are identical CPU and GPU,
//!   downstream math agrees to ~1 ULP).
//! - The ARITHMETIC chains diverge at 1-100 ULP because modern GPU
//!   drivers fuse multiply-add operations into FMA instructions
//!   (single rounding instead of two). Naga / wgpu does not expose a
//!   way to disable this.
//! - When FMA-induced 1-ULP differences land at the d² comparison
//!   that picks the closest Worley feature, the cascade through `f1`,
//!   `f2`, `f_blended` can grow to 100s of ULPs (~0.1-0.3 % relative
//!   error at typical magnitudes).
//!
//! Strict bit-identity is therefore a Phase 6.1 cross-platform
//! determinism-hardening goal that requires either driver-side flags
//! we don't currently have or a soft-float WGSL noise implementation.
//! For v1, the practical gate is **approximate-equality with tight
//! tolerance** — catches algorithm bugs (wrong constants, broken
//! hash, wrong loop bounds) while tolerating FMA-level driver math.
//!
//! # Tolerance
//!
//! Per-component absolute difference must stay below
//! [`ABS_TOLERANCE`] (5e-3). Real algorithm bugs produce 0.1-1.0
//! magnitude divergence; FMA noise tops out at 5e-4. The 5e-3
//! tolerance has 10× margin over observed FMA noise.
//!
//! # Run
//!
//! ```text
//! cargo test -p flowplan-render --features hardware-tests --test cloud_noise_bit_identity -- --nocapture
//! ```
//!
//! Or `./flowplan test-cloud-bit-identity`.

// Test-file relaxations per CODING_RULES § Testing Requirements.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::too_many_lines,
    dead_code,
    unused_imports
)]

use flowplan_render::{CLOUD_NOISE_TEST_WGSL_SOURCE, CLOUD_NOISE_WGSL_SOURCE};
use flowplan_world::noise::{cloud_perlin_3d, worley_3d};
#[cfg(feature = "hardware-tests")]
use wgpu::util::DeviceExt;

// Other workspace + dev-deps belong elsewhere; pin as unused.
use flowplan_assets as _;
use flowplan_sim as _;
use glam as _;
use image as _;
use insta as _;
use naga as _;
#[cfg(feature = "shader-hot-reload")]
use notify as _;
use pollster as _;
use wgpu as _;

// Test parameters — match the WGSL `cloud_noise_test.wgsl` constants.
// Derived from the WORLD_SEED constant (1234567890123456789u64). Any
// change to either side requires updating both.
const TEST_SEED: u64 = 0x1122_10F4_7DE9_8115; // = 1_234_567_890_123_456_789
const TEST_WORLEY_OCTAVE: u32 = 240;
const TEST_PERLIN_OCTAVE: u32 = 241;
const TEST_WORLEY_SPACING: f32 = 2000.0;
const TEST_PERLIN_SPACING: f32 = 600.0;
const TEST_WORLEY_K: u32 = 1;

const NUM_TEST_POINTS: usize = 256;

/// Per-component absolute difference tolerance. See module doc for
/// rationale. Algorithm bugs produce O(0.1) divergence; FMA noise
/// tops out around 5e-4 in observed measurements.
const ABS_TOLERANCE: f32 = 5e-3;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TestInput {
    pos: [f32; 3],
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct TestOutput {
    worley_f1: f32,
    worley_f2: f32,
    worley_blended: f32,
    perlin: f32,
}

#[cfg(feature = "hardware-tests")]
fn init_wgpu_compute() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("cloud-noise-bit-identity-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .expect("device request");
    Some((device, queue))
}

/// Generate a deterministic spread of test points covering varied
/// positions, altitudes, and wrap edges. Coverage:
///
/// - Origin region (small XZ near 0)
/// - Wrap-edge region (XZ near `WORLD_EXTENT_M`)
/// - Negative coords (forces wrap)
/// - Mixed altitudes (below / inside / above the test cell band)
fn build_test_points() -> Vec<TestInput> {
    let mut out = Vec::with_capacity(NUM_TEST_POINTS);
    // Stratified sampling: 16 X positions × 4 altitudes × 4 Z positions = 256.
    for ix in 0..16 {
        for iy in 0..4 {
            for iz in 0..4 {
                // X spans negative + positive + wrap-edge to exercise
                // wrap_coord at all relevant inputs.
                let x = match ix {
                    0..=3 => (ix as f32 - 2.0) * 250.0,  // tight near origin
                    4..=7 => (ix as f32 - 3.5) * 1500.0, // mid-range
                    8..=11 => -1_000_000.0 + (ix as f32 - 10.0) * 250.0, // negative wrap
                    _ => 4_999_900.0 + (ix as f32 - 14.0) * 50.0, // near WORLD_EXTENT_M
                };
                // Y altitudes covering below / inside / above test cell band.
                let y = match iy {
                    0 => -100.0,
                    1 => 1750.0, // mid-cloud
                    2 => 2200.0, // upper-cloud
                    _ => 3500.0, // above
                };
                let z = (iz as f32 - 1.5) * 800.0;
                out.push(TestInput {
                    pos: [x, y, z],
                    _pad: 0.0,
                });
            }
        }
    }
    debug_assert_eq!(out.len(), NUM_TEST_POINTS);
    out
}

fn cpu_compute_outputs(inputs: &[TestInput]) -> Vec<TestOutput> {
    inputs
        .iter()
        .map(|p| {
            let w = worley_3d(
                TEST_SEED,
                TEST_WORLEY_OCTAVE,
                p.pos[0],
                p.pos[1],
                p.pos[2],
                TEST_WORLEY_SPACING,
                TEST_WORLEY_K,
            );
            let perlin = cloud_perlin_3d(
                TEST_SEED,
                TEST_PERLIN_OCTAVE,
                p.pos[0],
                p.pos[1],
                p.pos[2],
                TEST_PERLIN_SPACING,
            );
            TestOutput {
                worley_f1: w.f1,
                worley_f2: w.f2,
                worley_blended: w.f_blended,
                perlin,
            }
        })
        .collect()
}

#[cfg(feature = "hardware-tests")]
fn gpu_compute_outputs(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    inputs: &[TestInput],
) -> Vec<TestOutput> {
    let composed_source = format!("{CLOUD_NOISE_WGSL_SOURCE}\n{CLOUD_NOISE_TEST_WGSL_SOURCE}");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cloud-noise-test-shader"),
        source: wgpu::ShaderSource::Wgsl(composed_source.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cloud-noise-test-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cloud-noise-test-pipeline-layout"),
        bind_group_layouts: &[&bgl],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cloud-noise-test-pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: "cs_cloud_noise_test",
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let input_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cloud-noise-test-inputs"),
        contents: bytemuck::cast_slice(inputs),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output_size = (inputs.len() * core::mem::size_of::<TestOutput>()) as u64;
    let output_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud-noise-test-outputs"),
        size: output_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud-noise-test-readback"),
        size: output_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cloud-noise-test-bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output_buf.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cloud-noise-test-encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cloud-noise-test-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg, &[]);
        // Workgroup size in WGSL is 64; dispatch enough groups for all points.
        let workgroups = inputs.len().div_ceil(64) as u32;
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output_buf, 0, &readback_buf, 0, output_size);
    queue.submit(Some(encoder.finish()));

    readback_buf
        .slice(..)
        .map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);

    let outputs: Vec<TestOutput> = {
        let data = readback_buf.slice(..).get_mapped_range();
        bytemuck::cast_slice::<u8, TestOutput>(&data).to_vec()
    };
    readback_buf.unmap();
    outputs
}

#[cfg(feature = "hardware-tests")]
#[test]
fn cloud_noise_bit_identity_cpu_gpu() {
    let Some((device, queue)) = init_wgpu_compute() else {
        eprintln!("cloud_noise_bit_identity: no GPU adapter — skipping");
        return;
    };

    let inputs = build_test_points();
    let cpu_out = cpu_compute_outputs(&inputs);
    let gpu_out = gpu_compute_outputs(&device, &queue, &inputs);

    assert_eq!(cpu_out.len(), gpu_out.len());

    // Stats accumulators. We report the worst case + the distribution
    // every run so trends are visible even when the assertion passes.
    let mut max_abs_diff = 0.0_f32;
    let mut max_ulp_diff: i64 = 0;
    let mut total_compared: usize = 0;
    let mut failures: Vec<(usize, &str, f32, f32, f32)> = Vec::new();

    for (i, (c, g)) in cpu_out.iter().zip(gpu_out.iter()).enumerate() {
        let pairs = [
            ("worley_f1", c.worley_f1, g.worley_f1),
            ("worley_f2", c.worley_f2, g.worley_f2),
            ("worley_blended", c.worley_blended, g.worley_blended),
            ("perlin", c.perlin, g.perlin),
        ];
        for (name, cpu_val, gpu_val) in pairs {
            total_compared += 1;
            let abs_diff = (cpu_val - gpu_val).abs();
            if abs_diff > max_abs_diff {
                max_abs_diff = abs_diff;
            }
            let ulp = (i64::from(cpu_val.to_bits()) - i64::from(gpu_val.to_bits())).abs();
            if ulp > max_ulp_diff {
                max_ulp_diff = ulp;
            }
            if abs_diff > ABS_TOLERANCE {
                failures.push((i, name, cpu_val, gpu_val, abs_diff));
            }
        }
    }

    println!(
        "cloud_noise_bit_identity: {} samples × 4 components = {} comparisons | \
         max |Δ| = {:.3e}, max |ULP| = {} | tolerance = {:.0e} ({} failures)",
        cpu_out.len(),
        total_compared,
        max_abs_diff,
        max_ulp_diff,
        ABS_TOLERANCE,
        failures.len(),
    );

    if !failures.is_empty() {
        eprintln!(
            "cloud_noise_bit_identity: {} failures (abs_diff > {}):",
            failures.len(),
            ABS_TOLERANCE
        );
        for (i, name, cpu_val, gpu_val, abs_diff) in failures.iter().take(20) {
            let p = inputs[*i].pos;
            eprintln!(
                "  point {} {} pos=({}, {}, {}): CPU = {} | GPU = {} | abs_diff = {:e}",
                i, name, p[0], p[1], p[2], cpu_val, gpu_val, abs_diff,
            );
        }
        if failures.len() > 20 {
            eprintln!("  ... ({} more)", failures.len() - 20);
        }
        panic!(
            "cloud_noise WGSL ↔ Rust agreement broken at {} sample(s). \
             max abs_diff {} exceeds tolerance {}. Likely real algorithm \
             divergence (wrong constants, broken hash, wrong loop bounds) — \
             FMA-level ULP noise is normally well below tolerance. See \
             module doc for tolerance rationale.",
            failures.len(),
            max_abs_diff,
            ABS_TOLERANCE,
        );
    }
}
