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
    let mut snap = SimSnapshot::default();
    // Camera pose aimed at the origin from behind-and-above. Quat derived so -Z body
    // points roughly toward the origin and +Y stays up.
    let cam_pos = Vec3::new(8.0, 6.0, 16.0);
    snap.camera.position = cam_pos;
    let forward = (Vec3::ZERO - cam_pos).normalize();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let basis = glam::Mat3::from_cols(right, up, -forward);
    snap.camera.orientation = Quat::from_mat3(&basis);
    snap.camera.fov_y_radians = 60.0_f32.to_radians();
    snap.vehicle.position = Vec3::new(0.0, 2.0, 0.0);
    snap.vehicle.orientation = Quat::IDENTITY;
    snap
}

#[cfg(feature = "hardware-tests")]
fn capture_png(device: &wgpu::Device, queue: &wgpu::Queue, renderer: &Renderer) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot-color"),
        size: wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
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

    let snap = build_snapshot();

    // Buffer-aligned row stride. wgpu requires bytes_per_row to be 256-byte aligned for
    // texture-to-buffer copies.
    let bpp: u32 = 4;
    let unpadded = SIZE.0 * bpp;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buf_size = u64::from(padded) * u64::from(SIZE.1);

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
        SIZE,
        &snap,
        &snap,
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
                rows_per_image: Some(SIZE.1),
            },
        },
        wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::Maintain::Wait);

    let mut pixels = Vec::with_capacity((SIZE.0 * SIZE.1 * 4) as usize);
    {
        let data = buf.slice(..).get_mapped_range();
        for y in 0..SIZE.1 {
            let start = (y * padded) as usize;
            let end = start + (SIZE.0 * bpp) as usize;
            pixels.extend_from_slice(&data[start..end]);
        }
    }
    buf.unmap();

    let img = image::RgbaImage::from_raw(SIZE.0, SIZE.1, pixels).expect("raw -> image");
    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageFormat::Png,
    )
    .expect("png encode");
    png_bytes
}

#[cfg(feature = "hardware-tests")]
#[test]
fn scene_minimal_snapshot() {
    let (device, queue) = init_wgpu();

    let terrain =
        CpuMesh::terrain_patch(24, 40.0, |x, z| ((x * 0.08).sin() + (z * 0.08).cos()) * 1.6);
    let cube = CpuMesh::cube(1.0, [1.0, 0.82, 0.28, 1.0]);

    let renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        SIZE,
        &terrain,
        TERRAIN_SHADING,
        &cube,
        VEHICLE_SHADING,
        2.0,
    );
    let png = capture_png(&device, &queue, &renderer);
    insta::assert_binary_snapshot!("scene_minimal.png", png);
}
