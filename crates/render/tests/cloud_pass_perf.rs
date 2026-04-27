//! Cloud-pass GPU perf-budget test (Phase 3.6.3c.6 / 3.2x B-perf).
//!
//! Tier-3 hardware-gated test (`hardware-tests` feature) that times
//! the full render frame including the volumetric cloud pass via
//! wgpu timestamp queries. Asserts the median measured frame time
//! stays under a documented budget so cloud-pass cost regressions are
//! caught at test time rather than playtest.
//!
//! # Run
//!
//! ```text
//! cargo test -p flowplan-render --features hardware-tests --test cloud_pass_perf -- --nocapture
//! ```
//!
//! The `--nocapture` flag surfaces the printed frame-time statistics
//! so you can monitor cost trends across iterations even when the
//! assertion passes.
//!
//! # Budget tuning
//!
//! The budget below (`TOTAL_FRAME_BUDGET_MS`) is set generously to
//! pass on a range of dev hardware while still catching catastrophic
//! regressions. Per user-pinned guidance (2026-04-26): **playtesting
//! signals trump the gate** — when in-game shows a regression that
//! the gate missed, tighten the budget. When the gate is too aggressive
//! and blocks legitimate work, document why and relax it.
//!
//! # Coverage limits
//!
//! Frame-level (not pass-level) timing — catches regressions in the
//! cloud pass + any other pass. Per-pass isolation is a future
//! refinement. The structural cost-bound test in
//! `volumetric_wgsl_parses.rs::volumetric_wgsl_cost_constants_within_budget`
//! catches constant-bump regressions before code reaches GPU.
//!
//! # Hardware variance
//!
//! Frame time varies 2-5× across GPU tiers. The budget below is set
//! for the lower-end of acceptable hardware (think GTX 1060 / RX 580
//! per DESIGN.md tier table). Reference-tier hardware (RTX 3060 /
//! RX 6600) should land 2-3× under budget.

// Test-file relaxations per CODING_RULES § Testing Requirements.
// `dead_code` / `unused_imports` apply only when `hardware-tests` is
// disabled; the gated test below keeps the dev-deps linked either way.
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

use flowplan_assets::CpuMesh;
use flowplan_render::{GpuWeatherCell, Renderer, ShadingParams, MAX_WEATHER_CELLS};
use flowplan_sim::SimSnapshot;
use glam::{Quat, Vec3};

// Other workspace + dev-deps belong elsewhere; pin as unused here so
// the test crate doesn't generate dead-extern warnings whether the
// hardware-tests feature is on or off.
use bytemuck as _;
use flowplan_world as _;
use image as _;
use insta as _;
use naga as _;
#[cfg(feature = "shader-hot-reload")]
use notify as _;
use pollster as _;
use wgpu as _;

// 1080p — locked as the perf-gate basis (per user direction
// 2026-04-26). Most desktop dev machines run 1080p; gate measurements
// here are directly comparable to in-game framerates at typical play
// resolution. Cost scales ~linearly with pixel count → for 4K, multiply
// by 4×; for 720p, divide by 2.25×.
const SIZE: (u32, u32) = (1920, 1080);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.52,
    g: 0.71,
    b: 0.88,
    a: 1.0,
};
const TERRAIN_SHADING: ShadingParams = ShadingParams::new(4.0, 0.5, 0.08, 4.0);
const VEHICLE_SHADING: ShadingParams = ShadingParams::new(3.0, 0.35, 0.35, 3.0);
const WATER_SHADING: ShadingParams = ShadingParams::new(3.0, 0.55, 0.12, 3.5);
const WATER_COLOR: [f32; 4] = [0.18, 0.38, 0.58, 1.0];

const FRAMES_TO_MEASURE: usize = 30;
const WARMUP_FRAMES: usize = 3;

/// Frame-time budget (ms) for the minimal-scene + cloud test at 1080p.
///
/// Per WEATHER.md the cloud-pass-specific budget is **≤ 0.6 ms** at
/// reference tier (resolution-independent). This test renders cloud +
/// minimal scene (flat terrain, single-cube vehicle, no streaming)
/// → frame cost is dominated by the cloud pass.
///
/// **Target: 144 Hz at 1080p** (per user direction 2026-04-26). 144 Hz
/// = 6.94 ms per frame. Cloud must stay small enough that the rest of
/// the frame (terrain streaming, water, vehicle, instances, ambient,
/// outline, widgets) fits in the remaining budget.
///
/// 4.5 ms accommodates the CM-6 boundary-blending refactor
/// (find_nearest_two + per-cell altitude pre-filter add ~1 ms over the
/// single-cell version, even when cell 2 is a sentinel and the blend
/// doesn't engage). At 4.5 ms cloud takes ~65% of a 144 Hz frame —
/// tight but doable; leaves 2.4 ms for everything else (terrain
/// streaming, water, vehicle, instances, ambient, outline, widgets).
/// The visual win from CM-6 (no hard cell seams; "clouds vanish when
/// leaving cell" symptom resolved) outweighs the cost.
///
/// **Hard limit at 4.5 ms** — FFT water (Phase 3.5b) is expected to
/// add ~1-2 ms to the rest of the frame. Any further cloud-pass cost
/// growth needs to come from a clear visual win that justifies eating
/// into the water/atmosphere budget. Future cloud-pass cost recovery
/// options (queued post-CM-N tuning):
/// 1. **GPU 3D noise textures** (AAA volumetric pattern, RDR2 / Horizon).
///    Bake Worley/Perlin into a tileable 3D texture; sample via
///    texture fetch. Saves 1-2 ms. Same baked data on CPU side
///    preserves bit-identity discipline.
/// 2. **BVH / spatial grid for nearest-cell lookup**. Replaces O(N)
///    scan with O(1). Saves ~0.6 ms.
/// 3. **Half-resolution cloud pass + TAA upsample**. Standard AAA
///    technique; cuts per-pixel cost ~4×. Big architecture change.
///
/// History:
/// - B-phase: 3.0 ms target (single-cell hardcoded)
/// - C2-polish: 3.5 ms (tessellation; per-step nearest-cell scan)
/// - CM-6: 4.5 ms (boundary blending; per-step nearest-two scan +
///   per-cell altitude pre-filter; Option 2 one-sqrt linearization
///   shipped but the actual bottleneck is the scan, not the sqrt)
///
/// **Tune as playtest reveals regressions** (per user-pinned guidance
/// 2026-04-26). Specifically:
/// - When in-game shows perf regression that this gate missed → tighten
///   the budget here so future regressions of similar magnitude fail.
/// - When this gate fails on legitimate work → measure on reference
///   hardware first; only relax if ref-tier hardware ALSO shows the
///   regression. Document the rationale with each adjustment.
const TOTAL_FRAME_BUDGET_MS: f64 = 4.5;

#[cfg(feature = "hardware-tests")]
fn init_wgpu_with_timestamps() -> Option<(wgpu::Device, wgpu::Queue, f32)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        // HighPerformance picks the discrete GPU when available.
        // Perf measurements should reflect the gameplay tier.
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;

    let supported = adapter.features();
    let timestamp_features =
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if !supported.contains(timestamp_features) {
        eprintln!(
            "cloud_pass_perf: adapter does not support timestamp queries \
             ({supported:?}). Skipping perf gate on this hardware."
        );
        return None;
    }

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("cloud-perf-device"),
            required_features: timestamp_features,
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .expect("device request");
    let period_ns = queue.get_timestamp_period();
    Some((device, queue, period_ns))
}

#[cfg(feature = "hardware-tests")]
fn build_cloud_view_snapshot() -> SimSnapshot {
    // Camera positioned to put the hardcoded test cell (Cumulus at
    // world origin XZ, altitude 1500-2500m) prominently in view.
    // Distance + altitude chosen so the cloud occupies a meaningful
    // fraction of the screen — exercising the per-pixel raymarch
    // cost on real workload.
    let cam_pos = Vec3::new(0.0, 800.0, 4000.0);
    let look_target = Vec3::new(0.0, 2000.0, 0.0);
    let mut snap = SimSnapshot::default();
    snap.camera.position = cam_pos;
    let forward = (look_target - cam_pos).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let basis = glam::Mat3::from_cols(right, up, -forward);
    snap.camera.orientation = Quat::from_mat3(&basis);
    snap.camera.fov_y_radians = 60.0_f32.to_radians();
    snap.vehicle.position = Vec3::ZERO;
    snap.vehicle.orientation = Quat::IDENTITY;
    snap
}

#[cfg(feature = "hardware-tests")]
#[test]
fn cloud_pass_within_perf_budget() {
    let Some((device, queue, ts_period_ns)) = init_wgpu_with_timestamps() else {
        // No GPU with timestamp support — skip without failing.
        return;
    };

    // Minimal scene: flat terrain + flat water + cube vehicle. The
    // cloud pass cost is what we're measuring; other passes contribute
    // a small constant baseline.
    let terrain = CpuMesh::terrain_patch(16, 100.0, |_, _| 0.0);
    let water = CpuMesh::water_grid(16, 200.0, WATER_COLOR);
    let vehicle = CpuMesh::cube(2.0, [0.7, 0.7, 0.8, 1.0]);

    let mut renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &vehicle,
        VEHICLE_SHADING,
    );

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cloud-perf-target"),
        size: wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // 2 timestamps × FRAMES_TO_MEASURE so we can resolve all frames in
    // one buffer rather than per-frame readback. Halves the GPU↔CPU
    // synchronization overhead vs single-frame mapping.
    let total_queries = (FRAMES_TO_MEASURE * 2) as u32;
    let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("cloud-perf-queries"),
        ty: wgpu::QueryType::Timestamp,
        count: total_queries,
    });
    let resolve_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud-perf-resolve"),
        size: u64::from(total_queries) * 8, // 8 bytes per timestamp
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud-perf-readback"),
        size: u64::from(total_queries) * 8,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let snap = build_cloud_view_snapshot();

    // Populate the volumetric cell buffer with one Cumulus test cell at
    // world origin (matches the camera's look target). Empty slots use
    // the sentinel pattern (seed_pos = 1e9) so the shader's
    // `find_nearest_active_cell` distance check naturally excludes
    // them per the tessellation-discipline contract.
    let empty_slot = GpuWeatherCell {
        seed_pos: [1.0e9, 1.0e9],
        base_altitude_m: 0.0,
        top_altitude_m: 0.0,
        density: 0.0,
        storm_phase: 0.0,
        cell_type: 0,
        coverage_strength: 0.0,
    };
    let mut test_cells = [empty_slot; MAX_WEATHER_CELLS];
    test_cells[0] = GpuWeatherCell {
        seed_pos: [0.0, 0.0],
        base_altitude_m: 1500.0,
        top_altitude_m: 2500.0,
        density: 0.6,
        storm_phase: 0.0,
        cell_type: 1, // Cumulus
        // Mid-strength margin so the perf test exercises the
        // smoothstep-modulated cloud_factor path (Phase 4 of the
        // continuous-margin refactor) — not the trivial
        // coverage=1 path that bypasses modulation.
        coverage_strength: 0.7,
    };
    renderer.update_weather_cells(&queue, &test_cells);

    // Warmup — first frames include shader compilation, pipeline
    // caching, etc. Discard them.
    for _ in 0..WARMUP_FRAMES {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cloud-perf-warmup"),
        });
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &target_view,
            SIZE,
            &snap,
            &snap,
            1.0,
            CLEAR,
            None,
            None,
        );
        queue.submit(Some(encoder.finish()));
    }
    device.poll(wgpu::Maintain::Wait);

    // Measurement loop. Each iteration writes start + end timestamps
    // around the renderer.render() call so we capture the entire
    // frame's GPU time including the cloud pass.
    for frame in 0..FRAMES_TO_MEASURE {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cloud-perf-frame"),
        });
        let q_start = (frame * 2) as u32;
        let q_end = q_start + 1;
        encoder.write_timestamp(&query_set, q_start);
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &target_view,
            SIZE,
            &snap,
            &snap,
            1.0,
            CLEAR,
            None,
            None,
        );
        encoder.write_timestamp(&query_set, q_end);
        queue.submit(Some(encoder.finish()));
    }

    // Resolve all queries to the GPU buffer, copy to readback,
    // map + read.
    let mut resolve_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cloud-perf-resolve"),
    });
    resolve_encoder.resolve_query_set(&query_set, 0..total_queries, &resolve_buf, 0);
    resolve_encoder.copy_buffer_to_buffer(
        &resolve_buf,
        0,
        &readback_buf,
        0,
        u64::from(total_queries) * 8,
    );
    queue.submit(Some(resolve_encoder.finish()));
    readback_buf
        .slice(..)
        .map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);

    let mut times_ns: Vec<u64> = Vec::with_capacity(FRAMES_TO_MEASURE);
    {
        let data = readback_buf.slice(..).get_mapped_range();
        let timestamps: &[u64] = bytemuck::cast_slice(&data);
        for frame in 0..FRAMES_TO_MEASURE {
            let start = timestamps[frame * 2];
            let end = timestamps[frame * 2 + 1];
            let delta_ticks = end.saturating_sub(start);
            let delta_ns = (delta_ticks as f64) * f64::from(ts_period_ns);
            times_ns.push(delta_ns as u64);
        }
    }
    readback_buf.unmap();

    times_ns.sort_unstable();
    let median_ns = times_ns[times_ns.len() / 2];
    let min_ns = *times_ns.first().unwrap();
    let max_ns = *times_ns.last().unwrap();
    let median_ms = median_ns as f64 / 1e6;
    let min_ms = min_ns as f64 / 1e6;
    let max_ms = max_ns as f64 / 1e6;

    println!(
        "cloud_pass_perf: {} frames @ {}x{} | median = {:.3} ms ({:.0} fps) \
         | min = {:.3} ms | max = {:.3} ms | budget = {:.1} ms",
        FRAMES_TO_MEASURE,
        SIZE.0,
        SIZE.1,
        median_ms,
        1000.0 / median_ms,
        min_ms,
        max_ms,
        TOTAL_FRAME_BUDGET_MS
    );

    assert!(
        median_ms <= TOTAL_FRAME_BUDGET_MS,
        "Median frame time {median_ms:.3} ms exceeds budget {TOTAL_FRAME_BUDGET_MS:.1} ms. \
         Likely cloud-pass cost regression. Investigate via in-engine \
         FPS check + the structural perf test in volumetric_wgsl_parses.rs."
    );
}
