//! Phase 2 Milestone 2.1 visual snapshot harness.
//!
//! Tier-3 test per [`TEST_PLAN.md § Test Tiers`](../../../TEST_PLAN.md#test-tiers): requires
//! a wgpu-capable adapter and is gated behind the `hardware-tests` feature. Renders a
//! deterministic scene (seeded camera + cube pose, no sim tick) to an offscreen texture,
//! reads it back, encodes as PNG, and byte-compares via `insta::assert_binary_snapshot!`.
//!
//! Tolerance is byte-exact, which is acceptable as long as reference images are captured
//! on the reference hardware. A perceptual-diff tolerance (SSIM) becomes appropriate once
//! the test matrix spans more GPUs; see the note in `TEST_PLAN.md § Visual Snapshot Tests`.
//!
//! Run:
//!   cargo test -p flowplan-render --features hardware-tests --test visual_snapshot
//! Accept new snapshots:
//!   cargo insta accept

// Test-file relaxations per CODING_RULES § Testing Requirements. `expect` / `unwrap` in
// tests are idiomatic. Doc-markdown is relaxed because the module doc includes literal
// cargo invocations. Integer casts at the wgpu/image boundary are bounded by construction.
// `dead_code` / `unused_imports` apply only when `hardware-tests` is disabled; the gated
// items keep the dev-deps linked either way.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    dead_code,
    unused_imports
)]

use flowplan_assets::CpuMesh;
use flowplan_render::{Renderer, ShadingParams};
use flowplan_sim::SimSnapshot;
use glam::{Quat, Vec3};

// Dev-dep markers. When `hardware-tests` is off the bodies below are cfg'd out and the
// integration-test binary would otherwise flag these as unused crate dependencies.
// `bytemuck` / `notify` are inherited from the render crate's regular deps; mark them
// so the test target doesn't trip the lint either.
use bytemuck as _;
use flowplan_world as _;
use image as _;
use insta as _;
#[cfg(feature = "shader-hot-reload")]
use notify as _;
use pollster as _;
use wgpu as _;

const SIZE: (u32, u32) = (320, 240);
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

#[cfg(feature = "hardware-tests")]
fn init_wgpu() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no wgpu adapter available for hardware test");
    pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("snapshot-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .expect("device request")
}

#[cfg(feature = "hardware-tests")]
fn build_snapshot() -> SimSnapshot {
    // Camera above lowland, looking toward the mountain range.
    build_camera_snapshot(
        Vec3::new(150.0, 220.0, 320.0),
        Vec3::new(0.0, 60.0, 0.0),
        60.0,
        Vec3::new(40.0, 30.0, -30.0),
    )
}

#[cfg(feature = "hardware-tests")]
fn build_camera_snapshot(
    cam_pos: Vec3,
    look_target: Vec3,
    fov_deg: f32,
    vehicle_pos: Vec3,
) -> SimSnapshot {
    let mut snap = SimSnapshot::default();
    snap.camera.position = cam_pos;
    let forward = (look_target - cam_pos).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let basis = glam::Mat3::from_cols(right, up, -forward);
    snap.camera.orientation = Quat::from_mat3(&basis);
    snap.camera.fov_y_radians = fov_deg.to_radians();
    snap.vehicle.position = vehicle_pos;
    snap.vehicle.orientation = Quat::IDENTITY;
    snap
}

#[cfg(feature = "hardware-tests")]
fn capture_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &Renderer,
    snap: &SimSnapshot,
    size: (u32, u32),
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot-color"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // Buffer-aligned row stride. wgpu requires bytes_per_row to be 256-byte aligned for
    // texture-to-buffer copies.
    let bpp: u32 = 4;
    let unpadded = size.0 * bpp;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buf_size = u64::from(padded) * u64::from(size.1);

    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("snapshot-readback"),
        size: buf_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("snapshot-encoder"),
    });
    renderer.render(
        queue,
        &mut encoder,
        &target_view,
        size,
        snap,
        snap,
        1.0,
        CLEAR,
    );
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &buf,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size.1),
            },
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);

    let mut pixels = Vec::with_capacity((size.0 * size.1 * 4) as usize);
    {
        let data = buf.slice(..).get_mapped_range();
        for y in 0..size.1 {
            let start = (y * padded) as usize;
            let end = start + (size.0 * bpp) as usize;
            pixels.extend_from_slice(&data[start..end]);
        }
    }
    buf.unmap();

    let img = image::RgbaImage::from_raw(size.0, size.1, pixels).expect("raw -> image");
    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageFormat::Png,
    )
    .expect("png encode");
    png_bytes
}

#[cfg(feature = "hardware-tests")]
fn build_beaver_snapshot(float_offset: f32) -> SimSnapshot {
    let mut snap = SimSnapshot::default();
    // Pull the camera back and up from a representative 10 m aircraft pose.
    let cam_pos = Vec3::new(18.0, 9.0, 22.0);
    snap.camera.position = cam_pos;
    let forward = (Vec3::ZERO - cam_pos).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let basis = glam::Mat3::from_cols(right, up, -forward);
    snap.camera.orientation = Quat::from_mat3(&basis);
    snap.camera.fov_y_radians = 55.0_f32.to_radians();
    // CG-centered mesh: raise position by the float offset so the floats sit at y=0
    // (water level) rather than dipping through it.
    snap.vehicle.position = Vec3::new(0.0, float_offset, 0.0);
    snap.vehicle.orientation = Quat::IDENTITY;
    snap
}

/// Renders the committed Beaver glTF to an insta snapshot so we can iterate on the
/// Blender export visually. First run produces a `.new` baseline to inspect; accept
/// only when the mesh displays correctly.
#[cfg(feature = "hardware-tests")]
#[test]
fn beaver_preview() {
    let (device, queue) = init_wgpu();

    // Gently varying terrain so the preview shows beach / field / forest tiers
    // alongside the aircraft without overwhelming the silhouette. Peaks rise to
    // ~4 m, so the rock tier stays out of this shot (the scene_minimal test covers
    // the full range).
    let terrain = CpuMesh::terrain_patch(32, 60.0, |x, z| {
        ((x * 0.05).sin() + (z * 0.05).cos()) * 1.5 + ((x * 0.015).sin() * (z * 0.015).cos()) * 1.5
    });
    let glb_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/aircraft/northlander.glb"
    );
    let glb_bytes = std::fs::read(glb_path).expect("read northlander.glb");
    let beaver = flowplan_assets::load_mesh(&glb_bytes).expect("parse northlander.glb");
    // No load-time shift: the Blender origin is authored at the CG. The preview
    // lifts vehicle.position by the float offset so floats touch the water surface.
    let beaver_float_offset = (-beaver.bbox_min_y()).max(0.0);

    #[allow(clippy::items_after_statements)]
    const BEAVER_SIZE: (u32, u32) = (512, 384);
    let water = CpuMesh::water_grid(24, 200.0, WATER_COLOR);
    let renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        BEAVER_SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &beaver,
        VEHICLE_SHADING,
    );
    let snap = build_beaver_snapshot(beaver_float_offset);
    let png = capture_png(&device, &queue, &renderer, &snap, BEAVER_SIZE);
    insta::assert_binary_snapshot!("beaver_preview.png", png);
}

#[cfg(feature = "hardware-tests")]
#[test]
fn scene_extended() {
    use flowplan_world::{HeightSampler, Terrain};

    let (device, queue) = init_wgpu();

    // Use the actual game terrain so the snapshot reflects the in-game view: full
    // Phase 2.2.4 octave mix, beach flattening, mountain-scale amplitudes. Seed picked
    // for a frame with a mix of water, lowland, and high peaks.
    let game_terrain = Terrain::from_seed(0xBEAC_1234_5678_9ABC_u64);
    let terrain = CpuMesh::terrain_patch(128, 800.0, |x, z| game_terrain.height(x, z));
    let water = CpuMesh::water_grid(48, 900.0, WATER_COLOR);
    let glb_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/aircraft/northlander.glb"
    );
    let glb_bytes = std::fs::read(glb_path).expect("read northlander.glb");
    let beaver = flowplan_assets::load_mesh(&glb_bytes).expect("parse northlander.glb");
    let beaver_float_offset = (-beaver.bbox_min_y()).max(0.0);

    #[allow(clippy::items_after_statements)]
    const EXTENDED_SIZE: (u32, u32) = (1024, 640);

    let renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        EXTENDED_SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &beaver,
        VEHICLE_SHADING,
    );

    // Camera pose: low over the water, wide FOV, looking toward the mountains so the
    // plane in foreground, water mid-ground, rising terrain beyond. Vehicle position
    // raised to float on water surface (beaver_float_offset above y=0).
    let snap = build_camera_snapshot(
        Vec3::new(-60.0, 25.0, 180.0),
        Vec3::new(80.0, 80.0, -200.0),
        62.0,
        Vec3::new(-20.0, beaver_float_offset, 140.0),
    );
    let png = capture_png(&device, &queue, &renderer, &snap, EXTENDED_SIZE);
    insta::assert_binary_snapshot!("scene_extended.png", png);
}

#[cfg(feature = "hardware-tests")]
#[test]
fn scene_minimal_snapshot() {
    let (device, queue) = init_wgpu();

    // Mountain-scale terrain per the Phase 2.2.4 tuning: amplitudes span sand up
    // through snow. Large half-extent so camera can see peaks, valleys, and water.
    let terrain = CpuMesh::terrain_patch(64, 400.0, |x, z| {
        ((x * 0.008).sin() + (z * 0.008).cos()) * 160.0
            + ((x * 0.025).sin() * (z * 0.025).cos()) * 60.0
            + ((x * 0.08).cos() + (z * 0.08).sin()) * 15.0
    });
    // Cube built at 2 m directly now that the Renderer renders meshes at authored scale.
    let cube = CpuMesh::cube(2.0, [1.0, 0.82, 0.28, 1.0]);
    let water = CpuMesh::water_grid(16, 80.0, WATER_COLOR);

    let renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &cube,
        VEHICLE_SHADING,
    );
    let snap = build_snapshot();
    let png = capture_png(&device, &queue, &renderer, &snap, SIZE);
    insta::assert_binary_snapshot!("scene_minimal.png", png);
}
