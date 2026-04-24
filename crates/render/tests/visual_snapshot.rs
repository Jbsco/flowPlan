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
use flowplan_render::{InstanceGpuData, Renderer, SceneParams, ShadingParams};
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
#[allow(dead_code)] // only used under `hardware-tests`
fn capture_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut Renderer,
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
        device,
        queue,
        &mut encoder,
        &target_view,
        size,
        snap,
        snap,
        1.0,
        CLEAR,
        None, // no orientation widget in snapshot renders
        None, // no attitude widget
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
    let mut renderer = Renderer::new(
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
    let png = capture_png(&device, &queue, &mut renderer, &snap, BEAVER_SIZE);
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

    let mut renderer = Renderer::new(
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
    let png = capture_png(&device, &queue, &mut renderer, &snap, EXTENDED_SIZE);
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

    let mut renderer = Renderer::new(
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
    let png = capture_png(&device, &queue, &mut renderer, &snap, SIZE);
    insta::assert_binary_snapshot!("scene_minimal.png", png);
}

/// Top-down water render. Dedicated view for iterating on wave patterns, noise
/// displacement, foam, and color tinting without the cockpit-view perspective
/// complicating pattern inspection. Camera at 100 m altitude looking straight
/// down at a ~60 m × 60 m patch of water, centered at world origin.
///
/// Renders 3 PNGs at three different `t` values so visible time-variation of
/// the wave field can be verified from one test run: `t=0`, `t=10`, `t=30`.
/// Anything reading as "static" across frames would indicate a bug in the
/// time-dependent shader code (wave phase, amp modulation, noise drift all
/// depend on `t`).
///
/// Output files written to `target/water_topdown/`:
///   - `water_topdown_t0.png`
///   - `water_topdown_t10.png`
///   - `water_topdown_t30.png`
///
/// Skips insta (no regression tracking) — this is an iterative debug tool, not
/// a regression guard. For regression, use the cockpit-view `beaver_preview` /
/// `scene_extended` snapshots.
#[cfg(feature = "hardware-tests")]
#[test]
fn water_topdown() {
    let (device, queue) = init_wgpu();

    // Tiny hidden terrain and vehicle — the renderer requires them for init but
    // the top-down view only shows water.
    let terrain = CpuMesh::terrain_patch(8, 2000.0, |_x, _z| -200.0); // way below water
    let water = CpuMesh::water_grid(512, 60.0, WATER_COLOR);
    let cube = CpuMesh::cube(0.1, [0.0, 0.0, 0.0, 0.0]); // invisible placeholder

    #[allow(clippy::items_after_statements)]
    const TOPDOWN_SIZE: (u32, u32) = (512, 512);

    let mut renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        TOPDOWN_SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &cube,
        VEHICLE_SHADING,
    );

    // Top-down camera: 100 m altitude at world origin, looking straight down.
    // Camera-local +Y points toward world -Z (so "up in screen" = world -Z).
    let cam_orient = Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
    let mut snap = SimSnapshot::default();
    snap.camera.position = Vec3::new(0.0, 100.0, 0.0);
    snap.camera.orientation = cam_orient;
    snap.camera.fov_y_radians = 35.0_f32.to_radians(); // 60m × 60m window at 100m alt
                                                       // Vehicle parked far below the water plane (invisible in top-down view).
    snap.vehicle.position = Vec3::new(0.0, -1000.0, 0.0);
    snap.vehicle.orientation = Quat::IDENTITY;

    // Ensure the output directory exists.
    let out_dir = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/water_topdown"
    ));
    std::fs::create_dir_all(&out_dir).expect("mkdir target/water_topdown");

    // Render at three time points to verify time-variation is real.
    for (label, t) in &[("t0", 0.0_f32), ("t10", 10.0), ("t30", 30.0)] {
        let params = SceneParams {
            time_seconds: *t,
            ..SceneParams::default()
        };
        renderer.set_scene(&queue, &params);
        let png = capture_png(&device, &queue, &mut renderer, &snap, TOPDOWN_SIZE);
        let path = out_dir.join(format!("water_topdown_{label}.png"));
        std::fs::write(&path, &png).expect("write water_topdown PNG");
        println!("wrote {}", path.display());
    }
}

/// Wide top-down water render — zoomed out ~3× from `water_topdown` (180 m × 180 m
/// window vs 60 m × 60 m). Same camera-angle, same three time points. Reveals:
///   - Large-scale wave patterns (swell interference, long rollers)
///   - Whether the macro noise octave (150 m wavelength) produces visible
///     bumps/rollers at this scale
///   - Whether pattern "ovals" change character at longer viewing distance
///     (suggesting it's about the wave scales themselves, not close-up artifacts)
///
/// Uses altitude 300 m + same 35° FOV → covers ~190 m at the water plane. Water
/// mesh is expanded to 300 m total extent (150 m half) so the full visible
/// area is covered by tessellated water even with the mesh-follow-camera
/// snapping (which is handled by the Renderer).
///
/// Output files written to `target/water_topdown/`:
///   - `water_topdown_wide_t0.png`
///   - `water_topdown_wide_t10.png`
///   - `water_topdown_wide_t30.png`
#[cfg(feature = "hardware-tests")]
#[test]
fn water_topdown_wide() {
    let (device, queue) = init_wgpu();

    let terrain = CpuMesh::terrain_patch(8, 2000.0, |_x, _z| -200.0);
    let water = CpuMesh::water_grid(512, 150.0, WATER_COLOR);
    let cube = CpuMesh::cube(0.1, [0.0, 0.0, 0.0, 0.0]);

    #[allow(clippy::items_after_statements)]
    const TOPDOWN_SIZE: (u32, u32) = (512, 512);

    let mut renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        TOPDOWN_SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &cube,
        VEHICLE_SHADING,
    );

    // Top-down camera, altitude 300 m → ~190 m wide frame at 35° FOV.
    let cam_orient = Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
    let mut snap = SimSnapshot::default();
    snap.camera.position = Vec3::new(0.0, 300.0, 0.0);
    snap.camera.orientation = cam_orient;
    snap.camera.fov_y_radians = 35.0_f32.to_radians();
    snap.vehicle.position = Vec3::new(0.0, -1000.0, 0.0);
    snap.vehicle.orientation = Quat::IDENTITY;

    let out_dir = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/water_topdown"
    ));
    std::fs::create_dir_all(&out_dir).expect("mkdir target/water_topdown");

    for (label, t) in &[("t0", 0.0_f32), ("t10", 10.0), ("t30", 30.0)] {
        let params = SceneParams {
            time_seconds: *t,
            ..SceneParams::default()
        };
        renderer.set_scene(&queue, &params);
        let png = capture_png(&device, &queue, &mut renderer, &snap, TOPDOWN_SIZE);
        let path = out_dir.join(format!("water_topdown_wide_{label}.png"));
        std::fs::write(&path, &png).expect("write water_topdown_wide PNG");
        println!("wrote {}", path.display());
    }
}

/// Phase 3.6.3c.5c.1 verification shot. Renders a hand-built cluster of
/// box primitives via the instance pipeline — decoupled from the full
/// world bake + streaming stack so the render path can be verified in
/// isolation.
///
/// Scene: flat-ish terrain at world origin + 6 "house" boxes arranged
/// in a ring around (0, ground, 0), various sizes / rotations / colors.
/// Camera 15 m above the ground, 25 m back, pitched slightly down.
///
/// Output: `target/instance_preview/instance_preview.png`. First run
/// produces a raw PNG to inspect manually. No insta snapshot — this is
/// an iterative debug tool.
#[cfg(feature = "hardware-tests")]
#[test]
fn instance_preview() {
    let (device, queue) = init_wgpu();

    // Gentle terrain centered at origin, ground ≈ 0 m.
    let terrain = CpuMesh::terrain_patch(64, 200.0, |x, z| {
        ((x * 0.02).sin() + (z * 0.02).cos()) * 1.0
    });
    // Water far below so it doesn't occlude the houses (but still present
    // as scene fills).
    let water = CpuMesh::water_grid(16, 200.0, WATER_COLOR);
    // Placeholder vehicle — hidden via `set_vehicle_visible(false)` below.
    let placeholder = CpuMesh::cube(0.1, [0.0, 0.0, 0.0, 0.0]);

    #[allow(clippy::items_after_statements)]
    const INSTANCE_SIZE: (u32, u32) = (800, 600);

    let mut renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        INSTANCE_SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &placeholder,
        VEHICLE_SHADING,
    );
    renderer.set_vehicle_visible(false);

    // 6 house boxes arranged in a ring at radius 12 m from origin.
    // Ground y ≈ 0, houses sit on it (center y = half-height). Varied
    // sizes and rotations + distinct colors to verify per-instance data
    // is actually reaching the shader.
    #[allow(clippy::many_single_char_names)]
    let instances: Vec<InstanceGpuData> = (0..6_u32)
        .map(|i| {
            let angle = (i as f32) * core::f32::consts::TAU / 6.0;
            let rx = 12.0 * angle.cos();
            let rz = 12.0 * angle.sin();
            let width = 6.0 + (i as f32 % 3.0);
            let height = 4.0 + (i as f32 % 2.0);
            let depth = 8.0 + ((i as f32) * 1.5) % 3.0;
            // Weathered-wood palette varied per house.
            let red = 130 + ((i * 20) % 60) as u8;
            let green = 85 + ((i * 15) % 40) as u8;
            let blue = 70 + ((i * 10) % 30) as u8;
            InstanceGpuData::from_fields(
                [rx, height * 0.5, rz],
                angle * 0.7,
                [width, height, depth],
                [red, green, blue, 255],
            )
        })
        .collect();
    renderer.upload_instances(&device, &queue, &instances);

    // Camera: 15 m above ground, 25 m south, pitched -20° at the cluster.
    let snap = build_camera_snapshot(
        Vec3::new(0.0, 15.0, 25.0),
        Vec3::new(0.0, 3.0, 0.0),
        60.0,
        Vec3::new(0.0, -1000.0, 0.0), // vehicle hidden anyway
    );

    let png = capture_png(&device, &queue, &mut renderer, &snap, INSTANCE_SIZE);
    let out_dir = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/instance_preview"
    ));
    std::fs::create_dir_all(&out_dir).expect("mkdir target/instance_preview");
    let path = out_dir.join("instance_preview.png");
    std::fs::write(&path, &png).expect("write instance_preview.png");
    println!("wrote {} ({} instances)", path.display(), instances.len());
}

/// Camera-relative-rendering regression shot (Phase 3.6.3c.5d).
///
/// Same scene geometry as `instance_preview` but offset by 2.5 Mm in x
/// and z (world scale where the 5000 km toroidal world is worst-case
/// for f32 precision). Before the camera-relative fix, this renders as
/// a flickering mess; after, it should look identical to the origin
/// version modulo static ~0.3m quantization error. Diff the two PNGs
/// to verify.
///
/// Output: `target/instance_preview/instance_preview_far.png`.
#[cfg(feature = "hardware-tests")]
#[test]
fn instance_preview_far() {
    let (device, queue) = init_wgpu();

    // World-scale offset — cells wrap at 5 Mm, so 2.5 Mm is the
    // worst-case f32-precision position in the toroidal world.
    let off_x = 2_500_000.0_f32;
    let off_z = 1_800_000.0_f32;

    let terrain = CpuMesh::terrain_patch_at(64, 200.0, (off_x, off_z), |x, z| {
        let lx = x - off_x;
        let lz = z - off_z;
        ((lx * 0.02).sin() + (lz * 0.02).cos()) * 1.0
    });
    let water = CpuMesh::water_grid(16, 200.0, WATER_COLOR);
    let placeholder = CpuMesh::cube(0.1, [0.0, 0.0, 0.0, 0.0]);

    #[allow(clippy::items_after_statements)]
    const INSTANCE_SIZE: (u32, u32) = (800, 600);

    let mut renderer = Renderer::new(
        &device,
        &queue,
        FORMAT,
        INSTANCE_SIZE,
        &terrain,
        TERRAIN_SHADING,
        &water,
        WATER_SHADING,
        &placeholder,
        VEHICLE_SHADING,
    );
    renderer.set_vehicle_visible(false);

    // Same 6-box ring as `instance_preview`, translated to the far
    // coordinate.
    #[allow(clippy::many_single_char_names)]
    let instances: Vec<InstanceGpuData> = (0..6_u32)
        .map(|i| {
            let angle = (i as f32) * core::f32::consts::TAU / 6.0;
            let rx = 12.0 * angle.cos();
            let rz = 12.0 * angle.sin();
            let width = 6.0 + (i as f32 % 3.0);
            let height = 4.0 + (i as f32 % 2.0);
            let depth = 8.0 + ((i as f32) * 1.5) % 3.0;
            let red = 130 + ((i * 20) % 60) as u8;
            let green = 85 + ((i * 15) % 40) as u8;
            let blue = 70 + ((i * 10) % 30) as u8;
            InstanceGpuData::from_fields(
                [off_x + rx, height * 0.5, off_z + rz],
                angle * 0.7,
                [width, height, depth],
                [red, green, blue, 255],
            )
        })
        .collect();
    renderer.upload_instances(&device, &queue, &instances);

    // Camera pose mirrors `instance_preview` but at the far offset.
    let snap = build_camera_snapshot(
        Vec3::new(off_x, 15.0, off_z + 25.0),
        Vec3::new(off_x, 3.0, off_z),
        60.0,
        Vec3::new(0.0, -1000.0, 0.0),
    );

    let png = capture_png(&device, &queue, &mut renderer, &snap, INSTANCE_SIZE);
    let out_dir = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/instance_preview"
    ));
    std::fs::create_dir_all(&out_dir).expect("mkdir target/instance_preview");
    let path = out_dir.join("instance_preview_far.png");
    std::fs::write(&path, &png).expect("write instance_preview_far.png");
    println!(
        "wrote {} ({} instances at offset x={}, z={})",
        path.display(),
        instances.len(),
        off_x,
        off_z
    );
}
